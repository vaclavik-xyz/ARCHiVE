//! Shared audio helpers for the extractors (voicemail, voice memos): output
//! format, ffmpeg discovery, transcoding, and filename-component sanitizers.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// Output format for extracted audio.
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

/// "2020-09-13T12:26:40+00:00" -> "2020-09-13_122640"; anything unexpected -> "unknown".
pub fn compact_date(iso: &str) -> String {
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

/// Sanitize a string for use as a filename component: keep `[A-Za-z0-9+]`, map
/// other characters to `_`, and map empty input to `"unknown"`.
pub fn sanitize_component(s: &str) -> String {
    if s.is_empty() {
        return "unknown".to_string();
    }
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '+' { c } else { '_' })
        .collect()
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

/// Build the ffmpeg argument vector to transcode `input` into `output` for
/// `format`. `Amr` needs no transcoding and returns `None`.
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

/// Run ffmpeg to transcode `raw` into `dest`. Returns `true` on success.
pub fn run_transcode(raw: &Path, dest: &Path, format: AudioFormat) -> bool {
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
    fn compact_date_for_normal_and_malformed_input() {
        assert_eq!(compact_date("2020-09-13T12:26:40+00:00"), "2020-09-13_122640");
        assert_eq!(compact_date(""), "unknown");
        assert_eq!(compact_date("!!!!!!!!!!T12:26:40Z"), "unknown");
    }

    #[test]
    fn sanitize_component_keeps_safe_maps_rest_and_handles_empty() {
        assert_eq!(sanitize_component("+420776452878"), "+420776452878");
        assert_eq!(sanitize_component("a/b \"c\""), "a_b__c_");
        assert_eq!(sanitize_component(""), "unknown");
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
