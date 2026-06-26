//! Voicemail audio: output format, filenames, and ffmpeg transcoding helpers.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::voicemail::Voicemail;

/// Output format for extracted voicemail audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Raw `.amr` copied straight from the backup (no transcoding).
    Amr,
    /// AAC in an `.m4a` container (transcoded via ffmpeg).
    M4a,
    /// PCM `.wav` (transcoded via ffmpeg).
    Wav,
}

impl AudioFormat {
    /// Parse a CLI value (`amr`/`m4a`/`wav`, case-insensitive).
    pub fn from_cli(s: &str) -> Option<AudioFormat> {
        match s.to_ascii_lowercase().as_str() {
            "amr" => Some(AudioFormat::Amr),
            "m4a" => Some(AudioFormat::M4a),
            "wav" => Some(AudioFormat::Wav),
            _ => None,
        }
    }

    /// File extension (no leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            AudioFormat::Amr => "amr",
            AudioFormat::M4a => "m4a",
            AudioFormat::Wav => "wav",
        }
    }

    /// Whether producing this format requires transcoding via ffmpeg.
    pub fn needs_ffmpeg(self) -> bool {
        self != AudioFormat::Amr
    }
}

/// Build a readable, collision-free audio filename `<date>_<sender>_<rowid>.<ext>`.
/// `date` is the record's ISO timestamp compacted to `YYYY-MM-DD_HHMMSS` (or
/// `unknown`); `sender` keeps `[A-Za-z0-9+]` and maps other characters to `_`
/// (or `unknown` when empty). `rowid` guarantees uniqueness.
pub fn audio_filename(date: &str, sender: &str, rowid: i64, ext: &str) -> String {
    format!("{}_{}_{}.{ext}", compact_date(date), sanitize_sender(sender), rowid)
}

/// "2020-09-13T12:26:40+00:00" -> "2020-09-13_122640"; anything unexpected -> "unknown".
fn compact_date(iso: &str) -> String {
    if iso.len() < 19 || !iso.is_char_boundary(10) || !iso.is_char_boundary(19) {
        return "unknown".to_string();
    }
    let date = &iso[..10]; // YYYY-MM-DD
    // Validate the date portion: digits everywhere except '-' at positions 4 and 7.
    let well_formed = date
        .as_bytes()
        .iter()
        .enumerate()
        .all(|(i, &b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
    if !well_formed {
        return "unknown".to_string();
    }
    let time: String = iso[11..19].chars().filter(|c| c.is_ascii_digit()).collect(); // HHMMSS
    if time.len() == 6 {
        format!("{date}_{time}")
    } else {
        "unknown".to_string()
    }
}

fn sanitize_sender(sender: &str) -> String {
    if sender.is_empty() {
        return "unknown".to_string();
    }
    sender
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '+' { c } else { '_' })
        .collect()
}

/// Build the ffmpeg argument vector to transcode `input` (a fetched `.amr`)
/// into `output` for `format`. `Amr` needs no transcoding and returns `None`.
pub fn transcode_args(input: &Path, output: &Path, format: AudioFormat) -> Option<Vec<OsString>> {
    let mut args: Vec<OsString> = vec!["-y".into(), "-i".into(), input.as_os_str().to_owned()];
    match format {
        AudioFormat::Amr => return None,
        AudioFormat::M4a => {
            args.push("-c:a".into());
            args.push("aac".into());
        }
        AudioFormat::Wav => {}
    }
    args.push(output.as_os_str().to_owned());
    Some(args)
}

/// Whether an `ffmpeg` binary is on PATH (probed by running `ffmpeg -version`).
pub fn ffmpeg_available() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Per-run audio extraction outcome, surfaced in the JSON envelope.
pub struct AudioSummary {
    /// The output format that was produced.
    pub format: AudioFormat,
    /// Output-relative directory the files were written to.
    pub dir: String,
    /// Files written (including raw `.amr` kept as a transcode fallback).
    pub extracted: usize,
    /// Rows with no audio present in the backup.
    pub missing: usize,
}

/// Subdirectory (under the export dir) that receives the audio files.
const AUDIO_DIR: &str = "voicemail_audio";

/// Fetch (and optionally transcode) each voicemail's audio into
/// `<out>/voicemail_audio/`, filling each record's `audio_file` in place.
///
/// Best-effort: a row whose audio is absent in the backup is counted `missing`
/// and left `None`; a per-file transcode failure keeps the raw `.amr` and links
/// it. Only directory creation / temp-dir failures are fatal (returned as I/O
/// errors). The caller must have verified ffmpeg availability for non-`amr`
/// formats before calling (see `resolve_audio_format`).
pub fn extract_audio(
    backup: &archive_core::Backup,
    items: &mut [Voicemail],
    out: &Path,
    format: AudioFormat,
) -> std::io::Result<AudioSummary> {
    let audio_dir = out.join(AUDIO_DIR);
    std::fs::create_dir_all(&audio_dir)?;
    let scratch = tempfile::TempDir::new()?;

    let mut extracted = 0usize;
    let mut missing = 0usize;

    for item in items.iter_mut() {
        let src = format!("Library/Voicemail/{}.amr", item.rowid);

        if format == AudioFormat::Amr {
            let name = audio_filename(&item.date, &item.sender, item.rowid, "amr");
            let dest = audio_dir.join(&name);
            match backup.fetch("HomeDomain", &src, &dest) {
                Ok(Some(_)) => {
                    item.audio_file = Some(format!("{AUDIO_DIR}/{name}"));
                    extracted += 1;
                }
                Ok(None) => missing += 1,
                Err(why) => {
                    eprintln!("voicemail {}: audio fetch failed: {why}", item.rowid);
                    missing += 1;
                }
            }
            continue;
        }

        // Transcoding path: fetch the raw `.amr` to scratch, then transcode.
        let raw = scratch.path().join(format!("{}.amr", item.rowid));
        match backup.fetch("HomeDomain", &src, &raw) {
            Ok(Some(_)) => {}
            Ok(None) => {
                missing += 1;
                continue;
            }
            Err(why) => {
                eprintln!("voicemail {}: audio fetch failed: {why}", item.rowid);
                missing += 1;
                continue;
            }
        }

        let name = audio_filename(&item.date, &item.sender, item.rowid, format.extension());
        let dest = audio_dir.join(&name);
        if run_transcode(&raw, &dest, format) {
            item.audio_file = Some(format!("{AUDIO_DIR}/{name}"));
            extracted += 1;
        } else {
            // Transcode failed: keep the raw `.amr` as a fallback.
            let amr_name = audio_filename(&item.date, &item.sender, item.rowid, "amr");
            let amr_dest = audio_dir.join(&amr_name);
            if std::fs::copy(&raw, &amr_dest).is_ok() {
                eprintln!("voicemail {}: ffmpeg transcode failed; kept raw .amr", item.rowid);
                item.audio_file = Some(format!("{AUDIO_DIR}/{amr_name}"));
                extracted += 1;
            } else {
                eprintln!("voicemail {}: transcode and raw fallback both failed", item.rowid);
                missing += 1;
            }
        }
    }

    Ok(AudioSummary { format, dir: AUDIO_DIR.to_string(), extracted, missing })
}

/// Run ffmpeg to transcode `raw` into `dest`. Returns `true` on success.
fn run_transcode(raw: &Path, dest: &Path, format: AudioFormat) -> bool {
    let Some(args) = transcode_args(raw, dest, format) else {
        return false;
    };
    Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn from_cli_parses_known_formats_case_insensitively() {
        assert_eq!(AudioFormat::from_cli("amr"), Some(AudioFormat::Amr));
        assert_eq!(AudioFormat::from_cli("M4A"), Some(AudioFormat::M4a));
        assert_eq!(AudioFormat::from_cli("Wav"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_cli("ogg"), None);
    }

    #[test]
    fn extension_and_needs_ffmpeg() {
        assert_eq!(AudioFormat::Amr.extension(), "amr");
        assert_eq!(AudioFormat::M4a.extension(), "m4a");
        assert_eq!(AudioFormat::Wav.extension(), "wav");
        assert!(!AudioFormat::Amr.needs_ffmpeg());
        assert!(AudioFormat::M4a.needs_ffmpeg());
        assert!(AudioFormat::Wav.needs_ffmpeg());
    }

    #[test]
    fn filename_is_readable_for_normal_input() {
        let name = audio_filename("2020-09-13T12:26:40+00:00", "+420776452878", 3, "m4a");
        assert_eq!(name, "2020-09-13_122640_+420776452878_3.m4a");
    }

    #[test]
    fn filename_falls_back_for_empty_sender_and_date() {
        assert_eq!(audio_filename("", "", 7, "amr"), "unknown_unknown_7.amr");
    }

    #[test]
    fn filename_sanitizes_unsafe_sender_chars() {
        // Spaces, slashes, and quotes collapse to underscores; '+' and alnum survive.
        let name = audio_filename("2020-09-13T12:26:40+00:00", "a/b \"c\"", 5, "wav");
        assert_eq!(name, "2020-09-13_122640_a_b__c__5.wav");
    }

    #[test]
    fn filename_is_unique_per_rowid() {
        let a = audio_filename("2020-09-13T12:26:40+00:00", "+420", 1, "amr");
        let b = audio_filename("2020-09-13T12:26:40+00:00", "+420", 2, "amr");
        assert_ne!(a, b);
    }

    #[test]
    fn transcode_args_for_amr_is_none() {
        assert!(transcode_args(Path::new("in.amr"), Path::new("out.amr"), AudioFormat::Amr).is_none());
    }

    #[test]
    fn transcode_args_for_m4a_and_wav() {
        let m4a = transcode_args(Path::new("in.amr"), Path::new("out.m4a"), AudioFormat::M4a).unwrap();
        let m4a: Vec<String> = m4a.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(m4a, vec!["-y", "-i", "in.amr", "-c:a", "aac", "out.m4a"]);

        let wav = transcode_args(Path::new("in.amr"), Path::new("out.wav"), AudioFormat::Wav).unwrap();
        let wav: Vec<String> = wav.iter().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(wav, vec!["-y", "-i", "in.amr", "out.wav"]);
    }

    #[test]
    fn filename_rejects_malformed_date_portion() {
        // 19+ chars but the date portion isn't YYYY-MM-DD digits → unknown.
        assert_eq!(
            audio_filename("!!!!!!!!!!T12:26:40Z", "+420", 1, "amr"),
            "unknown_+420_1.amr"
        );
    }

    // Integration test against a real backup. Set ARCHIVE_TEST_BACKUP (and
    // ARCHIVE_TEST_PASSWORD if encrypted). Skipped when unset so CI stays green.
    #[test]
    fn extracts_real_voicemail_audio_as_amr() {
        let Ok(dir) = std::env::var("ARCHIVE_TEST_BACKUP") else {
            eprintln!("skipping: set ARCHIVE_TEST_BACKUP to run");
            return;
        };
        let pw = std::env::var("ARCHIVE_TEST_PASSWORD").ok();
        let backup =
            archive_core::Backup::open(Path::new(&dir), pw.as_deref()).expect("open backup");

        // Load the real voicemail metadata.
        let scratch = tempfile::TempDir::new().unwrap();
        let db = scratch.path().join("voicemail.db");
        let Some(db) = backup
            .fetch("HomeDomain", "Library/Voicemail/voicemail.db", &db)
            .expect("fetch voicemail.db")
        else {
            eprintln!("backup has no voicemail store; skipping");
            return;
        };
        let mut items = crate::voicemail::parse(&db).expect("parse voicemail");

        let out = scratch.path().join("out");
        let summary = extract_audio(&backup, &mut items, &out, AudioFormat::Amr).expect("extract");

        // The summary must be internally consistent with the records.
        assert_eq!(summary.dir, "voicemail_audio");
        let linked = items.iter().filter(|v| v.audio_file.is_some()).count();
        assert_eq!(summary.extracted, linked);
        assert_eq!(summary.extracted + summary.missing, items.len());
        // Every linked file exists on disk and is non-empty.
        for v in items.iter().filter_map(|v| v.audio_file.as_ref()) {
            let p = out.join(v);
            assert!(p.is_file(), "linked audio should exist: {}", p.display());
            assert!(std::fs::metadata(&p).unwrap().len() > 0, "audio should be non-empty");
        }
    }
}
