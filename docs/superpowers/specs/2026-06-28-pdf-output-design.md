# PDF output for the in-process extractors

**Date:** 2026-06-28
**Status:** Approved (research-backed)

## Problem

Every HTML-bearing `archive` command already renders a self-contained HTML
document. A repair/recovery shop wants a customer-ready **PDF** deliverable.

## Approach

Add `Format::Pdf` and render it from the **same HTML** each command already
produces, then print that HTML to PDF with a **headless browser**
(Chrome/Chromium/Edge) — exactly the cross-platform mechanism imessage-exporter
uses. Zero new crates: the browser-resolution and `--print-to-pdf` spawn logic
(~150 std-only lines) is **ported** from imessage-exporter (a binary crate that
cannot be imported) into a new `archive/src/pdf.rs`.

## Pieces

- **`archive/src/pdf.rs`** — `resolve_browser(explicit)` (per-OS candidate list +
  `--chrome-path` override + `PATH` lookup), `html_to_pdf(browser, html, out)`
  (the `--headless --print-to-pdf` spawn with a poll/timeout watchdog and atomic
  rename), and pure, unit-tested helpers (`pick_browser`, `browser_argv`,
  `file_url`). No real browser is spawned in tests.
- **`Format::Pdf`** in `format.rs` (cli token `pdf`, extension `pdf`); accepted by
  `from_cli`/`export_format` alongside csv/json/html.
- **`write_or_pdf(out_file, rendered, format, chrome)`** in `main.rs` — the single
  place that turns a rendered string into a file. For non-PDF it is a plain write;
  for PDF it writes the HTML to a temp file **inside the output dir** (so relative
  media siblings resolve under a `file://` root), prints it to the `.pdf`, and
  removes the temp HTML. A missing browser is a **usage error** (exit 1) with a
  `--chrome-path` hint.
- **`--chrome-path`** global CLI flag.

## Wiring

Each HTML-bearing command's format match maps `Html | Pdf` to its existing
`*_html` renderer; the write step calls `write_or_pdf`. Covers contacts, calls,
voicemail, voice-memos, safari-history/bookmarks, calendar, reminders, mail,
notes, photos, attachments, whatsapp, timeline, recover-deleted, apps, and health
(its block arms). The agent-first JSON envelope is unchanged; `outputs` points at
the `.pdf`.

## Scope (v1)

Browser path only. **Not** in v1: the macOS Quartz/Swift helper (build.rs +
bundled binary), image downscaling / PDF recompression (would pull `lopdf`/image
work), the `recover` one-shot package (stays HTML), or a pure-Rust HTML→PDF
engine (loses askama/CSS fidelity). Fonts/emoji depend on the browser's system
fonts (fine on macOS/Windows; minimal Linux containers may lack fonts).
