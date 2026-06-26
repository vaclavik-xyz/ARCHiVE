/*!
 Attachment converter selection.
*/

use std::{
    fmt::{Display, Formatter, Result},
    path::Path,
    process::Command,
};

pub trait Converter {
    /// Detect the converter available in the current shell environment.
    fn determine() -> Option<Self>
    where
        Self: Sized;

    /// Program name for this converter.
    fn name(&self) -> &'static str
    where
        Self: Sized;
}

#[derive(Debug, PartialEq, Eq)]
pub enum ImageType {
    Jpeg,
    Gif,
    Png,
}

impl ImageType {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Gif => "gif",
            Self::Png => "png",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum VideoType {
    Mp4,
}

impl VideoType {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AudioType {
    Mp4,
}

impl AudioType {
    pub fn to_str(&self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
/// Program used to convert or encode images.
pub enum ImageConverter {
    /// macOS built-in image converter.
    Sips,
    /// ImageMagick.
    Imagemagick,
}

impl Converter for ImageConverter {
    fn determine() -> Option<ImageConverter> {
        if exists(ImageConverter::Sips.name()) {
            return Some(ImageConverter::Sips);
        }
        if exists(ImageConverter::Imagemagick.name()) {
            return Some(ImageConverter::Imagemagick);
        }
        eprintln!("No HEIC converter found, image attachments will not be converted!");
        None
    }

    fn name(&self) -> &'static str {
        match self {
            ImageConverter::Sips => "sips",
            ImageConverter::Imagemagick => "magick",
        }
    }
}

impl Display for ImageConverter {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, PartialEq, Eq)]
/// Program used to convert or encode audio.
pub enum AudioConverter {
    /// macOS built-in audio converter.
    AfConvert,
    /// FFmpeg.
    Ffmpeg,
}

impl Converter for AudioConverter {
    fn determine() -> Option<AudioConverter> {
        if exists(AudioConverter::AfConvert.name()) {
            return Some(AudioConverter::AfConvert);
        }
        if exists(AudioConverter::Ffmpeg.name()) {
            return Some(AudioConverter::Ffmpeg);
        }
        eprintln!("No CAF converter found, audio attachments will not be converted!");
        None
    }

    fn name(&self) -> &'static str {
        match self {
            AudioConverter::AfConvert => "afconvert",
            AudioConverter::Ffmpeg => "ffmpeg",
        }
    }
}

impl Display for AudioConverter {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.name())
    }
}

#[derive(Debug, PartialEq, Eq)]
/// Program used to convert or encode video.
pub enum VideoConverter {
    /// FFmpeg.
    Ffmpeg,
}

impl Converter for VideoConverter {
    fn determine() -> Option<VideoConverter> {
        if exists(VideoConverter::Ffmpeg.name()) {
            return Some(VideoConverter::Ffmpeg);
        }
        eprintln!("No MOV converter found, video attachments will not be converted!");
        None
    }

    fn name(&self) -> &'static str {
        match self {
            VideoConverter::Ffmpeg => "ffmpeg",
        }
    }
}

impl Display for VideoConverter {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.name())
    }
}

/// Hardware H.264 encoder supported by FFmpeg.
#[derive(Debug, PartialEq, Eq)]
pub enum HardwareEncoder {
    /// NVIDIA GPU-accelerated H.264 encoder (`NVENC`).
    Nvenc,
    /// Intel Quick Sync Video H.264 encoder (`QSV`).
    Qsv,
    /// Apple `VideoToolbox` H.264 encoder on macOS.
    VideoToolbox,
}

impl HardwareEncoder {
    /// Detect the best available hardware encoder in priority order.
    pub fn detect() -> Option<Self> {
        if let Ok(output) = Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
        {
            let out = String::from_utf8_lossy(&output.stdout);
            if out.contains("h264_nvenc") {
                return Some(Self::Nvenc);
            }
            if out.contains("h264_qsv") {
                return Some(Self::Qsv);
            }
            if out.contains("h264_videotoolbox") {
                return Some(Self::VideoToolbox);
            }
        }
        None
    }

    /// FFmpeg codec name for this encoder.
    pub fn codec_name(&self) -> &'static str {
        match self {
            HardwareEncoder::Nvenc => "h264_nvenc",
            HardwareEncoder::Qsv => "h264_qsv",
            HardwareEncoder::VideoToolbox => "h264_videotoolbox",
        }
    }
}

// MARK: PDF
/// A located headless-browser binary used to render HTML exports to PDF.
///
/// Unlike the other converters, the launcher is frequently a full path (the
/// macOS `.app` bundle executable is not on `PATH`), so this resolves to the
/// concrete launcher string rather than a fixed program name.
#[derive(Debug, PartialEq, Eq)]
pub struct PdfConverter {
    /// Absolute path or `PATH`-resolvable name of the browser launcher.
    pub launcher: String,
}

impl PdfConverter {
    /// Candidate Chrome/Chromium/Edge launchers to probe, in priority order.
    fn candidates() -> &'static [&'static str] {
        #[cfg(target_os = "macos")]
        {
            &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "google-chrome",
                "chromium",
                "microsoft-edge",
            ]
        }
        #[cfg(target_os = "windows")]
        {
            &[
                r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                "chrome",
                "msedge",
            ]
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            &[
                "google-chrome-stable",
                "google-chrome",
                "chromium",
                "chromium-browser",
                "microsoft-edge-stable",
                "microsoft-edge",
                "brave-browser",
            ]
        }
    }

    /// Resolve a usable launcher. An explicit `chrome_path` wins when it points
    /// at an existing file; otherwise known install locations are probed.
    /// Returns [`None`] when no browser is found.
    pub fn resolve(chrome_path: Option<&str>) -> Option<Self> {
        if let Some(path) = chrome_path {
            if Path::new(path).exists() {
                return Some(Self {
                    launcher: path.to_string(),
                });
            }
            eprintln!(
                "--chrome-path `{path}` does not exist; falling back to auto-detection"
            );
        }
        for candidate in Self::candidates() {
            if let Some(found) = locate(candidate) {
                return Some(Self { launcher: found });
            }
        }
        None
    }
}

/// Resolve a single candidate to a usable launcher string: values containing a
/// path separator are checked on disk; bare names are looked up on `PATH`.
fn locate(candidate: &str) -> Option<String> {
    if candidate.contains(std::path::MAIN_SEPARATOR) {
        return Path::new(candidate)
            .exists()
            .then(|| candidate.to_string());
    }
    exists(candidate).then(|| candidate.to_string())
}

/// `true` when a shell program exists on the system.
#[cfg(not(target_family = "windows"))]
fn exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// `true` when a shell program exists on the system.
#[cfg(target_family = "windows")]
fn exists(name: &str) -> bool {
    Command::new("where")
        .arg(name)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod test {
    use super::{PdfConverter, exists};

    #[test]
    fn can_find_program() {
        assert!(exists("ls"));
    }

    #[test]
    fn can_miss_program() {
        assert!(!exists("fake_name"));
    }

    #[test]
    fn pdf_resolve_honors_existing_chrome_path() {
        // An explicit path to an existing file is returned verbatim.
        let resolved = PdfConverter::resolve(Some("/bin/sh")).expect("path exists");
        assert_eq!(resolved.launcher, "/bin/sh");
    }

    #[test]
    fn pdf_resolve_ignores_missing_chrome_path() {
        // A bogus override must never be returned; resolution either finds a
        // real browser by probing or yields nothing.
        let resolved = PdfConverter::resolve(Some("/no/such/chrome-binary-xyz"));
        assert_ne!(
            resolved.map(|c| c.launcher),
            Some("/no/such/chrome-binary-xyz".to_string())
        );
    }
}
