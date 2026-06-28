# `messages` Export Wrapper Implementation Plan

> **For agentic workers:** implement task-by-task with TDD. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Add `archive ... messages -f <txt|html|pdf>` that drives the bundled `imessage-exporter` binary to export conversation transcripts.

**Architecture:** Thin subprocess wrapper mirroring the existing `backup` command (`run_backup`). Pure helpers (binary discovery, argv, format validation) live in a new `archive/src/messages.rs`; the spawn is in `run_messages`. Auth + device info come from opening the backup with `archive-core` first.

**Tech Stack:** Rust (edition 2024), `std::process::Command`, `archive-core`.

## Global Constraints

- `archive` depends only on `archive-core` at compile time — the exporter coupling is a runtime process call, never a crate dependency.
- Agent contract: exactly one JSON object on stdout; progress on stderr; exit 0 success / 1 usage|other / 2 auth.
- `-x` (cleartext password) passed to the exporter **only when the backup is encrypted**.
- Keep root `README.md`, `archive/README.md`, and `AGENTS.md` in sync in the same change.

---

### Task 1: `archive_core::Backup::is_encrypted()`

**Files:** Modify `archive-core/src/lib.rs` (after `device_info`).

**Interfaces:** Produces `pub fn is_encrypted(&self) -> bool`.

- [ ] Add `pub fn is_encrypted(&self) -> bool { self.raw.is_encrypted() }`.
- [ ] `cargo test -p archive-core` passes (existing tests; no new test — trivial delegating accessor exercised by Task 2/3).

### Task 2: `messages.rs` pure helpers (TDD)

**Files:** Create `archive/src/messages.rs`; Test: same file `#[cfg(test)] mod tests`.

**Interfaces:** Produces `EXPORTER_TOOL`, `EXPORTER_ENV`, `normalize_format`, `messages_args`, `resolve_exporter_from`, `resolve_exporter`.

- [ ] Write failing tests: `normalize_format` (txt/html/pdf case-insensitive, reject csv/json/empty); `messages_args` (unencrypted omits `-x`; encrypted+password adds `-x`; encrypted+no-password omits); `resolve_exporter_from` (env override → sibling → PATH name; empty env treated as unset).
- [ ] Implement helpers to pass.
- [ ] `cargo test -p archive` (messages module) passes.

### Task 3: wire `messages` command (TDD)

**Files:** Modify `archive/src/main.rs` — add `mod messages;`, `Command::Messages { format }`, dispatch arm, `run_messages`, `messages` row in `KNOWN_STORES`, `cli_tests`.

**Interfaces:** Consumes Task 1 + Task 2.

- [ ] Write failing tests in `cli_tests`: `parses_messages_invocation`; `messages_rejects_unsupported_format` (format validated before backup open → usage error, exit 1).
- [ ] Implement `run_messages` (validate format → require `--out`/`--backup` → `open_backup` for auth+device+`is_encrypted` → create `<out>/messages` → resolve exporter → spawn, forward stdout→stderr, wait, check status → JSON envelope).
- [ ] Add `("messages", true, "HomeDomain", "Library/SMS/sms.db")` to `KNOWN_STORES`.
- [ ] `cargo test -p archive` passes; `cargo clippy -p archive -p archive-core` clean.

### Task 4: docs

**Files:** Modify `README.md`, `archive/README.md`, `AGENTS.md`.

- [ ] `archive/README.md`: quick-start `messages` example + Status checklist entry.
- [ ] root `README.md`: add `messages` to the data-type list and the `archive` section examples.
- [ ] `AGENTS.md`: `messages` command contract (flags, envelope, exit codes, env override, binary resolution).
