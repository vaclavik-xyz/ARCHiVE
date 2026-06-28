# `messages` command — drive imessage-exporter from `archive`

**Date:** 2026-06-28
**Status:** Approved

## Problem

`archive` can extract Messages *attachments* (media) but not the *text* of
iMessage/SMS/RCS conversations. The workspace already ships a mature, tested
binary — `imessage-exporter` — that produces full transcripts (txt/html/pdf).
A user driving the agent-first `archive` CLI gets attachments without the
conversation text, which is the core of the data.

## Goal

Add `archive --backup <DIR> -o <OUT> messages -f <txt|html|pdf>` that produces a
conversation transcript by **driving the existing `imessage-exporter` binary** —
without re-implementing message decoding.

## Approach: thin subprocess wrapper

`archive` shells out to the `imessage-exporter` binary, exactly mirroring the
existing `backup` command's wrapper of `idevicebackup2`:

- the child's stdout (human progress) is forwarded to *our* stderr, so the agent
  contract (exactly one JSON object on stdout) holds;
- the child's stderr inherits to our stderr;
- a non-zero child exit becomes an `other` error (exit 1).

Rejected alternatives:

- **Library dependency** on `imessage-exporter` — it is a *binary-only* crate
  (no `lib.rs`); exposing its export pipeline as a library means modifying a
  crate that tracks ReagentX upstream, creating merge friction. Avoided.
- **Re-implementing** an `sms.db` reader in `archive-core` — duplicates the
  hard, well-tested part (typedstream/schema reverse-engineering). Avoided.

This keeps `imessage-exporter` pristine and independently testable; `archive`
only orchestrates it. Crate independence (`archive` depends only on
`archive-core`) is preserved — the coupling is a runtime process call, not a
compile-time dependency.

## Binary resolution

`imessage-exporter` builds alongside `archive` in the same workspace `target/`.
Resolution order (`resolve_exporter`):

1. `ARCHIVE_IMESSAGE_EXPORTER` env var (explicit override), if set and non-empty;
2. a sibling binary next to the running `archive` executable
   (`<exe_dir>/imessage-exporter` + platform `EXE_SUFFIX`);
3. the bare name `imessage-exporter` (resolved on `PATH` at spawn).

A spawn failure yields an `other` error (exit 1) with a hint to build the
workspace or set the env override.

## Flag mapping

`archive` flags map onto `imessage-exporter`:

| archive            | imessage-exporter            | note                                        |
|--------------------|------------------------------|---------------------------------------------|
| `--backup <DIR>`   | `-p <DIR>` (`--db-path`)      | iOS backup root                             |
| `-f txt\|html\|pdf`| `-f <fmt>` (`--format`)       | only these three; else usage error (exit 1) |
| `-o <OUT>`         | `-o <OUT>/messages` (`--export-path`) | namespaced subdir so reuse of `-o` doesn't collide |
| (always)           | `-a iOS` (`--platform`)       | pins iOS backup source                      |
| `--password`/env   | `-x <pw>` (`--cleartext-password`) | **only when the backup is encrypted**  |

## Auth contract

`run_messages` opens the backup with `archive-core` *first*. This:

- enforces the auth contract: an encrypted backup without the right password
  fails here with exit 2 (`auth`), before any subprocess;
- yields `device` info for the envelope;
- reports `is_encrypted()`, deciding whether `-x` is passed.

Because `archive-core` and `imessage-exporter` share crabapple, a password that
opens the backup here also decrypts it in the exporter — no double-auth mismatch.
`-x` is omitted for unencrypted backups (keeps the password out of the process
table when unnecessary).

## Output / envelope

The exporter writes its transcript tree (per-conversation files + `attachments/`)
under `<OUT>/messages`. Success envelope (one JSON object on stdout):

```json
{
  "ok": true,
  "command": "messages",
  "format": "html",
  "output": "<OUT>/messages",
  "device": { "name": "...", "model": "...", "ios": "...", "serial": "...", "udid": "..." }
}
```

Consistent with `backup`/`recover`: the envelope points the agent at the output
location rather than embedding message bodies.

## inspect

**Final decision (revised during implementation):** `messages` is **not** added
to `inspect`'s `KNOWN_STORES`. Advertising it as a supported store implied
`recover` would cover it (recover "runs every supported extractor"), but `recover`
runs only the in-process extractors and excludes `messages`. To keep `inspect`
and `recover` consistent, the row was dropped. Message-data presence is still
discoverable via the `attachments` store, which reads the same `sms.db`. (The
original plan to add a `messages` row was reverted on review.)

## Out of scope (possible follow-ups)

- Folding `messages` into the `recover` package (would add the subprocess
  dependency to `recover`).
- Structured per-message JSON (no tool emits it today; new work in
  `imessage-database`).
- Date-range / conversation filters (`-s`/`-e`/`-t` pass-through).

## Testing

Pure helpers unit-tested in `archive/src/messages.rs`:

- `normalize_format` — accepts txt/html/pdf case-insensitively; rejects others.
- `messages_args` — argv shape; `-x` present iff encrypted-with-password.
- `resolve_exporter_from` — env override → sibling → PATH name.

CLI parse + format-rejection tested in `main.rs` `cli_tests`. The live spawn is
covered by manual run against a real backup (cannot run the exporter in unit
tests).
