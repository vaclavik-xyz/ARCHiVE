//! Convert a rendered HTML file to PDF by driving a headless browser.
//!
//! This module does not render messages itself: it takes an already-produced
//! HTML file and prints it to PDF with a headless browser (Chrome, Chromium, or
//! Edge), shelling out exactly the way the attachment converters do so no heavy
//! Rust dependency is pulled into the build. Two pieces of work live here:
//!
//! 1. [`resolve_browser`] — locate a usable browser launcher. An explicit path
//!    wins when it exists; otherwise a per-OS list of known install locations
//!    and `PATH`-resolvable names is probed in priority order. The decision is
//!    factored into the pure, filesystem-free [`pick_browser`] so candidate
//!    selection can be unit-tested without touching disk.
//! 2. [`html_to_pdf`] — spawn the browser headless, watch it with a timeout
//!    watchdog (some browser builds write the PDF and then hang instead of
//!    exiting), and move the finished PDF into place atomically.
//!
//! Everything is `std`-only and never panics on untrusted input: slice access
//! is checked, loops are bounded by a deadline, and failures surface as
//! [`io::Result`] / [`Option`] rather than aborting.

use std::{
    fs::{File, remove_dir_all, remove_file, rename},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};

/// Upper bound on how long to wait for a single browser render to finish. A
/// legitimate render takes seconds; this only guards against a browser that
/// never exits.
const PDF_RENDER_TIMEOUT: Duration = Duration::from_secs(900);

/// How often to poll a running browser for completion.
const PDF_POLL_INTERVAL: Duration = Duration::from_millis(250);

// MARK: Browser resolution

/// Candidate Chrome/Chromium/Edge launchers to probe, in priority order, for the
/// current target OS. Absolute paths point at typical install locations (the
/// macOS `.app` bundle executable is not on `PATH`); bare names are resolved via
/// `PATH`. Returned as owned [`PathBuf`]s so the caller can probe them with a
/// uniform `exists` predicate.
pub fn browser_candidates() -> Vec<PathBuf> {
    let raw: &[&str] = {
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
    };
    raw.iter().map(PathBuf::from).collect()
}

/// Pure browser-selection policy, independent of the real filesystem.
///
/// An explicit launcher wins when `exists` reports it present; a supplied but
/// missing override is ignored (the auto-detection still runs) so a stale
/// `--chrome-path` never produces a bogus launcher. Otherwise the first
/// candidate the probe accepts is returned, or [`None`] when none match.
///
/// `exists` is injected so unit tests can drive selection deterministically
/// without spawning processes or touching disk.
pub fn pick_browser(
    explicit: Option<&Path>,
    candidates: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    // An explicit override wins only when it exists; a bogus override must never
    // win, so fall through to auto-detection rather than returning it.
    if let Some(path) = explicit.filter(|p| exists(p)) {
        return Some(path.to_path_buf());
    }
    candidates.iter().find(|c| exists(c)).cloned()
}

/// Resolve a usable headless-browser launcher.
///
/// `explicit` (e.g. a `--chrome-path` value) wins when it points at an existing
/// path; otherwise the per-OS [`browser_candidates`] are probed in order, with
/// absolute paths checked on disk and bare names looked up on `PATH`. Returns
/// [`None`] when no browser is found.
pub fn resolve_browser(explicit: Option<&Path>) -> Option<PathBuf> {
    let candidates = browser_candidates();
    pick_browser(explicit, &candidates, launcher_exists)
}

/// Whether a launcher is usable: a value containing a path separator is checked
/// on disk; a bare program name is resolved on `PATH` (`which`/`where`).
fn launcher_exists(candidate: &Path) -> bool {
    if has_separator(candidate) {
        return candidate.exists();
    }
    // A bare name with no separator: probe `PATH`.
    candidate
        .to_str()
        .map(on_path)
        .unwrap_or(false)
}

/// Whether `path` contains a path separator (so it should be treated as a
/// concrete location rather than a `PATH`-resolvable program name).
fn has_separator(path: &Path) -> bool {
    path.to_string_lossy()
        .chars()
        .any(std::path::is_separator)
}

/// `true` when a bare program `name` is resolvable on `PATH`.
#[cfg(not(target_family = "windows"))]
fn on_path(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// `true` when a bare program `name` is resolvable on `PATH`.
#[cfg(target_family = "windows")]
fn on_path(name: &str) -> bool {
    Command::new("where")
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

// MARK: Argv construction

/// Build the headless-browser argument vector for printing `html_file` to
/// `out_pdf`.
///
/// `user_data_dir` is a throwaway browser profile: a fresh one per render avoids
/// singleton-lock contention when several renders run back to back. The last
/// argument is the `file://` URL of the HTML input (see [`file_url`]). Classic
/// `--headless` is used deliberately (no `--virtual-time-budget`, which makes
/// some Chrome builds write the PDF and then hang); the watchdog in
/// [`html_to_pdf`] guards against any remaining hang.
pub fn browser_argv(html_file: &Path, out_pdf: &Path, user_data_dir: &Path) -> Vec<String> {
    vec![
        "--headless".to_string(),
        "--disable-gpu".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-crash-reporter".to_string(),
        "--no-pdf-header-footer".to_string(),
        "--run-all-compositor-stages-before-draw".to_string(),
        format!("--user-data-dir={}", user_data_dir.display()),
        format!("--print-to-pdf={}", out_pdf.display()),
        file_url(html_file),
    ]
}

/// Build a `file://` URL for `path`, percent-encoding everything outside the
/// RFC 3986 unreserved set (and the path separators), so spaces, `+`, and
/// diacritics in conversation filenames survive the trip to the browser.
///
/// The path is resolved to an absolute one first so the URL is rooted at
/// `file:///…`. Otherwise a relative path (`out/chat.html`) would put its first
/// segment in the authority slot (`file://out/chat.html`, host `out`) and a
/// Windows drive path (`C:\…`) would land as `file://C:/…` — either of which the
/// browser fails to load. Canonicalization needs the file to exist (it always
/// does at render time) and falls back to leading-slash rooting otherwise.
pub fn file_url(path: &Path) -> String {
    let absolute = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let normalized = absolute.to_string_lossy().replace('\\', "/");
    // Drop Windows' `\\?\` verbatim prefix (now `//?/`) and guarantee a single
    // leading slash, so the result is always `file:///…`, never `file://host/…`.
    let stripped = normalized.strip_prefix("//?/").unwrap_or(&normalized);
    let rooted = if stripped.starts_with('/') {
        stripped.to_string()
    } else {
        format!("/{stripped}")
    };
    let mut url = String::from("file://");
    for ch in rooted.chars() {
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

// MARK: Rendering

/// Print a single HTML file to PDF with a headless `browser`.
///
/// Renders to a temporary sibling of `out_pdf` and replaces the destination only
/// once the new PDF is known-complete, so a stale output is never mistaken for
/// fresh and the destination is updated atomically. A throwaway browser profile
/// is created under the system temp dir (and removed afterward) to avoid
/// singleton-lock contention.
///
/// A watchdog guards the render: the browser is polled, and once a complete PDF
/// is on disk it is stopped even if it has not exited on its own (some builds
/// never do). If the browser neither finishes nor produces a usable PDF within
/// [`PDF_RENDER_TIMEOUT`], it is killed and an error is returned.
pub fn html_to_pdf(browser: &Path, html_file: &Path, out_pdf: &Path) -> io::Result<()> {
    // A unique throwaway profile keeps concurrent/sequential renders from
    // fighting over Chrome's singleton lock.
    let profile = std::env::temp_dir().join(format!(
        "imessage-exporter-pdf-{}-{}",
        std::process::id(),
        unique_suffix(out_pdf)
    ));

    let result = render_with_profile(browser, html_file, out_pdf, &profile);

    // The throwaway profile is no longer needed regardless of outcome.
    let _ = remove_dir_all(&profile);
    result
}

/// Spawn the browser with a specific `user_data_dir` and run the watchdog loop.
fn render_with_profile(
    browser: &Path,
    html_file: &Path,
    out_pdf: &Path,
    user_data_dir: &Path,
) -> io::Result<()> {
    // Render to a temporary sibling and only replace the destination once the
    // new PDF is known-complete.
    let mut tmp = out_pdf.as_os_str().to_owned();
    tmp.push(".part");
    let tmp = PathBuf::from(tmp);
    let _ = remove_file(&tmp);

    let argv = browser_argv(html_file, &tmp, user_data_dir);

    let mut child = Command::new(browser)
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|why| {
            io::Error::new(
                why.kind(),
                format!("could not launch {}: {why}", browser.display()),
            )
        })?;

    let finalize = |tmp: &Path| -> io::Result<()> {
        rename(tmp, out_pdf).map_err(|why| {
            io::Error::new(
                why.kind(),
                format!("could not finalize {}: {why}", out_pdf.display()),
            )
        })
    };

    let deadline = Instant::now() + PDF_RENDER_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if pdf_is_complete(&tmp) {
                    return finalize(&tmp);
                }
                let _ = remove_file(&tmp);
                return Err(io::Error::other(format!(
                    "{} exited with {status} without writing a usable PDF",
                    browser.display()
                )));
            }
            Ok(None) => {
                // The browser is still running. If it has already written a
                // complete PDF (some builds never exit afterward), stop it.
                if pdf_is_complete(&tmp) {
                    let _ = child.kill();
                    let _ = child.wait();
                    return finalize(&tmp);
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    if pdf_is_complete(&tmp) {
                        return finalize(&tmp);
                    }
                    let _ = remove_file(&tmp);
                    return Err(io::Error::other(format!(
                        "{} did not finish within {}s",
                        browser.display(),
                        PDF_RENDER_TIMEOUT.as_secs()
                    )));
                }
                sleep(PDF_POLL_INTERVAL);
            }
            Err(why) => {
                return Err(io::Error::new(
                    why.kind(),
                    format!("error while waiting for {}: {why}", browser.display()),
                ));
            }
        }
    }
}

/// A short, path-derived suffix used to keep throwaway profile names distinct
/// when several renders share a process id.
fn unique_suffix(out_pdf: &Path) -> String {
    out_pdf
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "render".to_string())
}

/// Whether `pdf` exists and ends with the PDF end-of-file marker, which
/// indicates the browser finished writing it. Reads only the trailing window and
/// uses checked access throughout, so a truncated or non-PDF file is simply
/// reported incomplete rather than causing a panic.
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
    let Ok(offset) = i64::try_from(window) else {
        return false;
    };
    if file.seek(SeekFrom::End(-offset)).is_err() {
        return false;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(5).any(|w| w == b"%%EOF")
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: pick_browser

    #[test]
    fn pick_browser_explicit_wins_when_present() {
        let explicit = PathBuf::from("/opt/custom/chrome");
        let candidates = vec![PathBuf::from("/usr/bin/google-chrome")];
        // Only the explicit path exists; it must be returned verbatim.
        let got = pick_browser(Some(&explicit), &candidates, |p| p == explicit);
        assert_eq!(got, Some(explicit));
    }

    #[test]
    fn pick_browser_explicit_wins_over_candidates() {
        let explicit = PathBuf::from("/opt/custom/chrome");
        let candidates = vec![PathBuf::from("/usr/bin/google-chrome")];
        // Both exist: the explicit override still takes priority.
        let got = pick_browser(Some(&explicit), &candidates, |_| true);
        assert_eq!(got, Some(explicit));
    }

    #[test]
    fn pick_browser_falls_back_when_explicit_missing() {
        let explicit = PathBuf::from("/no/such/chrome-binary-xyz");
        let first = PathBuf::from("/usr/bin/google-chrome");
        let candidates = vec![first.clone(), PathBuf::from("/usr/bin/chromium")];
        // The bogus override is ignored; the first existing candidate wins.
        let got = pick_browser(Some(&explicit), &candidates, |p| p != explicit);
        assert_eq!(got, Some(first));
    }

    #[test]
    fn pick_browser_returns_first_existing_candidate() {
        let first = PathBuf::from("/usr/bin/google-chrome");
        let second = PathBuf::from("/usr/bin/chromium");
        let candidates = vec![first, second.clone()];
        // First candidate is absent, second is present: the second wins.
        let got = pick_browser(None, &candidates, |p| p == second);
        assert_eq!(got, Some(second));
    }

    #[test]
    fn pick_browser_none_when_nothing_exists() {
        let candidates = vec![
            PathBuf::from("/usr/bin/google-chrome"),
            PathBuf::from("/usr/bin/chromium"),
        ];
        // Nothing exists and no explicit override: None.
        let got = pick_browser(None, &candidates, |_| false);
        assert_eq!(got, None);
    }

    #[test]
    fn pick_browser_none_with_missing_explicit_and_no_candidates() {
        let explicit = PathBuf::from("/no/such/chrome");
        let got = pick_browser(Some(&explicit), &[], |_| false);
        assert_eq!(got, None);
    }

    #[test]
    fn browser_candidates_is_nonempty_for_target_os() {
        assert!(!browser_candidates().is_empty());
    }

    // MARK: argv construction

    #[test]
    fn argv_contains_headless_print_and_file_url() {
        let argv = browser_argv(
            Path::new("/tmp/Jana.html"),
            Path::new("/tmp/out/Jana.pdf.part"),
            Path::new("/tmp/profile"),
        );
        assert!(argv.iter().any(|a| a == "--headless"), "argv: {argv:?}");
        assert!(argv.iter().any(|a| a == "--disable-gpu"));
        assert!(argv.iter().any(|a| a == "--no-pdf-header-footer"));
        // --print-to-pdf carries the output path.
        assert!(
            argv.iter()
                .any(|a| a == "--print-to-pdf=/tmp/out/Jana.pdf.part"),
            "argv: {argv:?}"
        );
        // --user-data-dir carries the throwaway profile.
        assert!(
            argv.iter().any(|a| a == "--user-data-dir=/tmp/profile"),
            "argv: {argv:?}"
        );
        // The final argument is the file:// URL of the HTML input.
        let last = argv.last().expect("argv is non-empty");
        assert!(last.starts_with("file:///"), "last arg: {last}");
        assert!(last.ends_with("/Jana.html"), "last arg: {last}");
    }

    #[test]
    fn argv_print_to_pdf_argument_present() {
        let argv = browser_argv(
            Path::new("/tmp/a.html"),
            Path::new("/tmp/a.pdf"),
            Path::new("/tmp/p"),
        );
        assert!(argv.iter().any(|a| a.starts_with("--print-to-pdf=")));
    }

    // MARK: file:// URL building

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
    fn file_url_roots_relative_paths() {
        // A non-existent relative path keeps its segments out of the authority
        // slot: `out/Jana.html` must become file:///out/Jana.html, not
        // file://out/Jana.html (host `out`).
        let url = file_url(Path::new("out/Jana.html"));
        assert_eq!(url, "file:///out/Jana.html");
    }

    #[test]
    fn file_url_roots_windows_drive_paths() {
        // `C:\Users\me\Jana.html` must become file:///C:/Users/me/Jana.html,
        // not file://C:/Users/me/Jana.html (host `C:`).
        let url = file_url(Path::new("C:\\Users\\me\\Jana.html"));
        assert_eq!(url, "file:///C:/Users/me/Jana.html");
    }

    #[test]
    fn file_url_canonicalizes_existing_path_to_rooted_url() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ime-fileurl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("chat.html");
        std::fs::File::create(&file)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        let url = file_url(&file);
        assert!(url.starts_with("file:///"), "got {url}");
        assert!(url.ends_with("/chat.html"), "got {url}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // MARK: PDF completeness

    #[test]
    fn pdf_completeness_detects_eof_marker() {
        use std::io::Write;

        let dir = std::env::temp_dir().join(format!("ime-pdf-complete-{}", std::process::id()));
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

    #[test]
    fn pdf_completeness_false_for_tiny_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("ime-pdf-tiny-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tiny = dir.join("tiny.pdf");
        std::fs::File::create(&tiny).unwrap().write_all(b"%%").unwrap();
        // Shorter than the marker: must be reported incomplete, not panic.
        assert!(!pdf_is_complete(&tiny));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // MARK: resolve_browser (real filesystem)

    #[test]
    fn resolve_browser_honors_existing_explicit_path() {
        // An existing explicit path is returned verbatim. `/bin/sh` exists on
        // every unix CI host; skip the assertion elsewhere.
        let sh = Path::new("/bin/sh");
        if sh.exists() {
            assert_eq!(resolve_browser(Some(sh)), Some(sh.to_path_buf()));
        }
    }

    #[test]
    fn resolve_browser_ignores_missing_explicit_path() {
        // A bogus override must never be returned; resolution either finds a real
        // browser by probing or yields nothing.
        let bogus = Path::new("/no/such/chrome-binary-xyz");
        let resolved = resolve_browser(Some(bogus));
        assert_ne!(resolved.as_deref(), Some(bogus));
    }
}
