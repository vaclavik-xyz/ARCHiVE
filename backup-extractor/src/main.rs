mod contacts;
mod format;
#[cfg(test)]
mod test_fixtures;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::format::Format;

#[derive(Parser)]
#[command(name = "backup-extractor", about = "Extract personal data from an iOS backup")]
struct Cli {
    /// Path to the iOS backup directory.
    #[arg(long)]
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
}

/// A failure with a machine-stable `kind` and a documented exit code.
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
    }
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
    // Decrypt the address book to a unique temp file OUTSIDE the export dir, and
    // always remove it (success or failure) so the plaintext DB never lingers.
    let tmp = std::env::temp_dir().join(format!("be-addressbook-{}.sqlitedb", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let Some(db) = backup
        .fetch("HomeDomain", "Library/AddressBook/AddressBook.sqlitedb", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        // Store absent is a clean success with zero output.
        return Ok(serde_json::json!({
            "ok": true, "command": "contacts", "count": 0, "outputs": [],
            "note": "this backup has no contacts", "device": device
        }));
    };

    let parsed = contacts::parse(&db);
    let _ = std::fs::remove_file(&db);
    let people = parsed.map_err(|e| AppError::other(e.to_string()))?;

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
}
