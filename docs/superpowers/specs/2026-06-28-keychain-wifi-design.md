# Keychain Wi-Fi password recovery

**Date:** 2026-06-28
**Status:** Approved (research-backed). **Caveat:** the inner item crypto is
verified only against synthetic round-trip vectors — a real encrypted backup is
needed to confirm end-to-end.

## Problem

For device migration / recovery a shop often needs the saved **Wi-Fi passwords**
from an iOS backup. They live in the backup keychain, which is included **only in
encrypted backups**.

## Feasibility (from research)

crabapple decrypts the keychain *file* (`KeychainDomain/keychain-backup.plist`)
like any other backup file, and — crucially — exposes the already-unwrapped
**protection-class keys** via `Backup.manifest.keys()`
(`ProtectionClassKey.key: EncryptionKey`). So the hard keybag crypto is solved.
What remained was a second decryption layer **inside** the plist, which we
implement.

## Architecture

- **`archive-core::keychain`** — pure decryption/parsing. `extract_wifi(plist_bytes,
  class_keys) -> Vec<WifiCredential{ssid,password}>`. For each `genp` (generic
  password) item that looks like Wi-Fi/AirPort: read the protected blob (4-byte LE
  protection-class id ‖ RFC3394-wrapped per-item key ‖ AES-256-GCM payload),
  RFC3394-unwrap the item key against `class_keys[class_id]` (`aes-kw`), then
  AES-256-GCM decrypt (`aes-gcm`) the secret. Plist parsed with `plist`. Total and
  panic-free: malformed input / missing class key / failed decrypt → that item is
  skipped, never an error. Tested with synthetic wrap+GCM round-trip vectors.
- **`archive-core::Backup::wifi_credentials()`** — locate the keychain entry,
  `decrypt_entry` it, build the `class_id -> key` map from `manifest.keys()`, call
  `extract_wifi`. Empty when the backup has no keychain (unencrypted backups).
- **`archive wifi -f <csv|json|html|pdf>`** — `run_wifi` opens the backup
  (auth/exit-2 enforced), recovers, writes `<OUT>/wifi.<ext>`. Empty → `count: 0`
  + a note that the keychain needs an encrypted backup. The HTML/JSON show the
  PSKs in plaintext (the point); **passwords are never logged to stderr** (only a
  count). Envelope `note` flags that passwords are plaintext.

## New crates

`aes-gcm = 0.11.0`, `aes-kw = 0.3.1`, `plist = 1.9.0` (small pure-Rust; versions
aligned with crabapple 0.4.7's RustCrypto generation, so `aes 0.9` is shared).

## Scope (v1)

Wi-Fi (`genp`/AirPort) **only**, modern AES-GCM backups. **Not** in v1:
website/app passwords (`inet`), certs/keys/identities, legacy AES-CBC-only
backups, or ThisDeviceOnly protection classes whose keys are absent from a
portable backup (skipped). Sensitive output — the explicit `wifi` subcommand is
the opt-in.

## Known limitation

The exact protected-blob layout (nonce/AAD/CBC-vs-GCM) is reverse-engineered and
**not validated against a real iOS keychain** in this build (no encrypted-backup
fixture available). The crypto round-trip is proven on synthetic vectors; if
Apple's real layout differs (e.g. fixed nonce, non-empty AAD), `extract_wifi`
returns nothing rather than wrong data, and the layout constants are the single
place to adjust once a real backup confirms them.
