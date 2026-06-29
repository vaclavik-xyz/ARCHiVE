//! Recover saved Wi-Fi passwords from a decrypted iOS backup keychain.
//!
//! # Where this fits
//!
//! Keychain recovery from a backup is a two-layer decryption problem:
//!
//! 1. **File layer** (the controller, via crabapple): locate
//!    `KeychainDomain/keychain-backup.plist`, file-decrypt it, and read the
//!    per-protection-class keys from crabapple's manifest keybag.
//! 2. **Item layer** (this module): each keychain item's value is independently
//!    encrypted with a per-item key that is itself AES-key-wrapped under a
//!    protection-class key. This module decrypts those items and extracts Wi-Fi
//!    credentials. Interface is decoupled from crabapple: plain plist bytes plus a
//!    `HashMap<u32, Vec<u8>>` of protection-class id → raw class key.
//!
//! # Item format (validated against a real iOS 16 encrypted backup)
//!
//! Each `genp` item is a plist dict whose **`v_Data`** is the protected blob:
//!
//! ```text
//! ┌───────────┬───────────┬──────────────┬──────────────────┬───────────────────────┐
//! │ version   │ class id  │ wrapped len  │ RFC3394 wrapped   │ Apple-GCM ciphertext  │
//! │ u32 LE =3 │ u32 LE    │ u32 LE (=40) │ per-item key      │ ciphertext ‖ tag(16)  │
//! └───────────┴───────────┴──────────────┴──────────────────┴───────────────────────┘
//! ```
//!
//! The wrapped key is RFC 3394-unwrapped with `class_keys[class id]` (which also
//! integrity-checks the key). Apple encrypts the payload with AES-256-GCM using a
//! **blank IV** (J0 = all-zeros), which is exactly AES-256-CTR with the counter
//! block starting at 1 (32-bit big-endian increment of the last four bytes). The
//! trailing 16-byte GCM tag **is** verified before decrypting (a mismatch skips
//! the item); under the blank IV the tag is `GHASH_H(ciphertext) XOR H`.
//!
//! The decrypted plaintext is an **ASN.1 DER** `SET OF SEQUENCE { UTF8String key,
//! value }` — the item's real attributes. Wi-Fi items have `svce = "AirPort"` and
//! `agrp = "apple"`; the SSID is `acct` and the password is `v_Data`.
//!
//! Robustness: this module is total and panic-free. A malformed plist yields an
//! empty `Vec`; any item that cannot be unwrapped/decrypted/parsed is skipped.
//! "This device only" protection classes (whose keys are not transferable in a
//! portable backup) fail to unwrap and are skipped — Wi-Fi PSKs use the
//! transferable `AfterFirstUnlock` class and decrypt fine.

use std::collections::HashMap;

use aes::Aes256;
use aes::cipher::{BlockCipherEncrypt, KeyInit};
use aes_kw::KwAes256;
use plist::Value;
use serde::Serialize;

/// The keychain plist's generic-password array — where Wi-Fi PSKs (and other
/// app/service secrets) live.
const GENP_KEY: &str = "genp";

/// The keychain plist's internet-password array — Safari/app website logins.
const INET_KEY: &str = "inet";

/// The keychain plist's certificate and key arrays (no passwords; counted by the
/// inventory census).
const CERT_KEY: &str = "cert";
const KEYS_KEY: &str = "keys";

/// Only version-3 (AES-GCM) items are supported; older CBC items are skipped.
const ITEM_VERSION_GCM: u32 = 3;

/// AES-GCM authentication tag length (trailing 16 bytes of the payload; verified).
const GCM_TAG_LEN: usize = 16;

/// One recovered Wi-Fi credential: the network name and its plaintext password.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WifiCredential {
    /// The Wi-Fi network name (SSID), from the item's decrypted `acct` attribute.
    pub ssid: String,
    /// The recovered pre-shared key / password in plaintext.
    pub password: String,
}

/// One recovered website/app login: where it applies, the account, and the
/// plaintext password.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PasswordCredential {
    /// Service/host the login is for (the item's `srvr`, e.g. `accounts.google.com`).
    pub service: String,
    /// Account / username (the item's `acct`); may be empty.
    pub account: String,
    /// The recovered password in plaintext.
    pub password: String,
    /// Protocol marker (the item's `ptcl`, e.g. `htps`); empty when absent.
    pub protocol: String,
}

/// Extract saved Wi-Fi credentials from decrypted keychain plist bytes.
///
/// `plist_bytes` is the decrypted `keychain-backup.plist`; `class_keys` maps
/// protection-class id → raw class key (from crabapple's manifest keybag). Total
/// and panic-free: any item that cannot be decoded is skipped. Only `genp`
/// AirPort/Wi-Fi items are returned.
pub fn extract_wifi(plist_bytes: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Vec<WifiCredential> {
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items_array(&root, GENP_KEY) {
        let Some(attrs) = item_attributes(item, class_keys) else {
            continue;
        };
        // Wi-Fi marker comes from the DECRYPTED, authoritative attributes.
        let is_wifi = attr(&attrs, "svce").is_some_and(|v| v.eq_ignore_ascii_case(b"AirPort"))
            && attr(&attrs, "agrp").is_some_and(|v| v == b"apple");
        if !is_wifi {
            continue;
        }
        let (Some(ssid), Some(pw)) = (
            attr(&attrs, "acct").and_then(utf8_nonempty),
            attr(&attrs, "v_Data").and_then(utf8_nonempty),
        ) else {
            continue;
        };
        out.push(WifiCredential { ssid, password: pw });
    }
    out
}

/// Extract saved website/app passwords (the keychain `inet` array) from decrypted
/// keychain plist bytes. Same decryption pipeline as [`extract_wifi`]; total and
/// panic-free. An item is returned only when it yields a non-empty plaintext
/// password and at least one of service/account — items that fail to decrypt
/// (e.g. non-transferable *ThisDeviceOnly* classes) are skipped.
pub fn extract_passwords(plist_bytes: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Vec<PasswordCredential> {
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items_array(&root, INET_KEY) {
        let Some(attrs) = item_attributes(item, class_keys) else {
            continue;
        };
        // Keep only USER website/app logins, identified by the entitlement-enforced
        // access group: the bulk of the `inet` array on a real device is Apple's
        // internal iCloud-Keychain-sync machinery (com.apple.security.ckks, …)
        // holding 88-byte sync key material, NOT passwords. Anchor on `agrp`.
        let agrp = attr(&attrs, "agrp").and_then(utf8_nonempty).unwrap_or_default();
        if !is_user_credential_group(&agrp) {
            continue;
        }
        let Some(password) = attr(&attrs, "v_Data").and_then(utf8_nonempty) else {
            continue;
        };
        let service = attr(&attrs, "srvr").and_then(utf8_nonempty).unwrap_or_default();
        let account = attr(&attrs, "acct").and_then(utf8_nonempty).unwrap_or_default();
        if service.is_empty() && account.is_empty() {
            continue;
        }
        let protocol = attr(&attrs, "ptcl").and_then(utf8_nonempty).unwrap_or_default();
        out.push(PasswordCredential { service, account, password, protocol });
    }
    out
}

/// Whether an `inet` item's access group holds *user* credentials worth showing:
/// Safari/WebKit AutoFill website passwords (`com.apple.cfnetwork`) and any
/// third-party app's own group. Apple's internal keychain-sync/cloud-storage
/// groups (`com.apple.security.ckks`, `com.apple.ProtectedCloudStorage`, …) hold
/// machine secrets, not passwords, and are excluded. The group is
/// entitlement-enforced, so it cannot be forged by item content.
fn is_user_credential_group(agrp: &str) -> bool {
    !agrp.is_empty() && (agrp == "com.apple.cfnetwork" || !agrp.starts_with("com.apple."))
}

/// One keychain item's NON-SECRET metadata for the inventory census. Deliberately
/// carries no password/secret value — only enough to see what an item is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeychainItemMeta {
    /// Which keychain array the item is in: `genp`/`inet`/`cert`/`keys`.
    pub array: String,
    /// Service (`svce`) or server host (`srvr`) the item belongs to; may be empty.
    pub service: String,
    /// Account / username (`acct`); may be empty.
    pub account: String,
    /// Owning access group (`agrp`); identifies the app/owner. May be empty.
    pub access_group: String,
    /// Protection class id from the item header (7 = AfterFirstUnlock /
    /// transferable; 10/11 = ThisDeviceOnly / not transferable in a portable backup).
    pub protection_class: u32,
    /// Item format version from the header (3 = AES-GCM; 1/2 = legacy AES-CBC).
    pub version: u32,
    /// Whether the item decrypted (false ≈ non-transferable ThisDeviceOnly, or an
    /// unsupported format) — it then carries only header fields, no svce/acct/agrp.
    pub decrypted: bool,
}

/// A non-secret census of the keychain: per-item metadata across the
/// genp/inet/cert/keys arrays — service, account, access group, protection class,
/// item version — but **never** a password or secret value. Useful to see scope
/// before exporting secrets and to triage how many items are non-transferable
/// (ThisDeviceOnly) and will not migrate. Total and panic-free.
pub fn inventory(plist_bytes: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Vec<KeychainItemMeta> {
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for array in [GENP_KEY, INET_KEY, CERT_KEY, KEYS_KEY] {
        for item in items_array(&root, array) {
            let Some(blob) = item.as_dictionary().and_then(|a| a.get("v_Data")).and_then(Value::as_data) else {
                continue;
            };
            let (version, protection_class) = item_header(blob).unwrap_or((0, 0));
            let attrs = decrypt_item(blob, class_keys).map(|p| parse_der_attributes(&p));
            let decrypted = attrs.is_some();
            let attrs = attrs.unwrap_or_default();
            let service = attr(&attrs, "srvr")
                .or_else(|| attr(&attrs, "svce"))
                .and_then(utf8_nonempty)
                .unwrap_or_default();
            let account = attr(&attrs, "acct").and_then(utf8_nonempty).unwrap_or_default();
            let access_group = attr(&attrs, "agrp").and_then(utf8_nonempty).unwrap_or_default();
            out.push(KeychainItemMeta {
                array: array.to_string(),
                service,
                account,
                access_group,
                protection_class,
                version,
                decrypted,
            });
        }
    }
    out
}

/// The (version, protection-class) header of a keychain item blob, without
/// decrypting. `None` when the blob is too short.
fn item_header(blob: &[u8]) -> Option<(u32, u32)> {
    let version = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?);
    let class = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
    Some((version, class))
}

/// The decrypted DER attributes of one keychain item dict, or `None` when the
/// item has no protected `v_Data` blob or it cannot be decrypted.
fn item_attributes(item: &Value, class_keys: &HashMap<u32, Vec<u8>>) -> Option<Vec<(String, Vec<u8>)>> {
    let blob = item.as_dictionary()?.get("v_Data")?.as_data()?;
    let plaintext = decrypt_item(blob, class_keys)?;
    Some(parse_der_attributes(&plaintext))
}

/// Look up one decrypted attribute's value bytes by key.
fn attr<'a>(attrs: &'a [(String, Vec<u8>)], key: &str) -> Option<&'a [u8]> {
    attrs.iter().find(|(n, _)| n == key).map(|(_, v)| v.as_slice())
}

/// The items under a top-level keychain plist array key (`genp`/`inet`/…); empty
/// when the key is absent or not an array.
fn items_array<'a>(root: &'a Value, key: &str) -> Vec<&'a Value> {
    root.as_dictionary()
        .and_then(|d| d.get(key))
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Valid, non-empty UTF-8 → owned `String`, else `None`.
fn utf8_nonempty(v: &[u8]) -> Option<String> {
    std::str::from_utf8(v).ok().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Decrypt one keychain item's protected blob to its DER attribute bytes.
/// `None` when the version is unsupported, the class key is absent, the key wrap
/// fails (e.g. a non-transferable ThisDeviceOnly class), or the blob is malformed.
fn decrypt_item(blob: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Option<Vec<u8>> {
    let version = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?);
    if version != ITEM_VERSION_GCM {
        return None;
    }
    let class = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
    let wrap_len = u32::from_le_bytes(blob.get(8..12)?.try_into().ok()?) as usize;
    let class_key = class_keys.get(&class)?;
    let wrapped = blob.get(12..12usize.checked_add(wrap_len)?)?;
    let ct = blob.get(12 + wrap_len..)?;
    let item_key = aes_kw_unwrap(class_key, wrapped)?;
    apple_gcm_decrypt(&item_key, ct)
}

/// RFC 3394 unwrap of a 32-byte AES-256 per-item key. The unwrap validates the
/// integrity check value, so success means the key is correct. `None` on any
/// failure (wrong KEK length, bad wrap, non-256-bit unwrapped key).
fn aes_kw_unwrap(class_key: &[u8], wrapped: &[u8]) -> Option<Vec<u8>> {
    if class_key.len() != 32 {
        return None;
    }
    // A 32-byte key wraps to 40 bytes; require exactly that to keep the item key
    // AES-256 (and reject absurd lengths up front).
    if wrapped.len() != 40 {
        return None;
    }
    let kek = KwAes256::new_from_slice(class_key).ok()?;
    let mut buf = vec![0u8; wrapped.len() - 8];
    kek.unwrap_key(wrapped, &mut buf).ok()?;
    Some(buf)
}

/// Decrypt Apple keychain GCM ciphertext (`edata` = ciphertext ‖ 16-byte tag)
/// with the 32-byte item key. Apple uses a blank GCM IV (J0 = all zeros), so the
/// keystream reduces to AES-256-CTR with the counter block starting at 1. The
/// AES-GCM tag **is** verified before decrypting; a tag mismatch (corruption or
/// tampering) yields `None`. Never panics.
fn apple_gcm_decrypt(key32: &[u8], edata: &[u8]) -> Option<Vec<u8>> {
    let split = edata.len().checked_sub(GCM_TAG_LEN)?;
    let ct = edata.get(..split)?;
    let tag = edata.get(split..)?;
    let cipher = Aes256::new_from_slice(key32).ok()?;
    if blank_iv_gcm_tag(&cipher, ct).as_slice() != tag {
        return None;
    }
    Some(ctr_apply(&cipher, ct))
}

/// AES-256-CTR keystream XOR with the counter block starting at 1 (32-bit
/// big-endian increment of the last four bytes). Symmetric: encrypt == decrypt.
fn ctr_apply(cipher: &Aes256, data: &[u8]) -> Vec<u8> {
    let mut counter = [0u8; 16];
    counter[15] = 1;
    let mut out = data.to_vec();
    for chunk in out.chunks_mut(16) {
        let mut block = aes::cipher::array::Array::from(counter);
        cipher.encrypt_block(&mut block);
        for (b, k) in chunk.iter_mut().zip(block.0.iter()) {
            *b ^= *k;
        }
        for i in (12..16).rev() {
            counter[i] = counter[i].wrapping_add(1);
            if counter[i] != 0 {
                break;
            }
        }
    }
    out
}

/// The AES-256-GCM tag over `ct` (no AAD) under Apple's blank IV. With J0 =
/// `0^128`, the hash subkey `H = E(0)` and the tag mask `E(J0) = E(0)` coincide,
/// so the tag is `GHASH_H(ct) XOR H`.
fn blank_iv_gcm_tag(cipher: &Aes256, ct: &[u8]) -> [u8; 16] {
    use ghash::GHash;
    use ghash::universal_hash::UniversalHash;
    let mut h = aes::cipher::array::Array::from([0u8; 16]);
    cipher.encrypt_block(&mut h);
    let mut ghash = GHash::new(&h);
    ghash.update_padded(ct);
    let mut len_block = [0u8; 16];
    len_block[8..].copy_from_slice(&(ct.len() as u64).wrapping_mul(8).to_be_bytes());
    ghash.update(&[len_block.into()]);
    let s = ghash.finalize();
    let mut tag = [0u8; 16];
    for (t, (sb, hb)) in tag.iter_mut().zip(s.iter().zip(h.0.iter())) {
        *t = sb ^ hb;
    }
    tag
}

/// Minimal, bounds-checked ASN.1 DER reader for the decrypted keychain item: a
/// `SET OF SEQUENCE { UTF8String key, value }`. Returns each `(key, value-content
/// bytes)`. Never panics on malformed input.
fn parse_der_attributes(der: &[u8]) -> Vec<(String, Vec<u8>)> {
    fn read_len(b: &[u8], i: &mut usize) -> Option<usize> {
        let first = *b.get(*i)?;
        *i += 1;
        if first < 0x80 {
            return Some(first as usize);
        }
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut len = 0usize;
        for _ in 0..n {
            len = (len << 8) | (*b.get(*i)? as usize);
            *i += 1;
        }
        Some(len)
    }
    let mut out = Vec::new();
    if der.first() != Some(&0x31) {
        return out;
    }
    let mut i = 1;
    let Some(set_len) = read_len(der, &mut i) else { return out };
    let end = i.saturating_add(set_len).min(der.len());
    while i < end {
        if der.get(i) != Some(&0x30) {
            break;
        }
        i += 1;
        let Some(seq_len) = read_len(der, &mut i) else { break };
        let seq_end = i.saturating_add(seq_len).min(der.len());
        if der.get(i) != Some(&0x0c) {
            i = seq_end;
            continue;
        }
        i += 1;
        let Some(klen) = read_len(der, &mut i) else { break };
        let Some(kbytes) = der.get(i..i.saturating_add(klen)) else { break };
        let key = String::from_utf8_lossy(kbytes).into_owned();
        i = i.saturating_add(klen);
        let value = if i < seq_end {
            i += 1; // value type tag
            match read_len(der, &mut i) {
                Some(vlen) => der.get(i..i.saturating_add(vlen)).unwrap_or(&[]).to_vec(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        out.push((key, value));
        i = seq_end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_kw::KwAes256;
    use plist::{Dictionary, Value};

    // --- DER + item builders mirroring the real format ---------------------

    fn der_len(n: usize) -> Vec<u8> {
        assert!(n < 128, "test values stay in short-form DER length");
        vec![n as u8]
    }
    fn der_utf8(s: &str) -> Vec<u8> {
        let mut t = vec![0x0c];
        t.extend(der_len(s.len()));
        t.extend(s.as_bytes());
        t
    }
    fn build_der(attrs: &[(&str, &str)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, v) in attrs {
            let mut seq_body = der_utf8(k);
            seq_body.extend(der_utf8(v));
            let mut seq = vec![0x30];
            seq.extend(der_len(seq_body.len()));
            seq.extend(seq_body);
            body.extend(seq);
        }
        let mut out = vec![0x31];
        out.extend(der_len(body.len()));
        out.extend(body);
        out
    }
    fn kw_wrap(kek: &[u8], key: &[u8]) -> Vec<u8> {
        let k = KwAes256::new_from_slice(kek).unwrap();
        let mut out = vec![0u8; key.len() + 8];
        k.wrap_key(key, &mut out).unwrap();
        out
    }
    /// Build a genp item the same way iOS does: CTR-encrypt the DER, append the
    /// real blank-IV GCM tag, prepend the version/class/wraplen + wrapped key.
    fn make_item(class_id: u32, class_key: &[u8], item_key: &[u8], attrs: &[(&str, &str)]) -> Value {
        let der = build_der(attrs);
        let cipher = Aes256::new_from_slice(item_key).unwrap();
        let ct = ctr_apply(&cipher, &der);
        let tag = blank_iv_gcm_tag(&cipher, &ct);
        let wrapped = kw_wrap(class_key, item_key);
        let mut blob = Vec::new();
        blob.extend(3u32.to_le_bytes());
        blob.extend(class_id.to_le_bytes());
        blob.extend((wrapped.len() as u32).to_le_bytes());
        blob.extend(&wrapped);
        blob.extend(&ct);
        blob.extend(tag);
        let mut d = Dictionary::new();
        d.insert("v_Data".into(), Value::Data(blob));
        Value::Dictionary(d)
    }
    /// Like [`make_item`] but flips one ciphertext byte so the GCM tag no longer
    /// verifies (the wrapped key is still valid).
    fn make_tampered_item(class_id: u32, class_key: &[u8], item_key: &[u8], attrs: &[(&str, &str)]) -> Value {
        let item = make_item(class_id, class_key, item_key, attrs);
        let mut blob = item.as_dictionary().unwrap().get("v_Data").unwrap().as_data().unwrap().to_vec();
        // Byte just past the 12-byte header + 40-byte wrapped key = first ct byte.
        blob[52] ^= 0xff;
        let mut d = Dictionary::new();
        d.insert("v_Data".into(), Value::Data(blob));
        Value::Dictionary(d)
    }
    fn keychain_plist(items: Vec<Value>) -> Vec<u8> {
        keychain_plist_keyed(GENP_KEY, items)
    }
    fn keychain_plist_keyed(key: &str, items: Vec<Value>) -> Vec<u8> {
        let mut root = Dictionary::new();
        root.insert(key.into(), Value::Array(items));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();
        buf
    }

    const CK: [u8; 32] = [0x11; 32];
    const IK: [u8; 32] = [0x22; 32];
    const CLASS: u32 = 7;

    // --- Tests -------------------------------------------------------------

    #[test]
    fn round_trip_recovers_wifi_password() {
        let item = make_item(CLASS, &CK, &IK, &[
            ("svce", "AirPort"),
            ("acct", "Internet_C9"),
            ("agrp", "apple"),
            ("v_Data", "s3cr3tpass"),
        ]);
        let plist = keychain_plist(vec![item]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());

        let got = extract_wifi(&plist, &keys);
        assert_eq!(got, vec![WifiCredential { ssid: "Internet_C9".into(), password: "s3cr3tpass".into() }]);
    }

    #[test]
    fn recovers_inet_passwords() {
        // A Safari web login (cfnetwork) and a third-party app login (own group)
        // are kept; an Apple-internal keychain-sync item, a passwordless item, and
        // one under a missing class key are all skipped.
        let web = make_item(CLASS, &CK, &IK, &[
            ("agrp", "com.apple.cfnetwork"),
            ("srvr", "accounts.google.com"),
            ("acct", "jane@gmail.com"),
            ("ptcl", "htps"),
            ("v_Data", "hunter2"),
        ]);
        let app = make_item(CLASS, &CK, &IK, &[
            ("agrp", "ABCDE12345.com.example.app"),
            ("srvr", "example.com"),
            ("acct", "user"),
            ("v_Data", "pw"),
        ]);
        let sync = make_item(CLASS, &CK, &IK, &[
            ("agrp", "com.apple.security.ckks"),
            ("srvr", "WiFi"),
            ("acct", "uuid"),
            ("v_Data", "88-byte-sync-key-material"),
        ]);
        let no_pw = make_item(CLASS, &CK, &IK, &[("agrp", "com.apple.cfnetwork"), ("srvr", "nopass.com"), ("v_Data", "")]);
        let other_class = make_item(11, &[0x99; 32], &IK, &[("agrp", "com.apple.cfnetwork"), ("srvr", "tdo.com"), ("v_Data", "secret")]);

        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_passwords(
            &keychain_plist_keyed(INET_KEY, vec![web, app, sync, no_pw, other_class]),
            &keys,
        );
        assert_eq!(
            got,
            vec![
                PasswordCredential {
                    service: "accounts.google.com".into(),
                    account: "jane@gmail.com".into(),
                    password: "hunter2".into(),
                    protocol: "htps".into(),
                },
                PasswordCredential {
                    service: "example.com".into(),
                    account: "user".into(),
                    password: "pw".into(),
                    protocol: String::new(),
                },
            ]
        );
    }

    #[test]
    fn internal_sync_group_is_excluded() {
        // The dominant inet content on a real device is Apple keychain-sync
        // machinery — it must never surface as a "password".
        for grp in ["com.apple.security.ckks", "com.apple.ProtectedCloudStorage", "com.apple.sbd"] {
            let item = make_item(CLASS, &CK, &IK, &[("agrp", grp), ("srvr", "Manatee"), ("v_Data", "secret")]);
            let mut keys = HashMap::new();
            keys.insert(CLASS, CK.to_vec());
            assert!(
                extract_passwords(&keychain_plist_keyed(INET_KEY, vec![item]), &keys).is_empty(),
                "{grp} must be excluded"
            );
        }
    }

    #[test]
    fn inet_item_without_service_or_account_skipped() {
        let item = make_item(CLASS, &CK, &IK, &[("agrp", "com.apple.cfnetwork"), ("v_Data", "orphan-secret")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert!(extract_passwords(&keychain_plist_keyed(INET_KEY, vec![item]), &keys).is_empty());
    }

    #[test]
    fn inventory_reports_metadata_without_secrets() {
        let genp = make_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "Net"), ("agrp", "apple"), ("v_Data", "pw")]);
        let inet = make_item(CLASS, &CK, &IK, &[("srvr", "example.com"), ("acct", "user"), ("agrp", "com.apple.cfnetwork"), ("v_Data", "secret")]);
        // A ThisDeviceOnly (class 11) item whose class key is NOT in the map: the
        // header is still read, but it never decrypts.
        let sealed = make_item(11, &[0x99; 32], &IK, &[("svce", "X"), ("v_Data", "tdo")]);

        let mut root = Dictionary::new();
        root.insert(GENP_KEY.into(), Value::Array(vec![genp]));
        root.insert(INET_KEY.into(), Value::Array(vec![inet, sealed]));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();

        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let inv = inventory(&buf, &keys);
        assert_eq!(inv.len(), 3);

        let g = inv.iter().find(|m| m.array == "genp").unwrap();
        assert_eq!(g.service, "AirPort");
        assert_eq!(g.account, "Net");
        assert!(g.decrypted);
        assert_eq!((g.version, g.protection_class), (3, 7));

        let i = inv.iter().find(|m| m.array == "inet" && m.decrypted).unwrap();
        assert_eq!(i.service, "example.com");
        assert_eq!(i.access_group, "com.apple.cfnetwork");

        let s = inv.iter().find(|m| !m.decrypted).unwrap();
        assert_eq!((s.version, s.protection_class), (3, 11));
        assert_eq!(s.service, "");
        assert_eq!(s.account, "");

        // The metadata type carries no secret field — serialized JSON has no
        // password/value/secret key.
        let json = serde_json::to_string(&inv).unwrap();
        assert!(!json.contains("password") && !json.contains("v_Data") && !json.contains("secret"));
    }

    #[test]
    fn non_airport_item_ignored() {
        // A generic password whose decrypted service is not AirPort is skipped.
        let item = make_item(CLASS, &CK, &IK, &[
            ("svce", "RPIdentity-SameAccountDevice"),
            ("acct", "someid"),
            ("agrp", "com.apple.rapport"),
            ("v_Data", "not-a-wifi-secret"),
        ]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn airport_with_non_apple_agrp_ignored() {
        let item = make_item(CLASS, &CK, &IK, &[
            ("svce", "AirPort"),
            ("acct", "Net"),
            ("agrp", "com.thirdparty"),
            ("v_Data", "pw"),
        ]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn missing_class_key_skips_item() {
        let item = make_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "Net"), ("agrp", "apple"), ("v_Data", "pw")]);
        // class key for CLASS not provided.
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn tampered_ciphertext_fails_tag_and_skips() {
        let item = make_tampered_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "Net"), ("agrp", "apple"), ("v_Data", "pw")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        // The wrapped key still unwraps, but the flipped ciphertext fails the GCM
        // tag, so the item is skipped rather than parsed into garbage.
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn wrong_class_key_fails_unwrap_and_skips() {
        let item = make_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "Net"), ("agrp", "apple"), ("v_Data", "pw")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, [0x99; 32].to_vec()); // wrong KEK → unwrap integrity fails
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn multiple_networks_recovered() {
        let items = vec![
            make_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "NetA"), ("agrp", "apple"), ("v_Data", "passA")]),
            make_item(CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "NetB"), ("agrp", "apple"), ("v_Data", "passB")]),
        ];
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_wifi(&keychain_plist(items), &keys);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].ssid, "NetA");
        assert_eq!(got[1].password, "passB");
    }

    #[test]
    fn malformed_inputs_never_panic() {
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        assert!(extract_wifi(b"not a plist", &keys).is_empty());
        assert!(extract_wifi(b"", &keys).is_empty());
        // genp item with a too-short / garbage v_Data blob.
        let mut d = Dictionary::new();
        d.insert("v_Data".into(), Value::Data(vec![3, 0, 0, 0, 7]));
        assert!(extract_wifi(&keychain_plist(vec![Value::Dictionary(d)]), &keys).is_empty());
        // Fuzz: deterministic pseudo-random bytes as both plist and blob.
        let mut x = 0x1234_5678u32;
        let mut junk = Vec::new();
        for _ in 0..4096 {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            junk.push((x >> 24) as u8);
        }
        let _ = extract_wifi(&junk, &keys); // must not panic
    }

    #[test]
    fn der_parser_reads_set_of_sequences() {
        let der = build_der(&[("svce", "AirPort"), ("acct", "MyNet")]);
        let attrs = parse_der_attributes(&der);
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0], ("svce".to_string(), b"AirPort".to_vec()));
        assert_eq!(attrs[1], ("acct".to_string(), b"MyNet".to_vec()));
        // Garbage DER yields nothing, no panic.
        assert!(parse_der_attributes(&[0x31, 0x84, 0xff, 0xff, 0xff, 0xff]).is_empty());
    }
}
