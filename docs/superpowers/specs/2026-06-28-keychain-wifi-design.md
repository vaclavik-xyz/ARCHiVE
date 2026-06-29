# Keychain Wi-Fi password recovery

**Date:** 2026-06-28 (item format corrected 2026-06-29)
**Status:** Implemented and **validated end-to-end against a real iOS 16
encrypted backup** (recovered 3 saved Wi-Fi networks with their passwords). The
initial item format (guessed from research) was wrong and returned nothing on
real data; the real format below was reverse-engineered against the backup and
the reference implementation (`dunhamsteve/ios`).

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
  class_keys) -> Vec<WifiCredential{ssid,password}>`. For each `genp` item, the
  protected blob is its **`v_Data`**, laid out as `version(u32 LE = 3) ‖ class
  id(u32 LE) ‖ wrapped_key_len(u32 LE) ‖ RFC3394-wrapped per-item key ‖
  ciphertext‖tag`. Steps: RFC3394-unwrap the 32-byte item key with
  `class_keys[class id]` (`aes-kw` — this also integrity-checks the key);
  AES-decrypt the ciphertext — Apple uses **AES-256-GCM with a blank IV (J0 = all
  zeros)**, which is exactly **AES-256-CTR with the counter starting at 1**; the
  trailing 16-byte GCM tag **is verified** before decrypting (under the blank IV
  the tag is `GHASH_H(ct) XOR H`, `H = E(0)`), and a mismatch skips the item. The
  plaintext is an
  **ASN.1 DER** `SET OF SEQUENCE { UTF8String key, value }` of the item's real
  attributes. Wi-Fi items have decrypted `svce = "AirPort"` and `agrp = "apple"`;
  SSID = `acct`, password = `v_Data`. Total and panic-free: a missing class key
  (e.g. non-transferable *ThisDeviceOnly* classes), bad unwrap, or malformed item
  is skipped. Deps: `aes` (CTR), `aes-kw` (unwrap), `plist`. Tested with synthetic
  round-trip vectors in the **real** format and validated on a real backup.
- **`archive-core::Backup::wifi_credentials()`** — locate the keychain entry,
  `decrypt_entry` it, build the `class_id -> key` map from `manifest.keys()`, call
  `extract_wifi`. Empty when the backup has no keychain (unencrypted backups).
- **`archive wifi -f <csv|json|html|pdf>`** — `run_wifi` opens the backup
  (auth/exit-2 enforced), recovers, writes `<OUT>/wifi.<ext>`. Empty → `count: 0`
  + a note that the keychain needs an encrypted backup. `pdf` is rejected for this
  command (it would write a temporary plaintext HTML sidecar). The HTML/JSON show
  the PSKs in plaintext (the point); **passwords are never logged to stderr**
  (only a count). Envelope `note` flags that passwords are plaintext.

## New crates

`aes = 0.9.1` (AES block cipher for the CTR decrypt), `aes-kw = 0.3.1` (RFC3394
unwrap), `plist = 1.9.0` (binary plist). Small pure-Rust, aligned with crabapple
0.4.7's RustCrypto generation (`aes 0.9` shared). The first cut used `aes-gcm`,
dropped once the format turned out to be a blank-IV GCM = plain CTR.

## Scope (v1)

Wi-Fi (`genp`/AirPort) **only**, version-3 (AES-GCM) items. **Not** in v1:
website/app passwords (`inet`), certs/keys/identities, legacy AES-CBC items
(versions 1/2), or *ThisDeviceOnly* protection classes whose keys are not
transferable in a portable backup (they fail to unwrap and are skipped — Wi-Fi
PSKs use the transferable `AfterFirstUnlock` class). Sensitive output — the
explicit `wifi` subcommand is the opt-in.

## Validation

Validated on a real iOS 16.0.3 encrypted backup: recovered 3 saved networks
(`Internet_C9`, `O2-Internet-319`, the personal-hotspot SSID) with their
passwords. The decryption is robust by construction: the RFC3394 unwrap
integrity-checks the per-item key, so a wrong key (or a forged item) fails to
unwrap and is skipped rather than producing wrong data.
