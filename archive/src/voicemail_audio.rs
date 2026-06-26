//! Voicemail audio: output format, filenames, and ffmpeg transcoding helpers.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

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
    let time: String = iso[11..19].chars().filter(|c| c.is_ascii_digit()).collect(); // HHMMSS
    if date.len() == 10 && time.len() == 6 {
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
}
