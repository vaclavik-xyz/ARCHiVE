# Upstream synchronization

ARCHiVE tracks `ReagentX/imessage-exporter`'s `develop` branch. Synchronize with
a merge commit, preserving both upstream history and the fork's ARCHiVE/PDF
features. Do not replace this repository with the upstream tree.

The 2026-09-07 synchronization imports upstream commit
`4d90fc8d20a745c0a8acc1e01c0631c4bab89cb4`. Shared exact dependency pins in
`archive` and `archive-core` must match the upstream crates. Keep the PDF-only
`jpeg-encoder` and `lopdf` dependencies and CLI options when resolving conflicts.

`--use-message-times` stamps the final PDF after rendering, recompression and
chunk merging. Image resizing preserves the attachment timestamps when enabled.

`rust-toolchain.toml` pins the compiler and Clippy version for both local work
and CI. Update that pin explicitly and validate before adopting new lint rules.

Before delivering an upstream synchronization:

```sh
cargo clippy --workspace --all-targets --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --workspace --locked
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/smoke-messages.py
python3 scripts/smoke-messages.py --pdf
# On macOS, also verify the alternate browser engine:
python3 scripts/smoke-messages.py --pdf --chrome-path '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
```

The smoke test creates a synthetic iOS backup from the checked-in database
schema. It exercises the `archive messages` subprocess envelope and summary,
TXT/HTML/PDF transcripts, and message timestamps through the standalone
exporter. It never needs personal data. PDF checks require a rendering engine;
the default Linux CI runs the TXT/HTML paths. Tests gated by
`ARCHIVE_TEST_BACKUP` require a separately supplied real backup.
