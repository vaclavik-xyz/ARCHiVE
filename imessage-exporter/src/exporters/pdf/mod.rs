/*!
 PDF export.

 The PDF exporter does not render messages itself. It reuses the [`HTML`]
 exporter to produce one HTML file per conversation (plus a copied, converted
 attachment tree), downscales image attachments so embedded images stay small,
 and then prints each conversation's HTML to a PDF with a headless browser
 (Chrome/Chromium/Edge). Both external tools are invoked via the same shell-out
 approach the attachment converters already use, so no heavy Rust dependency is
 pulled into the build.
*/

use std::{
    ffi::OsString,
    fs::{File, read_dir, remove_dir_all, remove_file},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

/// Upper bound on how long to wait for one conversation to render to PDF.
/// Large conversations (thousands of images) legitimately take minutes; this
/// only guards against a browser that never finishes.
const PDF_RENDER_TIMEOUT: Duration = Duration::from_secs(1800);

/// How often to poll a running browser for completion.
const PDF_POLL_INTERVAL: Duration = Duration::from_millis(250);

use crate::{
    app::{
        compatibility::models::{Converter, ImageConverter, PdfConverter},
        error::RuntimeError,
        runtime::Config,
    },
    exporters::{html::HTML, shared::driver::run_export},
};

/// Render the selected conversations to PDF.
///
/// Resolves a browser first so a missing dependency fails fast, then runs the
/// HTML export, downscales images, and converts each conversation to PDF.
pub fn run_pdf_export(config: &Config) -> Result<(), RuntimeError> {
    let chrome = PdfConverter::resolve(config.options.pdf.chrome_path.as_deref()).ok_or_else(
        || {
            RuntimeError::InvalidOptions(
                "No Chrome, Chromium, or Edge found. Install Google Chrome or pass --chrome-path <path/to/chrome>"
                    .to_string(),
            )
        },
    )?;

    // Render conversations to HTML and copy/convert their attachments.
    run_export(&mut HTML::new(config)?)?;

    // Shrink image attachments before they get embedded into the PDFs.
    let attachments = config.attachment_path();
    if attachments.is_dir() {
        downscale_images(
            &attachments,
            config.options.pdf.max_image_size,
            config.options.pdf.image_quality,
        );
    }

    // Convert every per-conversation HTML file to PDF.
    let export_path = &config.options.export_path;
    let html_files = collect_html_files(export_path)?;
    if html_files.is_empty() {
        eprintln!("No HTML files were produced, so there is nothing to convert to PDF.");
        return Ok(());
    }

    println!(
        "Converting {} conversation(s) to PDF using {}...",
        html_files.len(),
        chrome.launcher
    );

    let work_root =
        std::env::temp_dir().join(format!("imessage-exporter-chrome-{}", std::process::id()));
    let mut failures = 0u32;
    for (idx, html) in html_files.iter().enumerate() {
        let pdf = html.with_extension("pdf");
        // A fresh profile per conversation avoids the singleton-lock contention
        // a shared profile can cause across sequential browser launches.
        let profile = work_root.join(idx.to_string());
        match html_to_pdf(&chrome.launcher, html, &pdf, &profile) {
            Ok(()) => {
                if !config.options.pdf.keep_html {
                    let _ = remove_file(html);
                }
            }
            Err(why) => {
                failures += 1;
                eprintln!(
                    "Failed to convert {} to PDF: {why}. Keeping the HTML file.",
                    html.display()
                );
            }
        }
    }
    // The throwaway browser profiles are no longer needed.
    let _ = remove_dir_all(&work_root);

    if failures > 0 {
        eprintln!("{failures} conversation(s) could not be converted to PDF; their HTML was kept.");
    }
    Ok(())
}

/// Collect the top-level `.html` files in `dir` (the per-conversation exports),
/// sorted for deterministic output. The attachments subdirectory is skipped
/// because it contains no top-level HTML.
fn collect_html_files(dir: &Path) -> Result<Vec<PathBuf>, RuntimeError> {
    let mut files = Vec::new();
    for entry in read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.is_file() && has_extension(&path, "html") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// Downscale every image under `dir` so its longest edge is at most
/// `max_size` pixels, re-encoding JPEGs at `quality`. Resizing is a best-effort
/// pass: a missing converter or a single failure is reported and skipped rather
/// than aborting the export.
fn downscale_images(dir: &Path, max_size: u32, quality: u8) {
    let Some(converter) = ImageConverter::determine() else {
        eprintln!(
            "No image downscaler (sips or ImageMagick) is available; images will be embedded at full size."
        );
        return;
    };

    let mut images = Vec::new();
    collect_images(dir, &mut images);
    if images.is_empty() {
        return;
    }

    let mut resized = 0u64;
    for image in &images {
        if resize_image(&converter, image, max_size, quality) {
            resized += 1;
        }
    }
    if resized > 0 {
        println!("Downscaled {resized} image attachment(s) for PDF embedding.");
    }
}

/// Recursively gather image files (jpg/jpeg/png/gif) under `dir`.
fn collect_images(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, out);
        } else if is_image(&path) {
            out.push(path);
        }
    }
}

/// Resize a single image in place. `sips -Z` and ImageMagick `-resize NxN>`
/// both only ever shrink, so images already within `max_size` are unchanged.
/// Returns `true` when the converter ran successfully.
fn resize_image(converter: &ImageConverter, file: &Path, max_size: u32, quality: u8) -> bool {
    let max = max_size.to_string();
    let q = quality.to_string();
    let is_jpeg = has_extension(file, "jpg") || has_extension(file, "jpeg");

    let args: Vec<OsString> = match converter {
        ImageConverter::Sips => {
            let mut args: Vec<OsString> =
                vec!["-Z".into(), max.clone().into()];
            if is_jpeg {
                args.push("-s".into());
                args.push("formatOptions".into());
                args.push(q.clone().into());
            }
            args.push(file.as_os_str().to_owned());
            args
        }
        ImageConverter::Imagemagick => {
            let mut args: Vec<OsString> = vec![
                file.as_os_str().to_owned(),
                "-resize".into(),
                format!("{max}x{max}>").into(),
            ];
            if is_jpeg {
                args.push("-quality".into());
                args.push(q.into());
            }
            // Output path equals input path to resize in place.
            args.push(file.as_os_str().to_owned());
            args
        }
    };

    run_quiet(converter.name(), args)
}

/// Print a single HTML file to PDF with a headless browser.
///
/// Uses classic headless without `--virtual-time-budget`: that flag makes some
/// Chrome builds write the PDF and then hang instead of exiting. A watchdog
/// guards against any remaining hang — once a complete PDF is on disk, the
/// browser is stopped even if it has not exited on its own.
fn html_to_pdf(launcher: &str, html: &Path, pdf: &Path, user_data_dir: &Path) -> Result<(), String> {
    let _ = remove_file(pdf);
    let mut child = Command::new(launcher)
        .arg("--headless")
        .arg("--disable-gpu")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-crash-reporter")
        .arg("--no-pdf-header-footer")
        .arg("--run-all-compositor-stages-before-draw")
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg(format!("--print-to-pdf={}", pdf.display()))
        .arg(file_url(html))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|why| format!("could not launch {launcher}: {why}"))?;

    let deadline = Instant::now() + PDF_RENDER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() || pdf_is_complete(pdf) {
                    Ok(())
                } else {
                    Err(format!("{launcher} exited with {status} without writing a usable PDF"))
                };
            }
            Ok(None) => {
                // The browser is still running. If it has already written a
                // complete PDF (some builds never exit afterward), stop it.
                if pdf_is_complete(pdf) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return if pdf_is_complete(pdf) {
                        Ok(())
                    } else {
                        Err(format!(
                            "{launcher} did not finish within {}s",
                            PDF_RENDER_TIMEOUT.as_secs()
                        ))
                    };
                }
                sleep(PDF_POLL_INTERVAL);
            }
            Err(why) => return Err(format!("error while waiting for {launcher}: {why}")),
        }
    }
}

/// Whether `pdf` exists and ends with the PDF end-of-file marker, which
/// indicates the browser finished writing it.
fn pdf_is_complete(pdf: &Path) -> bool {
    let Ok(mut file) = File::open(pdf) else {
        return false;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return false;
    };
    if len < b"%%EOF".len() as u64 {
        return false;
    }
    // The marker sits at the very end, optionally followed by whitespace.
    let window = len.min(1024);
    if file.seek(SeekFrom::End(-(window as i64))).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(5).any(|w| w == b"%%EOF")
}

/// Run `program` with `args`, discarding output. Returns `true` on success.
fn run_quiet(program: &str, args: Vec<OsString>) -> bool {
    match Command::new(program)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("{program} exited with {status}");
            false
        }
        Err(why) => {
            eprintln!("Could not run {program}: {why}");
            false
        }
    }
}

/// Build a `file://` URL for `path`, percent-encoding everything outside the
/// RFC 3986 unreserved set (and the path separators), so spaces, `+`, and
/// diacritics in conversation filenames survive the trip to the browser.
fn file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file://");
    for ch in normalized.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' => url.push(ch),
            _ => {
                let mut buf = [0u8; 4];
                for &byte in ch.encode_utf8(&mut buf).as_bytes() {
                    url.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    url
}

/// Whether `path` ends with the given (case-insensitive) extension.
fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether `path` is a raster image the downscaler can handle.
fn is_image(path: &Path) -> bool {
    has_extension(path, "jpg")
        || has_extension(path, "jpeg")
        || has_extension(path, "png")
        || has_extension(path, "gif")
}

#[cfg(test)]
mod tests {
    use super::{file_url, has_extension, is_image};
    use std::path::Path;

    #[test]
    fn file_url_encodes_spaces_and_plus() {
        let url = file_url(Path::new("/tmp/Messages with Jana +420.html"));
        assert_eq!(url, "file:///tmp/Messages%20with%20Jana%20%2B420.html");
    }

    #[test]
    fn file_url_encodes_diacritics_as_utf8() {
        // `č` is U+010D -> UTF-8 0xC4 0x8D.
        let url = file_url(Path::new("/tmp/Ondráčková.html"));
        assert!(url.contains("Ondr%C3%A1%C4%8Dkov%C3%A1.html"));
        assert!(url.starts_with("file:///tmp/"));
    }

    #[test]
    fn detects_image_extensions_case_insensitively() {
        assert!(is_image(Path::new("a/b/IMG.JPG")));
        assert!(is_image(Path::new("a/b/photo.jpeg")));
        assert!(is_image(Path::new("a/b/shot.PNG")));
        assert!(is_image(Path::new("a/b/anim.gif")));
        assert!(!is_image(Path::new("a/b/movie.mov")));
        assert!(!is_image(Path::new("a/b/clip.mp4")));
    }

    #[test]
    fn has_extension_matches_last_component_only() {
        assert!(has_extension(Path::new("Jana O. - 123.html"), "html"));
        assert!(!has_extension(Path::new("Jana O. - 123.html"), "pdf"));
    }

    #[test]
    fn pdf_completeness_detects_eof_marker() {
        use super::pdf_is_complete;
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("ime-pdf-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let missing = dir.join("missing.pdf");
        assert!(!pdf_is_complete(&missing));

        let partial = dir.join("partial.pdf");
        std::fs::File::create(&partial)
            .unwrap()
            .write_all(b"%PDF-1.7\nstream without a terminator")
            .unwrap();
        assert!(!pdf_is_complete(&partial));

        let complete = dir.join("complete.pdf");
        std::fs::File::create(&complete)
            .unwrap()
            .write_all(b"%PDF-1.7\n... body ...\n%%EOF\n")
            .unwrap();
        assert!(pdf_is_complete(&complete));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
