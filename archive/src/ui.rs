//! The `ui` command: a tiny local web app (wizard) so a non-technical user can
//! recover a backup without the CLI. Binds to `127.0.0.1` only, serves one
//! embedded HTML page and a small JSON API that maps 1:1 onto the existing
//! commands (`inspect`, `recover`). No new dependencies: a hand-rolled std
//! HTTP handler for the handful of routes the wizard needs.
//!
//! Stdout contract: prints one JSON envelope (the "listening" report), then
//! serves until `/api/quit` (or Ctrl-C). Progress goes to stderr.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::{AppError, Cli, Command};

/// The wizard page, embedded at compile time from `templates/ui.html`.
const UI_HTML: &str = include_str!("../templates/ui.html");

/// State shared between requests: the fixed password (from `--password`, may be
/// absent) and the last `recover` job.
struct UiState {
    cli_password: Option<String>,
    job: Mutex<UiJob>,
    /// Per-run CSRF token: embedded in the served page, required on every POST.
    token: String,
}

/// Background `recover` job: at most one at a time (the UI starts another only
/// when the previous one finished).
#[derive(Clone)]
struct UiJob {
    running: bool,
    /// `None` until the job finishes; then the full JSON envelope (`ok: false`
    /// on error — the UI renders the message either way).
    result: Option<Value>,
    /// `index.html` of the last successful run, so `/api/open` can reveal it.
    index_html: Option<PathBuf>,
}

impl UiJob {
    fn idle() -> Self {
        UiJob { running: false, result: None, index_html: None }
    }
}

/// Launch the wizard: bind `127.0.0.1:<port>`, print the envelope, open the
/// browser, serve until quit. `password` is the fixed `--password` (the UI can
/// also send its own per request).
pub fn run(port: Option<u16>, password: Option<&str>) -> Result<Value, AppError> {
    let listener = TcpListener::bind(("127.0.0.1", port.unwrap_or(8099)))
        .map_err(|e| AppError::other(format!("cannot bind 127.0.0.1:{}: {e}", port.unwrap_or(8099))))?;
    // Report the actual bound port (bind to 0 lets the OS pick a free one).
    let port = listener.local_addr().map_err(|e| AppError::other(e.to_string()))?.port();
    let url = format!("http://127.0.0.1:{port}");
    let envelope = json!({
        "ok": true, "command": "ui", "url": url, "port": port,
        "note": "serving the recovery wizard; stop with Ctrl-C or /api/quit"
    });
    println!("{envelope}");
    eprintln!("ARCHiVE UI on {url} — opening a browser…");
    open_browser(&url);

    let state = Arc::new(UiState {
        cli_password: password.map(str::to_string),
        job: Mutex::new(UiJob::idle()),
        token: generate_token(),
    });
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let state = Arc::clone(&state);
        // One thread per request keeps `/api/status` polling responsive while a
        // `recover` runs in its own worker thread.
        std::thread::spawn(move || {
            let _ = handle_connection(&mut stream, &state);
        });
    }
    Ok(envelope)
}

/// Dispatch one request. Errors are answered as JSON envelopes, never panics.
fn handle_connection(stream: &mut TcpStream, state: &Arc<UiState>) -> std::io::Result<()> {
    let Some(request) = read_request(stream)? else {
        return Ok(());
    };
    let ParsedRequest { method, path, query, body, headers } = request;

    let mut quit = false;
    let response = match (method.as_str(), path.as_str()) {
        ("GET", "/") => respond_html(&UI_HTML.replace("__CSRF_TOKEN__", &state.token)),
        ("GET", "/api/ping") => respond_json(&json!({"ok": true})),
        ("GET", "/api/inspect") => {
            let params = parse_query(&query);
            respond_json(&inspect(&params, state))
        }
        ("POST", "/api/recover") => {
            if !authorized(&headers, &state.token) {
                respond_json_status(&forbidden(), 403)
            } else {
                let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
                respond_json(&start_recover(&payload, state))
            }
        }
        ("GET", "/api/status") => respond_json(&job_status(state)),
        // Side-effecting routes are POST + token: a cross-origin `<img src>` or
        // link can fire GETs (and form POSTs skip CORS preflight), so any
        // visited webpage could otherwise stop the server or open files. The
        // per-run token is only known to the page this server itself served.
        ("POST", "/api/open") => {
            if !authorized(&headers, &state.token) {
                respond_json_status(&forbidden(), 403)
            } else {
                respond_json(&open_last_export(state))
            }
        }
        ("POST", "/api/quit") => {
            if !authorized(&headers, &state.token) {
                respond_json_status(&forbidden(), 403)
            } else {
                quit = true;
                respond_json(&json!({"ok": true, "note": "bye"}))
            }
        }
        _ => respond_json_status(&json!({"ok": false, "error": "not found", "kind": "usage"}), 404),
    };
    stream.write_all(&response)?;
    stream.flush()?;
    if quit {
        std::process::exit(0);
    }
    Ok(())
}

/// Whether the request carries the correct `X-Archive-Token` header.
fn authorized(headers: &HashMap<String, String>, token: &str) -> bool {
    headers.get("x-archive-token").map(|v| v == token).unwrap_or(false)
}

/// The 403 envelope for a missing/wrong CSRF token.
fn forbidden() -> Value {
    json!({"ok": false, "error": "invalid or missing CSRF token", "kind": "usage"})
}

/// One parsed HTTP request: method, path, query, body, headers (lowercase keys).
struct ParsedRequest {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
    headers: HashMap<String, String>,
}

/// Max accepted body size (10 MB) — requests above it are rejected instead of
/// being read to EOF (an unbounded read would let a rogue request OOM the tool).
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024;

/// Read one HTTP request. `None` on a malformed / empty read.
fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<ParsedRequest>> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    // Read until the header block ends, then up to Content-Length bytes.
    let header_end;
    loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_header_end(&buf) {
            header_end = pos;
            break;
        }
        if buf.len() > 1 << 20 {
            return Ok(None); // header absurdly large — bail
        }
    }
    let head = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default().to_string();
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (target, String::new()),
    };
    let content_length = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if !k.trim().eq_ignore_ascii_case("content-length") {
                return None;
            }
            v.trim().parse::<usize>().ok()
        })
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Ok(None); // rogue oversized request — do not read it
    }
    let mut headers: HashMap<String, String> = HashMap::new();
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let mut body: Vec<u8> = buf[header_end + 4..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(content_length);
    Ok(Some(ParsedRequest { method, path, query, body, headers }))
}

/// Per-run CSRF token: 16 hex chars from the OS entropy pool (fallback: a
/// time+pid mix). It only needs to be unguessable to other local web pages.
fn generate_token() -> String {
    let mut bytes = [0u8; 8];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        // A read failure must not leave the zeros in place — that would make
        // the token a known constant. Fall through to the PRNG instead.
        if f.read_exact(&mut bytes).is_ok() {
            return bytes.iter().map(|b| format!("{b:02x}")).collect();
        }
    }
    // Fallback entropy source (non-unix or /dev/urandom unavailable/failed).
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let mut h = nanos ^ (pid << 32);
    for b in bytes.iter_mut() {
        h = h.wrapping_mul(0x100000001b3);
        *b = (h >> 24) as u8;
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Index of the `\r\n\r\n` header/body separator.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// Percent-decode a query value (also turns `+` into a space).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => match bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(&String::from_utf8_lossy(h), 16).ok()) {
                Some(b) => {
                    out.push(b);
                    i += 3;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `a=1&b=2` into a map (decoded).
fn parse_query(query: &str) -> HashMap<String, String> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (percent_decode(k), percent_decode(v)))
        .collect()
}

/// Run `inspect` in-process and return its envelope. Errors (bad path, wrong
/// password) come back as `ok: false` envelopes — the UI shows them directly.
fn inspect(params: &HashMap<String, String>, state: &UiState) -> Value {
    let Some(backup) = params.get("backup").map(|s| s.trim()).filter(|s| !s.is_empty()) else {
        return json!({"ok": false, "error": "missing backup path", "kind": "usage"});
    };
    let password = params
        .get("password")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| state.cli_password.clone());
    let cli = Cli {
        backup: Some(PathBuf::from(backup)),
        password: None,
        out: None,
        chrome_path: None,
        command: Command::Inspect,
    };
    match crate::run_inspect(&cli, password.as_deref()) {
        Ok(v) => v,
        Err(e) => json!({"ok": false, "error": e.message, "kind": e.kind}),
    }
}

/// Start a `recover` job in a background thread. Only one job at a time; the
/// UI polls `/api/status`. The output directory defaults to
/// `<backup>/../ARCHiVE-export-<timestamp>` when not supplied.
fn start_recover(payload: &Value, state: &Arc<UiState>) -> Value {
    let backup = payload
        .get("backup")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let Some(backup) = backup else {
        return json!({"ok": false, "error": "missing backup path", "kind": "usage"});
    };
    let password = payload
        .get("password")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| state.cli_password.clone());
    let out = payload
        .get("out")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(&backup)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(format!("ARCHiVE-export-{}", chrono::Utc::now().format("%Y%m%d-%H%M%S")))
        });

    {
        let mut job = state.job.lock().unwrap();
        if job.running {
            return json!({"ok": false, "error": "a recovery is already running", "kind": "usage"});
        }
        job.running = true;
        job.result = None;
        job.index_html = None;
    }

    let cli = Cli {
        backup: Some(PathBuf::from(&backup)),
        password: None,
        out: Some(out),
        chrome_path: None,
        command: Command::Recover { no_files: false },
    };
    let state = Arc::clone(state);
    std::thread::spawn(move || {
        let result = crate::run_recover(&cli, password.as_deref(), false);
        let mut job = UiJob::idle();
        match result {
            Ok(envelope) => {
                job.index_html = envelope
                    .get("outputs")
                    .and_then(Value::as_array)
                    .and_then(|a| a.first())
                    .and_then(Value::as_str)
                    .map(PathBuf::from);
                job.result = Some(envelope);
            }
            Err(e) => {
                job.result = Some(json!({"ok": false, "error": e.message, "kind": e.kind}));
            }
        }
        let mut slot = state.job.lock().unwrap();
        *slot = job;
    });
    json!({"ok": true, "started": true})
}

/// Current job state as JSON for `/api/status`.
fn job_status(state: &UiState) -> Value {
    let job = state.job.lock().unwrap();
    json!({
        "ok": true,
        "running": job.running,
        "result": job.result,
        "index_html": job.index_html.as_ref().map(|p| p.to_string_lossy().into_owned()),
    })
}

/// Open the last successful export's `index.html` with the system default app.
/// Only the recorded `index_html` path may be opened — no arbitrary paths.
fn open_last_export(state: &UiState) -> Value {
    let Some(index) = state.job.lock().unwrap().index_html.clone() else {
        return json!({"ok": false, "error": "no finished export to open yet", "kind": "usage"});
    };
    match open_path(&index) {
        Ok(()) => json!({"ok": true, "opened": index.to_string_lossy().into_owned()}),
        Err(e) => json!({"ok": false, "error": format!("cannot open: {e}"), "kind": "other"}),
    }
}

/// Open a URL in the default browser (platform-appropriate, best-effort).
fn open_browser(url: &str) {
    let _ = open_anything(&[
        #[cfg(target_os = "macos")]
        ("open", vec![url.to_string()]),
        #[cfg(all(unix, not(target_os = "macos")))]
        ("xdg-open", vec![url.to_string()]),
        #[cfg(windows)]
        ("cmd", vec!["/C".to_string(), "start".to_string(), String::new(), url.to_string()]),
    ]);
}

/// Open a file with the system default application (best-effort).
fn open_path(path: &std::path::Path) -> Result<(), String> {
    let shown = path.to_string_lossy().into_owned();
    open_anything(&[
        #[cfg(target_os = "macos")]
        ("open", vec![shown]),
        #[cfg(all(unix, not(target_os = "macos")))]
        ("xdg-open", vec![shown]),
        #[cfg(windows)]
        ("explorer", vec![shown]),
    ])
}

/// Try each (program, args) candidate in order; succeed at the first spawn.
fn open_anything(candidates: &[(&str, Vec<String>)]) -> Result<(), String> {
    for (program, args) in candidates {
        if std::process::Command::new(program).args(args).spawn().is_ok() {
            return Ok(());
        }
    }
    Err("no opener available".to_string())
}

// --- HTTP responses ---------------------------------------------------------

fn respond_html(html: &str) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        html.len()
    )
    .into_bytes();
    out.extend_from_slice(html.as_bytes());
    out
}

fn respond_json(v: &Value) -> Vec<u8> {
    respond_json_status(v, 200)
}

fn respond_json_status(v: &Value, status: u16) -> Vec<u8> {
    let text = v.to_string();
    let mut out = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        if status == 200 { "OK" } else { "Error" },
        text.len()
    )
    .into_bytes();
    out.extend_from_slice(text.as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_decode_handles_escapes_and_plus() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("%C5%A1patn%C3%BD"), "špatný");
        assert_eq!(percent_decode("plain"), "plain");
        assert_eq!(percent_decode("bad%2"), "bad%2"); // truncated escape kept
    }

    #[test]
    fn parse_query_splits_pairs() {
        let q = parse_query("backup=/tmp/x&password=hes%20lo&empty=");
        assert_eq!(q.get("backup").map(String::as_str), Some("/tmp/x"));
        assert_eq!(q.get("password").map(String::as_str), Some("hes lo"));
        assert_eq!(q.get("empty").map(String::as_str), Some(""));
        assert!(!q.contains_key("missing"));
    }

    #[test]
    fn find_header_end_finds_crlf_crlf() {
        let buf = b"GET / HTTP/1.1\r\nHost: x\r\n\r\nbody";
        // The separator starts right after the last header line; the body
        // follows at position + 4.
        assert_eq!(find_header_end(buf), Some(23));
        assert_eq!(&buf[find_header_end(buf).unwrap()..find_header_end(buf).unwrap() + 4], b"\r\n\r\n");
    }

    #[test]
    fn respond_json_sets_content_length() {
        let v = json!({"ok": true});
        let resp = respond_json(&v);
        let text = String::from_utf8(resp).unwrap();
        let body = v.to_string();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains(&format!("Content-Length: {}", body.len())));
        assert!(text.ends_with(&body));
    }

    #[test]
    fn job_status_reports_idle_shape() {
        let state = UiState { cli_password: None, job: Mutex::new(UiJob::idle()), token: "t".into() };
        let v = job_status(&state);
        assert_eq!(v["ok"], true);
        assert_eq!(v["running"], false);
        assert!(v["result"].is_null());
        assert!(v["index_html"].is_null());
    }

    #[test]
    fn inspect_requires_backup_path() {
        let state = UiState { cli_password: None, job: Mutex::new(UiJob::idle()), token: "t".into() };
        let params = HashMap::new();
        let err = inspect(&params, &state);
        assert_eq!(err["ok"], false);
        assert_eq!(err["kind"], "usage");
    }

    #[test]
    fn start_recover_requires_backup_path() {
        let state = Arc::new(UiState {
            cli_password: None,
            job: Mutex::new(UiJob::idle()),
            token: "t".into(),
        });
        let err = start_recover(&json!({}), &state);
        assert_eq!(err["ok"], false);
        assert_eq!(err["kind"], "usage");
        // Whitespace-only is treated the same as missing (consistent with inspect).
        let err = start_recover(&json!({"backup": "   "}), &state);
        assert_eq!(err["ok"], false);
        assert_eq!(err["kind"], "usage");
    }

    #[test]
    fn open_last_export_without_job_reports_usage() {
        let state = UiState { cli_password: None, job: Mutex::new(UiJob::idle()), token: "t".into() };
        let v = open_last_export(&state);
        assert_eq!(v["ok"], false);
        assert_eq!(v["kind"], "usage");
    }

    #[test]
    fn authorized_checks_the_token_header() {
        let mut headers = HashMap::new();
        assert!(!authorized(&headers, "secret"));
        headers.insert("x-archive-token".into(), "wrong".into());
        assert!(!authorized(&headers, "secret"));
        headers.insert("x-archive-token".into(), "secret".into());
        assert!(authorized(&headers, "secret"));
    }

    #[test]
    fn generate_token_produces_hex_and_varies() {
        let a = generate_token();
        let b = generate_token();
        assert_eq!(a.len(), 16);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        // Two tokens from the entropy pool differ (astronomically unlikely to collide).
        assert_ne!(a, b);
    }
}
