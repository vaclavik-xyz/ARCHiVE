# Deleted-record recovery (SQLite carving)

**Date:** 2026-06-28
**Status:** Approved (research-backed; built via parallel module workflow)

## Problem

Customers come to a repair/recovery shop precisely because they deleted
something. SQLite does not zero deleted rows — it unlinks the cell and returns
its bytes to a free region until the page is reused. Since crabapple already
decrypts each backup database to disk, those freed bytes are recoverable.

## Goal

`archive --backup <DIR> -o <OUT> recover-deleted -f <FORMAT> [--store messages|calls|contacts|all]`
recovers deleted rows from the backup's SQLite databases, best-effort.

## Architecture

Two layers, matching the research recommendation:

1. **`archive-core::carve` — a generic, schema-less carver.** Pure big-endian
   byte parsing (no rusqlite) of the four classic deleted-data regions: freelist
   pages, in-page freeblocks, the unallocated gap, and `-wal` frame bodies
   (including superseded frames). SQLite records are self-describing via serial
   types, so values decode without the table schema. `carve_sqlite(main, wal) ->
   Vec<CarvedRecord>` where `CarvedRecord { rowid, source, values, truncated }`
   and `CarvedValue ∈ {Null,Int,Real,Text,Blob}`. Fully bounds-checked (every
   slice access via `.get`, varints capped at 9 bytes, global caps on
   pages/candidates/record size, visited-sets on all chains) so arbitrary/corrupt
   input never panics. Tested with **real rusqlite fixtures** (insert → delete →
   carve → assert recovered) plus fuzz/no-panic tests.

2. **`archive::recover_deleted` — per-store signatures.** Maps generic
   `CarvedRecord`s to a uniform `DeletedRecord { store, source, rowid, date,
   summary }`:
   - **messages** (`sms.db`): anchored by a 36-char GUID; body = longest non-GUID
     text; date = largest plausible Cocoa value (`cocoa_any_to_iso`).
   - **calls** (`CallHistory.storedata`): anchored by a REAL in the Cocoa-seconds
     date range; address = phone-ish text/blob; duration = a small REAL.
   - **contacts** (`AddressBook.sqlitedb`): softest — joins alphabetic text
     values (names). Noisier; labelled best-effort.
   Near-duplicates (same row surviving in several regions) are de-duped.

## Command flow

For each selected store: `fetch` the DB **and its `-wal` sidecar** to a temp dir
(WAL absent → fine), `carve_sqlite`, apply the store signature, accumulate. Sort
chronologically (undated last). Write one `deleted.<ext>`. The HTML carries a
best-effort warning banner.

Envelope: `{ ok, command:"recover-deleted", count, stores:[{store,recovered}],
outputs, device, note }` where `note` states the best-effort/partial nature.

## Honesty / scope

Recoverability is partial and unpredictable (VACUUM, auto_vacuum, page reuse, and
checkpointed WALs destroy remnants) and carved rows can include false positives.
The output and `note` say so plainly. v1 covers messages/calls/contacts and emits
the summary view; it does **not** do overflow-page reassembly, cross-table joins
(handle→number), NoteStore body recovery, or any DB write-back.
