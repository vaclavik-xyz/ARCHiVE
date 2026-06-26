mod calls;
mod contacts;
mod datetime;
mod format;
mod sqlite_util;
#[allow(dead_code)]
mod voicemail;
#[cfg(test)]
mod test_fixtures;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::format::Format;

#[derive(Parser)]
#[command(name = "backup-extractor", about = "Extract personal data from an iOS backup")]
struct Cli {
    /// Path to the iOS backup directory.
    #[arg(long, required = true)]
    backup: PathBuf,
    /// Password for an encrypted backup (ignored for unencrypted backups).
    #[arg(long, global = true)]
    password: Option<String>,
    /// Output directory (required for export commands; unused by `inspect`).
    #[arg(long, short = 'o', global = true)]
    out: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Export contacts.
    Contacts {
        /// Output format: csv, json, vcf, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export call history.
    Calls {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Report (as JSON) which data stores the backup contains. Read-only;
    /// does not need `--out`.
    Inspect,
}

/// A failure with a machine-stable `kind` and a documented exit code.
#[derive(Debug)]
struct AppError {
    kind: &'static str,
    message: String,
    code: i32,
}
impl AppError {
    fn auth(m: impl Into<String>) -> Self { Self { kind: "auth", message: m.into(), code: 2 } }
    fn usage(m: impl Into<String>) -> Self { Self { kind: "usage", message: m.into(), code: 1 } }
    fn other(m: impl Into<String>) -> Self { Self { kind: "other", message: m.into(), code: 1 } }
}

fn device_json(d: &backup_core::DeviceInfo) -> serde_json::Value {
    serde_json::json!({ "name": d.device_name, "ios": d.product_version, "udid": d.udid })
}

/// Map a `backup-core` open error to the right `AppError`: a locked/encrypted
/// backup needs auth (exit 2); anything else is a usage/other error (exit 1).
fn open_error(e: backup_core::BackupError) -> AppError {
    let msg = e.to_string();
    match e {
        backup_core::BackupError::Locked(_) => AppError::auth(msg),
        _ => AppError::other(msg),
    }
}

fn main() {
    match run() {
        // Success: one JSON object to stdout (the agent contract).
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap()),
        Err(e) => {
            let envelope = serde_json::json!({ "ok": false, "error": e.message, "kind": e.kind });
            println!("{}", serde_json::to_string_pretty(&envelope).unwrap());
            std::process::exit(e.code);
        }
    }
}

fn run() -> Result<serde_json::Value, AppError> {
    let cli = Cli::parse();
    // Password: flag wins, else env var; never prompt in this increment.
    let password = cli
        .password
        .clone()
        .or_else(|| std::env::var("BACKUP_EXTRACTOR_PASSWORD").ok());
    match &cli.command {
        Command::Contacts { format } => run_contacts(&cli, password.as_deref(), format),
        Command::Calls { format } => run_calls(&cli, password.as_deref(), format),
        Command::Inspect => run_inspect(&cli, password.as_deref()),
    }
}

/// Fetch and parse the address book from a backup into memory, using a secure
/// auto-cleaned temp dir (random name, removed on every return path so the
/// decrypted DB never lingers). `Ok(None)` when the backup has no contacts store.
fn load_contacts(
    backup: &backup_core::Backup,
) -> Result<Option<Vec<contacts::Contact>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("AddressBook.sqlitedb");
    let Some(db) = backup
        .fetch("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let people = contacts::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(people))
    // `scratch` drops here, removing the temp dir and the decrypted DB.
}

fn run_contacts(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown contacts format `{format}` (use csv, json, vcf, html)")))?;

    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export contacts"))?;

    let backup = backup_core::Backup::open(&cli.backup, password)
        .map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(people) = load_contacts(&backup)? else {
        // Store absent is a clean success with zero output.
        return Ok(serde_json::json!({
            "ok": true, "command": "contacts", "count": 0, "outputs": [],
            "note": "this backup has no contacts", "device": device
        }));
    };

    let rendered = match format {
        Format::Csv => format::contacts_csv(&people),
        Format::Json => format::contacts_json(&people),
        Format::Vcf => format::contacts_vcard(&people),
        Format::Html => format::contacts_html(&people),
    };
    let out_file = out.join(format!("contacts.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    // Human progress to stderr; the machine-readable result goes to stdout.
    eprintln!("Wrote {} contact(s) to {}", people.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "contacts", "count": people.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Validate a `calls` format string: csv/json/html only (vcf is meaningless for
/// calls). Returns a usage `AppError` (exit 1) on a bad or unsupported format.
fn calls_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown calls format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for calls (use csv, json, html)"));
    }
    Ok(f)
}

/// Fetch and parse the call history into memory via a secure auto-cleaned temp
/// dir. `Ok(None)` when the backup has no call-history store.
fn load_calls(backup: &backup_core::Backup) -> Result<Option<Vec<calls::Call>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("CallHistory.storedata");
    let Some(db) = backup
        .fetch("HomeDomain", "Library/CallHistoryDB/CallHistory.storedata", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let calls = calls::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(calls))
}

fn run_calls(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = calls_format(format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export calls"))?;

    let backup = backup_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(calls) = load_calls(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "calls", "count": 0, "outputs": [],
            "note": "this backup has no call history", "device": device
        }));
    };

    let rendered = match format {
        Format::Csv => format::calls_csv(&calls),
        Format::Json => format::calls_json(&calls),
        Format::Html => format::calls_html(&calls),
        Format::Vcf => unreachable!("calls_format rejects vcf"),
    };
    let out_file = out.join(format!("calls.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} call(s) to {}", calls.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "calls", "count": calls.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// One row of `inspect` output: a known store and its availability.
struct StoreStatus {
    name: &'static str,
    present: bool,
    supported: bool,
    count: Option<usize>,
}

fn inspect_json(device: serde_json::Value, stores: &[StoreStatus]) -> serde_json::Value {
    let stores: Vec<_> = stores
        .iter()
        .map(|s| serde_json::json!({
            "type": s.name, "present": s.present, "supported": s.supported, "count": s.count
        }))
        .collect();
    serde_json::json!({ "ok": true, "command": "inspect", "device": device, "stores": stores })
}

// Known stores: (type, supported-in-this-build, domain, relative_path).
const KNOWN_STORES: &[(&str, bool, &str, &str)] = &[
    ("contacts", true, "HomeDomain", "Library/AddressBook/AddressBook.sqlitedb"),
    ("calls", true, "HomeDomain", "Library/CallHistoryDB/CallHistory.storedata"),
    ("photos", false, "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
    ("notes", false, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite"),
];

fn run_inspect(cli: &Cli, password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let backup = backup_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    let mut stores = Vec::new();
    for &(name, supported, domain, path) in KNOWN_STORES {
        let present = backup
            .has(domain, path)
            .map_err(|e| AppError::other(e.to_string()))?;
        // `count` is best-effort: a present-but-unparseable store yields `None`
        // (count: null) without aborting inspect. `.ok()` drops a read error,
        // `.flatten()` collapses store-absent `Ok(None)`, `.map(len)` counts a
        // successful read.
        let count = if present && supported {
            match name {
                "contacts" => load_contacts(&backup).ok().flatten().map(|p| p.len()),
                "calls" => load_calls(&backup).ok().flatten().map(|c| c.len()),
                _ => None,
            }
        } else {
            None
        };
        stores.push(StoreStatus { name, present, supported, count });
    }
    Ok(inspect_json(device, &stores))
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn parses_contacts_invocation() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "-o", "/out", "contacts", "-f", "vcf",
        ])
        .unwrap();
        assert_eq!(cli.backup, PathBuf::from("/b"));
        match cli.command {
            Command::Contacts { format } => assert_eq!(format, "vcf"),
            _ => panic!("expected Contacts"),
        }
    }

    #[test]
    fn rejects_missing_backup() {
        assert!(Cli::try_parse_from(["backup-extractor", "-o", "/out", "contacts", "-f", "csv"]).is_err());
    }

    #[test]
    fn open_error_maps_locked_to_auth_else_other() {
        assert_eq!(open_error(backup_core::BackupError::Locked("x".into())).code, 2);
        assert_eq!(open_error(backup_core::BackupError::Open("x".into())).code, 1);
    }

    #[test]
    fn parses_out_after_subcommand() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "contacts", "-f", "csv", "-o", "/out",
        ])
        .unwrap();
        assert_eq!(cli.backup, PathBuf::from("/b"));
        assert_eq!(cli.out, Some(PathBuf::from("/out")));
    }

    #[test]
    fn parses_calls_invocation() {
        let cli = Cli::try_parse_from([
            "backup-extractor", "--backup", "/b", "-o", "/out", "calls", "-f", "json",
        ])
        .unwrap();
        match cli.command {
            Command::Calls { format } => assert_eq!(format, "json"),
            _ => panic!("expected Calls"),
        }
    }

    #[test]
    fn calls_format_rejects_vcf_accepts_others() {
        assert_eq!(calls_format("csv").unwrap(), Format::Csv);
        assert_eq!(calls_format("json").unwrap(), Format::Json);
        assert_eq!(calls_format("html").unwrap(), Format::Html);
        assert_eq!(calls_format("vcf").unwrap_err().code, 1);
        assert_eq!(calls_format("nope").unwrap_err().code, 1);
    }

    #[test]
    fn inspect_json_has_typed_stores() {
        let device = serde_json::json!({ "name": "iPhone", "ios": "17.5", "udid": "x" });
        let v = inspect_json(
            device,
            &[
                StoreStatus { name: "contacts", present: true, supported: true, count: Some(12) },
                StoreStatus { name: "calls", present: true, supported: false, count: None },
            ],
        );
        assert_eq!(v["ok"], true);
        assert_eq!(v["command"], "inspect");
        assert_eq!(v["stores"][0]["type"], "contacts");
        assert_eq!(v["stores"][0]["present"], true);
        assert_eq!(v["stores"][0]["count"], 12);
        assert_eq!(v["stores"][1]["count"], serde_json::Value::Null);
        assert_eq!(v["stores"][1]["supported"], false);
    }
}
