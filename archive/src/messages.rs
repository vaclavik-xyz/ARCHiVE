//! The `messages` command drives the bundled `imessage-exporter` binary to
//! export iMessage/SMS/RCS conversations (txt, html, pdf). `archive` does not
//! re-implement message decoding — it orchestrates the mature, separately-tested
//! exporter and keeps the agent contract (one JSON object on stdout). The pure
//! pieces (binary discovery, argv, format validation) live here and are
//! unit-tested; the actual spawn is in `run_messages`.

use std::ffi::OsString;
use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::datetime::cocoa_to_iso;
use crate::sqlite_util::table_columns;

/// The bundled exporter binary, built alongside `archive` in the same workspace.
pub const EXPORTER_TOOL: &str = "imessage-exporter";

/// Environment override for the exporter binary location, for installs where it
/// is neither a sibling of `archive` nor on `PATH`.
pub const EXPORTER_ENV: &str = "ARCHIVE_IMESSAGE_EXPORTER";

/// Canonicalize a `messages` format argument. The exporter accepts only `txt`,
/// `html`, and `pdf` (case-insensitive); anything else returns `None` so the
/// caller emits a usage error instead of letting the child process fail.
pub fn normalize_format(format: &str) -> Option<&'static str> {
    match format.to_ascii_lowercase().as_str() {
        "txt" => Some("txt"),
        "html" => Some("html"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

/// Argv for `imessage-exporter -a iOS -p <backup> -f <format> -o <out> [-x <pw>]
/// [--chrome-path <c>]`. `-a iOS` pins the source as an iOS backup directory (so
/// the password is accepted and the path is treated as a backup root). The
/// password is forwarded only for an encrypted backup; on an unencrypted one it
/// is unnecessary and is kept out of the process table. `--chrome-path` is
/// forwarded only for `pdf` (the exporter rejects it for other formats).
pub fn messages_args(
    backup: &Path,
    out: &Path,
    format: &str,
    encrypted: bool,
    password: Option<&str>,
    chrome_path: Option<&Path>,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![
        "-a".into(),
        "iOS".into(),
        "-p".into(),
        backup.as_os_str().to_owned(),
        "-f".into(),
        format.into(),
        "-o".into(),
        out.as_os_str().to_owned(),
    ];
    if let (true, Some(pw)) = (encrypted, password) {
        args.push("-x".into());
        args.push(pw.into());
    }
    if let (true, Some(c)) = (format == "pdf", chrome_path) {
        args.push("--chrome-path".into());
        args.push(c.as_os_str().to_owned());
    }
    args
}

/// Decide which exporter binary to invoke, given the override env value, the
/// directory holding the running `archive` binary, and a probe for whether a
/// sibling exporter exists there. Order: explicit env override → sibling of
/// `archive` → bare tool name (resolved on `PATH` at spawn). An empty env value
/// is treated as unset.
pub fn resolve_exporter_from(
    env_override: Option<OsString>,
    exe_dir: Option<&Path>,
    sibling_exists: impl Fn(&Path) -> bool,
) -> OsString {
    if let Some(p) = env_override.filter(|p| !p.is_empty()) {
        return p;
    }
    if let Some(dir) = exe_dir {
        let sibling = dir.join(format!("{EXPORTER_TOOL}{}", std::env::consts::EXE_SUFFIX));
        if sibling_exists(&sibling) {
            return sibling.into_os_string();
        }
    }
    OsString::from(EXPORTER_TOOL)
}

/// Resolve the exporter binary from the real environment (see
/// [`resolve_exporter_from`]).
pub fn resolve_exporter() -> OsString {
    let env_override = std::env::var_os(EXPORTER_ENV);
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(Path::parent);
    resolve_exporter_from(env_override, exe_dir, |p| p.exists())
}

/// At-a-glance counts read directly from `Library/SMS/sms.db`. The exporter owns
/// the transcript; this read-only aggregate feeds the per-folder summary so the
/// customer sees totals without us re-implementing message decoding.
pub struct MessageStats {
    pub total: usize,
    pub sent: usize,
    pub received: usize,
    pub imessage: usize,
    pub sms: usize,
    pub with_attachments: usize,
    pub chats: usize,
    pub date_from: String,
    pub date_to: String,
    pub by_year: Vec<(String, usize)>,
}

/// Convert an sms.db `message.date` to a UTC ISO string. Modern iOS stores Apple
/// Cocoa nanoseconds (since 2001-01-01); very old databases store seconds — large
/// magnitudes are scaled down before [`cocoa_to_iso`].
fn message_date_iso(raw: i64) -> Option<String> {
    if raw == 0 {
        return None;
    }
    let secs = if raw.unsigned_abs() > 1_000_000_000_000 {
        raw as f64 / 1_000_000_000.0
    } else {
        raw as f64
    };
    cocoa_to_iso(secs)
}

/// Read message totals/period from an sms.db, tolerating absent optional columns
/// across iOS versions.
pub fn stats(db_path: &Path) -> rusqlite::Result<MessageStats> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "message")?;
    let count_where = |w: &str| -> rusqlite::Result<usize> {
        conn.query_row(&format!("SELECT COUNT(*) FROM message WHERE {w}"), [], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
    };

    let total = conn.query_row("SELECT COUNT(*) FROM message", [], |r| r.get::<_, i64>(0))? as usize;
    let (sent, received) = if cols.contains("is_from_me") {
        (count_where("is_from_me = 1")?, count_where("is_from_me = 0")?)
    } else {
        (0, 0)
    };
    let imessage = if cols.contains("service") { count_where("service = 'iMessage'")? } else { 0 };
    // Everything that is not iMessage (SMS/MMS/RCS); 0 when the column is absent.
    let sms = if cols.contains("service") { total - imessage } else { 0 };
    // Prefer the authoritative join table; fall back to the denormalized cache
    // column when it is absent (older/odd schemas) or the join table is missing.
    let with_attachments = conn
        .query_row("SELECT COUNT(DISTINCT message_id) FROM message_attachment_join", [], |r| r.get::<_, i64>(0))
        .map(|n| n as usize)
        .or_else(|_| {
            if cols.contains("cache_has_attachments") {
                count_where("cache_has_attachments = 1")
            } else {
                Ok(0)
            }
        })
        .unwrap_or(0);
    // Conversations that actually contain messages; fall back to the raw `chat`
    // table (which can include empty/deleted threads) when the join is absent.
    let chats = conn
        .query_row("SELECT COUNT(DISTINCT chat_id) FROM chat_message_join", [], |r| r.get::<_, i64>(0))
        .or_else(|_| conn.query_row("SELECT COUNT(*) FROM chat", [], |r| r.get::<_, i64>(0)))
        .map(|n| n as usize)
        .unwrap_or(0);

    let mut dates: Vec<String> = Vec::new();
    if cols.contains("date") {
        let mut stmt = conn.prepare("SELECT date FROM message WHERE date IS NOT NULL AND date <> 0")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        for raw in rows.flatten() {
            if let Some(iso) = message_date_iso(raw) {
                dates.push(iso);
            }
        }
    }
    let by_year = crate::summary::year_rows(dates.iter().map(String::as_str));
    let (date_from, date_to) =
        crate::summary::iso_range(dates.iter().map(String::as_str)).unwrap_or_default();

    Ok(MessageStats { total, sent, received, imessage, sms, with_attachments, chats, date_from, date_to, by_year })
}

/// Build the customer-facing messages summary from [`stats`].
pub fn summary(s: &MessageStats) -> crate::summary::Summary {
    crate::summary::Summary::new("messages", "Zprávy", "zpráv", s.total)
        .count("Odeslaných", s.sent)
        .count("Přijatých", s.received)
        .count("iMessage", s.imessage)
        .count("SMS/MMS", s.sms)
        .count("S přílohou", s.with_attachments)
        .count("Konverzací", s.chats)
        .period_from(Some((s.date_from.clone(), s.date_to.clone())))
        .breakdown("Po letech", s.by_year.clone())
        .note("Počty vychází přímo z databáze zpráv (Library/SMS/sms.db); RCS se počítá podle služby.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn stats_and_summary_from_sms_db() {
        let dir = std::env::temp_dir().join(format!("be-msg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("sms.db");
        let _ = std::fs::remove_file(&db);
        let conn = Connection::open(&db).unwrap();
        // Apple Cocoa nanoseconds since 2001-01-01 for a 2023-era timestamp.
        let d2023: i64 = 707_788_800 * 1_000_000_000;
        conn.execute_batch(&format!(
            "CREATE TABLE message (ROWID INTEGER PRIMARY KEY, text TEXT, service TEXT, is_from_me INTEGER, date INTEGER, cache_has_attachments INTEGER);
             CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, guid TEXT);
             CREATE TABLE message_attachment_join (message_id INTEGER, attachment_id INTEGER);
             CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
             INSERT INTO message VALUES (1,'ahoj','iMessage',1,{d},0);
             INSERT INTO message VALUES (2,'cau','iMessage',0,{d},1);
             INSERT INTO message VALUES (3,'sms','SMS',0,{d},0);
             INSERT INTO chat VALUES (1,'g1');
             INSERT INTO chat VALUES (2,'g2');
             INSERT INTO message_attachment_join VALUES (2,100);
             INSERT INTO chat_message_join VALUES (1,1),(1,2),(2,3);",
            d = d2023
        ))
        .unwrap();
        drop(conn);

        let s = stats(&db).unwrap();
        assert_eq!(s.total, 3);
        assert_eq!(s.sent, 1);
        assert_eq!(s.received, 2);
        assert_eq!(s.imessage, 2);
        assert_eq!(s.sms, 1);
        assert_eq!(s.with_attachments, 1);
        assert_eq!(s.chats, 2);
        assert!(!s.date_from.is_empty());
        assert_eq!(s.by_year.len(), 1);

        let sum = summary(&s);
        assert_eq!(sum.total, 3);
        assert!(sum.period.is_some());
        assert!(sum.counts.iter().any(|(l, n)| l == "iMessage" && *n == 2));
        std::fs::remove_dir_all(&dir).ok();
    }

    fn as_strings(args: &[OsString]) -> Vec<String> {
        args.iter().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn normalize_format_accepts_exporter_types_case_insensitively() {
        assert_eq!(normalize_format("txt"), Some("txt"));
        assert_eq!(normalize_format("HTML"), Some("html"));
        assert_eq!(normalize_format("Pdf"), Some("pdf"));
        // Formats other archive commands use, but the exporter does not support.
        assert_eq!(normalize_format("csv"), None);
        assert_eq!(normalize_format("json"), None);
        assert_eq!(normalize_format("vcf"), None);
        assert_eq!(normalize_format(""), None);
    }

    #[test]
    fn messages_args_unencrypted_omits_password() {
        let args = as_strings(&messages_args(
            Path::new("/b"),
            Path::new("/out"),
            "html",
            false,
            Some("secret"),
            None,
        ));
        assert_eq!(args, vec!["-a", "iOS", "-p", "/b", "-f", "html", "-o", "/out"]);
        assert!(!args.iter().any(|a| a == "secret"));
    }

    #[test]
    fn messages_args_encrypted_passes_password() {
        let args = as_strings(&messages_args(
            Path::new("/b"),
            Path::new("/out"),
            "pdf",
            true,
            Some("secret"),
            None,
        ));
        assert_eq!(
            args,
            vec!["-a", "iOS", "-p", "/b", "-f", "pdf", "-o", "/out", "-x", "secret"]
        );
    }

    #[test]
    fn messages_args_encrypted_without_password_omits_flag() {
        let args = as_strings(&messages_args(
            Path::new("/b"),
            Path::new("/out"),
            "txt",
            true,
            None,
            None,
        ));
        assert_eq!(args, vec!["-a", "iOS", "-p", "/b", "-f", "txt", "-o", "/out"]);
    }

    #[test]
    fn messages_args_forwards_chrome_path_only_for_pdf() {
        // --chrome-path is forwarded for pdf...
        let pdf = as_strings(&messages_args(
            Path::new("/b"),
            Path::new("/out"),
            "pdf",
            false,
            None,
            Some(Path::new("/opt/chrome")),
        ));
        assert_eq!(
            pdf,
            vec!["-a", "iOS", "-p", "/b", "-f", "pdf", "-o", "/out", "--chrome-path", "/opt/chrome"]
        );
        // ...but not for non-pdf formats (the exporter rejects it there).
        let html = as_strings(&messages_args(
            Path::new("/b"),
            Path::new("/out"),
            "html",
            false,
            None,
            Some(Path::new("/opt/chrome")),
        ));
        assert!(!html.iter().any(|a| a == "--chrome-path"));
    }

    #[test]
    fn resolve_exporter_prefers_env_override() {
        let got = resolve_exporter_from(
            Some(OsString::from("/custom/exporter")),
            Some(Path::new("/usr/bin")),
            |_| true,
        );
        assert_eq!(got, OsString::from("/custom/exporter"));
    }

    #[test]
    fn resolve_exporter_uses_sibling_when_present() {
        let got = resolve_exporter_from(None, Some(Path::new("/opt/bin")), |_| true);
        let expect: PathBuf =
            Path::new("/opt/bin").join(format!("{EXPORTER_TOOL}{}", std::env::consts::EXE_SUFFIX));
        assert_eq!(got, expect.into_os_string());
    }

    #[test]
    fn resolve_exporter_falls_back_to_path_name() {
        let got = resolve_exporter_from(None, Some(Path::new("/opt/bin")), |_| false);
        assert_eq!(got, OsString::from(EXPORTER_TOOL));
        // An empty env override is treated as unset, not as an empty path.
        let got2 = resolve_exporter_from(Some(OsString::new()), None, |_| false);
        assert_eq!(got2, OsString::from(EXPORTER_TOOL));
    }
}
