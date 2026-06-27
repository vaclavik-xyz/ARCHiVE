//! Read WhatsApp messages from `ChatStorage.sqlite` and extract attached media
//! from the WhatsApp shared app-group container.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};
use serde::Serialize;

use crate::datetime::cocoa_any_to_iso;
use crate::sqlite_util::table_columns;

/// Backup domain of the WhatsApp shared container (DB and media).
const WA_DOMAIN: &str = "AppDomainGroup-group.net.whatsapp.WhatsApp.shared";
/// Subdirectory (under the export dir) that receives extracted media.
const WA_DIR: &str = "whatsapp_media";

/// One WhatsApp message.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WaMessage {
    /// Chat/contact display name (`ZPARTNERNAME`); empty when unknown.
    pub chat: String,
    /// Sender JID (`ZFROMJID`); empty when from me / unknown.
    pub sender: String,
    /// Whether the message was sent by the backup's owner.
    pub from_me: bool,
    /// Send/receive time as ISO 8601 UTC (Cocoa epoch); empty if unconvertible.
    pub date: String,
    /// Message text (`ZTEXT`); empty for media-only messages.
    pub text: String,
    /// Domain-relative media path; empty when the message has no media.
    pub source_path: String,
    /// Output-relative path to the extracted media (`whatsapp_media/<name>`);
    /// `None` until extraction runs or when the file is absent from the backup.
    pub media_file: Option<String>,
}

impl WaMessage {
    /// Whether the attached media is an image (drives inline display in the HTML).
    pub fn is_image(&self) -> bool {
        let Some(f) = self.media_file.as_ref() else { return false };
        matches!(
            f.rsplit('.').next().map(str::to_ascii_lowercase).as_deref(),
            Some("jpg" | "jpeg" | "png" | "gif" | "heic" | "webp")
        )
    }
}

/// Last path component of a (possibly `/`-containing) name.
fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Map a `ZMEDIALOCALPATH` to its domain-relative path: WhatsApp media lives under
/// `Message/Media/...`, while the column usually stores `Media/...`. Best-effort.
fn to_media_path(local: &str) -> String {
    if local.is_empty() || local.starts_with("Message/") {
        local.to_string()
    } else if local.starts_with("Media/") {
        format!("Message/{local}")
    } else {
        local.to_string()
    }
}

/// Parse all messages, joining the chat name and media path, ordered by date.
/// Schema-tolerant for the optional value columns.
pub fn parse(db_path: &Path) -> rusqlite::Result<Vec<WaMessage>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cols = table_columns(&conn, "ZWAMESSAGE")?;
    let text_sel = if cols.contains("ZTEXT") { "m.ZTEXT" } else { "NULL" };
    let from_me_sel = if cols.contains("ZISFROMME") { "m.ZISFROMME" } else { "NULL" };
    let from_jid_sel = if cols.contains("ZFROMJID") { "m.ZFROMJID" } else { "NULL" };

    let sql = format!(
        "SELECT s.ZPARTNERNAME, {from_jid_sel}, {from_me_sel}, m.ZMESSAGEDATE, {text_sel}, md.ZMEDIALOCALPATH \
         FROM ZWAMESSAGE m \
         LEFT JOIN ZWACHATSESSION s ON s.Z_PK = m.ZCHATSESSION \
         LEFT JOIN ZWAMEDIAITEM md ON md.Z_PK = m.ZMEDIAITEM \
         ORDER BY m.ZMESSAGEDATE"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let chat: Option<String> = row.get(0)?;
        let sender: Option<String> = row.get(1)?;
        let from_me: Option<i64> = row.get(2)?;
        let date: Option<f64> = row.get(3)?;
        let text: Option<String> = row.get(4)?;
        let media: Option<String> = row.get(5)?;
        let from_me = from_me == Some(1);
        Ok(WaMessage {
            chat: chat.unwrap_or_default(),
            // Sender is only meaningful for incoming messages.
            sender: if from_me { String::new() } else { sender.unwrap_or_default() },
            from_me,
            date: date.map(|d| d.round() as i64).and_then(cocoa_any_to_iso).unwrap_or_default(),
            text: text.unwrap_or_default(),
            source_path: to_media_path(&media.unwrap_or_default()),
            media_file: None,
        })
    })?;
    rows.collect()
}

/// Per-run media-extraction outcome, surfaced in the JSON envelope.
pub struct WaSummary {
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Media files written.
    pub extracted: usize,
    /// Media-bearing messages whose file was absent from the backup.
    pub missing: usize,
}

/// Output filename `<n>_<basename>` (1-based index ensures uniqueness).
pub(crate) fn output_name(n: usize, name: &str) -> String {
    format!("{n}_{}", basename(name))
}

/// Fetch each message's media into `<out>/whatsapp_media/`, filling `media_file`
/// in place. Best-effort: a missing file is counted. Only directory creation is
/// fatal. `extracted + missing` counts only media-bearing messages.
pub fn extract_media(
    backup: &archive_core::Backup,
    items: &mut [WaMessage],
    out: &Path,
) -> std::io::Result<WaSummary> {
    let media_dir = out.join(WA_DIR);
    std::fs::create_dir_all(&media_dir)?;

    let mut extracted = 0usize;
    let mut with_media = 0usize;
    for (i, item) in items.iter_mut().enumerate() {
        if item.source_path.is_empty() {
            continue;
        }
        with_media += 1;
        let name = output_name(i + 1, &item.source_path);
        let dest = media_dir.join(&name);
        match backup.fetch(WA_DOMAIN, &item.source_path, &dest) {
            Ok(Some(_)) => {
                item.media_file = Some(format!("{WA_DIR}/{name}"));
                extracted += 1;
            }
            Ok(None) => {}
            Err(why) => eprintln!("whatsapp media {}: fetch failed: {why}", item.source_path),
        }
    }

    Ok(WaSummary { dir: WA_DIR.to_string(), extracted, missing: with_media - extracted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::make_whatsapp;

    #[test]
    fn parses_messages_with_chat_sender_and_media() {
        let dir = std::env::temp_dir().join(format!("be-wa-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("ChatStorage.sqlite");
        let _ = std::fs::remove_file(&db);
        make_whatsapp(&db);

        let msgs = parse(&db).unwrap();
        assert_eq!(msgs.len(), 3);

        let first = &msgs[0];
        assert_eq!(first.chat, "Jana");
        assert!(first.from_me);
        assert_eq!(first.sender, ""); // from me → no sender
        assert_eq!(first.text, "Ahoj");
        assert_eq!(first.date, "2020-01-06T10:40:00+00:00");
        assert_eq!(first.source_path, "");
        assert_eq!(first.media_file, None);

        let media = &msgs[1];
        assert!(!media.from_me);
        assert_eq!(media.sender, "420776452878@s.whatsapp.net");
        assert_eq!(media.text, ""); // media-only
        assert_eq!(media.source_path, "Message/Media/420776452878@s.whatsapp.net/7/d/photo.jpg");

        assert_eq!(msgs[2].text, "Měj se");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn to_media_path_prefixes_message_dir() {
        assert_eq!(to_media_path("Media/x/y.jpg"), "Message/Media/x/y.jpg");
        assert_eq!(to_media_path("Message/Media/a.jpg"), "Message/Media/a.jpg");
        assert_eq!(to_media_path(""), "");
    }

    #[test]
    fn output_name_and_is_image() {
        assert_eq!(output_name(1, "Message/Media/x/photo.jpg"), "1_photo.jpg");
        assert_ne!(output_name(1, "a.jpg"), output_name(2, "a.jpg"));

        let mut m = WaMessage {
            chat: String::new(), sender: String::new(), from_me: false, date: String::new(),
            text: String::new(), source_path: String::new(), media_file: Some("whatsapp_media/1_p.JPG".into()),
        };
        assert!(m.is_image());
        m.media_file = Some("whatsapp_media/1_v.mp4".into());
        assert!(!m.is_image());
        m.media_file = None;
        assert!(!m.is_image());
    }

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_whatsapp_media() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup = archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("ChatStorage.sqlite");
        let Some(db) = backup
            .fetch(WA_DOMAIN, "ChatStorage.sqlite", &db)
            .expect("fetch ChatStorage.sqlite")
        else {
            eprintln!("backup has no WhatsApp store; skipping");
            return;
        };
        let mut items = parse(&db).expect("parse whatsapp");

        let out = scratch.path().join("out");
        let summary = extract_media(&backup, &mut items, &out).expect("extract");
        assert_eq!(summary.dir, "whatsapp_media");
        let with_media = items.iter().filter(|m| !m.source_path.is_empty()).count();
        assert_eq!(summary.extracted + summary.missing, with_media);
        for v in items.iter().filter_map(|m| m.media_file.as_ref()) {
            assert!(out.join(v).is_file(), "linked media should exist: {v}");
        }
    }
}
