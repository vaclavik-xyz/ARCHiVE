//! The `messages` command drives the bundled `imessage-exporter` binary to
//! export iMessage/SMS/RCS conversations (txt, html, pdf). `archive` does not
//! re-implement message decoding — it orchestrates the mature, separately-tested
//! exporter and keeps the agent contract (one JSON object on stdout). The pure
//! pieces (binary discovery, argv, format validation) live here and are
//! unit-tested; the actual spawn is in `run_messages`.

use std::ffi::OsString;
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
