# PDF Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `pdf` export format that reuses the HTML exporter and converts each conversation to a small, faithful PDF via headless Chrome.

**Architecture:** PDF export is an orchestration layer over the existing HTML exporter. It runs the HTML export, downscales image attachments with `sips`/ImageMagick, then prints each per-conversation HTML file to PDF with headless Chrome. Both external tools are invoked through the project's existing `run_command` shell-out helper — no new heavy Rust dependency.

**Tech Stack:** Rust (edition 2024), `clap`, headless Chrome/Chromium/Edge, `sips`/ImageMagick.

## Global Constraints

- Rust edition 2024; no new heavy crate dependencies — shell out to system tools (matches existing `sips`/`ffmpeg`/`afconvert`/ImageMagick usage).
- Output: one PDF per conversation. Faithful look (reuse HTML rendering + CSS). All messages + all attachments.
- Image downscale defaults: `--max-image-size 1600` (longest edge px), `--image-quality 80` (JPEG).
- Non-image attachments: preview in PDF, originals kept in the attachments folder beside the PDF.
- Code & commit messages in English; conventional commits; no `Co-Authored-By`.

---

### Task 1: `ExportType::Pdf`

**Files:**
- Modify: `imessage-exporter/src/app/export_type.rs`

**Interfaces:**
- Produces: `ExportType::Pdf`, `ExportType::from_cli("pdf") == Some(Pdf)`, `extension() == ".pdf"`, `Display == "pdf"`.

- [ ] Add `Pdf` variant; `from_cli` matches `"pdf"`; `extension` returns `".pdf"`; `Display` writes `"pdf"`.
- [ ] Update the existing `cant_parse_invalid` test: `"pdf"` now parses; replace with another invalid string (e.g. `"json"`, `"docx"`).
- [ ] Add `can_parse_pdf_any_case` test.
- [ ] `cargo test -p imessage-exporter export_type`; commit `feat: add Pdf export type`.

### Task 2: CLI options + PDF defaults

**Files:**
- Modify: `imessage-exporter/src/app/options.rs`

**Interfaces:**
- Produces: `Options { max_image_size: u32, image_quality: u8, chrome_path: Option<String>, keep_html: bool }`; new CLI flags; `SUPPORTED_FILE_TYPES = "txt, html, pdf"`.

- [ ] Add arg-name consts: `OPTION_MAX_IMAGE_SIZE="max-image-size"`, `OPTION_IMAGE_QUALITY="image-quality"`, `OPTION_CHROME_PATH="chrome-path"`, `OPTION_KEEP_HTML="keep-html"`.
- [ ] Update `SUPPORTED_FILE_TYPES` to `"txt, html, pdf"`; update `ABOUT` to mention pdf.
- [ ] Add the four `Arg`s in the command builder (`get_command`/`get_runtime_options`); `max-image-size` default `"1600"`, `image-quality` default `"80"`, both validated as numbers; `keep-html` is a flag; `chrome-path` takes a value.
- [ ] Add the four fields to `Options` struct + `Debug` impl.
- [ ] Parse them in `from_args`; parse numbers with clear `InvalidOptions` errors.
- [ ] **PDF-mode defaults:** when `export_type == Some(Pdf)`:
  - if `attachment_manager_type` is `None`, use `AttachmentManagerMode::Basic` (converts HEIC→JPEG so Chrome can render; copies video/audio originals).
  - force `no_lazy = true` (Chrome must load every image before printing).
- [ ] Add the new flags to the `format_deps` "requires --format" list so they error without `--format`.
- [ ] `cargo test -p imessage-exporter options`; commit `feat: add pdf CLI options and defaults`.

### Task 3: `PdfConverter` (Chrome detection)

**Files:**
- Modify: `imessage-exporter/src/app/compatibility/models.rs`

**Interfaces:**
- Produces: `enum PdfConverter { Chrome, Chromium, Edge }`, `impl Converter`, plus `pub fn launcher(&self) -> String` returning the absolute macOS `.app` launcher path (or bare binary name on other platforms), and `PdfConverter::resolve(chrome_path: Option<&str>) -> Option<String>`.

- [ ] Add `PdfConverter` enum and a `candidates()` helper listing, per OS, the launcher paths to probe. On macOS probe absolute `.app` paths:
  - `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`
  - `/Applications/Chromium.app/Contents/MacOS/Chromium`
  - `/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge`
  and also bare `google-chrome`, `chromium`, `chromium-browser`, `microsoft-edge` via `exists()` for Linux.
- [ ] `resolve(chrome_path)`: if `chrome_path` is `Some` and the file exists, return it; else return the first detected candidate; else `None`.
- [ ] Test: `resolve(Some("/bin/sh"))` returns `Some("/bin/sh")` (file exists); `resolve(Some("/no/such"))` falls through.
- [ ] `cargo test -p imessage-exporter models`; commit `feat: add PdfConverter detection`.

### Task 4: `@media print` CSS

**Files:**
- Modify: `imessage-exporter/src/exporters/html/resources/style.css`

- [ ] Append an `@media print` block:
  - `@page { size: A4; margin: 12mm; }`
  - `* { -webkit-print-color-adjust: exact; print-color-adjust: exact; }`
  - `.message { break-inside: avoid; }` (and reply/announcement containers as present)
  - `img { max-width: 100%; height: auto; }`
- [ ] `cargo build -p imessage-exporter` (CSS is `include_str!`); commit `feat: add print stylesheet for pdf export`.

### Task 5: PDF orchestration module + runtime wiring

**Files:**
- Create: `imessage-exporter/src/exporters/pdf/mod.rs`
- Modify: `imessage-exporter/src/exporters/mod.rs` (add `pub mod pdf;`)
- Modify: `imessage-exporter/src/app/runtime.rs` (dispatch `ExportType::Pdf`)

**Interfaces:**
- Consumes: `HTML::new`, `run_export`, `Config`, `PdfConverter::resolve`, `Options.{max_image_size,image_quality,chrome_path,keep_html,export_path,attachment_path}`.
- Produces: `pub fn run_pdf_export(config: &Config) -> Result<(), RuntimeError>`.

- [ ] `run_pdf_export`:
  1. `let chrome = PdfConverter::resolve(config.options.chrome_path.as_deref()).ok_or_else(|| RuntimeError::InvalidOptions("No Chrome/Chromium/Edge found. Install Google Chrome or pass --chrome-path".into()))?;`
  2. `run_export(&mut HTML::new(config)?)?;` (writes `<chat>.html` + attachments folder into `export_path`).
  3. `downscale_images(config)` — walk `config.attachment_path()` recursively; for each image whose longest edge > `max_image_size`, run sips/IM in place.
  4. For each `*.html` in `export_path` (top level), `html_to_pdf(&chrome, &html, &pdf)`; on success remove the `.html` unless `keep_html`.
- [ ] `downscale_images`: detect ImageConverter (`ImageConverter::determine()`); for each `.jpg/.jpeg/.png/.gif`: read dims (`sips -g pixelWidth -g pixelHeight`), and if `max(w,h) > max_image_size` run:
  - sips: `sips -Z <max> -s formatOptions <q> <file>`
  - ImageMagick: `magick <file> -resize <max>x<max>\> -quality <q> <file>`
  Failures are non-fatal (`eprintln!` + continue), matching existing converters.
- [ ] `html_to_pdf`: spawn Chrome:
  `<chrome> --headless=new --disable-gpu --no-pdf-header-footer --run-all-compositor-stages-before-draw --virtual-time-budget=600000 --user-data-dir=<temp> --print-to-pdf=<out.pdf> file://<in.html>`
  Use a fresh temp `--user-data-dir` (so it works while the user has Chrome open). Non-zero exit → non-fatal warning, keep the HTML.
- [ ] `exporters/mod.rs`: `pub mod pdf;`
- [ ] `runtime.rs`: in the `match export_type` block add `ExportType::Pdf => { crate::exporters::pdf::run_pdf_export(self)?; }`.
- [ ] `cargo build -p imessage-exporter`; commit `feat: add pdf export orchestration`.

### Task 6: Build, test, cross-compile, validate on real data

- [ ] `cargo test -p imessage-exporter` (all green).
- [ ] Local smoke test against `imessage-database/test_data` if a chat DB fixture is usable, else skip to remote.
- [ ] Cross-compile: `cargo build --release -p imessage-exporter --target x86_64-apple-darwin`.
- [ ] `scp` the binary to `macrecepce@100.89.117.93`.
- [ ] Run on remote against the latest iPhone backup, filtered to the target conversation:
  `imessage-exporter -p ios -i "<backup dir>" -f pdf -t "Jana Ondráčková" -o ~/Downloads/pdf_out`
  (use `--conversation-filter` value that matches; verify with an HTML run first to confirm the filter selects the right chat).
- [ ] Verify: PDF produced; message count matches HTML/txt reference (target ~36,130); images embedded; total size << 223 MB; record wall-clock time.
- [ ] After each commit: `roborev show HEAD` and fix reported findings.

## Self-Review notes

- Spec coverage: ExportType (T1), CLI+defaults (T2), PdfConverter (T3), print CSS (T4), orchestration+downscale+Chrome+cleanup+runtime (T5), testing+real-data validation (T6). All spec sections covered.
- Non-image attachments: handled by reusing HTML exporter's attachment copying (Basic mode keeps video/audio originals in the folder; HTML references them) — no extra task needed.
- Video poster frames (ffmpeg) are out of scope for the target conversation (photos only; remote has no ffmpeg) — Basic mode keeps originals; revisit if a video-heavy conversation is requested.

---

## Addendum 2026-06-25: Quartz/WebKit PDF engine

The Chrome path works but produces large PDFs. We added a second engine,
selectable with `--pdf-engine` (`quartz` is the default on macOS, `chrome`
elsewhere and via the flag). An explicit `--chrome-path` implies `chrome`.

**What we measured (the 35k-message test conversation, ~5,900 pages):**
The PDF size is dominated by the *text*, not images or the engine.
Definitive composition at 400 px images: embedded images ~55 MB, message text
~140 MB, duplicated font subsets ~30–45 MB. iExplorer's 223 MB output is *also*
searchable; it is smaller mainly because it shares ~8 embedded fonts (we emit
one subset per page) and packs ~2,800 pages vs our ~5,900. The text of 35k
messages is the floor every engine pays — so neither engine, nor image
downscaling, makes this conversation dramatically smaller. Quartz lands at
~236 MB (≈ iExplorer) while being complete (~4,700 more messages) and
reproducible.

**Engine design (`native/webkit2pdf/main.swift`, built by `build.rs`):**
A faceless `WKWebView` renders the HTML. `NSPrintOperation` deadlocks in a
headless process, so we paginate ourselves: read every top-level message's
`offsetTop`, break pages at message boundaries (never mid-bubble), render each
A4 slice with `WKWebView.createPDF` (Quartz output: compact, searchable, keeps
embedded JPEGs so no recompression pass), and merge slices with PDFKit. The
helper is compiled for the target arch and embedded via `include_bytes!`;
`exporters/pdf/mod.rs` extracts it and renders chunks in parallel
(`QUARTZ_PARALLELISM`).

**Not implemented (diminishing returns, text floor remains):** font sharing
across pages (super-page render + lopdf MediaBox crop-split, ~ -30–45 MB) and
denser pagination (smaller print font, less faithful look).
