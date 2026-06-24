# PDF Export — Design

Date: 2026-06-24
Status: Approved (brainstorming)
Repo: `vaclavik-xyz/imessage-exporter` (fork of `ReagentX/imessage-exporter`)
Branch: `feat/pdf-export`

## Goal

Add a `pdf` export format alongside the existing `txt` and `html` exporters. A
client needs iMessage data as PDF. Previous attempts (via Cursor) produced PDFs
that were extremely slow to generate and enormous in size; chunking by month was
used as a workaround. The objective here is a PDF that contains **all messages
and all attachments**, **looks faithful** (supports every feature the HTML
exporter supports), and is at the same time **small and fast** to produce.

## Key decisions (from brainstorming)

1. **Fidelity over reinvention.** The PDF must look faithful and support all
   features the HTML exporter already supports (bubbles, tapbacks, replies, URL
   previews, stickers, app balloons, expressives, edited messages, …). We do
   **not** build a PDF layout engine from scratch — we reuse the existing HTML
   rendering.
2. **HTML → PDF via headless Chrome is acceptable.** The machine running the
   export may require Chrome / Chromium / Edge to be installed. This is the
   chosen conversion path because it yields faithful output for free.
3. **Non-playable attachments → preview in PDF + originals in a folder beside
   it.** Video shows a poster frame, audio/documents show an icon + name +
   duration; the original files live in the attachments folder next to the PDF
   and are linked from it. Nothing is embedded inside the PDF except (downscaled)
   images.
4. **One PDF per conversation.** Mirrors how the HTML exporter writes one file
   per chat. This bounds file size naturally and makes month-chunking
   unnecessary.
5. **Defaults approved:** image downscale ~1600 px longest edge, JPEG quality
   80, both tunable via CLI. Intermediate HTML is cleaned up after a successful
   conversion, retained with `--keep-html`.

## Why this is small and fast (the core problem)

- **One PDF per conversation** → size is bounded per file, conversions run
  independently and can be parallelized.
- **Image downscaling is the dominant size lever.** A 12 MP iPhone photo (~3 MB)
  becomes ~150 KB at ~1600 px / JPEG 80. This is what caused the previous
  bloat. Tunable via `--max-image-size` / `--image-quality`.
- **Chrome produces a vector PDF with real, selectable text** (not a single
  rasterized bitmap per page). Text stays searchable (valuable for an
  evidentiary use case) and only images contribute meaningful weight.
- Identical referenced files embed once (same `file://` URL → one image stream).
- Non-image originals stay in the attachments folder, **not** inside the PDF.

## Architecture

```
messages ──HTML exporter──▶ <chat>.html (+ attachments folder)
                                │   ▲ images downscaled (sips -Z), print.css injected
                                ▼
                        Chrome --headless --print-to-pdf
                                │
                                ▼
                            <chat>.pdf   (vector text + embedded small JPEGs)
```

The PDF exporter is an **orchestration layer over the HTML exporter**, not a new
`MessageWriter`. It reuses the entire HTML rendering pipeline, then post-processes
each per-conversation HTML file into a PDF. Both the image downscaling and the
Chrome invocation go through the project's existing `run_command` shell-out
helper — the same mechanism already used for `sips`, `afconvert`, `ffmpeg`, and
ImageMagick. **No heavy Rust dependency is added to the build.**

## Components / changes

### 1. `ExportType::Pdf` — `app/export_type.rs`
- New enum variant `Pdf`.
- `from_cli("pdf") => Some(Pdf)` (case-insensitive, like existing).
- `extension() => ".pdf"`.
- `Display => "pdf"`.
- Update the existing test `cant_parse_invalid` which currently asserts `"pdf"`
  is unparseable — it must now parse.

### 2. `PdfConverter` — `app/compatibility/models.rs`
- New enum mirroring `ImageConverter`, implementing the `Converter` trait.
- Variants for Google Chrome / Chromium / Microsoft Edge.
- `determine()` probes via the existing `exists()` helper, in preference order;
  returns `None` with an actionable message ("install Google Chrome or pass
  `--chrome-path`") when none is found.
- `name()` returns the binary/launcher to invoke.
- Allow an explicit override (CLI `--chrome-path`) that bypasses detection.
- macOS note: the Chrome app launcher lives at
  `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome`; detection must
  account for the `.app` path, not just a bare `PATH` lookup.

### 3. PDF orchestration module — `exporters/pdf/`
- Runs the HTML export into a working directory (e.g. `.pdf_work/` under the
  export path) by reusing `run_export(&mut HTML::new(...))`.
- For each produced `<chat>.html`:
  - Invoke Chrome:
    `chrome --headless=new --no-pdf-header-footer --print-to-pdf=<chat>.pdf
    <chat>.html` plus the flags needed for reliable rendering of local images
    (e.g. `--virtual-time-budget`, `--run-all-compositor-stages-before-draw`).
  - Write `<chat>.pdf` into the export path next to the attachments folder.
- Remove the intermediate HTML after a successful conversion; keep it when
  `--keep-html` is set, or when the conversion failed (for debugging).
- Per-conversation conversion failure is **non-fatal**: warn and continue (same
  pattern as the existing converters' `eprintln!` + continue).

### 4. Image downscaling — `app/compatibility/converters/image.rs`
- Extend image handling so that, in PDF mode, images are resized in addition to
  any HEIC→JPEG conversion:
  - `sips`: `-Z <max-px>` to fit the longest edge, `-s formatOptions <quality>`.
  - ImageMagick: `-resize <N>x<N>\>` (only shrink) `-quality <quality>`.
- PDF mode forces an enabled attachment-copy mode (images must be copied and
  referenced by `file://` path for Chrome to load them) and forces `no_lazy`
  (so `loading="lazy"` is omitted and Chrome renders every image before
  printing).
- Video poster frames reuse the existing ffmpeg path (extract a single frame,
  e.g. `-frames:v 1`); audio/documents render as an icon + name + duration line.
  Originals remain in the attachments folder, linked.

### 5. `print.css` — HTML `<head>` in PDF mode
- `@page` A4 margins.
- `-webkit-print-color-adjust: exact` / `print-color-adjust: exact` so bubble
  colors and backgrounds are preserved.
- `break-inside: avoid` on message bubbles so a message is not split across a
  page boundary.
- `max-width` on images so they never overflow the page.

### 6. Runtime wiring — `app/runtime.rs`
- Add `ExportType::Pdf => run_pdf_export(self)?` to the `match export_type`
  block (alongside `Html` and `Txt`).

### 7. CLI surface — `app/options.rs`
Minimal additions with sensible defaults (YAGNI):
- `--format pdf` (existing flag, new accepted value; update `SUPPORTED_FILE_TYPES`
  and `ABOUT` help text).
- `--max-image-size 1600` (longest edge in px).
- `--image-quality 80` (JPEG quality).
- `--chrome-path <path>` (override converter detection).
- `--keep-html` (retain intermediate HTML next to the PDFs).

## Error handling

- **Chrome missing:** actionable error instructing the user to install Chrome or
  pass `--chrome-path`. Surfaced before doing work where possible.
- **Single-conversation conversion failure:** non-fatal — retain that chat's
  HTML, print a warning, continue with the rest.
- **`sips` / `ffmpeg` missing:** fall back to the original image / skip the
  thumbnail (already the existing behavior of the converters).

## Testing

- **Unit:** `from_cli("pdf")`, `extension()`, `Display`; `PdfConverter::determine`
  with a mocked `exists()`; the updated `cant_parse_invalid` test.
- **Integration:** a small fixture conversation → run the PDF pipeline → assert a
  `.pdf` file is produced, contains the conversation's text (PDF text
  extraction), and its size is within a sane bound.
- **Real-data validation:** run against the last iPhone backup on the remote Mac.
  Verify the PDF contains **all** messages (compare message count against the
  HTML export, where iExplorer previously dropped messages) and that total
  output size is reasonable.

## Out of scope (YAGNI for now)

- Splitting a single extremely large conversation across multiple PDFs (per-chat
  bounding is expected to suffice; revisit only if a real conversation is too big
  for Chrome to render in one page). Noted as a fallback, not built.
- Embedding original video/audio/document files inside the PDF (explicitly
  rejected in favor of a side-by-side attachments folder).
- Combined single-PDF-for-all-conversations mode (rejected: this is the case that
  bloated and slowed previous attempts).
