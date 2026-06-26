# backup-extractor Increment 2 — Calls, Voicemail & Contact Addresses

**Status:** approved design (pending user spec review)
**Date:** 2026-06-26
**Builds on:** Increment 1 (`docs/superpowers/specs/2026-06-25-backup-extractor-design.md`) — contacts + inspect.

## Goal

Add three pure-SQLite extractors to `backup-extractor`, all following the
Increment 1 extractor → formatter → JSON-envelope pattern:

1. **calls** — call history (`ZCALLRECORD`) → csv/json/html.
2. **contact addresses** — extend the existing contacts extractor with postal
   addresses (finishes the contacts model deferred in Increment 1).
3. **voicemail** — voicemail metadata (`voicemail.db`) → csv/json/html.

No new heavy machinery: no streaming, no protobuf, no large-binary extraction.
Every store is a small SQLite database read in memory, exactly like contacts.

## Architecture

Each new store mirrors the contacts shape already in the crate:

- a parser module (`calls.rs`, `voicemail.rs`; addresses extend `contacts.rs`)
  with a `parse(db_path) -> rusqlite::Result<Vec<T>>` opened read-only,
- rendering functions in `format.rs` (`calls_*`, `voicemail_*`; addresses
  extend `contacts_*`),
- an askama template per record type (`templates/calls.html`,
  `templates/voicemail.html`; `templates/contacts.html` gains an address block),
- CLI wiring in `main.rs` (`Command::Calls`, `Command::Voicemail`, a `run_*` and
  `load_*` per store, plus `inspect` coverage),
- a synthetic fixture in `test_fixtures.rs` per store for always-on unit tests,
  plus the env-gated integration test against a real backup.

**New shared module — `datetime.rs`:** two timestamp epochs appear across these
stores (Cocoa/Core-Data 2001 epoch and Unix epoch), and voicemail mixes both in
one table. A single utility module owns the conversions so no parser hand-rolls
epoch math.

```rust
//! Convert iOS timestamp epochs to ISO 8601 (RFC 3339) UTC strings.

/// Seconds since the Cocoa / Core Data reference date (2001-01-01 00:00:00 UTC)
/// → ISO 8601 UTC. Used by ZDATE (calls) and trashed_date (voicemail).
pub fn cocoa_to_iso(seconds: f64) -> Option<String>;

/// Seconds since the Unix epoch → ISO 8601 UTC. Used by voicemail.date.
pub fn unix_to_iso(seconds: i64) -> Option<String>;
```

Both use `chrono` (`=0.4.44`, already a direct dep of `imessage-database` at
the same pin and default features — `clock`/`iana-time-zone` are already resolved
in the workspace lock, so declaring it in `backup-extractor` pulls no new crates)
and `DateTime::<Utc>::from_timestamp(...).map(|d| d.to_rfc3339())`.
The Cocoa→Unix offset is the constant `978_307_200`. Returns `None` for
out-of-range / sentinel inputs (callers map that to a JSON `null`).

## Tech Stack

Rust 2024, `rusqlite` (`=0.40.0`, bundled), `serde`/`serde_json`, `csv`,
`askama` (`=0.16.0`), `tempfile`, and `chrono` (`=0.4.44`, newly declared in
`backup-extractor/Cargo.toml`). No new transitive trees.

## Global Constraints

- **Agent-first contract unchanged.** A command that parses far enough to run
  prints exactly one JSON object on stdout; human progress goes to stderr. The
  two output channels and exit codes are exactly as Increment 1's `AGENTS.md`
  documents them — this increment must not weaken that wording:
  - `0` success (including store-absent),
  - `2` auth only (locked/wrong-password backup) — emitted as the JSON error
    envelope (`{ ok:false, error, kind:"auth" }`),
  - `1` usage/other (bad `--backup` path, parse/IO failure) — JSON envelope.
  - **The one documented non-JSON exception:** a malformed clap invocation
    (unknown flag, missing required arg, bad subcommand) exits `2` with **empty
    stdout** and a usage message on **stderr** — clap's own error channel, never
    a JSON envelope. `AGENTS.md` already explains how an agent disambiguates this
    clap exit-2 from the auth exit-2 (presence of a JSON object on stdout).

  Password from `--password` or `BACKUP_EXTRACTOR_PASSWORD`; never a blocking
  prompt.
- **`#![warn(missing_docs)]`** stays on `backup-core`; new `backup-extractor`
  public-ish items get doc comments to match the existing style.
- **All version-pinned deps stay `=`-pinned**, matching the workspace.
- **Defensive column detection.** Every parser probes the table's real columns
  via `PRAGMA table_info(<table>)` and selects only columns that exist, mapping
  absent optional columns to `null`. Required core columns missing ⇒ a parse
  error (unsupported/corrupt schema), not a silent empty result.
- **Determinism.** Every extractor returns rows in a defined order
  (calls/voicemail by timestamp ascending; addresses in DB order within a
  contact) so output is stable for tests and diffing.
- **No invented semantics.** Where a field's meaning is version-dependent and
  undocumented (FaceTime audio/video, voicemail `flags` bits), surface the raw
  value and, at most, a clearly-documented best-effort derivation marked as such
  in `AGENTS.md`. Never present a guess as certain.

---

## Feature A — Calls

### Data source

- **Domain / path:** `HomeDomain` / `Library/CallHistoryDB/CallHistory.storedata`
  (already declared in `KNOWN_STORES`; a Core Data SQLite store).
- **Table:** `ZCALLRECORD`, one row per call event (cellular, FaceTime, and on
  newer iOS third-party VoIP calls).
- **Timestamp:** `ZDATE` is the Cocoa/Core-Data reference date (seconds since
  2001-01-01 UTC, stored as REAL); convert with `cocoa_to_iso`.
- **Sources:** iLEAPP `callHistory.py`, kacos2000 `callhistory_storedata.sql`,
  mac4n6 APOLLO `call_history.txt`.

### `Call` data model

`#[derive(Debug, Clone, PartialEq, Serialize)]`

| field | source column | semantics / derivation |
|---|---|---|
| `number` | `ZADDRESS` | Remote party. **BLOB of ASCII bytes** — read as `Vec<u8>` → `String::from_utf8_lossy`, trim trailing NULs. May be a phone number **or** an Apple ID/email (FaceTime). Empty when NULL. |
| `date` | `ZDATE` | ISO 8601 UTC via `cocoa_to_iso`. |
| `duration_seconds` | `ZDURATION` | REAL seconds → rounded to `i64`. `0` for unanswered. |
| `direction` | `ZORIGINATED` | `0` → `"incoming"`, `1` → `"outgoing"`. |
| `answered` | `ZANSWERED` | `1` → `true`, `0` → `false` (missed/declined/no-answer). |
| `service` | `ZSERVICE_PROVIDER` (fallback `ZCALLTYPE`) | `"phone"` (`com.apple.Telephony` or `ZCALLTYPE==1`), `"facetime"` (`com.apple.FaceTime` or `ZCALLTYPE in (8,16)`), else the raw bundle id (third-party) or `"unknown"`. |
| `video` | `ZCALLTYPE` | `Option<bool>`: `8` → `Some(true)`, `16` → `Some(false)`, else `None`. **Best-effort, low confidence** (undocumented, version-dependent). |
| `call_type` | `ZCALLTYPE` | Raw integer, preserved for fidelity (the honest backing for `video`). `Option<i64>`. |
| `location` | `ZLOCATION` | Optional free-text carrier/region hint; `null` when absent/empty. |
| `country` | `ZISO_COUNTRY_CODE` | Optional ISO 3166-1 alpha-2, uppercased; `null` when absent. |

### Parsing (`calls.rs`)

1. Open read-only. Probe `PRAGMA table_info(ZCALLRECORD)` into a column set.
2. **Required core columns:** `ZDATE`, `ZDURATION`, `ZADDRESS`, `ZORIGINATED`,
   `ZANSWERED`, `ZCALLTYPE`. Missing any ⇒ `rusqlite` error surfaced as a parse
   failure (these are present iOS 9–18).
3. **Optional columns** (`ZSERVICE_PROVIDER`, `ZLOCATION`, `ZISO_COUNTRY_CODE`):
   include in the `SELECT` only when present; otherwise treat as `NULL`.
4. `SELECT ... FROM ZCALLRECORD ORDER BY ZDATE` (ascending; deterministic).
5. `ZADDRESS` read via `row.get::<_, Option<Vec<u8>>>` then decode lossily.
6. Derive `direction` / `answered` / `service` / `video` per the table above.

Build the column list and `SELECT` string from the probed set (a small
`has_col(&set, "ZNAME")`-style helper), so an older/newer schema never makes the
whole query fail.

### Output formats

- `csv`, `json` (agent default), `html`. `-f vcf` for `calls` is a **usage
  error** (exit 1, `kind:"usage"`, message names the valid set) — vCard has no
  meaning for calls. The shared `Format` enum is reused; `run_calls` rejects
  `Format::Vcf`.
- CSV header: `number,date,duration_seconds,direction,answered,service,video,call_type,location,country`.
- HTML: `templates/calls.html`, a table mirroring `contacts.html`'s style.

### CLI

- `Command::Calls { format }`; `run_calls(&cli, password, format)`;
  `load_calls(&backup) -> Result<Option<Vec<calls::Call>>, AppError>` mirroring
  `load_contacts` (secure `tempfile::TempDir`, fetch
  `HomeDomain` / `Library/CallHistoryDB/CallHistory.storedata`, parse, RAII
  cleanup).
- Envelope: `{ ok, command:"calls", count, outputs:[...], device }`. Store-absent
  ⇒ `{ ok:true, command:"calls", count:0, outputs:[], note:"this backup has no
  call history", device }`.

---

## Feature B — Contact Addresses

Extends the **existing** contacts extractor; no new command. Finishes the model
the Increment 1 `contacts.rs` doc comment flagged as deferred.

### Data source

`AddressBook.sqlitedb`, the same DB already read for names/phones/emails. Postal
addresses use the multi-value mechanism with a structured sub-table:

- `ABMultiValue` rows with `property = 5` are address **slots** (one per
  Home/Work address). Their `value` is empty — the parts live elsewhere.
- `ABMultiValueEntry.parent_id = ABMultiValue.UID` ties parts to the slot.
  **Critical: join on `UID`, not `ROWID`** (they often coincide, so a ROWID join
  passes tests then silently breaks; authoritative sources — Linux Sleuthing,
  iLEAPP — join on `UID`).
- `ABMultiValueEntry.key → ABMultiValueEntryKey.ROWID`, and the component meaning
  comes from `ABMultiValueEntryKey.value` (`Street`, `City`, `State`, `ZIP`,
  `Country`, `CountryCode`). **Resolve by name (case-insensitive), never by
  hard-coded integer key IDs** (IDs are account/version dependent, and IM keys
  `username`/`service` interleave).
- The slot's Home/Work label comes from `ABMultiValueLabel` exactly as
  phones/emails do (strip `_$!<…>!$_`).
- **Sources:** iLEAPP `addressBook.py`, Linux Sleuthing iOS6 AddressBook writeup,
  kacos2000 `AddressBook_sqlite.sql`.

### `Address` data model

`#[derive(Debug, Clone, PartialEq, Serialize)]`, added to `Contact` as
`pub addresses: Vec<Address>`:

| field | component key | notes |
|---|---|---|
| `label` | `ABMultiValueLabel` | `Home`/`Work`/custom; empty when unlabeled. |
| `street` | `Street` | |
| `city` | `City` | |
| `state` | `State` | |
| `zip` | `ZIP` | |
| `country` | `Country` | one/both/neither of country/country_code may be present |
| `country_code` | `CountryCode` | |

Absent components are empty strings (consistent with the existing string-typed
contact fields).

### Parsing changes (`contacts.rs`)

- The existing per-person multi-value query is extended to also surface the row
  `UID` and to recognise `property = 5` rows.
- For each `property = 5` slot, a second prepared statement fetches its parts:
  `SELECT k.value, e.value FROM ABMultiValueEntry e JOIN ABMultiValueEntryKey k
  ON k.ROWID = e.key WHERE e.parent_id = ?1`, dispatching on `lower(k.value)`
  into the `Address` fields. The slot label is resolved via the existing
  `clean_label` path.
- A contact may have multiple addresses; preserve DB order within the contact.

### Format changes (all four contact formatters)

- **JSON:** automatic via `Serialize` (new `addresses` array).
- **CSV:** new `addresses` column joining each address to a single readable
  string, e.g. `Home: Street, City, State ZIP, Country`; multiple addresses
  joined with `; `.
- **vCard:** emit one `ADR` per address —
  `ADR;TYPE=<label>:;;<street>;<city>;<state>;<zip>;<country>` with every
  component run through `vcard_escape` and the label through `vcard_param`
  (reusing Increment 1's injection-safe helpers). The 7-field ADR structure
  (po-box and extended-address empty) follows RFC 2426.
- **HTML:** `templates/contacts.html` gains an address block per contact.

---

## Feature C — Voicemail

### Data source

- **Domain / path:** `HomeDomain` / `Library/Voicemail/voicemail.db`. **New
  entry in `KNOWN_STORES`** (not present in Increment 1).
- **Table:** `voicemail`, one row per received visual voicemail. `ROWID` is also
  the basename of the on-disk `<ROWID>.amr` audio (not extracted this
  increment).
- **MIXED EPOCHS — the central gotcha:** `date` is **Unix** epoch seconds
  (decode with `unix_to_iso`, no offset); `trashed_date` is **Cocoa 2001** epoch
  seconds (`cocoa_to_iso`); `duration` is a plain seconds count (not a
  timestamp). `trashed_date == 0` is the "not deleted" sentinel.
- **Sources:** Cheeky4n6Monkey voicemail writeup, iLEAPP `voicemail.py`.

### `Voicemail` data model

`#[derive(Debug, Clone, PartialEq, Serialize)]`

| field | source column | semantics / derivation |
|---|---|---|
| `sender` | `sender` | Caller phone number (TEXT, nullable → empty). |
| `date` | `date` | ISO 8601 UTC via `unix_to_iso` (Unix epoch — **no offset**). |
| `duration_seconds` | `duration` | Integer seconds. |
| `trashed` | `trashed_date` | `true` when `trashed_date != 0`. |
| `trashed_at` | `trashed_date` | `Option<String>`: ISO via `cocoa_to_iso` when `!= 0`, else `null`. |
| `expiration` | `expiration` | Optional ISO via `unix_to_iso`; `null` when `0`/absent. |
| `flags` | `flags` | Raw integer, preserved (bit meanings undocumented; do not decode). |

### Parsing (`voicemail.rs`)

1. Open read-only; probe `PRAGMA table_info(voicemail)`.
2. **Required core columns:** `date`, `sender`, `duration`, `trashed_date`,
   `flags`. **Optional:** `expiration` (version-dependent).
3. `SELECT ... FROM voicemail ORDER BY date` (ascending).
4. Apply the correct epoch per column (Unix for `date`/`expiration`, Cocoa for
   `trashed_date`). Guard the `trashed_date == 0` sentinel before converting.
5. The optional `map` table and `receiver`/`callback_num` columns are **out of
   scope** this increment (account/receiver resolution); ignore them.

### Output formats & CLI

- `csv`, `json` (default), `html`; `-f vcf` ⇒ usage error.
- CSV header: `sender,date,duration_seconds,trashed,trashed_at,expiration,flags`.
- `templates/voicemail.html`.
- `Command::Voicemail { format }`; `run_voicemail`; `load_voicemail`.
- Envelope `{ ok, command:"voicemail", count, outputs, device }`; store-absent ⇒
  `count:0`, `note:"this backup has no voicemail"`.

---

## `inspect` generalization

Increment 1 only counted contacts. Now `contacts`, `calls`, and `voicemail` are
all supported and should report a `count` when present. `run_inspect` dispatches
on the store name to the matching `load_*` helper for supported+present stores;
unsupported stores (`photos`, `notes`) keep `count: null`. `KNOWN_STORES` adds a
`voicemail` row and flips `calls` to `supported: true`.

```rust
const KNOWN_STORES: &[(&str, bool, &str, &str)] = &[
    ("contacts",  true, "HomeDomain", "Library/AddressBook/AddressBook.sqlitedb"),
    ("calls",     true, "HomeDomain", "Library/CallHistoryDB/CallHistory.storedata"),
    ("voicemail", true, "HomeDomain", "Library/Voicemail/voicemail.db"),
    ("photos",    false, "CameraRollDomain", "Media/PhotoData/Photos.sqlite"),
    ("notes",     false, "AppDomainGroup-group.com.apple.notes", "NoteStore.sqlite"),
];
```

## Agent contract / `AGENTS.md`

Add a section per new command (`calls`, `voicemail`) and extend the `contacts`
section with the new `addresses` field, each with the JSON envelope shape and a
field table. Document the honest-uncertainty fields explicitly:

- `calls.video` / `calls.call_type` — FaceTime audio/video is a **best-effort,
  version-dependent** derivation; `call_type` is the raw `ZCALLTYPE`.
- `voicemail.flags` — raw bitmask, **not decoded** (use `trashed` for deletion
  status).
- `calls.number` — may be an Apple ID/email for FaceTime calls.

Exit codes and the JSON/stderr split are unchanged; the existing exit-code table
stays as-is.

## Testing strategy

Mirrors Increment 1: always-on unit tests against synthetic SQLite fixtures plus
the env-gated real-backup integration test. The synthetic fixtures are plain
SQLite tables we create and query directly (no crabapple, so none of the
Manifest NSKeyedArchiver fragility that blocked a backup-level fixture in
Increment 1).

- **`datetime.rs`:** unit tests for `cocoa_to_iso` (known Cocoa second → known
  ISO) and `unix_to_iso`, plus the +978307200 relationship and out-of-range
  `None`.
- **`make_callhistory(path)`** fixture: a `ZCALLRECORD` with rows covering
  incoming/outgoing, answered/missed, a phone call (`ZCALLTYPE=1`,
  `com.apple.Telephony`), a FaceTime video (`8`) and audio (`16`), a `ZADDRESS`
  stored as an ASCII **BLOB**, and a NULL `ZLOCATION`/`ZISO_COUNTRY_CODE`. Tests
  assert decoding, epoch conversion, direction/answered/service/video, ordering,
  and that a missing optional column still parses.
- **Contacts addresses:** extend `make_addressbook` with the
  `ABMultiValueEntry`/`ABMultiValueEntryKey` tables and a contact carrying a Home
  address; assert the `UID` join, name-keyed component mapping, label, and the
  vCard `ADR` + CSV `addresses` rendering (including an injection payload in an
  address component).
- **`make_voicemail(path)`** fixture: a `voicemail` table with an active row
  (`trashed_date=0`) and a trashed row (`trashed_date` in Cocoa seconds); assert
  Unix vs Cocoa epoch handling, `trashed` derivation, NULL `sender`, ordering,
  and that the absent optional `expiration` column still parses.
- **Formatters:** csv/json/html for calls and voicemail (header + a row),
  vcf-rejection for both commands, and the extended contacts formatters.
- **CLI:** `Cli::try_parse_from` for `calls`/`voicemail` invocations and the
  `Format::Vcf`-rejection path.

## Error handling / exit codes

Unchanged from Increment 1. `Backup::open` errors map through the existing
`open_error` (`Locked` → auth/exit 2, else other/exit 1). Parse/IO failures are
`other` (exit 1). Store-absent is a **success** (exit 0) with `count: 0`.

## File structure summary

```
backup-extractor/
  Cargo.toml                      # + chrono = "=0.4.44"
  src/
    main.rs                       # + Command::Calls/Voicemail, run_/load_ ×2,
                                  #   inspect generalization, KNOWN_STORES update
    datetime.rs                   # NEW: cocoa_to_iso / unix_to_iso (+ tests)
    calls.rs                      # NEW: Call model + parse (+ tests)
    voicemail.rs                  # NEW: Voicemail model + parse (+ tests)
    contacts.rs                   # + Address model, address parsing
    format.rs                     # + calls_*, voicemail_*, contact ADR/addresses
    test_fixtures.rs              # + make_callhistory, make_voicemail,
                                  #   addressbook addresses
  templates/
    contacts.html                 # + address block
    calls.html                    # NEW
    voicemail.html                # NEW
AGENTS.md                         # + calls/voicemail sections, contacts addresses
```

## Deferred to later increments

- Voicemail/attachment **audio file** extraction (`<ROWID>.amr`) and the
  large-binary streaming it implies in `backup-core`.
- Call **group-participant** handles (the version-varying
  `Z_NREMOTEPARTICIPANTHANDLES` join) — single `ZADDRESS` only this increment.
- Voicemail `map`/`receiver` account resolution.
- `photos`, `notes`, the `pdf` output format, vCard line folding.