mod audio;
mod attachments;
mod calendar;
mod calls;
mod contacts;
mod datetime;
mod format;
mod notes;
mod photos;
mod safari;
mod sqlite_util;
mod voice_memos;
mod voicemail;
mod voicemail_audio;
#[cfg(test)]
mod test_fixtures;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::format::Format;

#[derive(Parser)]
#[command(name = "archive", about = "Extract personal data from an iOS backup")]
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
    /// Export voicemail metadata (optionally extract audio with --audio).
    Voicemail {
        /// Output format: csv, json, html.
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
        /// Output format: csv, json, html.
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
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Safari bookmarks.
    SafariBookmarks {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export calendar events.
    Calendar {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Apple Notes (title, folder, dates, body text).
    Notes {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
    },
    /// Export Camera Roll metadata and files (files on by default; --no-files to skip).
    Photos {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip file extraction (metadata catalog only).
        #[arg(long)]
        no_files: bool,
    },
    /// Export Messages attachment metadata and files (files on by default; --no-files to skip).
    Attachments {
        /// Output format: csv, json, html.
        #[arg(long, short = 'f')]
        format: String,
        /// Skip file extraction (metadata catalog only).
        #[arg(long)]
        no_files: bool,
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

fn device_json(d: &archive_core::DeviceInfo) -> serde_json::Value {
    serde_json::json!({ "name": d.device_name, "ios": d.product_version, "udid": d.udid })
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
        Command::Photos { format, no_files } => run_photos(&cli, password.as_deref(), format, *no_files),
        Command::Attachments { format, no_files } => run_attachments(&cli, password.as_deref(), format, *no_files),
        Command::Inspect => run_inspect(&cli, password.as_deref()),
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

fn run_contacts(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown contacts format `{format}` (use csv, json, vcf, html)")))?;

    let out = cli
        .out
        .as_deref()
        .ok_or_else(|| AppError::usage("--out is required to export contacts"))?;

    let backup = archive_core::Backup::open(&cli.backup, password)
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

    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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

/// Validate a `voicemail` format string: csv/json/html only.
fn voicemail_format(format: &str) -> Result<Format, AppError> {
    let f = Format::from_cli(format)
        .ok_or_else(|| AppError::usage(format!("unknown voicemail format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for voicemail (use csv, json, html)"));
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

    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());

    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_voicemail(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "voicemail", "count": 0, "outputs": [],
            "note": "this backup has no voicemail", "device": device
        }));
    };

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
        Format::Html => format::voicemail_html(&items),
        Format::Vcf => unreachable!("voicemail_format rejects vcf"),
    };
    let out_file = out.join(format!("voicemail.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
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
        .ok_or_else(|| AppError::usage(format!("unknown voice-memos format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage("vcf is not a valid format for voice-memos (use csv, json, html)"));
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

    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::voice_memos_html(&items),
        Format::Vcf => unreachable!("voice_memos_format rejects vcf"),
    };
    let out_file = out.join(format!("voice-memos.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
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
        .ok_or_else(|| AppError::usage(format!("unknown {command} format `{format}` (use csv, json, html)")))?;
    if f == Format::Vcf {
        return Err(AppError::usage(format!("vcf is not a valid format for {command} (use csv, json, html)")));
    }
    Ok(f)
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

fn run_safari_history(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "safari-history")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export Safari history"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::safari_history_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("safari-history.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} history visit(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "safari-history", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_safari_bookmarks(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "safari-bookmarks")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export Safari bookmarks"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::safari_bookmarks_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("safari-bookmarks.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} bookmark(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "safari-bookmarks", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_calendar(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "calendar")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export calendar"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::calendar_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("calendar.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} event(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "calendar", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_notes(cli: &Cli, password: Option<&str>, format: &str) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "notes")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export notes"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::notes_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("notes.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} note(s) to {}", items.len(), out_file.display());
    Ok(serde_json::json!({
        "ok": true, "command": "notes", "count": items.len(),
        "outputs": [out_file.to_string_lossy()], "device": device
    }))
}

fn run_photos(cli: &Cli, password: Option<&str>, format: &str, no_files: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "photos")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export photos"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
    let device = device_json(backup.device_info());
    std::fs::create_dir_all(out).map_err(|e| AppError::other(e.to_string()))?;
    let Some(mut items) = load_photos(&backup)? else {
        return Ok(serde_json::json!({
            "ok": true, "command": "photos", "count": 0, "outputs": [],
            "note": "this backup has no photos", "device": device
        }));
    };

    let summary = if no_files {
        None
    } else {
        Some(photos::extract_photos(&backup, &mut items, out).map_err(|e| AppError::other(e.to_string()))?)
    };

    let rendered = match format {
        Format::Csv => format::photos_csv(&items),
        Format::Json => format::photos_json(&items),
        Format::Html => format::photos_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("photos.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
    eprintln!("Wrote {} asset(s) to {}", items.len(), out_file.display());

    let mut envelope = serde_json::json!({
        "ok": true, "command": "photos", "count": items.len(),
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

fn run_attachments(cli: &Cli, password: Option<&str>, format: &str, no_files: bool) -> Result<serde_json::Value, AppError> {
    let format = export_format(format, "attachments")?;
    let out = cli.out.as_deref().ok_or_else(|| AppError::usage("--out is required to export attachments"))?;
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
        Format::Html => format::attachments_html(&items),
        Format::Vcf => unreachable!("export_format rejects vcf"),
    };
    let out_file = out.join(format!("attachments.{}", format.extension()));
    std::fs::write(&out_file, rendered).map_err(|e| AppError::other(e.to_string()))?;
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
    ("voicemail", true, "HomeDomain", "Library/Voicemail/voicemail.db"),
    ("voice-memos", true, "AppDomainGroup-group.com.apple.VoiceMemos", "Recordings/CloudRecordings.db"),
    ("safari-history", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/History.db"),
    ("safari-bookmarks", true, "AppDomain-com.apple.mobilesafari", "Library/Safari/Bookmarks.db"),
    ("calendar", true, "HomeDomain", "Library/Calendar/Calendar.sqlitedb"),
    ("notes", true, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite"),
    ("photos", true, "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
    ("attachments", true, "HomeDomain", "Library/SMS/sms.db"),
];

fn run_inspect(cli: &Cli, password: Option<&str>) -> Result<serde_json::Value, AppError> {
    let backup = archive_core::Backup::open(&cli.backup, password).map_err(open_error)?;
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
                "voicemail" => load_voicemail(&backup).ok().flatten().map(|v| v.len()),
                "voice-memos" => load_voice_memos(&backup).ok().flatten().map(|v| v.len()),
                "safari-history" => load_safari_history(&backup).ok().flatten().map(|v| v.len()),
                "safari-bookmarks" => load_safari_bookmarks(&backup).ok().flatten().map(|v| v.len()),
                "calendar" => load_calendar(&backup).ok().flatten().map(|v| v.len()),
                "notes" => load_notes(&backup).ok().flatten().map(|v| v.len()),
                "photos" => load_photos(&backup).ok().flatten().map(|v| v.len()),
                "attachments" => load_attachments(&backup).ok().flatten().map(|v| v.len()),
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
        assert_eq!(cli.backup, PathBuf::from("/b"));
        match cli.command {
            Command::Contacts { format } => assert_eq!(format, "vcf"),
            _ => panic!("expected Contacts"),
        }
    }

    #[test]
    fn rejects_missing_backup() {
        assert!(Cli::try_parse_from(["archive", "-o", "/out", "contacts", "-f", "csv"]).is_err());
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
        assert_eq!(cli.backup, PathBuf::from("/b"));
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
            Command::Photos { format, no_files } => {
                assert_eq!(format, "json");
                assert!(no_files);
            }
            _ => panic!("expected Photos"),
        }
        let s = KNOWN_STORES.iter().find(|(n, ..)| *n == "photos").unwrap();
        assert!(s.1, "photos must now be supported");
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
