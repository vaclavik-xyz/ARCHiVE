mod accounts;
mod app_databases;
mod app_files;
mod audio;
mod attachments;
mod backup_diff;
mod bluetooth;
mod calendar;
mod calls;
mod certificates;
mod contacts;
mod data_usage;
mod datetime;
mod db_export;
mod device_backup;
mod device_model;
mod device_usage;
mod enrich;
mod format;
mod health;
mod homescreen;
mod interactions;
mod keyboard_lexicon;
mod known_networks;
mod mail;
mod messages;
mod notes;
mod package;
mod pdf;
mod photos;
mod photos_deleted;
mod recover;
mod recover_deleted;
mod redact;
mod reminders;
mod safari;
mod schema_check;
mod search;
mod significant_locations;
mod sqlite_util;
mod stats;
mod summary;
mod timeline;
mod voice_memos;
mod voicemail;
mod voicemail_audio;
mod whatsapp;
#[cfg(test)]
mod test_fixtures;
#[cfg(test)]
mod cross_version_tests;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::format::Format;

#[derive(Parser)]
#[command(name = "archive", about = "Extract personal data from an iOS backup")]
struct Cli {
    /// Path to the iOS backup directory (required for every command except `backup`,
    /// which creates one).
    #[arg(long)]
    backup: Option<PathBuf>,
    /// Password for an encrypted backup (ignored for unencrypted backups).
    #[arg(long, global = true)]
    password: Option<String>,
    /// Output directory (required for export commands; unused by `inspect`).
    #[arg(long, short = 'o', global = true)]
    out: Option<PathBuf>,
    /// Path to a headless Chrome/Chromium/Edge for `-f pdf` (auto-detected if omitted).
    #[arg(long, global = true)]
    chrome_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Export contacts.
    Contacts {
        /// Output format: csv, json, vcf, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export call history.
    Calls {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export configured accounts (Apple ID, Google, Exchange, …); metadata only,
    /// no passwords.
    Accounts {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export remembered Wi-Fi networks (SSID list, no passwords); works on any
    /// backup. For passwords use `wifi` (encrypted backups only).
    KnownNetworks {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Reconstruct the Home Screen layout (pages, dock, folders) from
    /// SpringBoard's IconState.plist; works on any backup.
    HomescreenLayout {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Per-process network data usage (cellular/Wi-Fi byte counters) from
    /// DataUsage.sqlite.
    DataUsage {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Per-app foreground usage (time, sessions) from CoreDuet's knowledgeC.db.
    DeviceUsage {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Per-contact communication history (who, which app, how often, when) from
    /// CoreDuet's interactionC.db.
    Interactions {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Paired and previously-seen Bluetooth devices (names + MAC addresses) from
    /// the system Bluetooth databases.
    BluetoothDevices {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Recorded location history from the routined "Significant Locations"
    /// database (usually excluded from standard backups).
    SignificantLocations {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// The user's custom keyboard words (LocalDictionary "Add to Dictionary"
    /// entries); empty when the owner never added one.
    KeyboardLexicon {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export voicemail metadata (optionally extract audio with --audio).
    Voicemail {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Also extract each voicemail's audio into <out>/voicemail_audio/.
        #[arg(long)]
        audio: bool,
        /// Audio output format: amr (raw, default), m4a, or wav (m4a/wav need ffmpeg).
        #[arg(long)]
        audio_format: Option<String>,
    },
    /// Export Voice Memos metadata and audio (audio on by default; --no-audio to skip).
    VoiceMemos {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip audio extraction (metadata only).
        #[arg(long)]
        no_audio: bool,
        /// Transcode audio to m4a or wav (needs ffmpeg). Default: raw native copy.
        #[arg(long)]
        audio_format: Option<String>,
    },
    /// Export Safari browsing history.
    SafariHistory {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Safari bookmarks.
    SafariBookmarks {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export calendar events.
    Calendar {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Apple Notes (title, folder, dates, body text).
    Notes {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Camera Roll metadata and files (files on by default; --no-files to skip).
    Photos {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip file extraction (metadata catalog only).
        #[arg(long)]
        no_files: bool,
        /// Render a text-only summary report (no gallery, no files copied):
        /// device, totals, quality, period, per-year and per-album counts.
        /// Writes `<out>/photos-summary.<ext>` (html/pdf). Fast — probes the
        /// backup for quality counts without copying media.
        #[arg(long)]
        summary: bool,
    },
    /// Recover "Recently Deleted" photos/videos still inside the 30-day purge
    /// window (files on by default; --no-files to skip).
    PhotosRecentlyDeleted {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip file extraction (metadata catalog only).
        #[arg(long)]
        no_files: bool,
    },
    /// Export Messages attachment metadata and files (files on by default; --no-files to skip).
    Attachments {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip file extraction (metadata catalog only).
        #[arg(long)]
        no_files: bool,
    },
    /// Export WhatsApp messages and media (files on by default; --no-files to skip).
    Whatsapp {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip media extraction (transcript only).
        #[arg(long)]
        no_files: bool,
    },
    /// Export iMessage/SMS/RCS conversations (txt, html, pdf) by driving the
    /// bundled imessage-exporter; writes a transcript tree under <out>/messages.
    Messages {
        /// Output format: txt, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Apple Health: workouts and per-type quantity summaries.
    Health {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Apple Reminders (lists, items, due/completion, priority).
    Reminders {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Apple Mail messages (local/POP3 `.emlx`; often absent on iOS).
    Mail {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// List installed third-party apps (bundle ids) from the backup manifest.
    Apps {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Merge every in-process extractor into one chronological timeline.
    Timeline {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Mask phone numbers and email local parts in the output (for sharing).
        #[arg(long)]
        redact: bool,
    },
    /// Activity dashboard: per-category event counts and date ranges across
    /// every in-process extractor (a statistical view over the timeline).
    Stats {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Per-app database recoverability report: which app databases are readable
    /// plain SQLite vs encrypted/other (so you can see what's extractable).
    AppDatabases {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Extract a named app's document/media files from its backup container(s).
    AppFiles {
        /// App to extract: matched as a case-insensitive substring of the backup
        /// domain (e.g. `viber`, `whatsapp`, `com.burbn.instagram`).
        #[arg(long)]
        app: String,
        /// Output format for the manifest: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Copy every file, not just media (images/videos/audio).
        #[arg(long)]
        all: bool,
    },
    /// Recover DELETED rows from backup SQLite databases by carving freed
    /// pages/freeblocks/WAL (best-effort). `--store`: messages, calls, contacts, all.
    RecoverDeleted {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Which store(s) to carve: messages | calls | contacts | notes |
        /// calendar | safari | photos | all.
        #[arg(long, default_value = "all")]
        store: String,
    },
    /// Check each SQLite store's live schema against the columns its extractor
    /// needs, flagging drift (renamed/removed columns) across iOS versions.
    SchemaCheck {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Case-file search: find one term across every in-process record (timeline)
    /// and the address book. Case-insensitive substring.
    Search {
        /// The term to search for (phone, name, keyword).
        #[arg(long, short = 'q')]
        query: String,
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
        /// Mask phone numbers and email local parts in the matched snippets.
        #[arg(long)]
        redact: bool,
    },
    /// Consolidate the extracted data into one queryable SQLite database
    /// (`<out>/archive.sqlite`): timeline + contacts + calls + whatsapp tables.
    DbExport,
    /// Diff two backups at the file level: which manifest files were added,
    /// removed, or changed size between `--backup` (A) and `--against` (B).
    Diff {
        /// The second (newer) backup directory to compare against.
        #[arg(long)]
        against: std::path::PathBuf,
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Bundle a directory of exports into one AES-256 encrypted zip for delivery
    /// (`<out>/archive-package.zip`).
    Package {
        /// The directory to package (e.g. a prior `recover`/export output dir).
        #[arg(long)]
        source: std::path::PathBuf,
        /// Zip encryption password (or set `ARCHIVE_ZIP_PASSWORD`); required.
        #[arg(long)]
        zip_password: Option<String>,
    },
    /// Recover saved Wi-Fi passwords from the keychain (encrypted backups only).
    Wifi {
        /// Output format: csv, json, html (no pdf — avoids a plaintext sidecar).
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Recover saved website/app passwords from the keychain (encrypted backups
    /// only). Sensitive: output contains plaintext passwords.
    Passwords {
        /// Output format: csv, json, html (no pdf — avoids a plaintext sidecar).
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Census of the keychain: per-item metadata (service, account, group, class)
    /// with NO secrets. Encrypted backups only.
    KeychainInventory {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Recover X.509 certificates from the keychain as a PEM bundle + metadata
    /// (encrypted backups only). Public certificates only — no private keys.
    Certificates {
        /// Output format: csv, json, html, pdf.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Recover VPN / enterprise-Wi-Fi (802.1X/EAP) credentials from the keychain
    /// (encrypted backups only). Sensitive: plaintext secrets. Best-effort
    /// marker-based detection.
    VpnCreds {
        /// Output format: csv, json, html (no pdf — avoids a plaintext sidecar).
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Run every extractor into <out>/ and write a customer index.html package.
    Recover {
        /// Skip large media extraction (metadata + HTML only).
        #[arg(long)]
        no_files: bool,
    },
    /// Create a fresh backup from a USB-connected iPhone via libimobiledevice.
    Backup {
        /// Force a full backup (default: incremental when <out> already has one).
        #[arg(long)]
        full: bool,
    },
    /// Report (as JSON) which data stores the backup contains. Read-only;
    /// does not need `--out`.
    Inspect,
    /// Verify the backup is complete (every manifest file present). Read-only;
    /// does not need `--out`.
    Integrity,
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

fn device_json(d: &archive_core::DeviceInfo) -> serde_json::Value {
    serde_json::json!({
        "name": d.device_name, "model": d.model, "ios": d.product_version,
        "serial": d.serial, "udid": d.udid
    })
}

/// Resolve `--backup` (a usage error when absent) and open it, mapping open
/// failures to the right `AppError`. Shared by every read command.
fn open_backup(cli: &Cli, password: Option<&str>) -> Result<archive_core::Backup, AppError> {
    let dir = cli
        .backup
        .as_deref()
        .ok_or_else(|| AppError::usage("--backup <DIR> is required"))?;
    archive_core::Backup::open(dir, password).map_err(open_error)
}

/// Map a `archive-core` open error to the right `AppError`: a locked/encrypted
/// backup needs auth (exit 2); anything else is a usage/other error (exit 1).
fn open_error(e: archive_core::BackupError) -> AppError {
    let msg = e.to_string();
    match e {
        archive_core::BackupError::Locked(_) => AppError::auth(msg),
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
        .or_else(|| std::env::var("ARCHIVE_PASSWORD").ok());
    match &cli.command {
        Command::Contacts { format } => run_contacts(&cli, password.as_deref(), format),
        Command::Calls { format } => run_calls(&cli, password.as_deref(), format),
        Command::Accounts { format } => run_accounts(&cli, password.as_deref(), format),
        Command::KnownNetworks { format } => run_known_networks(&cli, password.as_deref(), format),
        Command::HomescreenLayout { format } => run_homescreen(&cli, password.as_deref(), format),
        Command::DataUsage { format } => run_data_usage(&cli, password.as_deref(), format),
        Command::DeviceUsage { format } => run_device_usage(&cli, password.as_deref(), format),
        Command::Interactions { format } => run_interactions(&cli, password.as_deref(), format),
        Command::BluetoothDevices { format } => run_bluetooth_devices(&cli, password.as_deref(), format),
        Command::SignificantLocations { format } => run_significant_locations(&cli, password.as_deref(), format),
        Command::KeyboardLexicon { format } => run_keyboard_lexicon(&cli, password.as_deref(), format),
        Command::Voicemail { format, audio, audio_format } => {
            run_voicemail(&cli, password.as_deref(), format, *audio, audio_format.as_deref())
        }
        Command::VoiceMemos { format, no_audio, audio_format } => {
            run_voice_memos(&cli, password.as_deref(), format, *no_audio, audio_format.as_deref())
        }
        Command::SafariHistory { format } => run_safari_history(&cli, password.as_deref(), format),
        Command::SafariBookmarks { format } => run_safari_bookmarks(&cli, password.as_deref(), format),
        Command::Calendar { format } => run_calendar(&cli, password.as_deref(), format),
        Command::Notes { format } => run_notes(&cli, password.as_deref(), format),
        Command::Photos { format, no_files, summary } => run_photos(&cli, password.as_deref(), format, *no_files, *summary),
        Command::PhotosRecentlyDeleted { format, no_files } => {
            run_photos_recently_deleted(&cli, password.as_deref(), format, *no_files)
        }
        Command::Attachments { format, no_files } => run_attachments(&cli, password.as_deref(), format, *no_files),
        Command::Whatsapp { format, no_files } => run_whatsapp(&cli, password.as_deref(), format, *no_files),
        Command::Messages { format } => run_messages(&cli, password.as_deref(), format),
        Command::Health { format } => run_health(&cli, password.as_deref(), format),
        Command::Reminders { format } => run_reminders(&cli, password.as_deref(), format),
        Command::Mail { format } => run_mail(&cli, password.as_deref(), format),
        Command::Apps { format } => run_apps(&cli, password.as_deref(), format),
        Command::Timeline { format, redact } => run_timeline(&cli, password.as_deref(), format, *redact),
        Command::Stats { format } => run_stats(&cli, password.as_deref(), format),
        Command::AppDatabases { format } => run_app_databases(&cli, password.as_deref(), format),
        Command::AppFiles { app, format, all } => run_app_files(&cli, password.as_deref(), app, format, *all),
        Command::RecoverDeleted { format, store } => run_recover_deleted(&cli, password.as_deref(), format, store),
        Command::SchemaCheck { format } => run_schema_check(&cli, password.as_deref(), format),
        Command::Search { query, format, redact } => run_search(&cli, password.as_deref(), query, format, *redact),
        Command::DbExport => run_db_export(&cli, password.as_deref()),
        Command::Diff { against, format } => run_diff(&cli, password.as_deref(), against, format),
        Command::Package { source, zip_password } => run_package(&cli, source, zip_password.as_deref()),
        Command::Wifi { format } => run_wifi(&cli, password.as_deref(), format),
        Command::Passwords { format } => run_passwords(&cli, password.as_deref(), format),
        Command::KeychainInventory { format } => run_keychain_inventory(&cli, password.as_deref(), format),
        Command::Certificates { format } => run_certificates(&cli, password.as_deref(), format),
        Command::VpnCreds { format } => run_vpn_creds(&cli, password.as_deref(), format),
        Command::Recover { no_files } => run_recover(&cli, password.as_deref(), *no_files),
        Command::Backup { full } => run_backup(&cli, password.as_deref(), *full),
        Command::Inspect => run_inspect(&cli, password.as_deref()),
        Command::Integrity => run_integrity(&cli, password.as_deref()),
    }
}

/// Fetch and parse the address book from a backup into memory, using a secure
/// auto-cleaned temp dir (random name, removed on every return path so the
/// decrypted DB never lingers). `Ok(None)` when the backup has no contacts store.
fn load_contacts(
    backup: &archive_core::Backup,
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

/// Build the address-book reverse index for contact enrichment, or `None` when
/// the backup has no contacts / the index is empty. Read failures are logged and
/// treated as "no enrichment", never fatal — enrichment is always best-effort.
fn contact_index(backup: &archive_core::Backup) -> Option<enrich::ContactIndex> {
    let people = opt_or_log(load_contacts(backup), "contacts")?;
    let idx = enrich::ContactIndex::build(&people);
    (!idx.is_empty()).then_some(idx)
}

fn run_contacts(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown contacts format `{format}` (use csv, json, vcf, html, pdf)")))?;

    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export contacts"))?;

    let backup = open_backup(cli, password)?;
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
        Format::Html | Format::Pdf => format::contacts_html(&people),
    };
    let out_file = out.join(format!("contacts.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
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
        .ok_or_else(|| AppError::usage(format!("unknown calls format `{format}` (use csv, json, html, pdf)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for calls (use csv, json, html, pdf)"));
    }
    Ok(f)
}

/// Fetch and parse the call history into memory via a secure auto-cleaned temp
/// dir. `Ok(None)` when the backup has no call-history store.
fn load_calls(backup: &archive_core::Backup) -> Result<Option<Vec<calls::Call>>, AppError> {
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

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut calls) = load_calls(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "calls", "count": 0, "outputs": [],
            "note": "this backup has no call history", "device": device
        }));
    };
    if let Some(idx) = contact_index(&backup) {
        enrich::enrich_calls(&idx, &mut calls);
    }

    let rendered = match format {
        Format::Csv => format::calls_csv(&calls),
        Format::Json => format::calls_json(&calls),
        Format::Html | Format::Pdf => format::calls_html(&calls),
        Format::Vcf => unreachable!("calls_format rejects vcf"),
    };
    let out_file = out.join(format!("calls.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} call(s) to {}", calls.len(), out_file.display());

    let summary_file = write_type_summary(out, backup.device_info(), &calls::summary(&calls))?;

    Ok(serde_json::json!({
        "ok": true, "command": "calls", "count": calls.len(),
        "outputs": [out_file.to_string_lossy(), summary_file], "device": device
    }))
}

fn load_accounts(backup: &archive_core::Backup) -> Result<Option<Vec<accounts::Account>>, AppError> {
    load_store(
        backup,
        "HomeDomain",
        "Library/Accounts/Accounts3.sqlite",
        "Accounts3.sqlite",
        accounts::parse,
    )
}

fn run_accounts(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "accounts")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export accounts"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_accounts(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "accounts", "count": 0, "outputs": [],
            "note": "this backup has no accounts store", "device": device
        }));
    };

    let rendered = match format {
        Format::Csv => format::accounts_csv(&items),
        Format::Json => format::accounts_json(&items),
        Format::Html | Format::Pdf => format::accounts_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("accounts.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} account(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "accounts", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Fetch and parse the saved Wi-Fi networks plist. Probes the candidate paths in
/// order; returns the first that yields networks, else `Some(empty)` if a plist
/// exists but lists none, else `Ok(None)` when no plist is present.
fn load_known_networks(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<known_networks::KnownNetwork>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut found_file = false;
    for path in known_networks::PATHS {
        let tmp = scratch.path().join("wifi.plist");
        let Some(file) = backup
            .fetch(known_networks::DOMAIN, path, &tmp)
            .map_err(|e| AppError::other(e.to_string()))?
        else {
            continue;
        };
        found_file = true;
        let bytes = std::fs::read(&file).map_err(|e| AppError::other(e.to_string()))?;
        let nets = known_networks::parse(&bytes);
        if !nets.is_empty() {
            return Ok(Some(nets));
        }
    }
    Ok(found_file.then(Vec::new))
}

fn run_known_networks(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "known-networks")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export known networks"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let items = load_known_networks(&backup)?.unwrap_or_default();
    if items.is_empty() {
        // On iOS 16+ the plaintext saved-networks list is typically empty (the
        // inventory moved to the keychain). Distinguish that from older backups
        // and point users to `wifi` for keychain-derived SSIDs+passwords.
        return Ok(serde_json::json!({
            "ok": true, "command": "known-networks", "count": 0, "outputs": [],
            "note": "no saved networks in the plaintext Wi-Fi plist (empty on iOS 16+, where the list moved to the keychain — use `wifi` on an encrypted backup to recover SSIDs and passwords)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::known_networks_csv(&items),
        Format::Json => format::known_networks_json(&items),
        Format::Html | Format::Pdf => format::known_networks_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("known-networks.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} known network(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "known-networks", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Fetch and merge every Bluetooth device source (LE paired/other databases plus
/// the classic-devices plist). `Ok(None)` only when none of the sources exist;
/// `Ok(Some(_))` (possibly empty) when at least one source was present.
fn load_bluetooth_devices(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<bluetooth::BluetoothDevice>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut found = false;
    let mut all = Vec::new();

    for (path, table, kind) in bluetooth::LE_DATABASES {
        let tmp = scratch.path().join("ble.db");
        let Some(db) = backup
            .fetch(bluetooth::DOMAIN, path, &tmp)
            .map_err(|e| AppError::other(e.to_string()))?
        else {
            continue;
        };
        found = true;
        match bluetooth::parse_ledevices(&db, table, kind) {
            Ok(mut devs) => all.append(&mut devs),
            Err(e) => eprintln!("Skipping Bluetooth table {table}: {e}"),
        }
    }

    let tmp = scratch.path().join("classic.plist");
    if let Some(file) = backup
        .fetch(bluetooth::DOMAIN, bluetooth::CLASSIC_PLIST, &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    {
        found = true;
        let bytes = std::fs::read(&file).map_err(|e| AppError::other(e.to_string()))?;
        all.append(&mut bluetooth::parse_classic(&bytes));
    }

    Ok(found.then(|| bluetooth::merge(all)))
}

fn run_bluetooth_devices(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "bluetooth-devices")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export bluetooth devices"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let items = load_bluetooth_devices(&backup)?.unwrap_or_default();
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "bluetooth-devices", "count": 0, "outputs": [],
            "note": "no Bluetooth device databases in this backup",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::bluetooth_devices_csv(&items),
        Format::Json => format::bluetooth_devices_json(&items),
        Format::Html | Format::Pdf => format::bluetooth_devices_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("bluetooth-devices.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    let named = items.iter().filter(|d| d.is_named()).count();
    eprintln!("Wrote {} Bluetooth device(s) ({named} named) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "bluetooth-devices", "count": items.len(), "named": named,
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Fetch and parse the routined location history. Probes the candidate database
/// paths in order; returns the first that yields fixes, else `Some(empty)` if a
/// database exists but lists none, else `Ok(None)` when none are present.
fn load_significant_locations(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<significant_locations::LocationFix>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut found_file = false;
    for path in significant_locations::PATHS {
        let tmp = scratch.path().join("routined.sqlite");
        let Some(db) = backup
            .fetch(significant_locations::DOMAIN, path, &tmp)
            .map_err(|e| AppError::other(e.to_string()))?
        else {
            continue;
        };
        found_file = true;
        let fixes = significant_locations::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
        if !fixes.is_empty() {
            return Ok(Some(fixes));
        }
    }
    Ok(found_file.then(Vec::new))
}

fn run_significant_locations(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "significant-locations")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export significant locations"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let items = load_significant_locations(&backup)?.unwrap_or_default();
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "significant-locations", "count": 0, "outputs": [],
            "note": "no location history recovered — the routined store lives under Library/Caches, which iOS excludes from ordinary iTunes/Finder backups (recoverable only from a full filesystem extraction that includes the caches)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::significant_locations_csv(&items),
        Format::Json => format::significant_locations_json(&items),
        Format::Html | Format::Pdf => format::significant_locations_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("significant-locations.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} location fix(es) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "significant-locations", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Fetch and parse the custom keyboard words. Probes the candidate locations in
/// order; returns the first that yields words, else `Some(empty)` if a file
/// exists but lists none, else `Ok(None)` when no file is present.
fn load_keyboard_lexicon(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<keyboard_lexicon::LexiconWord>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut found_file = false;
    for (domain, path) in keyboard_lexicon::SOURCES {
        let tmp = scratch.path().join("LocalDictionary");
        let Some(file) = backup
            .fetch(domain, path, &tmp)
            .map_err(|e| AppError::other(e.to_string()))?
        else {
            continue;
        };
        found_file = true;
        let bytes = std::fs::read(&file).map_err(|e| AppError::other(e.to_string()))?;
        let words = keyboard_lexicon::parse(&bytes);
        if !words.is_empty() {
            return Ok(Some(words));
        }
    }
    Ok(found_file.then(Vec::new))
}

fn run_keyboard_lexicon(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "keyboard-lexicon")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export the keyboard lexicon"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let items = load_keyboard_lexicon(&backup)?.unwrap_or_default();
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "keyboard-lexicon", "count": 0, "outputs": [],
            "note": "no custom keyboard words found (LocalDictionary holds only words the user added via \"Add to Dictionary\"; it is empty on devices where none were added — the learned typing model is a separate, non-recoverable store)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::keyboard_lexicon_csv(&items),
        Format::Json => format::keyboard_lexicon_json(&items),
        Format::Html | Format::Pdf => format::keyboard_lexicon_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("keyboard-lexicon.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} custom keyboard word(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "keyboard-lexicon", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn load_homescreen(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<homescreen::IconSlot>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("IconState.plist");
    let Some(file) = backup
        .fetch(homescreen::DOMAIN, homescreen::PATH, &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let bytes = std::fs::read(&file).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(homescreen::parse(&bytes)))
}

fn run_homescreen(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "homescreen-layout")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export the home screen layout"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let Some(items) = load_homescreen(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "homescreen-layout", "count": 0, "outputs": [],
            "note": "this backup has no IconState.plist (home screen layout)", "device": device
        }));
    };
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "homescreen-layout", "count": 0, "outputs": [],
            "note": "home screen layout could not be parsed (unexpected IconState.plist shape)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::homescreen_csv(&items),
        Format::Json => format::homescreen_json(&items),
        Format::Html | Format::Pdf => format::homescreen_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("homescreen-layout.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} icon slot(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "homescreen-layout", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn load_data_usage(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<data_usage::DataUsage>>, AppError> {
    load_store(backup, data_usage::DOMAIN, data_usage::PATH, "DataUsage.sqlite", data_usage::parse)
}

fn run_data_usage(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "data-usage")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export data usage"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let Some(items) = load_data_usage(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "data-usage", "count": 0, "outputs": [],
            "note": "this backup has no DataUsage.sqlite", "device": device
        }));
    };
    if items.is_empty() {
        // Present but empty, or a schema we can't aggregate (no ZLIVEUSAGE/
        // ZPROCESS/ZHASPROCESS) — say so rather than write a misleading file.
        return Ok(serde_json::json!({
            "ok": true, "command": "data-usage", "count": 0, "outputs": [],
            "note": "DataUsage.sqlite has no per-process usage rows (empty, or an unsupported schema)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::data_usage_csv(&items),
        Format::Json => format::data_usage_json(&items),
        Format::Html | Format::Pdf => format::data_usage_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("data-usage.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote data usage for {} process(es) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "data-usage", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn load_device_usage(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<device_usage::AppUsage>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    for (domain, path) in device_usage::CANDIDATES {
        let tmp = scratch.path().join("knowledgeC.db");
        if let Some(db) = backup.fetch(domain, path, &tmp).map_err(|e| AppError::other(e.to_string()))? {
            return Ok(Some(device_usage::parse(&db).map_err(|e| AppError::other(e.to_string()))?));
        }
    }
    Ok(None)
}

fn run_device_usage(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "device-usage")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export device usage"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let Some(items) = load_device_usage(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "device-usage", "count": 0, "outputs": [],
            "note": "this backup has no knowledgeC.db (CoreDuet usage store)", "device": device
        }));
    };
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "device-usage", "count": 0, "outputs": [],
            "note": "knowledgeC.db has no /app/usage sessions (empty, or an unsupported schema)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::device_usage_csv(&items),
        Format::Json => format::device_usage_json(&items),
        Format::Html | Format::Pdf => format::device_usage_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("device-usage.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote app usage for {} app(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "device-usage", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn load_interactions(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<interactions::ContactInteractions>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    for (domain, path) in interactions::CANDIDATES {
        let tmp = scratch.path().join("interactionC.db");
        if let Some(db) = backup.fetch(domain, path, &tmp).map_err(|e| AppError::other(e.to_string()))? {
            return Ok(Some(interactions::parse(&db).map_err(|e| AppError::other(e.to_string()))?));
        }
    }
    Ok(None)
}

fn run_interactions(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "interactions")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export interactions"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let Some(items) = load_interactions(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "interactions", "count": 0, "outputs": [],
            "note": "this backup has no interactionC.db (CoreDuet interactions store)", "device": device
        }));
    };
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "interactions", "count": 0, "outputs": [],
            "note": "interactionC.db has no contact-linked interactions (empty, or an unsupported schema)",
            "device": device
        }));
    }

    let rendered = match format {
        Format::Csv => format::interactions_csv(&items),
        Format::Json => format::interactions_json(&items),
        Format::Html | Format::Pdf => format::interactions_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("interactions.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote interaction history for {} contact(s) to {}", items.len(), out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "interactions", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

/// Validate a `voicemail` format string: csv/json/html only.
fn voicemail_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown voicemail format `{format}` (use csv, json, html, pdf)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for voicemail (use csv, json, html, pdf)"));
    }
    Ok(f)
}

/// Fetch and parse voicemail metadata into memory via a secure auto-cleaned temp
/// dir. `Ok(None)` when the backup has no voicemail store.
fn load_voicemail(backup: &archive_core::Backup) -> Result<Option<Vec<voicemail::Voicemail>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("voicemail.db");
    let Some(db) = backup
        .fetch("HomeDomain", "Library/Voicemail/voicemail.db", &tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(None);
    };
    let items = voicemail::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(items))
}

/// Resolve the requested audio format from `--audio` / `--audio-format`.
/// `Ok(None)` = audio off. Usage error when `--audio-format` is given without
/// `--audio` or names an unknown format; a fatal error (exit 1) when a
/// transcoding format is requested but ffmpeg is not on PATH (fail fast, before
/// any extraction).
fn resolve_audio_format(
    audio: bool,
    audio_format: Option<&str>,
) -> Result<Option<audio::AudioFormat>, AppError> {
    use audio::AudioFormat;
    if !audio {
        if audio_format.is_some() {
            return Err(AppError::usage("--audio-format requires --audio"));
        }
        return Ok(None);
    }
    let fmt = match audio_format {
        None => AudioFormat::Amr,
        Some(s) => AudioFormat::from_cli(s).ok_or_else(|| {
            AppError::usage(format!("unknown audio format `{s}` (use amr, m4a, wav)"))
        })?,
    };
    if fmt.needs_ffmpeg() && !audio::ffmpeg_available() {
        return Err(AppError::other(format!(
            "audio format `{}` requires ffmpeg, which was not found on PATH; \
             install ffmpeg or use --audio-format amr",
            fmt.extension()
        )));
    }
    Ok(Some(fmt))
}

fn run_voicemail(
    cli: &Cli,
    password: Option<&str>,
    format: &str,
    audio: bool,
    audio_format: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let format = voicemail_format(format)?;
    // Resolve audio up front so a bad flag combo / missing ffmpeg fails fast.
    let audio_fmt = resolve_audio_format(audio, audio_format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export voicemail"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_voicemail(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "voicemail", "count": 0, "outputs": [],
            "note": "this backup has no voicemail", "device": device
        }));
    };

    if let Some(idx) = contact_index(&backup) {
        enrich::enrich_voicemail(&idx, &mut items);
    }

    // Extract audio before rendering so `audio_file` is populated in the output.
    let audio_summary = match audio_fmt {
        Some(fmt) => Some(
            voicemail_audio::extract_audio(&backup, &mut items, out, fmt)
                .map_err(|e| AppError::other(e.to_string()))?,
        ),
        None => None,
    };

    let rendered = match format {
        Format::Csv => format::voicemail_csv(&items),
        Format::Json => format::voicemail_json(&items),
        Format::Html | Format::Pdf => format::voicemail_html(&items),
        Format::Vcf => unreachable!("voicemail_format rejects vcf"),
    };
    let out_file = out.join(format!("voicemail.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} voicemail(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "voicemail", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = audio_summary {
        eprintln!(
            "Extracted {} audio file(s) ({} missing) to {}/voicemail_audio",
            s.extracted,
            s.missing,
            out.display()
        );
        envelope["audio"] = serde_json::json!({
            "format": s.format.extension(),
            "dir": s.dir,
            "extracted": s.extracted,
            "missing": s.missing
        });
    }
    Ok(envelope)
}

/// Validate a `voice-memos` format string: csv/json/html only.
fn voice_memos_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown voice-memos format `{format}` (use csv, json, html, pdf)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for voice-memos (use csv, json, html, pdf)"));
    }
    Ok(f)
}

/// Fetch and parse the Voice Memos store via a secure auto-cleaned temp dir,
/// trying the modern group container then the legacy `MediaDomain` location.
/// `Ok(None)` when the backup has neither.
fn load_voice_memos(
    backup: &archive_core::Backup,
) -> Result<Option<Vec<voice_memos::VoiceMemo>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("CloudRecordings.db");
    let mut db = None;
    for (domain, path) in voice_memos::DB_LOCATIONS {
        if let Some(p) = backup.fetch(domain, path, &tmp).map_err(|e| AppError::other(e.to_string()))? {
            db = Some(p);
            break;
        }
    }
    let Some(db) = db else { return Ok(None) };
    let items = voice_memos::parse(&db).map_err(|e| AppError::other(e.to_string()))?;
    Ok(Some(items))
}

/// Resolve the Voice Memos audio choice. Outer `None` = `--no-audio` (skip);
/// `Some(None)` = extract raw native copies; `Some(Some(fmt))` = transcode.
/// Usage error for `--audio-format` with `--no-audio`, an unknown format, or
/// `amr` (voicemail-only); fatal (exit 1) when a transcode format is requested
/// but ffmpeg is absent (fail fast, before any extraction).
fn resolve_vm_audio(
    no_audio: bool,
    audio_format: Option<&str>,
) -> Result<Option<Option<audio::AudioFormat>>, AppError> {
    use audio::AudioFormat;
    if no_audio {
        if audio_format.is_some() {
            return Err(AppError::usage("--audio-format conflicts with --no-audio"));
        }
        return Ok(None);
    }
    let fmt = match audio_format {
        None => return Ok(Some(None)), // raw native copy
        Some(s) => AudioFormat::from_cli(s)
            .ok_or_else(|| AppError::usage(format!("unknown audio format `{s}` (use m4a, wav)")))?,
    };
    if fmt == AudioFormat::Amr {
        return Err(AppError::usage("voice-memos supports --audio-format m4a or wav, not amr"));
    }
    if fmt.needs_ffmpeg() && !audio::ffmpeg_available() {
        return Err(AppError::other(format!(
            "audio format `{}` requires ffmpeg, which was not found on PATH; \
             install ffmpeg or omit --audio-format to keep native copies",
            fmt.extension()
        )));
    }
    Ok(Some(Some(fmt)))
}

fn run_voice_memos(
    cli: &Cli,
    password: Option<&str>,
    format: &str,
    no_audio: bool,
    audio_format: Option<&str>,
) -> Result<serde_json::Value, AppError> {
    let format = voice_memos_format(format)?;
    let audio_choice = resolve_vm_audio(no_audio, audio_format)?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export voice memos"))?;

    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_voice_memos(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "voice-memos", "count": 0, "outputs": [],
            "note": "this backup has no voice memos", "device": device
        }));
    };

    // Extract audio (unless --no-audio) before rendering so `audio_file` is set.
    let vm_summary = match audio_choice {
        Some(fmt) => Some(
            voice_memos::extract_voice_memos(&backup, &mut items, out, fmt)
                .map_err(|e| AppError::other(e.to_string()))?,
        ),
        None => None,
    };

    let rendered = match format {
        Format::Csv => format::voice_memos_csv(&items),
        Format::Json => format::voice_memos_json(&items),
        Format::Html | Format::Pdf => format::voice_memos_html(&items),
        Format::Vcf => unreachable!("voice_memos_format rejects vcf"),
    };
    let out_file = out.join(format!("voice-memos.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} voice memo(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "voice-memos", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = vm_summary {
        eprintln!(
            "Extracted {} recording(s) ({} missing) to {}/{}",
            s.extracted, s.missing, out.display(), s.dir
        );
        envelope["audio"] = serde_json::json!({
            "format": s.format, "dir": s.dir, "extracted": s.extracted, "missing": s.missing
        });
    }
    Ok(envelope)
}

/// Validate a csv/json/html export format (rejecting vcf) for `command`.
fn export_format(format: &str, command: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown {command} format `{format}` (use csv, json, html, pdf)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage(format!("vcf is not a valid format for {command} (use csv, json, html, pdf)")));
    }
    Ok(f)
}

// Write `rendered` to `out_file`. For PDF, `rendered` is the HTML the `Html`
// variant produces: it is written to a temp file *inside the output dir* (so any
// relative media siblings resolve under a `file://` root), printed to the `.pdf`
// via a headless browser, then removed. A missing browser is a usage error.
fn write_or_pdf(
    out_file: &std::path::Path,
    rendered: &str,
    format: Format,
    chrome: Option<&std::path::Path>,
) -> Result<(), AppError> {
    if format != Format::Pdf {
        return std::fs::write(out_file, rendered).map_err(|e| AppError::other(e.to_string()));
    }
    let parent = out_file.parent().unwrap_or_else(|| std::path::Path::new("."));
    let stem = out_file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    let tmp_html = parent.join(format!(".{stem}.pdf.html"));
    std::fs::write(&tmp_html, rendered).map_err(|e| AppError::other(e.to_string()))?;
    let outcome = match pdf::resolve_browser(chrome) {
        Some(browser) => {
            eprintln!("Rendering PDF with {} …", browser.display());
            pdf::html_to_pdf(&browser, &tmp_html, out_file).map_err(|e| AppError::other(e.to_string()))
        }
        None => Err(AppError::usage(
            "no headless browser found (install Chrome/Chromium/Edge, or pass --chrome-path <PATH>)",
        )),
    };
    let _ = std::fs::remove_file(&tmp_html);
    outcome
}

/// Write the per-folder `<type>-summary.md` next to a just-exported file and
/// return its path (for the command envelope's `outputs`). Markdown is
/// dependency-free, so this never needs a browser and never fails the export.
fn write_type_summary(
    out: &std::path::Path,
    device: &archive_core::DeviceInfo,
    s: &summary::Summary,
) -> Result<String, AppError> {
    let generated = chrono::Utc::now().to_rfc3339();
    let mut outputs: Vec<std::path::PathBuf> = Vec::new();
    summary::write_summary_md(out, &generated, device, s, &mut outputs)
        .map_err(|e| AppError::other(e.to_string()))?;
    Ok(outputs[0].to_string_lossy().into_owned())
}

/// Fetch + parse a single SQLite store into memory via a secure auto-cleaned temp
/// dir, returning `Ok(None)` when the file is absent. `parse` maps the on-disk DB
/// to records.
fn load_store<T>(
    backup: &archive_core::Backup,
    domain: &str,
    rel_path: &str,
    file_name: &str,
    parse: impl FnOnce(&std::path::Path) -> rusqlite::Result<Vec<T>>,
) -> Result<Option<Vec<T>>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join(file_name);
    let Some(db) = backup.fetch(domain, rel_path, &tmp).map_err(|e| AppError::other(e.to_string()))? else {
        return Ok(None);
    };
    Ok(Some(parse(&db).map_err(|e| AppError::other(e.to_string()))?))
}

fn load_safari_history(backup: &archive_core::Backup) -> Result<Option<Vec<safari::HistoryVisit>>, AppError> {
    load_store(backup, "AppDomain-com.apple.mobilesafari", "Library/Safari/History.db", "History.db", safari::parse_history)
}

fn load_safari_bookmarks(backup: &archive_core::Backup) -> Result<Option<Vec<safari::Bookmark>>, AppError> {
    load_store(backup, "AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db", "Bookmarks.db", safari::parse_bookmarks)
}

fn load_calendar(backup: &archive_core::Backup) -> Result<Option<Vec<calendar::CalendarEvent>>, AppError> {
    load_store(backup, "HomeDomain", "Library/Calendar/Calendar.sqlitedb", "Calendar.sqlitedb", calendar::parse)
}

fn load_notes(backup: &archive_core::Backup) -> Result<Option<Vec<notes::Note>>, AppError> {
    load_store(backup, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite", "NoteStore.sqlite", notes::parse)
}

fn load_photos(backup: &archive_core::Backup) -> Result<Option<Vec<photos::Photo>>, AppError> {
    load_store(backup, "CameraRollDomain", "Media/PhotoData/Photos.sqlite", "Photos.sqlite", photos::parse)
}

fn load_attachments(backup: &archive_core::Backup) -> Result<Option<Vec<attachments::Attachment>>, AppError> {
    load_store(backup, "HomeDomain", "Library/SMS/sms.db", "sms.db", attachments::parse)
}

fn load_whatsapp(backup: &archive_core::Backup) -> Result<Option<Vec<whatsapp::WaMessage>>, AppError> {
    load_store(
        backup,
        "AppDomainGroup-group.net.whatsapp.WhatsApp.shared",
        "ChatStorage.sqlite",
        "ChatStorage.sqlite",
        whatsapp::parse,
    )
}

// Health's `parse` returns a `HealthData` struct (workouts + quantity summary),
// not a `Vec`, so it cannot use `load_store`; fetch the secure DB and parse it.
fn load_health(backup: &archive_core::Backup) -> Result<Option<health::HealthData>, AppError> {
    let (domain, rel) = health::DB_LOCATIONS[0];
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("healthdb_secure.sqlite");
    let Some(db) = backup.fetch(domain, rel, &tmp).map_err(|e| AppError::other(e.to_string()))? else {
        return Ok(None);
    };
    Ok(Some(health::parse(&db).map_err(|e| AppError::other(e.to_string()))?))
}

// Reminders lives in a Core Data store whose filename carries a dynamic UUID, so
// the path is discovered from the manifest (not a fixed location).
fn load_reminders(backup: &archive_core::Backup) -> Result<Option<Vec<reminders::Reminder>>, AppError> {
    let paths = backup.list(reminders::DOMAIN, "").map_err(|e| AppError::other(e.to_string()))?;
    let Some(rel) = paths.into_iter().find(|p| reminders::is_store_path(p)) else {
        return Ok(None);
    };
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let tmp = scratch.path().join("reminders.sqlite");
    let Some(db) = backup.fetch(reminders::DOMAIN, &rel, &tmp).map_err(|e| AppError::other(e.to_string()))? else {
        return Ok(None);
    };
    Ok(Some(reminders::parse(&db).map_err(|e| AppError::other(e.to_string()))?))
}

// Mail is file-based (`.emlx`), not a single DB: enumerate every `.emlx` under
// MailDomain, decrypt each, and parse it. `Ok(None)` when no `.emlx` exists (the
// common case on iOS, which backs up mail only for local/POP3 mailboxes).
fn load_mail(backup: &archive_core::Backup) -> Result<Option<Vec<mail::MailMessage>>, AppError> {
    let paths = backup.list(mail::MAIL_DOMAIN, "").map_err(|e| AppError::other(e.to_string()))?;
    let emlx: Vec<String> = paths.into_iter().filter(|p| p.to_lowercase().ends_with(".emlx")).collect();
    if emlx.is_empty() {
        return Ok(None);
    }
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut messages = Vec::new();
    for (i, rel) in emlx.iter().enumerate() {
        let tmp = scratch.path().join(format!("m{i}.emlx"));
        if let Some(p) = backup.fetch(mail::MAIL_DOMAIN, rel, &tmp).map_err(|e| AppError::other(e.to_string()))? {
            let bytes = std::fs::read(&p).map_err(|e| AppError::other(e.to_string()))?;
            if let Some(m) = mail::parse_emlx(&bytes) {
                messages.push(m);
            }
        }
    }
    // `.emlx` files existed but none yielded a message (corrupt/unsupported):
    // treat as "no mail" so the caller emits the clear absent-store note rather
    // than writing a zero-row file.
    if messages.is_empty() {
        return Ok(None);
    }
    Ok(Some(messages))
}

fn run_safari_history(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "safari-history")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export Safari history"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_safari_history(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "safari-history", "count": 0, "outputs": [],
            "note": "this backup has no Safari history", "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::safari_history_csv(&items),
        Format::Json => format::safari_history_json(&items),
        Format::Html | Format::Pdf => format::safari_history_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("safari-history.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} history visit(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "safari-history", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_safari_bookmarks(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "safari-bookmarks")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export Safari bookmarks"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_safari_bookmarks(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "safari-bookmarks", "count": 0, "outputs": [],
            "note": "this backup has no Safari bookmarks", "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::safari_bookmarks_csv(&items),
        Format::Json => format::safari_bookmarks_json(&items),
        Format::Html | Format::Pdf => format::safari_bookmarks_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("safari-bookmarks.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} bookmark(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "safari-bookmarks", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_calendar(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "calendar")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export calendar"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_calendar(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "calendar", "count": 0, "outputs": [],
            "note": "this backup has no calendar", "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::calendar_csv(&items),
        Format::Json => format::calendar_json(&items),
        Format::Html | Format::Pdf => format::calendar_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("calendar.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} event(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "calendar", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_reminders(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "reminders")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export reminders"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_reminders(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "reminders", "count": 0, "outputs": [],
            "note": "this backup has no reminders", "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::reminders_csv(&items),
        Format::Json => format::reminders_json(&items),
        Format::Html | Format::Pdf => format::reminders_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("reminders.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} reminder(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "reminders", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_mail(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "mail")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export mail"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_mail(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "mail", "count": 0, "outputs": [],
            "note": "this backup has no mail (iOS backs up mail only for local/POP3 mailboxes)",
            "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::mail_csv(&items),
        Format::Json => format::mail_json(&items),
        Format::Html | Format::Pdf => format::mail_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("mail.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} mail message(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "mail", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Installed apps come from the backup manifest (per-app domains), not a data
// store, so this command is standalone: it is not in `KNOWN_STORES` (inspect)
// and not part of the `recover` package.
fn run_apps(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "apps")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export apps"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let apps = backup.app_bundle_ids().map_err(|e| AppError::other(e.to_string()))?;
    let rendered = match format {
        Format::Csv => format::apps_csv(&apps),
        Format::Json => format::apps_json(&apps),
        Format::Html | Format::Pdf => format::apps_html(&apps),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("apps.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} installed app(s) to {}", apps.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "apps", "count": apps.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Carve one store's database (+ its `-wal` sidecar) for deleted rows. `Ok(empty)`
// when the database is absent from the backup. The WAL often holds the freshest
// pre-deletion page images; an absent sidecar is fine.
fn carve_store(
    backup: &archive_core::Backup,
    domain: &str,
    db_rel: &str,
    store: &str,
) -> Result<Vec<recover_deleted::DeletedRecord>, AppError> {
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let main_tmp = scratch.path().join("db.sqlite");
    let Some(main_path) = backup
        .fetch(domain, db_rel, &main_tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    else {
        return Ok(Vec::new());
    };
    let main_bytes = std::fs::read(&main_path).map_err(|e| AppError::other(e.to_string()))?;
    let wal_tmp = scratch.path().join("db.sqlite-wal");
    let wal_rel = format!("{db_rel}-wal");
    let wal_bytes = match backup
        .fetch(domain, &wal_rel, &wal_tmp)
        .map_err(|e| AppError::other(e.to_string()))?
    {
        Some(p) => Some(std::fs::read(&p).map_err(|e| AppError::other(e.to_string()))?),
        None => None,
    };
    let carved = archive_core::carve::carve_sqlite(&main_bytes, wal_bytes.as_deref());
    // Exclude rows still live in the current DB (esp. live cells captured in WAL
    // frame images). Open the temp copy read-write so its -wal is applied,
    // yielding the true current state to compare against.
    let live = live_keys(&main_path, store);
    Ok(recover_deleted::recover(store, &carved, &live))
}

/// Read the keys of rows still LIVE in `db_path` for `store`, used to reject
/// carved candidates that are not actually deleted. Best-effort: any open/query
/// failure (schema drift, locked db) yields empty keys (exclude nothing).
fn live_keys(db_path: &std::path::Path, store: &str) -> recover_deleted::LiveKeys {
    let mut keys = recover_deleted::LiveKeys::default();
    let Ok(conn) = rusqlite::Connection::open(db_path) else {
        return keys;
    };
    // Best-effort: a failed query (schema drift) leaves that key set empty.
    let _ = match store {
        "messages" => message_live_keys(&conn, &mut keys),
        "calls" => rowid_live_keys(&conn, "SELECT Z_PK FROM ZCALLRECORD", &mut keys.rowids),
        "contacts" => rowid_live_keys(&conn, "SELECT ROWID FROM ABPerson", &mut keys.rowids),
        "notes" => rowid_live_keys(&conn, "SELECT Z_PK FROM ZICCLOUDSYNCINGOBJECT", &mut keys.rowids),
        "calendar" => rowid_live_keys(&conn, "SELECT ROWID FROM CalendarItem", &mut keys.rowids),
        "safari" => safari_live_keys(&conn, &mut keys),
        "photos" => {
            let table = photos_live_table(&conn);
            rowid_live_keys(&conn, &format!("SELECT Z_PK FROM {table}"), &mut keys.rowids)
        }
        _ => Ok(()),
    };
    keys
}

fn message_live_keys(conn: &rusqlite::Connection, keys: &mut recover_deleted::LiveKeys) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare("SELECT ROWID, guid FROM message")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)))?;
    for (rowid, guid) in rows.flatten() {
        keys.rowids.insert(rowid);
        if let Some(g) = guid {
            keys.guids.insert(g);
        }
    }
    Ok(())
}

/// The Camera Roll asset table present in `conn`: `ZASSET` on iOS 13+,
/// `ZGENERICASSET` on iOS ≤12 (renamed in iOS 13). Falls back to the canonical
/// `ZASSET` so a backup missing both still yields a best-effort (empty) result.
/// Shares the rename knowledge with `schema_check::table_candidates`.
fn photos_live_table(conn: &rusqlite::Connection) -> &'static str {
    schema_check::table_candidates("ZASSET")
        .into_iter()
        .find(|t| {
            conn.query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", [*t], |_| Ok(()))
                .is_ok()
        })
        .unwrap_or("ZASSET")
}

fn rowid_live_keys(conn: &rusqlite::Connection, sql: &str, out: &mut std::collections::HashSet<i64>) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
    for id in rows.flatten() {
        out.insert(id);
    }
    Ok(())
}

/// Live Safari keys: `history_visits` rowids plus `history_items` URLs. Safari
/// recovers URL-only rows (from `history_items`, a separate rowid space), so a
/// live row is excluded by its URL rather than by rowid.
fn safari_live_keys(conn: &rusqlite::Connection, keys: &mut recover_deleted::LiveKeys) -> rusqlite::Result<()> {
    rowid_live_keys(conn, "SELECT ROWID FROM history_visits", &mut keys.rowids)?;
    let mut stmt = conn.prepare("SELECT url FROM history_items")?;
    let rows = stmt.query_map([], |r| r.get::<_, Option<String>>(0))?;
    for u in rows.flatten().flatten() {
        keys.urls.insert(u);
    }
    Ok(())
}

/// (store, domain, db relative path) for each carvable store.
const CARVE_STORES: &[(&str, &str, &str)] = &[
    ("messages", "HomeDomain", "Library/SMS/sms.db"),
    ("calls", "HomeDomain", "Library/CallHistoryDB/CallHistory.storedata"),
    ("contacts", "HomeDomain", "Library/AddressBook/AddressBook.sqlitedb"),
    ("notes", "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite"),
    ("calendar", "HomeDomain", "Library/Calendar/Calendar.sqlitedb"),
    ("safari", "AppDomain-com.apple.mobilesafari", "Library/Safari/History.db"),
    ("photos", "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
];

fn run_recover_deleted(cli: &Cli, password: Option<&str>, format: &str, store: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "recover-deleted")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export recovered records"))?;
    // Resolve --store before opening the backup so a bad value is a clean usage error.
    let selected: Vec<&(&str, &str, &str)> = match store {
        "all" => CARVE_STORES.iter().collect(),
        s => match CARVE_STORES.iter().find(|(name, ..)| *name == s) {
            Some(entry) => vec![entry],
            None => return Err(AppError::usage(format!("unknown store `{s}` (use messages, calls, contacts, notes, calendar, safari, photos, all)"))),
        },
    };
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let mut all: Vec<recover_deleted::DeletedRecord> = Vec::new();
    let mut per_store: Vec<serde_json::Value> = Vec::new();
    for (name, domain, rel) in selected {
        let recovered = carve_store(&backup, domain, rel, name)?;
        eprintln!("recover-deleted: {name}: {} candidate(s)", recovered.len());
        per_store.push(serde_json::json!({ "store": name, "recovered": recovered.len() }));
        all.extend(recovered);
    }
    // Chronological view: dated rows first (ascending), then undated; ties by store.
    all.sort_by(|a, b| match (a.date.as_deref(), b.date.as_deref()) {
        (Some(x), Some(y)) => x.cmp(y).then_with(|| a.store.cmp(&b.store)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.store.cmp(&b.store),
    });

    let rendered = match format {
        Format::Csv => format::deleted_csv(&all),
        Format::Json => format::deleted_json(&all),
        Format::Html | Format::Pdf => format::deleted_html(&all),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("deleted.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} recovered record(s) to {}", all.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "recover-deleted", "count": all.len(),
        "stores": per_store, "outputs": [out_file.to_string_lossy()], "device": device,
        "note": "best-effort: recovered deleted rows are partial (limited by SQLite page reuse/VACUUM) and may include false positives"
    }))
}

// Check each known SQLite store's live schema against the columns its extractor
// depends on. Read-only; resolves each store's DB from the manifest, opens it
// read-only, and compares column sets. Never logs any data — only schema names.
fn run_schema_check(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    use rusqlite::{Connection, OpenFlags};

    let format = export_format(format, "schema-check")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export the schema-check report"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let mut reports: Vec<schema_check::StoreReport> = Vec::new();
    for store in schema_check::EXPECTATIONS {
        let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
        // Try each candidate (domain, path) in order; check the first one present.
        let mut matched: Option<(&str, &str, std::path::PathBuf)> = None;
        for (domain, rel_path) in store.locations {
            let file_name = rel_path.rsplit('/').next().unwrap_or("store.db");
            let dest = scratch.path().join(file_name);
            if let Some(path) = backup.fetch(domain, rel_path, &dest).map_err(|e| AppError::other(e.to_string()))? {
                matched = Some((domain, rel_path, path));
                break;
            }
        }
        // Report the matched location, or the first candidate when none is present.
        let (domain, rel_path) = match &matched {
            Some((d, r, _)) => (*d, *r),
            None => store.locations.first().map(|(d, r)| (*d, *r)).unwrap_or(("", "")),
        };
        let (status, tables) = match matched.as_ref().map(|(_, _, p)| p) {
            None => ("db_absent", Vec::new()),
            Some(path) => {
                // Read-only: never modifies the extracted copy. A DB that fails to
                // open (corrupt/locked) is reported with every table absent rather
                // than aborting the whole report.
                match Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
                    Ok(conn) => {
                        let tables: Vec<schema_check::TableReport> = store
                            .needs
                            .iter()
                            .map(|need| {
                                // Try the table's known cross-iOS names (e.g. photos'
                                // ZASSET ⇄ ZGENERICASSET); the first present one wins.
                                // PRAGMA returns an empty set for a missing table.
                                let cols = schema_check::table_candidates(need.table)
                                    .into_iter()
                                    .find_map(|t| sqlite_util::table_columns(&conn, t).ok().filter(|c| !c.is_empty()));
                                schema_check::check_table(need, cols.as_ref())
                            })
                            .collect();
                        (schema_check::store_status(&tables), tables)
                    }
                    Err(_) => {
                        let tables = store
                            .needs
                            .iter()
                            .map(|need| schema_check::check_table(need, None))
                            .collect();
                        ("drifted", tables)
                    }
                }
            }
        };
        reports.push(schema_check::StoreReport {
            command: store.command.into(),
            domain: domain.into(),
            rel_path: rel_path.into(),
            status,
            tables,
        });
    }

    let ok = reports.iter().filter(|r| r.status == "ok").count();
    let drifted = reports.iter().filter(|r| r.status == "drifted").count();
    let absent = reports.iter().filter(|r| r.status == "db_absent").count();

    let rendered = match format {
        Format::Csv => format::schema_check_csv(&reports),
        Format::Json => format::schema_check_json(&reports),
        Format::Html | Format::Pdf => format::schema_check_html(&reports),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("schema-check.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("schema-check: {ok} ok, {drifted} drifted, {absent} db-absent ({} checked)", reports.len());

    Ok(serde_json::json!({
        "ok": true, "command": "schema-check", "checked": reports.len(),
        "ok_stores": ok, "drifted": drifted, "db_absent": absent,
        "stores": reports, "outputs": [out_file.to_string_lossy()], "device": device,
        "note": "drift = an expected column/table is missing from the live schema (a renamed/removed column makes that extractor silently return empty); db_absent just means the database is not in this backup"
    }))
}

// Recover saved Wi-Fi passwords from the keychain. Encrypted backups only (an
// unencrypted backup does not include the keychain). Sensitive: the passwords are
// plaintext and are never logged to stderr — only a count is printed.
fn run_wifi(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "wifi")?;
    // PDF is intentionally not offered for this sensitive export: the shared PDF
    // path writes a temporary plaintext HTML sidecar next to the output before
    // rendering, which we will not do for plaintext passwords.
    if format == Format::Pdf {
        return Err(AppError::usage(
            "pdf is not available for wifi (it would write a temporary plaintext HTML file); use csv, json, or html",
        ));
    }
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export Wi-Fi credentials"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let creds = backup.wifi_credentials().map_err(|e| AppError::other(e.to_string()))?;
    if creds.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "wifi", "count": 0, "outputs": [],
            "note": "no Wi-Fi credentials recovered (the keychain is included only in ENCRYPTED backups; this backup may be unencrypted, have no saved networks, or use an unsupported keychain format)",
            "device": device
        }));
    }
    let rendered = match format {
        Format::Csv => format::wifi_csv(&creds),
        Format::Json => format::wifi_json(&creds),
        Format::Html => format::wifi_html(&creds),
        Format::Vcf | Format::Pdf => unreachable!("vcf rejected by export_format; pdf rejected above"),
    };
    let out_file = out.join(format!("wifi.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    // Count only — never log the recovered passwords.
    eprintln!("Recovered {} Wi-Fi network(s) to {}", creds.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "wifi", "count": creds.len(),
        "outputs": [out_file.to_string_lossy()], "device": device,
        "note": "recovered Wi-Fi passwords are plaintext — handle and transmit securely"
    }))
}

fn run_passwords(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "passwords")?;
    // PDF is intentionally not offered: the shared PDF path writes a temporary
    // plaintext HTML sidecar, which we will not do for plaintext passwords.
    if format == Format::Pdf {
        return Err(AppError::usage(
            "pdf is not available for passwords (it would write a temporary plaintext HTML file); use csv, json, or html",
        ));
    }
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export passwords"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let creds = backup.saved_passwords().map_err(|e| AppError::other(e.to_string()))?;
    if creds.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "passwords", "count": 0, "outputs": [],
            "note": "no saved passwords recovered (the keychain is included only in ENCRYPTED backups; this backup may be unencrypted, have no saved website/app logins, or only ThisDeviceOnly items that are not transferable)",
            "device": device
        }));
    }
    let rendered = match format {
        Format::Csv => format::passwords_csv(&creds),
        Format::Json => format::passwords_json(&creds),
        Format::Html => format::passwords_html(&creds),
        Format::Vcf | Format::Pdf => unreachable!("vcf rejected by export_format; pdf rejected above"),
    };
    let out_file = out.join(format!("passwords.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    // Count only — never log the recovered passwords.
    eprintln!("Recovered {} saved password(s) to {}", creds.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "passwords", "count": creds.len(),
        "outputs": [out_file.to_string_lossy()], "device": device,
        "note": "recovered passwords are plaintext — handle and transmit securely"
    }))
}

fn run_keychain_inventory(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    // Not sensitive: this census carries no secrets, so pdf is allowed.
    let format = export_format(format, "keychain-inventory")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export the keychain inventory"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let items = backup.keychain_inventory().map_err(|e| AppError::other(e.to_string()))?;
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "keychain-inventory", "count": 0, "outputs": [],
            "note": "no keychain items (the keychain is included only in ENCRYPTED backups)",
            "device": device
        }));
    }
    // Per-array summary: total items and how many decrypted (transferable).
    let mut summary = serde_json::Map::new();
    for arr in ["genp", "inet", "cert", "keys"] {
        let total = items.iter().filter(|m| m.array == arr).count();
        if total == 0 {
            continue;
        }
        let decrypted = items.iter().filter(|m| m.array == arr && m.decrypted).count();
        summary.insert(arr.to_string(), serde_json::json!({ "total": total, "decrypted": decrypted }));
    }
    let rendered = match format {
        Format::Csv => format::keychain_inventory_csv(&items),
        Format::Json => format::keychain_inventory_json(&items),
        Format::Html | Format::Pdf => format::keychain_inventory_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("keychain-inventory.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote keychain inventory ({} item(s)) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "keychain-inventory", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "summary": summary, "device": device
    }))
}

fn run_certificates(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    // Certificates are public (no secret key material), so pdf is allowed.
    let format = export_format(format, "certificates")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export certificates"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let items = backup.certificates().map_err(|e| AppError::other(e.to_string()))?;
    if items.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "certificates", "count": 0, "outputs": [],
            "note": "no certificates recovered (the keychain is included only in ENCRYPTED backups; certificates are commonly stored under a ThisDeviceOnly protection class whose keys are not transferable in a portable backup, so they cannot be decrypted)",
            "device": device
        }));
    }

    let infos: Vec<certificates::CertificateInfo> = items.iter().map(certificates::describe).collect();
    // Always write the PEM bundle alongside the metadata table.
    let pem_file = out.join("certificates.pem");
    std::fs::write(&pem_file, certificates::to_pem_bundle(&items)).map_err(|e| AppError::other(e.to_string()))?;

    let rendered = match format {
        Format::Csv => format::certificates_csv(&infos),
        Format::Json => format::certificates_json(&infos),
        Format::Html | Format::Pdf => format::certificates_html(&infos),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let meta_file = out.join(format!("certificates.{}", format.extension()));
    write_or_pdf(&meta_file, &rendered, format, cli.chrome_path.as_deref())?;
    let identities = infos.iter().filter(|c| c.has_private_key).count();
    eprintln!("Recovered {} certificate(s) ({identities} with a private key) to {} (+ certificates.pem)", infos.len(), meta_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "certificates", "count": infos.len(), "identities": identities,
        "outputs": [meta_file.to_string_lossy(), pem_file.to_string_lossy()], "device": device
    }))
}

fn run_vpn_creds(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "vpn-creds")?;
    // Sensitive (plaintext secrets): the shared PDF path writes a temporary
    // plaintext HTML sidecar, which we will not do here.
    if format == Format::Pdf {
        return Err(AppError::usage(
            "pdf is not available for vpn-creds (it would write a temporary plaintext HTML file); use csv, json, or html",
        ));
    }
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export VPN credentials"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let creds = backup.network_credentials().map_err(|e| AppError::other(e.to_string()))?;
    if creds.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "vpn-creds", "count": 0, "outputs": [],
            "note": "no VPN / enterprise-Wi-Fi credentials recovered (the keychain is included only in ENCRYPTED backups; this device may have none, or use markers this best-effort detector does not recognize — personal Wi-Fi PSKs are recovered by `wifi`)",
            "device": device
        }));
    }
    let rendered = match format {
        Format::Csv => format::vpn_creds_csv(&creds),
        Format::Json => format::vpn_creds_json(&creds),
        Format::Html => format::vpn_creds_html(&creds),
        Format::Vcf | Format::Pdf => unreachable!("vcf rejected by export_format; pdf rejected above"),
    };
    let out_file = out.join(format!("vpn-creds.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    // Count only — never log the recovered secrets.
    let (vpn, eap) = (creds.iter().filter(|c| c.kind == "vpn").count(), creds.iter().filter(|c| c.kind == "eap").count());
    eprintln!("Recovered {} network credential(s) ({vpn} vpn, {eap} eap) to {}", creds.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "vpn-creds", "count": creds.len(), "vpn": vpn, "eap": eap,
        "outputs": [out_file.to_string_lossy()], "device": device,
        "note": "recovered secrets are plaintext — handle and transmit securely"
    }))
}

// Merge every in-process extractor into one chronological timeline. Like
// `recover`, each store is best-effort (absent/unreadable is logged and skipped).
// A view over the other extractors, not a data store: standalone (not in
// `inspect`/`recover`). Conversation text (`messages`) and the app inventory are
// not event streams and are excluded.
/// Collect raw timeline events from every in-process extractor that has dates.
/// Shared by `timeline` (which finalizes/sorts them) and `stats` (which
/// aggregates them). An unreadable store is logged and skipped, never fatal.
fn collect_timeline_events(backup: &archive_core::Backup) -> Vec<timeline::Event> {
    let mut events: Vec<timeline::Event> = Vec::new();
    // Resolve handles to contact names once, so call/voicemail/WhatsApp events
    // carry names instead of bare numbers when the address book is available.
    let idx = contact_index(backup);
    if let Some(mut v) = opt_or_log(load_calls(backup), "calls") {
        if let Some(idx) = &idx {
            enrich::enrich_calls(idx, &mut v);
        }
        events.extend(timeline::from_calls(&v));
    }
    if let Some(v) = opt_or_log(load_accounts(backup), "accounts") {
        events.extend(timeline::from_accounts(&v));
    }
    if let Some(mut v) = opt_or_log(load_voicemail(backup), "voicemail") {
        if let Some(idx) = &idx {
            enrich::enrich_voicemail(idx, &mut v);
        }
        events.extend(timeline::from_voicemail(&v));
    }
    if let Some(v) = opt_or_log(load_voice_memos(backup), "voice-memos") {
        events.extend(timeline::from_voice_memos(&v));
    }
    if let Some(v) = opt_or_log(load_safari_history(backup), "safari-history") {
        events.extend(timeline::from_safari_history(&v));
    }
    if let Some(v) = opt_or_log(load_calendar(backup), "calendar") {
        events.extend(timeline::from_calendar(&v));
    }
    if let Some(v) = opt_or_log(load_notes(backup), "notes") {
        events.extend(timeline::from_notes(&v));
    }
    if let Some(v) = opt_or_log(load_photos(backup), "photos") {
        events.extend(timeline::from_photos(&v));
        events.extend(timeline::from_deleted(&v));
    }
    if let Some(v) = opt_or_log(load_attachments(backup), "attachments") {
        events.extend(timeline::from_attachments(&v));
    }
    if let Some(mut v) = opt_or_log(load_whatsapp(backup), "whatsapp") {
        if let Some(idx) = &idx {
            enrich::enrich_whatsapp(idx, &mut v);
        }
        events.extend(timeline::from_whatsapp(&v));
    }
    if let Some(v) = opt_or_log(load_reminders(backup), "reminders") {
        events.extend(timeline::from_reminders(&v));
    }
    if let Some(d) = opt_or_log(load_health(backup), "health") {
        events.extend(timeline::from_workouts(&d.workouts));
    }
    if let Some(v) = opt_or_log(load_mail(backup), "mail") {
        events.extend(timeline::from_mail(&v));
    }
    events
}

/// Build the case-file search corpus. Mirrors `collect_timeline_events`, but keeps
/// each record's human snippet separate from the text it is matched against: for
/// the handle-bearing stores (calls, voicemail, WhatsApp) the snippet shows the
/// resolved contact name while the searchable text also folds in the raw
/// number/JID, so a phone-number query still finds an enriched-to-a-name record.
fn collect_search_records(backup: &archive_core::Backup) -> Vec<search::SearchRecord> {
    let idx = contact_index(backup);
    let mut recs: Vec<search::SearchRecord> = Vec::new();
    let mut simple = |events: Vec<timeline::Event>| {
        recs.extend(events.iter().map(search::SearchRecord::from_event));
    };
    // Non-handle stores: searchable text is just the summary.
    if let Some(v) = opt_or_log(load_accounts(backup), "accounts") {
        simple(timeline::from_accounts(&v));
    }
    if let Some(v) = opt_or_log(load_voice_memos(backup), "voice-memos") {
        simple(timeline::from_voice_memos(&v));
    }
    if let Some(v) = opt_or_log(load_safari_history(backup), "safari-history") {
        simple(timeline::from_safari_history(&v));
    }
    if let Some(v) = opt_or_log(load_calendar(backup), "calendar") {
        simple(timeline::from_calendar(&v));
    }
    if let Some(v) = opt_or_log(load_notes(backup), "notes") {
        simple(timeline::from_notes(&v));
    }
    if let Some(v) = opt_or_log(load_photos(backup), "photos") {
        simple(timeline::from_photos(&v));
        simple(timeline::from_deleted(&v));
    }
    if let Some(v) = opt_or_log(load_attachments(backup), "attachments") {
        simple(timeline::from_attachments(&v));
    }
    if let Some(v) = opt_or_log(load_reminders(backup), "reminders") {
        simple(timeline::from_reminders(&v));
    }
    if let Some(d) = opt_or_log(load_health(backup), "health") {
        simple(timeline::from_workouts(&d.workouts));
    }
    if let Some(v) = opt_or_log(load_mail(backup), "mail") {
        simple(timeline::from_mail(&v));
    }
    // Handle-bearing stores: fold the raw number/JID into the searchable text.
    if let Some(mut v) = opt_or_log(load_calls(backup), "calls") {
        if let Some(idx) = &idx {
            enrich::enrich_calls(idx, &mut v);
        }
        let events = timeline::from_calls(&v);
        recs.extend(v.iter().zip(events.iter()).map(|(c, e)| search::SearchRecord::with_extra(e, &c.number)));
    }
    if let Some(mut v) = opt_or_log(load_voicemail(backup), "voicemail") {
        if let Some(idx) = &idx {
            enrich::enrich_voicemail(idx, &mut v);
        }
        let events = timeline::from_voicemail(&v);
        recs.extend(v.iter().zip(events.iter()).map(|(vm, e)| search::SearchRecord::with_extra(e, &vm.sender)));
    }
    if let Some(mut v) = opt_or_log(load_whatsapp(backup), "whatsapp") {
        if let Some(idx) = &idx {
            enrich::enrich_whatsapp(idx, &mut v);
        }
        let events = timeline::from_whatsapp(&v);
        // Fold in both the sender JID (incoming) and the chat JID (identifies the
        // conversation for own messages, where `sender` is empty).
        recs.extend(v.iter().zip(events.iter()).map(|(m, e)| {
            search::SearchRecord::with_extra(e, &format!("{} {}", m.sender, m.chat_jid))
        }));
    }
    recs
}

fn run_stats(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "stats")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export stats"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let stats = stats::summarize(&collect_timeline_events(&backup));
    let rendered = match format {
        Format::Csv => format::stats_csv(&stats),
        Format::Json => format::stats_json(&stats),
        Format::Html | Format::Pdf => format::stats_html(&stats),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("stats.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!(
        "Wrote stats for {} event(s) across {} categor(ies) to {}",
        stats.total_events,
        stats.categories.len(),
        out_file.display()
    );

    Ok(serde_json::json!({
        "ok": true, "command": "stats",
        "total_events": stats.total_events,
        "categories": stats.categories.len(),
        "earliest": stats.earliest, "latest": stats.latest,
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Case-file search across every in-process record + the address book. Read-only.
// Match snippets contain personal data, so they are written only to the output
// file — never logged to stderr (only the count and the user's own query are).
fn run_search(cli: &Cli, password: Option<&str>, query: &str, format: &str, redact: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "search")?;
    if query.trim().is_empty() {
        return Err(AppError::usage("--query must not be empty"));
    }
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export search results"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let records = collect_search_records(&backup);
    let contacts = opt_or_log(load_contacts(&backup), "contacts").unwrap_or_default();
    let hits = search::search(&records, &contacts, query);
    // Matching ran on the raw text; redaction masks identifiers only in the output —
    // the echoed query and the snippets alike, so a redacted report leaks nothing.
    let (display_query, hits) = search::apply_redaction(query, hits, redact);

    let rendered = match format {
        Format::Csv => format::search_csv(&hits),
        Format::Json => format::search_json(&hits),
        Format::Html | Format::Pdf => format::search_html(&hits, &display_query),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("search.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("search: {} match(es) for {display_query:?}", hits.len());

    Ok(serde_json::json!({
        "ok": true, "command": "search", "query": display_query, "matches": hits.len(), "redacted": redact,
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Consolidate the extracted data into one queryable SQLite database. Read-only on
// the backup; writes a single unencrypted archive.sqlite. Personal data goes only
// into that file — stderr/stdout carry only per-table row counts.
fn run_db_export(cli: &Cli, password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required for the SQLite export"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let idx = contact_index(&backup);
    // Same finalized (dated, chronological) timeline the `timeline` command exports.
    let events = timeline::finalize(collect_timeline_events(&backup));
    let contacts = opt_or_log(load_contacts(&backup), "contacts").unwrap_or_default();
    let mut calls = opt_or_log(load_calls(&backup), "calls").unwrap_or_default();
    let mut whatsapp = opt_or_log(load_whatsapp(&backup), "whatsapp").unwrap_or_default();
    if let Some(idx) = &idx {
        enrich::enrich_calls(idx, &mut calls);
        enrich::enrich_whatsapp(idx, &mut whatsapp);
    }

    let out_file = out.join("archive.sqlite");
    let counts = db_export::write(&out_file, &events, &contacts, &calls, &whatsapp)
        .map_err(|e| AppError::other(e.to_string()))?;
    eprintln!(
        "db-export: {} timeline, {} contacts, {} calls, {} whatsapp -> {}",
        counts.timeline,
        counts.contacts,
        counts.calls,
        counts.whatsapp,
        out_file.display()
    );

    Ok(serde_json::json!({
        "ok": true, "command": "db-export",
        "tables": {
            "timeline": counts.timeline, "contacts": counts.contacts,
            "calls": counts.calls, "whatsapp": counts.whatsapp
        },
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Diff two backups at the file (manifest) level. `--backup` is A (older),
// `--against` is B (newer); both are opened with the same `--password`. Read-only
// on both. Reports added/removed/modified files; the size comparison uses manifest
// sizes, so it works for encrypted backups.
fn run_diff(cli: &Cli, password: Option<&str>, against: &std::path::Path, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "diff")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export the diff"))?;
    let backup_a = open_backup(cli, password)?;
    let backup_b = archive_core::Backup::open(against, password).map_err(open_error)?;
    let device = device_json(backup_a.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let entries_a = backup_a.file_entries().map_err(|e| AppError::other(e.to_string()))?;
    let entries_b = backup_b.file_entries().map_err(|e| AppError::other(e.to_string()))?;
    let (summary, changes) = backup_diff::diff(&entries_a, &entries_b);

    let rendered = match format {
        Format::Csv => format::diff_csv(&changes),
        Format::Json => format::diff_json(&changes),
        Format::Html | Format::Pdf => format::diff_html(&changes, &summary),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("backup-diff.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!(
        "diff: +{} added, -{} removed, ~{} modified ({} unchanged) -> {}",
        summary.added,
        summary.removed,
        summary.modified,
        summary.unchanged,
        out_file.display()
    );

    Ok(serde_json::json!({
        "ok": true, "command": "diff",
        "summary": { "added": summary.added, "removed": summary.removed, "modified": summary.modified, "unchanged": summary.unchanged },
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

// Bundle a directory of exports into one AES-256 encrypted zip. Does not open the
// backup — a pure file operation over `--source`. The zip password comes from
// `--zip-password` or `ARCHIVE_ZIP_PASSWORD` and is never logged (only file/byte
// counts are printed).
fn run_package(cli: &Cli, source: &std::path::Path, zip_password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to write the package"))?;
    let pw = zip_password
        .map(str::to_string)
        .or_else(|| std::env::var("ARCHIVE_ZIP_PASSWORD").ok())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::usage("a zip password is required (pass --zip-password or set ARCHIVE_ZIP_PASSWORD)"))?;
    if !source.is_dir() {
        return Err(AppError::usage(format!("--source must be an existing directory: {}", source.display())));
    }
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let zip_path = out.join("archive-package.zip");
    let summary = package::package_dir(source, &zip_path, &pw).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!(
        "package: {} file(s), {} byte(s) -> {} (AES-256 encrypted)",
        summary.files,
        summary.bytes,
        zip_path.display()
    );

    Ok(serde_json::json!({
        "ok": true, "command": "package", "encrypted": true, "cipher": "AES-256",
        "files": summary.files, "bytes": summary.bytes,
        "outputs": [zip_path.to_string_lossy()]
    }))
}

/// Read the first `n` bytes of a file (for magic-byte sniffing), tolerant of
/// short files / open failures.
fn file_head(path: &std::path::Path, n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    match std::fs::File::open(path) {
        Ok(mut f) => {
            let k = f.read(&mut buf).unwrap_or(0);
            buf.truncate(k);
            buf
        }
        Err(_) => Vec::new(),
    }
}

fn run_app_databases(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "app-databases")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export the app-databases report"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    // Scan every third-party app **and app-group** container — the real store
    // often lives in an `AppDomainGroup-*` shared container (e.g. WhatsApp's
    // ChatStorage.sqlite), not the per-app `AppDomain-<bundle>`.
    let domains = backup.domains().map_err(|e| AppError::other(e.to_string()))?;
    let scratch = tempfile::TempDir::new().map_err(|e| AppError::other(e.to_string()))?;
    let mut rows: Vec<app_databases::AppDatabase> = Vec::new();
    for dom in &domains {
        let Some(label) = app_databases::third_party_label(dom) else { continue };
        let files = backup.list(dom, "").map_err(|e| AppError::other(e.to_string()))?;
        for f in files.iter().filter(|f| app_databases::is_db_like(f)) {
            let tmp = scratch.path().join("probe.db");
            match backup.fetch(dom, f, &tmp) {
                Ok(Some(p)) => {
                    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    let readable = app_databases::is_sqlite(&file_head(&p, 16));
                    let tables = if readable { app_databases::count_tables(&p) } else { None };
                    rows.push(app_databases::AppDatabase {
                        app: label.clone(),
                        domain: dom.clone(),
                        path: f.clone(),
                        bytes,
                        readable,
                        tables,
                    });
                    let _ = std::fs::remove_file(&p);
                }
                Ok(None) => {}
                Err(e) => eprintln!("app-databases: fetch {dom}/{f}: {e}"),
            }
        }
    }
    rows.sort_by(|a, b| a.app.cmp(&b.app).then(b.bytes.cmp(&a.bytes)));

    if rows.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "app-databases", "count": 0, "outputs": [],
            "note": "no third-party app database files found in this backup", "device": device
        }));
    }

    let readable = rows.iter().filter(|d| d.readable).count();
    let rendered = match format {
        Format::Csv => format::app_databases_csv(&rows),
        Format::Json => format::app_databases_json(&rows),
        Format::Html | Format::Pdf => format::app_databases_html(&rows),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("app-databases.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} app database(s) ({} readable) to {}", rows.len(), readable, out_file.display());

    Ok(serde_json::json!({
        "ok": true, "command": "app-databases", "count": rows.len(), "readable": readable,
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_app_files(cli: &Cli, password: Option<&str>, app: &str, format: &str, all: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "app-files")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to extract app files"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let app_lc = app.to_lowercase();
    let domains = backup.domains().map_err(|e| AppError::other(e.to_string()))?;
    let matched: Vec<&String> = domains
        .iter()
        .filter(|d| app_databases::third_party_label(d).is_some() && d.to_lowercase().contains(&app_lc))
        .collect();
    if matched.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "app-files", "count": 0, "outputs": [],
            "note": format!("no third-party app domain matched '{app}'"), "device": device
        }));
    }

    let mut manifest: Vec<app_files::ExtractedFile> = Vec::new();
    for dom in &matched {
        let files = backup.list(dom, "").map_err(|e| AppError::other(e.to_string()))?;
        for f in &files {
            if !all && !app_files::is_media(f) {
                continue;
            }
            let Some(rel) = app_files::safe_relpath(f) else { continue };
            let rel_out = format!("app-files/{}/{}", app_files::domain_dir(dom), rel);
            let dest = out.join(&rel_out);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| AppError::other(e.to_string()))?;
            }
            match backup.fetch(dom, f, &dest) {
                Ok(Some(p)) => {
                    let bytes = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                    manifest.push(app_files::ExtractedFile {
                        domain: (*dom).clone(),
                        path: f.clone(),
                        bytes,
                        category: app_files::category(f).to_string(),
                        file: rel_out,
                    });
                }
                Ok(None) => {}
                Err(e) => eprintln!("app-files: fetch {dom}/{f}: {e}"),
            }
        }
    }
    manifest.sort_by(|a, b| a.domain.cmp(&b.domain).then(a.path.cmp(&b.path)));

    if manifest.is_empty() {
        let what = if all { "files" } else { "media files" };
        return Ok(serde_json::json!({
            "ok": true, "command": "app-files", "count": 0, "outputs": [],
            "note": format!("matched {} domain(s) for '{app}' but found no {what}", matched.len()),
            "device": device
        }));
    }

    let total_bytes: u64 = manifest.iter().map(|f| f.bytes).sum();
    let rendered = match format {
        Format::Csv => format::app_files_csv(&manifest),
        Format::Json => format::app_files_json(&manifest),
        Format::Html | Format::Pdf => format::app_files_html(app, &manifest),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("app-files.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!(
        "Extracted {} file(s) ({} bytes) for '{app}' from {} domain(s) to {}/app-files/",
        manifest.len(),
        total_bytes,
        matched.len(),
        out.display()
    );

    Ok(serde_json::json!({
        "ok": true, "command": "app-files", "count": manifest.len(),
        "bytes": total_bytes, "domains": matched.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_timeline(cli: &Cli, password: Option<&str>, format: &str, redact: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "timeline")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export timeline"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let mut events = timeline::finalize(collect_timeline_events(&backup));
    if redact {
        for e in &mut events {
            e.summary = redact::redact_pii(&e.summary);
        }
    }
    let rendered = match format {
        Format::Csv => format::timeline_csv(&events),
        Format::Json => format::timeline_json(&events),
        Format::Html | Format::Pdf => format::timeline_html(&events),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("timeline.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} timeline event(s) to {}", events.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "timeline", "count": events.len(), "redacted": redact,
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_health(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "health")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export health"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(data) = load_health(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "health", "count": 0, "workouts": 0, "quantity_types": 0,
            "outputs": [], "note": "this backup has no Health database", "device": device
        }));
    };
    if data.workouts.is_empty() && data.quantity_summary.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "health", "count": 0, "workouts": 0, "quantity_types": 0,
            "outputs": [], "note": "Health database present but no workouts or known quantity samples",
            "device": device
        }));
    }
    // CSV splits the two heterogeneous tables into separate files; json/html keep
    // the whole HealthData in one document.
    let outputs: Vec<std::path::PathBuf> = match format {
        Format::Csv => {
            let wf = out.join("health-workouts.csv");
            let qf = out.join("health-quantity.csv");
            std::fs::write(&wf, format::health_workouts_csv(&data.workouts))
                .map_err(|e| AppError::other(e.to_string()))?;
            std::fs::write(&qf, format::health_quantity_csv(&data.quantity_summary))
                .map_err(|e| AppError::other(e.to_string()))?;
            vec![wf, qf]
        }
        Format::Json => {
            let f = out.join("health.json");
            std::fs::write(&f, format::health_json(&data)).map_err(|e| AppError::other(e.to_string()))?;
            vec![f]
        }
        Format::Html | Format::Pdf => {
            let f = out.join(format!("health.{}", format.extension()));
            write_or_pdf(&f, &format::health_html(&data), format, cli.chrome_path.as_deref())?;
            vec![f]
        }
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    eprintln!(
        "Wrote {} workout(s) and {} quantity type(s) to {}",
        data.workouts.len(),
        data.quantity_summary.len(),
        out.display()
    );
    let outputs: Vec<String> = outputs.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    Ok(serde_json::json!({
        "ok": true, "command": "health",
        "count": data.workouts.len() + data.quantity_summary.len(),
        "workouts": data.workouts.len(), "quantity_types": data.quantity_summary.len(),
        "outputs": outputs, "device": device
    }))
}

fn run_notes(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "notes")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export notes"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(items) = load_notes(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "notes", "count": 0, "outputs": [],
            "note": "this backup has no notes", "device": device
        }));
    };
    let rendered = match format {
        Format::Csv => format::notes_csv(&items),
        Format::Json => format::notes_json(&items),
        Format::Html | Format::Pdf => format::notes_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("notes.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} note(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "notes", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_photos(cli: &Cli, password: Option<&str>, format: &str, no_files: bool, summary: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "photos")?;
    // Validate the summary/format combo up front, before opening the backup, so a
    // bad invocation fails the same way regardless of what the backup contains.
    if summary && !matches!(format, Format::Html | Format::Pdf) {
        return Err(AppError::usage("--summary supports only -f html or -f pdf"));
    }
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export photos"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_photos(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "photos", "count": 0, "outputs": [],
            "note": "this backup has no photos", "device": device
        }));
    };

    if summary {
        return run_photos_summary(&backup, &items, out, format, device, cli);
    }

    let summary = if no_files {
        None
    } else {
        Some(photos::extract_photos(&backup, &mut items, out).map_err(|e| AppError::other(e.to_string()))?)
    };

    let rendered = match format {
        Format::Csv => format::photos_csv(&items),
        Format::Json => format::photos_json(&items),
        Format::Html | Format::Pdf => format::photos_html(&items, backup.device_info(), !no_files),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("photos.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} asset(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "photos", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = summary {
        eprintln!(
            "Extracted {} file(s) ({} as reduced-quality thumbnails, {} missing) to {}/{}",
            s.extracted, s.thumbnails, s.missing, out.display(), s.dir
        );
        envelope["files"] = serde_json::json!({
            "dir": s.dir, "extracted": s.extracted, "thumbnails": s.thumbnails, "missing": s.missing
        });
    }
    Ok(envelope)
}

/// Text-only summary report for `photos --summary`: no gallery, no files copied.
/// Quality counts come from a backup availability probe (cheap), so a quick
/// overview can be produced without the multi-gigabyte media extraction.
fn run_photos_summary(
    backup: &archive_core::Backup,
    items: &[photos::Photo],
    out: &std::path::Path,
    format: Format,
    device: serde_json::Value,
    cli: &Cli,
) -> Result<serde_json::Value, AppError> {
    // Format is validated by the caller (run_photos) before the backup is opened.
    let (originals, thumbnails, missing) = photos::availability(backup, items);
    let generated = chrono::Utc::now().to_rfc3339();
    let rendered =
        format::photos_summary_html(items, backup.device_info(), &generated, originals, thumbnails, missing);
    let out_file = out.join(format!("photos-summary.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote summary report ({} item(s)) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "photos", "mode": "summary", "count": items.len(),
        "outputs": [out_file.to_string_lossy()],
        "files": { "extracted": originals, "thumbnails": thumbnails, "missing": missing },
        "device": device
    }))
}

fn run_photos_recently_deleted(
    cli: &Cli,
    password: Option<&str>,
    format: &str,
    no_files: bool,
) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "photos-recently-deleted")?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export recently-deleted photos"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let all = match load_photos(&backup)? {
        Some(v) => v,
        None => {
            return Ok(serde_json::json!({
                "ok": true, "command": "photos-recently-deleted", "count": 0, "outputs": [],
                "note": "this backup has no photos", "device": device
            }));
        }
    };
    let mut trashed = photos_deleted::filter_trashed(all);
    if trashed.is_empty() {
        return Ok(serde_json::json!({
            "ok": true, "command": "photos-recently-deleted", "count": 0, "outputs": [],
            "note": "this backup has no recently-deleted photos", "device": device
        }));
    }

    let summary = if no_files {
        None
    } else {
        Some(
            photos::extract_into(&backup, &mut trashed, out, photos_deleted::DELETED_DIR)
                .map_err(|e| AppError::other(e.to_string()))?,
        )
    };
    let items = photos_deleted::into_deleted(trashed);

    let rendered = match format {
        Format::Csv => format::photos_deleted_csv(&items),
        Format::Json => format::photos_deleted_json(&items),
        Format::Html | Format::Pdf => format::photos_deleted_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("photos-recently-deleted.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} recently-deleted asset(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "photos-recently-deleted", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = summary {
        eprintln!(
            "Extracted {} file(s) ({} as reduced-quality thumbnails, {} missing) to {}/{}",
            s.extracted, s.thumbnails, s.missing, out.display(), s.dir
        );
        envelope["files"] = serde_json::json!({
            "dir": s.dir, "extracted": s.extracted, "thumbnails": s.thumbnails, "missing": s.missing
        });
    }
    Ok(envelope)
}

fn run_attachments(cli: &Cli, password: Option<&str>, format: &str, no_files: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "attachments")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export attachments"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_attachments(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "attachments", "count": 0, "outputs": [],
            "note": "this backup has no Messages store", "device": device
        }));
    };

    let summary = if no_files {
        None
    } else {
        Some(attachments::extract_attachments(&backup, &mut items, out).map_err(|e| AppError::other(e.to_string()))?)
    };

    let rendered = match format {
        Format::Csv => format::attachments_csv(&items),
        Format::Json => format::attachments_json(&items),
        Format::Html | Format::Pdf => format::attachments_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("attachments.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} attachment(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "attachments", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = summary {
        eprintln!("Extracted {} file(s) ({} missing) to {}/{}", s.extracted, s.missing, out.display(), s.dir);
        envelope["files"] = serde_json::json!({
            "dir": s.dir, "extracted": s.extracted, "missing": s.missing
        });
    }
    Ok(envelope)
}

/// Turn a `load_*` result into an `Option`, logging (not aborting) on error so
/// one unreadable store never aborts the whole recovery.
// Best-effort store load shared by `recover` and `timeline`: an error is logged
// to stderr (command-neutral, since both callers use it) and yields `None` so the
// run continues; `Ok(None)` (store absent) also yields `None`.
fn opt_or_log<T>(r: Result<Option<T>, AppError>, what: &str) -> Option<T> {
    match r {
        Ok(v) => v,
        Err(e) => {
            eprintln!("skipping {what}: {}", e.message);
            None
        }
    }
}

/// Accumulates the `recover` package: writes each section's HTML under `out` and
/// records it for the index and the JSON envelope.
struct Recovery<'a> {
    out: &'a std::path::Path,
    sections: Vec<recover::RecoverSection>,
    outputs: Vec<String>,
}

impl Recovery<'_> {
    fn add(
        &mut self,
        data_type: &str,
        label: &str,
        file: &str,
        html: String,
        count: usize,
        media: Option<recover::RecoverMedia>,
    ) -> Result<(), AppError> {
        let p = self.out.join(file);
        std::fs::write(&p, html).map_err(|e| AppError::other(e.to_string()))?;
        self.outputs.push(p.to_string_lossy().into_owned());
        self.sections.push(recover::RecoverSection {
            data_type: data_type.to_string(),
            label: label.to_string(),
            file: file.to_string(),
            count,
            media,
        });
        Ok(())
    }
}

/// Wrap an `extract_*` summary into `RecoverMedia`, logging and yielding `None`
/// on error (best-effort; the metadata HTML is still written).
fn media_or_log(
    result: std::io::Result<(String, usize, usize, usize)>,
    what: &str,
) -> Option<recover::RecoverMedia> {
    match result {
        Ok((dir, extracted, thumbnails, missing)) => Some(recover::RecoverMedia { dir, extracted, thumbnails, missing }),
        Err(e) => {
            eprintln!("recover: {what} files: {e}");
            None
        }
    }
}

fn run_whatsapp(cli: &Cli, password: Option<&str>, format: &str, no_files: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "whatsapp")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export whatsapp"))?;
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_whatsapp(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "whatsapp", "count": 0, "outputs": [],
            "note": "this backup has no WhatsApp store", "device": device
        }));
    };
    if let Some(idx) = contact_index(&backup) {
        enrich::enrich_whatsapp(&idx, &mut items);
    }

    let summary = if no_files {
        None
    } else {
        Some(whatsapp::extract_media(&backup, &mut items, out).map_err(|e| AppError::other(e.to_string()))?)
    };

    let rendered = match format {
        Format::Csv => format::whatsapp_csv(&items),
        Format::Json => format::whatsapp_json(&items),
        Format::Html | Format::Pdf => format::whatsapp_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("whatsapp.{}", format.extension()));
    write_or_pdf(&out_file, &rendered, format, cli.chrome_path.as_deref())?;
    eprintln!("Wrote {} WhatsApp message(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "whatsapp", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    });
    if let Some(s) = summary {
        eprintln!("Extracted {} media file(s) ({} missing) to {}/{}", s.extracted, s.missing, out.display(), s.dir);
        // `files`, consistent with photos/attachments and recover sections.
        envelope["files"] = serde_json::json!({
            "dir": s.dir, "extracted": s.extracted, "missing": s.missing
        });
    }
    Ok(envelope)
}

fn run_recover(cli: &Cli, password: Option<&str>, no_files: bool) -> Result<serde_json::Value, AppError> {
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required for recover"))?;
    let backup = open_backup(cli, password)?;
    let device = backup.device_info();
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;

    let mut rec = Recovery { out, sections: Vec::new(), outputs: Vec::new() };
    // Resolve numbers to contact names once for every section that benefits.
    let cidx = contact_index(&backup);

    if let Some(items) = opt_or_log(load_contacts(&backup), "contacts") {
        rec.add("contacts", "Kontakty", "contacts.html", format::contacts_html(&items), items.len(), None)?;
    }
    if let Some(mut items) = opt_or_log(load_calls(&backup), "calls") {
        if let Some(idx) = &cidx {
            enrich::enrich_calls(idx, &mut items);
        }
        rec.add("calls", "Hovory", "calls.html", format::calls_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_accounts(&backup), "accounts") {
        rec.add("accounts", "Účty", "accounts.html", format::accounts_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_known_networks(&backup), "known-networks") {
        rec.add("known-networks", "Wi-Fi sítě", "known-networks.html", format::known_networks_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_homescreen(&backup), "homescreen-layout")
        && !items.is_empty()
    {
        rec.add("homescreen-layout", "Rozložení plochy", "homescreen-layout.html", format::homescreen_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_data_usage(&backup), "data-usage")
        && !items.is_empty()
    {
        rec.add("data-usage", "Datový provoz", "data-usage.html", format::data_usage_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_device_usage(&backup), "device-usage")
        && !items.is_empty()
    {
        rec.add("device-usage", "Využití aplikací", "device-usage.html", format::device_usage_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_interactions(&backup), "interactions")
        && !items.is_empty()
    {
        rec.add("interactions", "Historie kontaktů", "interactions.html", format::interactions_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_bluetooth_devices(&backup), "bluetooth-devices")
        && !items.is_empty()
    {
        rec.add("bluetooth-devices", "Bluetooth zařízení", "bluetooth-devices.html", format::bluetooth_devices_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_significant_locations(&backup), "significant-locations")
        && !items.is_empty()
    {
        rec.add("significant-locations", "Historie polohy", "significant-locations.html", format::significant_locations_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_keyboard_lexicon(&backup), "keyboard-lexicon")
        && !items.is_empty()
    {
        rec.add("keyboard-lexicon", "Klávesnicová slova", "keyboard-lexicon.html", format::keyboard_lexicon_html(&items), items.len(), None)?;
    }
    if let Some(mut items) = opt_or_log(load_voicemail(&backup), "voicemail") {
        if let Some(idx) = &cidx {
            enrich::enrich_voicemail(idx, &mut items);
        }
        let media = if no_files {
            None
        } else {
            media_or_log(
                voicemail_audio::extract_audio(&backup, &mut items, out, audio::AudioFormat::Amr)
                    .map(|s| (s.dir, s.extracted, 0, s.missing)),
                "voicemail",
            )
        };
        rec.add("voicemail", "Hlasové zprávy", "voicemail.html", format::voicemail_html(&items), items.len(), media)?;
    }
    if let Some(mut items) = opt_or_log(load_voice_memos(&backup), "voice-memos") {
        let media = if no_files {
            None
        } else {
            media_or_log(
                voice_memos::extract_voice_memos(&backup, &mut items, out, None)
                    .map(|s| (s.dir, s.extracted, 0, s.missing)),
                "voice-memos",
            )
        };
        rec.add("voice-memos", "Hlasové poznámky", "voice-memos.html", format::voice_memos_html(&items), items.len(), media)?;
    }
    if let Some(items) = opt_or_log(load_safari_history(&backup), "safari-history") {
        rec.add("safari-history", "Historie Safari", "safari-history.html", format::safari_history_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_safari_bookmarks(&backup), "safari-bookmarks") {
        rec.add("safari-bookmarks", "Záložky Safari", "safari-bookmarks.html", format::safari_bookmarks_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_calendar(&backup), "calendar") {
        rec.add("calendar", "Kalendář", "calendar.html", format::calendar_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_notes(&backup), "notes") {
        rec.add("notes", "Poznámky", "notes.html", format::notes_html(&items), items.len(), None)?;
    }
    if let Some(mut items) = opt_or_log(load_photos(&backup), "photos") {
        let media = if no_files {
            None
        } else {
            media_or_log(
                photos::extract_photos(&backup, &mut items, out).map(|s| (s.dir, s.extracted, s.thumbnails, s.missing)),
                "photos",
            )
        };
        rec.add("photos", "Fotky a videa", "photos.html", format::photos_html(&items, backup.device_info(), !no_files), items.len(), media)?;

        // Recently Deleted: a dedicated recovery view (own folder + estimated
        // purge dates). `items` is consumed here as the photos section is done.
        let mut trashed = photos_deleted::filter_trashed(items);
        // These were just extracted into photos/; reset `file` so this section's
        // links and media counts reflect only the recently-deleted/ extraction.
        for t in &mut trashed {
            t.file = None;
        }
        if !trashed.is_empty() {
            let media = if no_files {
                None
            } else {
                media_or_log(
                    photos::extract_into(&backup, &mut trashed, out, photos_deleted::DELETED_DIR)
                        .map(|s| (s.dir, s.extracted, s.thumbnails, s.missing)),
                    "photos-recently-deleted",
                )
            };
            let deleted = photos_deleted::into_deleted(trashed);
            rec.add(
                "photos-recently-deleted",
                "Smazané fotky (obnovitelné)",
                "photos-recently-deleted.html",
                format::photos_deleted_html(&deleted),
                deleted.len(),
                media,
            )?;
        }
    }
    if let Some(mut items) = opt_or_log(load_attachments(&backup), "attachments") {
        let media = if no_files {
            None
        } else {
            media_or_log(
                attachments::extract_attachments(&backup, &mut items, out).map(|s| (s.dir, s.extracted, 0, s.missing)),
                "attachments",
            )
        };
        rec.add("attachments", "Přílohy zpráv", "attachments.html", format::attachments_html(&items), items.len(), media)?;
    }
    if let Some(mut items) = opt_or_log(load_whatsapp(&backup), "whatsapp") {
        if let Some(idx) = &cidx {
            enrich::enrich_whatsapp(idx, &mut items);
        }
        let media = if no_files {
            None
        } else {
            media_or_log(
                whatsapp::extract_media(&backup, &mut items, out).map(|s| (s.dir, s.extracted, 0, s.missing)),
                "whatsapp",
            )
        };
        rec.add("whatsapp", "WhatsApp", "whatsapp.html", format::whatsapp_html(&items), items.len(), media)?;
    }
    if let Some(data) = opt_or_log(load_health(&backup), "health") {
        let count = data.workouts.len() + data.quantity_summary.len();
        rec.add("health", "Zdraví", "health.html", format::health_html(&data), count, None)?;
    }
    if let Some(items) = opt_or_log(load_reminders(&backup), "reminders") {
        rec.add("reminders", "Připomínky", "reminders.html", format::reminders_html(&items), items.len(), None)?;
    }
    if let Some(items) = opt_or_log(load_mail(&backup), "mail") {
        rec.add("mail", "Mail", "mail.html", format::mail_html(&items), items.len(), None)?;
    }

    let generated = chrono::Utc::now().to_rfc3339();
    let index_html = recover::render_index(device, &generated, &rec.sections);
    let index_path = out.join("index.html");
    std::fs::write(&index_path, index_html).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Recovered {} section(s) to {}", rec.sections.len(), out.display());

    // index.html leads the outputs list.
    let mut all_outputs = vec![index_path.to_string_lossy().into_owned()];
    all_outputs.extend(rec.outputs);
    Ok(serde_json::json!({
        "ok": true, "command": "recover",
        "outputs": all_outputs, "sections": rec.sections, "device": device_json(device)
    }))
}

// Drive the bundled `imessage-exporter` binary to export conversation
// transcripts. `archive` does not re-implement message decoding; it orchestrates
// the mature exporter and translates the result into the agent contract.
fn run_messages(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    // Validate the format before anything else so a bad value is a clean usage
    // error (exit 1) rather than a child-process failure.
    let fmt = messages::normalize_format(format)
        .ok_or_else(|| AppError::usage(format!("unknown messages format `{format}` (use txt, html, pdf)")))?;
    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required for messages"))?;
    let backup_dir = cli
        .backup
        .as_deref()
        .ok_or_else(|| AppError::usage("--backup <DIR> is required"))?;

    // Open with archive-core first: this enforces the auth contract (an encrypted
    // backup without the right password fails here with exit 2), yields device
    // info for the envelope, and reports encryption — all before we shell out.
    // Because both layers share crabapple, a password that opens the backup here
    // also decrypts it in the exporter.
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    let encrypted = backup.is_encrypted();

    // Namespace the exporter's output tree so reusing `-o` for other commands
    // does not collide with it. The exporter creates this directory itself, but
    // creating it up front keeps the path well-defined for the envelope.
    let export_dir = out.join("messages");
    std::fs::create_dir_all(&export_dir).map_err(|e| AppError::other(e.to_string()))?;

    let exporter = messages::resolve_exporter();
    let args = messages::messages_args(backup_dir, &export_dir, fmt, encrypted, password, cli.chrome_path.as_deref());

    eprintln!("Exporting messages ({fmt}) to {} …", export_dir.display());
    // Forward the exporter's stdout progress to OUR stderr so the agent contract
    // (exactly one JSON object on stdout) holds; its stderr inherits to ours.
    // Draining the single piped stream then waiting cannot deadlock.
    let mut child = std::process::Command::new(&exporter)
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            AppError::other(format!(
                "could not run `{}` ({e}); build the workspace (cargo build --release) \
                 or set {} to the imessage-exporter binary",
                exporter.to_string_lossy(),
                messages::EXPORTER_ENV
            ))
        })?;
    if let Some(child_out) = child.stdout.take() {
        let mut reader = std::io::BufReader::new(child_out);
        let _ = std::io::copy(&mut reader, &mut std::io::stderr());
    }
    let status = child
        .wait()
        .map_err(|e| AppError::other(format!("running {}: {e}", messages::EXPORTER_TOOL)))?;
    if !status.success() {
        return Err(AppError::other(format!("{} failed ({status})", messages::EXPORTER_TOOL)));
    }

    eprintln!("Messages exported to {}", export_dir.display());
    Ok(serde_json::json!({
        "ok": true, "command": "messages",
        "format": fmt, "output": export_dir.to_string_lossy(),
        "device": device
    }))
}

fn run_backup(cli: &Cli, password: Option<&str>, full: bool) -> Result<serde_json::Value, AppError> {
    use device_backup::{backup_args, parse_udids, tool_available, BACKUP_TOOL, DEVICE_TOOL};
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required for backup"))?;

    // Fail fast when the external tools are not installed.
    for tool in [BACKUP_TOOL, DEVICE_TOOL] {
        if !tool_available(tool) {
            return Err(AppError::other(format!(
                "`{tool}` was not found on PATH; install libimobiledevice \
                 (e.g. `brew install libimobiledevice`) to use `backup`"
            )));
        }
    }

    // Require a connected device.
    let listed = std::process::Command::new(DEVICE_TOOL)
        .arg("-l")
        .output()
        .map_err(|e| AppError::other(format!("running {DEVICE_TOOL}: {e}")))?;
    let udids = parse_udids(&String::from_utf8_lossy(&listed.stdout));
    let Some(udid) = udids.first().cloned() else {
        return Err(AppError::other("no iOS device connected (idevice_id -l found none)"));
    };

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Backing up device {udid} to {} …", out.display());
    // Forward idevicebackup2's stdout progress to OUR stderr so the agent contract
    // (exactly one JSON object on stdout) holds; its stderr inherits to our stderr.
    // Draining the single piped stream then waiting cannot deadlock.
    let mut child = std::process::Command::new(BACKUP_TOOL)
        .args(backup_args(&udid, out, full))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::other(format!("running {BACKUP_TOOL}: {e}")))?;
    if let Some(child_out) = child.stdout.take() {
        let mut reader = std::io::BufReader::new(child_out);
        let _ = std::io::copy(&mut reader, &mut std::io::stderr());
    }
    let status = child
        .wait()
        .map_err(|e| AppError::other(format!("running {BACKUP_TOOL}: {e}")))?;
    if !status.success() {
        return Err(AppError::other(format!("{BACKUP_TOOL} failed ({status})")));
    }

    let dir = out.join(&udid);
    let mut envelope = serde_json::json!({
        "ok": true, "command": "backup",
        "dir": dir.to_string_lossy(), "udid": udid
    });
    let mut notes: Vec<String> = Vec::new();
    match archive_core::Backup::open(&dir, password) {
        Ok(b) => envelope["device"] = device_json(b.device_info()),
        Err(e) => notes.push(format!("backup created, but device info could not be read: {e}")),
    }
    if udids.len() > 1 {
        notes.push(format!("{} devices connected; backed up the first ({udid})", udids.len()));
    }
    if !notes.is_empty() {
        // Merge all notes so neither (unreadable result / multiple devices) clobbers the other.
        envelope["note"] = serde_json::json!(notes.join("; "));
    }
    Ok(envelope)
}

/// Cap on each integrity sample list, keeping the envelope bounded.
const INTEGRITY_SAMPLE_CAP: usize = 20;

fn run_integrity(cli: &Cli, password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());
    let r = backup
        .verify_integrity(INTEGRITY_SAMPLE_CAP)
        .map_err(|e| AppError::other(e.to_string()))?;
    let complete = r.missing == 0 && r.size_mismatch == 0;
    eprintln!(
        "Integrity: {}/{} files present, {} missing, {} size mismatch{}",
        r.present,
        r.total_files,
        r.missing,
        r.size_mismatch,
        if r.size_checked { "" } else { " (size check skipped: encrypted)" }
    );
    let mut envelope = serde_json::json!({
        "ok": true, "command": "integrity", "complete": complete,
        "total_files": r.total_files, "present": r.present, "missing": r.missing,
        "size_checked": r.size_checked, "size_mismatch": r.size_mismatch,
        "missing_sample": r.missing_sample, "mismatch_sample": r.mismatch_sample,
        "device": device
    });
    if !r.size_checked {
        envelope["note"] = serde_json::json!("size verification skipped (encrypted backup)");
    }
    Ok(envelope)
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
    ("accounts", true, "HomeDomain", "Library/Accounts/Accounts3.sqlite"),
    // Known Wi-Fi networks live at one of several plist paths (version-dependent),
    // so presence is detected specially in `run_inspect`; this path is the
    // preferred one and is not used directly for `has`.
    ("known-networks", true, known_networks::DOMAIN, ""),
    ("homescreen-layout", true, homescreen::DOMAIN, homescreen::PATH),
    ("data-usage", true, data_usage::DOMAIN, data_usage::PATH),
    // knowledgeC.db has lived at several domain/paths; presence is detected
    // specially in `run_inspect` by probing device_usage::CANDIDATES.
    ("device-usage", true, device_usage::DOMAIN, ""),
    // interactionC.db has lived under a couple of CoreDuet domains; presence is
    // detected specially in `run_inspect` by probing interactions::CANDIDATES.
    ("interactions", true, interactions::DOMAIN, ""),
    // Bluetooth devices live across two LE databases and a classic plist; presence
    // is detected specially in `run_inspect` by probing all of them.
    ("bluetooth-devices", true, bluetooth::DOMAIN, ""),
    // The routined location store has a few candidate file names; presence is
    // detected specially in `run_inspect` by probing significant_locations::PATHS.
    ("significant-locations", true, significant_locations::DOMAIN, ""),
    // Custom keyboard words live under one of a couple domains; presence is
    // detected specially in `run_inspect` by probing keyboard_lexicon::SOURCES.
    ("keyboard-lexicon", true, "KeyboardDomain", ""),
    ("voicemail", true, "HomeDomain", "Library/Voicemail/voicemail.db"),
    ("voice-memos", true, "AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db"),
    ("safari-history", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/History.db"),
    ("safari-bookmarks", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db"),
    ("calendar", true, "HomeDomain", "Library/Calendar/Calendar.sqlitedb"),
    ("notes", true, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite"),
    ("photos", true, "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
    // Shares Photos.sqlite with `photos`; the count is the trashed-asset subset.
    ("photos-recently-deleted", true, "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
    // `messages` is intentionally not listed here: it is exported out-of-process
    // by the `messages` command, not by `recover` (which runs the in-process
    // extractors). The presence of message data is already visible via the
    // `attachments` row below — both read the same `sms.db`.
    ("attachments", true, "HomeDomain", "Library/SMS/sms.db"),
    ("whatsapp", true, "AppDomainGroup-group.net.whatsapp.WhatsApp.shared", "ChatStorage.sqlite"),
    ("health", true, "HealthDomain", "Health/healthdb_secure.sqlite"),
    // Reminders and Mail live at dynamic paths (UUID store / .emlx files), so
    // their presence is detected specially in `run_inspect` (the path here is a
    // placeholder and is not used for `has`).
    ("reminders", true, reminders::DOMAIN, ""),
    ("mail", true, mail::MAIL_DOMAIN, ""),
];

fn run_inspect(cli: &Cli, password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let backup = open_backup(cli, password)?;
    let device = device_json(backup.device_info());

    let mut stores = Vec::new();
    for &(name, supported, domain, path) in KNOWN_STORES {
        // Voice Memos lives at a modern or a legacy location; report present if
        // either exists, matching what `load_voice_memos` will actually read.
        let present = if name == "voice-memos" {
            let mut any = false;
            for (d, p) in voice_memos::DB_LOCATIONS {
                if backup.has(d, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else if name == "reminders" {
            backup
                .list(reminders::DOMAIN, "")
                .map_err(|e| AppError::other(e.to_string()))?
                .iter()
                .any(|p| reminders::is_store_path(p))
        } else if name == "mail" {
            backup
                .list(mail::MAIL_DOMAIN, "")
                .map_err(|e| AppError::other(e.to_string()))?
                .iter()
                .any(|p| p.to_lowercase().ends_with(".emlx"))
        } else if name == "known-networks" {
            let mut any = false;
            for p in known_networks::PATHS {
                if backup.has(known_networks::DOMAIN, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else if name == "device-usage" {
            let mut any = false;
            for (d, p) in device_usage::CANDIDATES {
                if backup.has(d, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else if name == "interactions" {
            let mut any = false;
            for (d, p) in interactions::CANDIDATES {
                if backup.has(d, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else if name == "bluetooth-devices" {
            let mut any = backup
                .has(bluetooth::DOMAIN, bluetooth::CLASSIC_PLIST)
                .map_err(|e| AppError::other(e.to_string()))?;
            for (p, _, _) in bluetooth::LE_DATABASES {
                if any {
                    break;
                }
                any = backup.has(bluetooth::DOMAIN, p).map_err(|e| AppError::other(e.to_string()))?;
            }
            any
        } else if name == "significant-locations" {
            let mut any = false;
            for p in significant_locations::PATHS {
                if backup.has(significant_locations::DOMAIN, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else if name == "keyboard-lexicon" {
            let mut any = false;
            for (domain, p) in keyboard_lexicon::SOURCES {
                if backup.has(domain, p).map_err(|e| AppError::other(e.to_string()))? {
                    any = true;
                    break;
                }
            }
            any
        } else {
            backup
                .has(domain, path)
                .map_err(|e| AppError::other(e.to_string()))?
        };
        // `count` is best-effort: a present-but-unparseable store yields `None`
        // (count: null) without aborting inspect. `.ok()` drops a read error,
        // `.flatten()` collapses store-absent `Ok(None)`, `.map(len)` counts a
        // successful read.
        let count = if present && supported {
            match name {
                "contacts" => load_contacts(&backup).ok().flatten().map(|p| p.len()),
                "calls" => load_calls(&backup).ok().flatten().map(|c| c.len()),
                "accounts" => load_accounts(&backup).ok().flatten().map(|v| v.len()),
                "known-networks" => load_known_networks(&backup).ok().flatten().map(|v| v.len()),
                "homescreen-layout" => load_homescreen(&backup).ok().flatten().map(|v| v.len()),
                "data-usage" => load_data_usage(&backup).ok().flatten().map(|v| v.len()),
                "device-usage" => load_device_usage(&backup).ok().flatten().map(|v| v.len()),
                "interactions" => load_interactions(&backup).ok().flatten().map(|v| v.len()),
                "bluetooth-devices" => load_bluetooth_devices(&backup).ok().flatten().map(|v| v.len()),
                "significant-locations" => load_significant_locations(&backup).ok().flatten().map(|v| v.len()),
                "keyboard-lexicon" => load_keyboard_lexicon(&backup).ok().flatten().map(|v| v.len()),
                "voicemail" => load_voicemail(&backup).ok().flatten().map(|v| v.len()),
                "voice-memos" => load_voice_memos(&backup).ok().flatten().map(|v| v.len()),
                "safari-history" => load_safari_history(&backup).ok().flatten().map(|v| v.len()),
                "safari-bookmarks" => load_safari_bookmarks(&backup).ok().flatten().map(|v| v.len()),
                "calendar" => load_calendar(&backup).ok().flatten().map(|v| v.len()),
                "notes" => load_notes(&backup).ok().flatten().map(|v| v.len()),
                "photos" => load_photos(&backup).ok().flatten().map(|v| v.len()),
                "photos-recently-deleted" => {
                    load_photos(&backup).ok().flatten().map(|v| v.iter().filter(|p| p.trashed).count())
                }
                "attachments" => load_attachments(&backup).ok().flatten().map(|v| v.len()),
                "whatsapp" => load_whatsapp(&backup).ok().flatten().map(|v| v.len()),
                "health" => load_health(&backup).ok().flatten().map(|h| h.workouts.len() + h.quantity_summary.len()),
                "reminders" => load_reminders(&backup).ok().flatten().map(|v| v.len()),
                "mail" => load_mail(&backup).ok().flatten().map(|v| v.len()),
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
            "archive", "--backup", "/b", "-o", "/out", "contacts", "-f", "vcf",
        ])
        .unwrap();
        assert_eq!(cli.backup, Some(PathBuf::from("/b")));
        match cli.command {
            Command::Contacts { format } => assert_eq!(format, "vcf"),
            _ => panic!("expected Contacts"),
        }
    }

    #[test]
    fn parses_messages_invocation() {
        let cli = Cli::try_parse_from([
            "archive", "--backup", "/b", "-o", "/out", "messages", "-f", "html",
        ])
        .unwrap();
        match cli.command {
            Command::Messages { format } => assert_eq!(format, "html"),
            _ => panic!("expected Messages"),
        }
    }

    #[test]
    fn parses_health_reminders_mail_invocations() {
        let h = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "health", "-f", "json"]).unwrap();
        assert!(matches!(h.command, Command::Health { format } if format == "json"));
        let r = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "reminders", "-f", "csv"]).unwrap();
        assert!(matches!(r.command, Command::Reminders { format } if format == "csv"));
        let m = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "mail", "-f", "html"]).unwrap();
        assert!(matches!(m.command, Command::Mail { format } if format == "html"));
        let a = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "apps", "-f", "csv"]).unwrap();
        assert!(matches!(a.command, Command::Apps { format } if format == "csv"));
        let t = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "timeline", "-f", "json"]).unwrap();
        assert!(matches!(t.command, Command::Timeline { format, redact } if format == "json" && !redact));
        let s = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "stats", "-f", "json"]).unwrap();
        assert!(matches!(s.command, Command::Stats { format } if format == "json"));
        let ad = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "app-databases", "-f", "json"]).unwrap();
        assert!(matches!(ad.command, Command::AppDatabases { format } if format == "json"));
        let af = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "app-files", "--app", "viber", "-f", "json", "--all"]).unwrap();
        assert!(matches!(af.command, Command::AppFiles { app, format, all } if app == "viber" && format == "json" && all));
        // --app is required.
        assert!(Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "app-files", "-f", "json"]).is_err());
        let w = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "wifi", "-f", "html"]).unwrap();
        assert!(matches!(w.command, Command::Wifi { format } if format == "html"));
    }

    #[test]
    fn wifi_rejects_pdf_to_avoid_plaintext_sidecar() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "wifi", "-f", "pdf"]).unwrap();
        let err = run_wifi(&cli, None, "pdf").unwrap_err();
        assert_eq!(err.kind, "usage");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn passwords_rejects_pdf_to_avoid_plaintext_sidecar() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "passwords", "-f", "pdf"]).unwrap();
        let err = run_passwords(&cli, None, "pdf").unwrap_err();
        assert_eq!(err.kind, "usage");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn parses_recover_deleted_with_store_default_and_explicit() {
        let d = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "recover-deleted", "-f", "html"]).unwrap();
        assert!(matches!(d.command, Command::RecoverDeleted { ref format, ref store } if format == "html" && store == "all"));
        let m = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "recover-deleted", "-f", "csv", "--store", "messages"]).unwrap();
        assert!(matches!(m.command, Command::RecoverDeleted { ref store, .. } if store == "messages"));
    }

    #[test]
    fn photos_live_keys_read_from_ios12_zgenericasset() {
        // recover-deleted excludes still-live assets via the photos table; on iOS
        // ≤12 that table is ZGENERICASSET (renamed to ZASSET in iOS 13), so
        // live_keys must read it or every live photo is wrongly carved as deleted.
        let dir = std::env::temp_dir().join(format!("be-rd-gen-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("Photos.sqlite");
        let _ = std::fs::remove_file(&db);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE ZGENERICASSET (Z_PK INTEGER PRIMARY KEY);
             INSERT INTO ZGENERICASSET VALUES (1), (2), (3);",
        )
        .unwrap();
        drop(conn);

        let keys = live_keys(&db, "photos");
        assert_eq!(keys.rowids.len(), 3, "iOS 12 ZGENERICASSET live keys must be read");
        assert!(keys.rowids.contains(&1) && keys.rowids.contains(&3));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parses_schema_check_format() {
        let s = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "schema-check", "-f", "json"]).unwrap();
        assert!(matches!(s.command, Command::SchemaCheck { ref format } if format == "json"));
    }

    #[test]
    fn schema_check_rejects_vcf() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "schema-check", "-f", "vcf"]).unwrap();
        let err = run_schema_check(&cli, None, "vcf").unwrap_err();
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn photos_summary_rejects_csv_and_json_before_opening_backup() {
        // --summary is text-report only; csv/json must be rejected as a usage error
        // up front, regardless of (and without opening) the backup.
        for fmt in ["csv", "json"] {
            let cli = Cli::try_parse_from(["archive", "--backup", "/no/such/backup", "-o", "/o", "photos", "-f", fmt, "--summary"]).unwrap();
            let err = run_photos(&cli, None, fmt, false, true).unwrap_err();
            assert_eq!(err.kind, "usage", "format {fmt} should be rejected");
        }
    }

    #[test]
    fn parses_search_query_and_format() {
        let s = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "search", "-q", "novák", "-f", "json"]).unwrap();
        assert!(matches!(s.command, Command::Search { ref query, ref format, redact } if query == "novák" && format == "json" && !redact));
        // --redact flips the flag on for both search and timeline.
        let r = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "search", "-q", "x", "-f", "json", "--redact"]).unwrap();
        assert!(matches!(r.command, Command::Search { redact, .. } if redact));
        let t = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "timeline", "-f", "html", "--redact"]).unwrap();
        assert!(matches!(t.command, Command::Timeline { redact, .. } if redact));
    }

    #[test]
    fn search_rejects_empty_query() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "search", "-q", "  ", "-f", "json"]).unwrap();
        let err = run_search(&cli, None, "  ", "json", false).unwrap_err();
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn parses_bluetooth_devices() {
        let b = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "bluetooth-devices", "-f", "json"]).unwrap();
        assert!(matches!(b.command, Command::BluetoothDevices { format } if format == "json"));
        // It is advertised as a supported store for inspect/recover.
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "bluetooth-devices").unwrap();
        assert!(s.1);
    }

    #[test]
    fn parses_significant_locations() {
        let s = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "significant-locations", "-f", "json"]).unwrap();
        assert!(matches!(s.command, Command::SignificantLocations { format } if format == "json"));
        let st = KNOWN_STORES.iter().find(|(n, ..)| *n == "significant-locations").unwrap();
        assert!(st.1);
    }

    #[test]
    fn parses_keyboard_lexicon() {
        let k = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "keyboard-lexicon", "-f", "json"]).unwrap();
        assert!(matches!(k.command, Command::KeyboardLexicon { format } if format == "json"));
        let st = KNOWN_STORES.iter().find(|(n, ..)| *n == "keyboard-lexicon").unwrap();
        assert!(st.1);
    }

    #[test]
    fn parses_certificates() {
        let c = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "certificates", "-f", "json"]).unwrap();
        assert!(matches!(c.command, Command::Certificates { format } if format == "json"));
        // Keychain-derived (encrypted-only), so it is NOT a recover/inspect store.
        assert!(KNOWN_STORES.iter().all(|(n, ..)| *n != "certificates"));
    }

    #[test]
    fn parses_vpn_creds_and_rejects_pdf() {
        let v = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "vpn-creds", "-f", "json"]).unwrap();
        assert!(matches!(v.command, Command::VpnCreds { format } if format == "json"));
        // Keychain-derived (encrypted-only), not a recover/inspect store.
        assert!(KNOWN_STORES.iter().all(|(n, ..)| *n != "vpn-creds"));
        // pdf is rejected for this sensitive export (no plaintext sidecar).
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "vpn-creds", "-f", "pdf"]).unwrap();
        assert_eq!(run_vpn_creds(&cli, None, "pdf").unwrap_err().kind, "usage");
    }

    #[test]
    fn parses_db_export() {
        let d = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "db-export"]).unwrap();
        assert!(matches!(d.command, Command::DbExport));
    }

    #[test]
    fn db_export_requires_out() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "db-export"]).unwrap();
        let err = run_db_export(&cli, None).unwrap_err();
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn parses_diff_against_and_format() {
        let d = Cli::try_parse_from(["archive", "--backup", "/a", "-o", "/o", "diff", "--against", "/b", "-f", "json"]).unwrap();
        assert!(matches!(d.command, Command::Diff { ref against, ref format } if against == std::path::Path::new("/b") && format == "json"));
    }

    #[test]
    fn parses_package_source_and_password() {
        let p = Cli::try_parse_from(["archive", "-o", "/o", "package", "--source", "/case", "--zip-password", "pw"]).unwrap();
        assert!(matches!(p.command, Command::Package { ref source, ref zip_password }
            if source == std::path::Path::new("/case") && zip_password.as_deref() == Some("pw")));
    }

    #[test]
    fn package_rejects_missing_source_dir() {
        // With a password supplied, a non-existent source is a clean usage error.
        let cli = Cli::try_parse_from(["archive", "-o", "/o", "package", "--source", "/no/such/dir", "--zip-password", "pw"]).unwrap();
        let err = run_package(&cli, std::path::Path::new("/no/such/dir"), Some("pw")).unwrap_err();
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn recover_deleted_rejects_unknown_store() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/o", "recover-deleted", "-f", "json", "--store", "bogus"]).unwrap();
        let err = run_recover_deleted(&cli, None, "json", "bogus").unwrap_err();
        assert_eq!(err.kind, "usage");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn write_or_pdf_writes_plain_for_non_pdf_formats() {
        // The non-PDF path is a plain file write (no browser); the PDF path is
        // exercised manually (it needs a headless browser, absent in CI).
        let dir = std::env::temp_dir().join(format!("be-wop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.csv");
        write_or_pdf(&f, "a,b\n1,2\n", Format::Csv, None).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a,b\n1,2\n");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn known_stores_lists_new_batch() {
        for name in ["health", "reminders", "mail"] {
            let s = KNOWN_STORES.iter().find(|(n, ..)| *n == name).unwrap();
            assert!(s.1, "{name} must be supported");
        }
        // `apps` is manifest-derived, not a data store: it must NOT be advertised
        // by inspect (keeps inspect/recover store coverage consistent).
        assert!(KNOWN_STORES.iter().all(|(n, ..)| *n != "apps"), "apps must not be a store");
    }

    #[test]
    fn inspect_counts_every_supported_store() {
        // Every store advertised as supported by inspect must have a count arm in
        // run_inspect's `match name` (otherwise it reports `count: null`). This
        // list mirrors that match — when adding a store, add it here AND add the
        // matching `load_*` count arm.
        let counted = [
            "contacts", "calls", "accounts", "known-networks", "homescreen-layout", "data-usage", "device-usage", "interactions", "bluetooth-devices", "significant-locations", "keyboard-lexicon", "voicemail", "voice-memos",
            "safari-history", "safari-bookmarks", "calendar", "notes", "photos",
            "photos-recently-deleted", "attachments", "whatsapp", "health", "reminders", "mail",
        ];
        for (name, supported, ..) in KNOWN_STORES {
            if *supported {
                assert!(
                    counted.contains(name),
                    "{name} is a supported store but has no inspect count arm"
                );
            }
        }
    }

    #[test]
    fn messages_rejects_unsupported_format() {
        // Format is validated before the backup is opened, so an unsupported
        // value is a usage error (exit 1) without touching `--backup`.
        let cli =
            Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "messages", "-f", "csv"])
                .unwrap();
        let err = match run_messages(&cli, None, "csv") {
            Err(e) => e,
            Ok(_) => panic!("expected a usage error for an unsupported format"),
        };
        assert_eq!(err.kind, "usage");
        assert_eq!(err.code, 1);
    }

    #[test]
    fn missing_backup_is_a_runtime_usage_error() {
        // `--backup` is now optional at parse time (the `backup` command creates
        // one); a read command without it fails at runtime with a usage error.
        let cli = Cli::try_parse_from(["archive", "-o", "/out", "contacts", "-f", "csv"]).unwrap();
        let err = match open_backup(&cli, None) {
            Err(e) => e,
            Ok(_) => panic!("expected a usage error for missing --backup"),
        };
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn open_error_maps_locked_to_auth_else_other() {
        assert_eq!(open_error(archive_core::BackupError::Locked("x".into())).code, 2);
        assert_eq!(open_error(archive_core::BackupError::Open("x".into())).code, 1);
    }

    #[test]
    fn parses_out_after_subcommand() {
        let cli = Cli::try_parse_from([
            "archive", "--backup", "/b", "contacts", "-f", "csv", "-o", "/out",
        ])
        .unwrap();
        assert_eq!(cli.backup, Some(PathBuf::from("/b")));
        assert_eq!(cli.out, Some(PathBuf::from("/out")));
    }

    #[test]
    fn parses_calls_invocation() {
        let cli = Cli::try_parse_from([
            "archive", "--backup", "/b", "-o", "/out", "calls", "-f", "json",
        ])
        .unwrap();
        match cli.command {
            Command::Calls { format } => assert_eq!(format, "json"),
            _ => panic!("expected Calls"),
        }
    }

    #[test]
    fn parses_voicemail_invocation() {
        let cli = Cli::try_parse_from([
            "archive", "--backup", "/b", "-o", "/out", "voicemail", "-f", "csv",
        ])
        .unwrap();
        match cli.command {
            Command::Voicemail { format, audio, audio_format } => {
                assert_eq!(format, "csv");
                assert!(!audio);
                assert_eq!(audio_format, None);
            }
            _ => panic!("expected Voicemail"),
        }
    }

    #[test]
    fn voicemail_format_rejects_vcf() {
        assert_eq!(voicemail_format("json").unwrap(), Format::Json);
        assert_eq!(voicemail_format("vcf").unwrap_err().code, 1);
    }

    #[test]
    fn resolve_audio_off_by_default() {
        assert!(super::resolve_audio_format(false, None).unwrap().is_none());
    }

    #[test]
    fn resolve_audio_format_without_flag_is_usage_error() {
        let err = super::resolve_audio_format(false, Some("m4a")).unwrap_err();
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn resolve_audio_defaults_to_amr() {
        let fmt = super::resolve_audio_format(true, None).unwrap();
        assert_eq!(fmt, Some(crate::audio::AudioFormat::Amr));
    }

    #[test]
    fn resolve_unknown_audio_format_is_usage_error() {
        let err = super::resolve_audio_format(true, Some("ogg")).unwrap_err();
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn parses_voice_memos_invocation() {
        let cli = Cli::try_parse_from([
            "archive", "--backup", "/b", "-o", "/out", "voice-memos", "-f", "json",
        ])
        .unwrap();
        match cli.command {
            Command::VoiceMemos { format, no_audio, audio_format } => {
                assert_eq!(format, "json");
                assert!(!no_audio);
                assert_eq!(audio_format, None);
            }
            _ => panic!("expected VoiceMemos"),
        }
    }

    #[test]
    fn voice_memos_format_rejects_vcf() {
        assert_eq!(voice_memos_format("html").unwrap(), Format::Html);
        assert_eq!(voice_memos_format("vcf").unwrap_err().code, 1);
        assert_eq!(voice_memos_format("nope").unwrap_err().code, 1);
    }

    #[test]
    fn resolve_vm_audio_no_audio_skips() {
        assert!(super::resolve_vm_audio(true, None).unwrap().is_none());
    }

    #[test]
    fn resolve_vm_audio_default_is_raw() {
        assert_eq!(super::resolve_vm_audio(false, None).unwrap(), Some(None));
    }

    #[test]
    fn resolve_vm_audio_format_with_no_audio_is_usage_error() {
        let err = super::resolve_vm_audio(true, Some("m4a")).unwrap_err();
        assert_eq!(err.code, 1);
        assert_eq!(err.kind, "usage");
    }

    #[test]
    fn resolve_vm_audio_rejects_amr_and_unknown() {
        assert_eq!(super::resolve_vm_audio(false, Some("amr")).unwrap_err().kind, "usage");
        assert_eq!(super::resolve_vm_audio(false, Some("ogg")).unwrap_err().kind, "usage");
    }

    #[test]
    fn known_stores_lists_voice_memos_supported() {
        let vm = KNOWN_STORES.iter().find(|(n, ..)| *n == "voice-memos").unwrap();
        assert!(vm.1, "voice-memos must be supported");
    }

    #[test]
    fn parses_safari_and_calendar_invocations() {
        for cmd in ["safari-history", "safari-bookmarks", "calendar"] {
            let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", cmd, "-f", "json"]).unwrap();
            assert_eq!(cli.out, Some(PathBuf::from("/out")));
        }
    }

    #[test]
    fn export_format_rejects_vcf_accepts_others() {
        assert_eq!(export_format("csv", "calendar").unwrap(), Format::Csv);
        assert_eq!(export_format("html", "safari-history").unwrap(), Format::Html);
        assert_eq!(export_format("vcf", "calendar").unwrap_err().code, 1);
        assert_eq!(export_format("nope", "calendar").unwrap_err().code, 1);
    }

    #[test]
    fn known_stores_lists_safari_and_calendar_supported() {
        for name in ["safari-history", "safari-bookmarks", "calendar"] {
            let s = KNOWN_STORES.iter().find(|(n, ..)| *n == name).unwrap();
            assert!(s.1, "{name} must be supported");
        }
    }

    #[test]
    fn device_json_includes_model_and_serial() {
        let d = archive_core::DeviceInfo {
            device_name: "iPhone".into(),
            product_version: "17.5".into(),
            model: "iPhone14,2".into(),
            serial: "F2LABC".into(),
            udid: "00008110-x".into(),
        };
        let v = device_json(&d);
        assert_eq!(v["name"], "iPhone");
        assert_eq!(v["model"], "iPhone14,2");
        assert_eq!(v["ios"], "17.5");
        assert_eq!(v["serial"], "F2LABC");
        assert_eq!(v["udid"], "00008110-x");
    }

    #[test]
    fn parses_integrity_invocation_without_out() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "integrity"]).unwrap();
        assert!(matches!(cli.command, Command::Integrity));
        assert_eq!(cli.out, None);
    }

    #[test]
    fn parses_backup_invocation_without_backup_flag() {
        // `backup` creates a backup, so it does not require the global --backup.
        let cli = Cli::try_parse_from(["archive", "-o", "/out", "backup", "--full"]).unwrap();
        match cli.command {
            Command::Backup { full } => assert!(full),
            _ => panic!("expected Backup"),
        }
        assert!(cli.backup.is_none());
    }

    #[test]
    fn parses_recover_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "recover", "--no-files"]).unwrap();
        match cli.command {
            Command::Recover { no_files } => assert!(no_files),
            _ => panic!("expected Recover"),
        }
        let bare = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "recover"]).unwrap();
        match bare.command {
            Command::Recover { no_files } => assert!(!no_files),
            _ => panic!("expected Recover"),
        }
    }

    #[test]
    fn parses_whatsapp_invocation_and_supported() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "whatsapp", "-f", "html", "--no-files"]).unwrap();
        match cli.command {
            Command::Whatsapp { format, no_files } => {
                assert_eq!(format, "html");
                assert!(no_files);
            }
            _ => panic!("expected Whatsapp"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "whatsapp").unwrap();
        assert!(s.1, "whatsapp must be supported");
    }

    #[test]
    fn parses_attachments_invocation_and_supported() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "attachments", "-f", "html", "--no-files"]).unwrap();
        match cli.command {
            Command::Attachments { format, no_files } => {
                assert_eq!(format, "html");
                assert!(no_files);
            }
            _ => panic!("expected Attachments"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "attachments").unwrap();
        assert!(s.1, "attachments must be supported");
    }

    #[test]
    fn parses_photos_invocation_with_no_files_flag() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "photos", "-f", "json", "--no-files"]).unwrap();
        match cli.command {
            Command::Photos { format, no_files, summary } => {
                assert_eq!(format, "json");
                assert!(no_files);
                assert!(!summary);
            }
            _ => panic!("expected Photos"),
        }
        // --summary parses too
        let s2 = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "photos", "-f", "pdf", "--summary"]).unwrap();
        assert!(matches!(s2.command, Command::Photos { summary, .. } if summary));
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "photos").unwrap();
        assert!(s.1, "photos must now be supported");
    }

    #[test]
    fn parses_photos_recently_deleted_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "photos-recently-deleted", "-f", "json", "--no-files"]).unwrap();
        match cli.command {
            Command::PhotosRecentlyDeleted { format, no_files } => {
                assert_eq!(format, "json");
                assert!(no_files);
            }
            _ => panic!("expected PhotosRecentlyDeleted"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "photos-recently-deleted").unwrap();
        assert!(s.1, "photos-recently-deleted must be supported");
    }

    #[test]
    fn parses_device_usage_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "device-usage", "-f", "json"]).unwrap();
        match cli.command {
            Command::DeviceUsage { format } => assert_eq!(format, "json"),
            _ => panic!("expected DeviceUsage"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "device-usage").unwrap();
        assert!(s.1, "device-usage must be supported");
    }

    #[test]
    fn parses_interactions_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "interactions", "-f", "json"]).unwrap();
        match cli.command {
            Command::Interactions { format } => assert_eq!(format, "json"),
            _ => panic!("expected Interactions"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "interactions").unwrap();
        assert!(s.1, "interactions must be supported");
    }

    #[test]
    fn parses_data_usage_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "data-usage", "-f", "json"]).unwrap();
        match cli.command {
            Command::DataUsage { format } => assert_eq!(format, "json"),
            _ => panic!("expected DataUsage"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "data-usage").unwrap();
        assert!(s.1, "data-usage must be supported");
    }

    #[test]
    fn parses_homescreen_layout_invocation() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "homescreen-layout", "-f", "json"]).unwrap();
        match cli.command {
            Command::HomescreenLayout { format } => assert_eq!(format, "json"),
            _ => panic!("expected HomescreenLayout"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "homescreen-layout").unwrap();
        assert!(s.1, "homescreen-layout must be supported");
    }

    #[test]
    fn parses_notes_invocation_and_notes_supported() {
        let cli = Cli::try_parse_from(["archive", "--backup", "/b", "-o", "/out", "notes", "-f", "html"]).unwrap();
        match cli.command {
            Command::Notes { format } => assert_eq!(format, "html"),
            _ => panic!("expected Notes"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "notes").unwrap();
        assert!(s.1, "notes must now be supported");
    }

    #[test]
    fn known_stores_lists_voicemail_supported() {
        let vm = KNOWN_STORES.iter().find(|(n, ..)| *n == "voicemail").unwrap();
        assert!(vm.1, "voicemail must be supported");
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
