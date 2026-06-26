/*!
 Quartz PDF engine: render HTML to PDF with the bundled WebKit helper.

 `build.rs` compiles `native/webkit2pdf/main.swift` for the target architecture
 and drops the binary in `OUT_DIR`; it is embedded here with [`include_bytes!`].
 At export time the bytes are written to the work directory, marked executable,
 and invoked once per HTML chunk. The helper renders with WebKit and Apple's
 Quartz PDF engine, which keeps text searchable, preserves embedded JPEGs, and
 produces far smaller files than a headless browser.
*/

use std::{
    fs::write,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The `webkit2pdf` helper compiled by `build.rs`. Empty when the build could
/// not produce it (non-macOS target, or `swiftc` unavailable).
#[cfg(target_os = "macos")]
const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/webkit2pdf"));
#[cfg(not(target_os = "macos"))]
const HELPER: &[u8] = &[];

/// Whether the Quartz engine's helper was bundled into this build.
pub fn is_available() -> bool {
    !HELPER.is_empty()
}

/// Write the bundled helper into `dir` and mark it executable, returning its
/// path. Called once per export; the path is reused for every chunk.
pub fn extract_helper(dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join("webkit2pdf");
    write(&path, HELPER)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Render one HTML file to `pdf` with the helper. `root` is the directory the
/// web view is allowed to read, so relative `attachments/...` references
/// resolve. The helper self-limits its runtime, so no watchdog is needed here.
pub fn render(helper: &Path, html: &Path, pdf: &Path, root: &Path) -> Result<(), String> {
    let status = Command::new(helper)
        .arg(html)
        .arg(pdf)
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|why| format!("could not run webkit2pdf: {why}"))?;
    if !status.success() {
        return Err(format!("webkit2pdf exited with {status}"));
    }
    if !pdf.exists() {
        return Err("webkit2pdf exited without producing a PDF".to_string());
    }
    Ok(())
}
