//! Helpers for creating a backup from a connected iPhone via `libimobiledevice`
//! (`idevicebackup2` / `idevice_id`). The pure pieces (argv, parsing, tool
//! discovery) live here and are unit-tested; the actual spawn is in `run_backup`.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// The libimobiledevice backup client.
pub const BACKUP_TOOL: &str = "idevicebackup2";
/// The libimobiledevice device-listing tool.
pub const DEVICE_TOOL: &str = "idevice_id";

/// Argv for `idevicebackup2 --udid <udid> backup [--full] <out>`. The UDID binds
/// the operation to the selected device (important when several are connected).
pub fn backup_args(udid: &str, out: &Path, full: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["--udid".into(), udid.into(), "backup".into()];
    if full {
        args.push("--full".into());
    }
    args.push(out.as_os_str().to_owned());
    args
}

/// Parse `idevice_id -l` stdout into device UDIDs (trimmed, non-empty lines).
pub fn parse_udids(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Whether `tool` can be spawned (i.e. is installed on PATH). Probes by running
/// `tool --version`; success is "the process ran" regardless of its exit code,
/// since these tools' `--version` exit status is not guaranteed.
pub fn tool_available(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn backup_args_bind_udid_and_full() {
        let plain: Vec<String> = backup_args("UDID1", Path::new("/out"), false)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(plain, vec!["--udid", "UDID1", "backup", "/out"]);

        let full: Vec<String> = backup_args("UDID1", Path::new("/out"), true)
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(full, vec!["--udid", "UDID1", "backup", "--full", "/out"]);
    }

    #[test]
    fn parse_udids_trims_and_skips_blanks() {
        let out = "00008110-AAA\n  00008110-BBB \n\n\t\n";
        assert_eq!(parse_udids(out), vec!["00008110-AAA", "00008110-BBB"]);
        assert!(parse_udids("\n  \n").is_empty());
        assert!(parse_udids("").is_empty());
    }

    #[test]
    fn tool_available_false_for_absent_binary() {
        assert!(!tool_available("definitely_not_a_real_tool_qvery_unlikely_xyz"));
    }
}
