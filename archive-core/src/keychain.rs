//! Recover saved Wi-Fi passwords from a decrypted iOS backup keychain.
//!
//! # Where this fits
//!
//! iOS keychain recovery from a backup is a *two-layer* decryption problem:
//!
//! 1. **File layer** (handled elsewhere, by the controller via `crabapple`):
//!    locate `KeychainDomain/keychain-backup.plist` in the backup manifest and
//!    file-decrypt it. That yields the *keychain plist bytes* this module
//!    consumes. The controller also obtains the per-protection-class keys from
//!    crabapple's manifest keybag.
//!
//! 2. **Item layer** (this module): inside the keychain plist, each secret item
//!    (a saved Wi-Fi password, a generic password, …) carries its own AES-wrapped
//!    per-item key plus an AEAD-encrypted secret. Unwrapping that per-item key
//!    with the appropriate *class key* and then decrypting the payload reveals the
//!    secret.
//!
//! This module performs **only the item layer**, and **only for Wi-Fi**
//! (`genp`/AirPort) items in v1. Its interface is deliberately decoupled from
//! crabapple: it takes plain keychain plist bytes and a `HashMap<u32, Vec<u8>>`
//! mapping protection-class id → raw (already-unwrapped) class key. The controller
//! is responsible for turning crabapple's `ProtectionClassKey { class_id, key }`
//! values into that map (`key.as_ref().to_vec()` keyed by `class_id`).
//!
//! # Protected blob layout
//!
//! A protected item value (`v_Data`) is laid out as:
//!
//! ```text
//! ┌────────────┬────────────────────────┬──────────────────────────────────┐
//! │ class id   │ RFC 3394 wrapped key    │ AES-256-GCM blob                 │
//! │ 4 bytes LE │ (per-item key, wrapped) │ nonce(12) ‖ ciphertext ‖ tag(16) │
//! └────────────┴────────────────────────┴──────────────────────────────────┘
//! ```
//!
//! The first 4 bytes are a little-endian `u32` protection-class id (same framing
//! crabapple uses for file keys). The wrapped per-item key follows; for a 32-byte
//! AES-256 key the RFC 3394 wrap is 40 bytes (`0x28`). Everything after is the
//! AES-256-GCM payload: a 12-byte nonce, the ciphertext, then a 16-byte tag.
//!
//! Robustness rules (this module *never* errors on a single bad item — it skips):
//!
//! * If the class key for an item's class id is absent, the item is skipped.
//! * If the blob is malformed (too short, bad wrap, failed GCM), it is skipped.
//! * Some keychain-backup dumps surface items already partially decrypted: the
//!   `v_Data` is plaintext (a UTF-8 PSK) rather than a protected blob. Such values
//!   are used directly.
//! * The recovered secret may itself be a UTF-8 string or a nested binary plist
//!   wrapping the password; both are handled.

use std::collections::HashMap;

use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use aes_kw::KwAes256;
use plist::Value;
use serde::Serialize;

/// The keychain plist's generic-password array — where Wi-Fi PSKs live. v1 mines
/// only this; the other top-level arrays (`inet`, `cert`, `keys`) are out of
/// scope.
const GENP_KEY: &str = "genp";

/// Number of leading bytes of a protected blob holding the LE `u32` class id.
const CLASS_ID_LEN: usize = 4;

/// AES-GCM nonce length (96-bit), prepended to the ciphertext in the payload.
const GCM_NONCE_LEN: usize = 12;

/// AES-GCM authentication tag length (128-bit), appended after the ciphertext.
const GCM_TAG_LEN: usize = 16;

/// RFC 3394 adds one 8-byte semiblock of overhead over the wrapped key, so a
/// wrapped key is always at least this many bytes. Used only as a lower bound;
/// the exact wrapped length is discovered by trial unwrap (see [`split_blob`]).
const KW_MIN_LEN: usize = 16;

/// One recovered Wi-Fi credential: the network name and its plaintext password.
///
/// `Serialize` is derived so the controller can emit it directly in the
/// agent-first one-JSON-object-on-stdout contract without an intermediate DTO.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WifiCredential {
    /// The Wi-Fi network name (SSID), read from the item's `acct` attribute.
    pub ssid: String,
    /// The recovered pre-shared key / password in plaintext.
    pub password: String,
}

/// Extract saved Wi-Fi credentials from decrypted keychain plist bytes.
///
/// `plist_bytes` is the *decrypted* `keychain-backup.plist` (a binary or XML
/// plist). `class_keys` maps protection-class id → raw class key (already
/// unwrapped by the caller from crabapple's manifest keybag).
///
/// This function is total and panic-free on untrusted input: a malformed plist
/// yields an empty `Vec`, and any individual item that cannot be parsed,
/// key-unwrapped, or decrypted is silently skipped. Only `genp` (generic
/// password) items that look like AirPort/Wi-Fi entries are considered.
pub fn extract_wifi(
    plist_bytes: &[u8],
    class_keys: &HashMap<u32, Vec<u8>>,
) -> Vec<WifiCredential> {
    // A malformed/unsupported plist is not an error here — it simply yields no
    // credentials. `from_reader` needs `Read + Seek`; a `Cursor` over the bytes
    // satisfies both without copying.
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };
    let Some(dict) = root.as_dictionary() else {
        return Vec::new();
    };
    let Some(items) = dict.get(GENP_KEY).and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for item in items {
        let Some(attrs) = item.as_dictionary() else {
            continue;
        };
        if !is_wifi_item(attrs) {
            continue;
        }
        // SSID comes from the `acct` attribute; without it we cannot label the
        // credential, so skip.
        let Some(ssid) = attrs.get("acct").and_then(Value::as_string) else {
            continue;
        };
        if let Some(password) = recover_secret(attrs, class_keys) {
            out.push(WifiCredential {
                ssid: ssid.to_string(),
                password,
            });
        }
    }
    out
}

/// Whether a `genp` attribute dict denotes an AirPort/Wi-Fi entry.
///
/// Apple stores saved Wi-Fi PSKs as generic passwords whose service (`svce`) or
/// access group (`agrp`) names AirPort. We match only those two **authoritative**
/// attributes (case-insensitive substring), so we catch `AirPort` /
/// `com.apple.network.eap`-style services without hardcoding one exact string —
/// but we deliberately ignore the user-facing `labl`/`desc` free text, which
/// could carry a Wi-Fi-looking label on an unrelated secret and leak it.
fn is_wifi_item(attrs: &plist::Dictionary) -> bool {
    const NEEDLES: [&str; 2] = ["airport", "wifi"];
    for key in ["svce", "agrp"] {
        if let Some(val) = attrs.get(key).and_then(Value::as_string) {
            let lower = val.to_ascii_lowercase();
            if NEEDLES.iter().any(|n| lower.contains(n)) {
                return true;
            }
        }
    }
    false
}

/// Recover the plaintext secret for one keychain item.
///
/// Handles both the already-decrypted case (`v_Data` is a plaintext UTF-8 PSK)
/// and the protected-blob case (decrypt via the class keys). Returns `None` if
/// there is no usable `v_Data` or decryption fails — the caller skips the item.
fn recover_secret(
    attrs: &plist::Dictionary,
    class_keys: &HashMap<u32, Vec<u8>>,
) -> Option<String> {
    let v_data = attrs.get("v_Data")?;

    // Case A: the value is stored as a plist string — already plaintext.
    if let Some(s) = v_data.as_string() {
        return Some(s.to_string());
    }

    // Otherwise it must be a Data blob.
    let blob = v_data.as_data()?;

    // Case B: a protected blob → decrypt. Tried FIRST (before the plaintext
    // fallback) so a long plaintext PSK is never mistaken for a blob and dropped:
    // a WPA passphrase is up to 63 bytes — longer than a minimal protected blob —
    // and on a real blob decryption succeeds, while on plaintext the leading bytes
    // are not a valid class id so decryption simply returns None.
    if let Some(secret) = decrypt_protected_blob(blob, class_keys) {
        return Some(secret);
    }

    // Case C fallback: the Data is a raw plaintext PSK (not a protected blob, or
    // an unsupported layout). Accept only non-empty valid UTF-8 of plausible
    // length, so binary ciphertext is never surfaced as garbage text.
    match std::str::from_utf8(blob) {
        Ok(s) if (1..=128).contains(&s.chars().count()) => Some(s.to_string()),
        _ => None,
    }
}

/// Decrypt a protected item blob into its plaintext secret.
///
/// Steps: read the LE `u32` class id, look up its class key, RFC 3394-unwrap the
/// per-item key, then AES-256-GCM decrypt the payload. Returns `None` on any
/// failure (missing key, bad wrap, GCM auth failure, non-text result), never
/// panicking. All slice access is bounds-checked via `.get(..)`.
fn decrypt_protected_blob(
    blob: &[u8],
    class_keys: &HashMap<u32, Vec<u8>>,
) -> Option<String> {
    let class_bytes: [u8; CLASS_ID_LEN] = blob.get(..CLASS_ID_LEN)?.try_into().ok()?;
    let class_id = u32::from_le_bytes(class_bytes);

    let class_key = class_keys.get(&class_id)?;
    // RFC 3394 / AES-256 key wrap requires a 256-bit KEK.
    if class_key.len() != 32 {
        return None;
    }

    let rest = blob.get(CLASS_ID_LEN..)?;
    let (item_key, gcm_blob) = split_blob(rest, class_key)?;

    // GCM payload: nonce(12) ‖ ciphertext ‖ tag(16). `aes-gcm`'s `decrypt`
    // expects ciphertext-with-appended-tag, so we hand it everything after the
    // nonce; it validates the tag internally.
    let nonce = gcm_blob.get(..GCM_NONCE_LEN)?;
    let ct_and_tag = gcm_blob.get(GCM_NONCE_LEN..)?;
    if ct_and_tag.len() < GCM_TAG_LEN {
        return None;
    }

    // `nonce` is exactly GCM_NONCE_LEN bytes (sliced above), so this conversion
    // into the fixed-size `Nonce` array cannot fail; `.ok()?` keeps us panic-free
    // regardless of input.
    let nonce = Nonce::try_from(nonce).ok()?;
    let cipher = Aes256Gcm::new_from_slice(&item_key).ok()?;
    let plaintext = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: ct_and_tag,
                aad: &[],
            },
        )
        .ok()?;

    decode_secret(&plaintext)
}

/// Split the post-class-id remainder into `(unwrapped per-item key, gcm blob)`.
///
/// The wrapped-key length is not stored inline, so we discover it by trial: try
/// each plausible RFC 3394 wrapped length (multiples of 8, from the minimum up to
/// what leaves room for a GCM payload) and accept the first that unwraps to a
/// valid AES key (16/24/32 bytes) *and* leaves a remainder large enough for a GCM
/// nonce+tag. The common iOS case is a 40-byte wrap of a 32-byte key, which this
/// finds on the first viable candidate.
fn split_blob(rest: &[u8], class_key: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let kek = KwAes256::new_from_slice(class_key).ok()?;
    let min_gcm = GCM_NONCE_LEN + GCM_TAG_LEN;

    // Wrapped key is RFC 3394: a multiple of 8 bytes, > 8. Unwrapped length is
    // (wrapped - 8) and must be a valid AES key size.
    let mut wrapped_len = KW_MIN_LEN; // 16: smallest wrap (of an 8-byte key) — but we require AES sizes below.
    while wrapped_len + min_gcm <= rest.len() {
        if let Some(wrapped) = rest.get(..wrapped_len) {
            let unwrapped_len = wrapped_len - 8;
            if matches!(unwrapped_len, 16 | 24 | 32) {
                let mut buf = vec![0u8; unwrapped_len];
                if kek.unwrap_key(wrapped, &mut buf).is_ok() {
                    // Only AES-256 item keys are used for GCM here; require 32.
                    if unwrapped_len == 32 {
                        let gcm = rest.get(wrapped_len..)?.to_vec();
                        return Some((buf, gcm));
                    }
                }
            }
        }
        wrapped_len += 8;
    }
    None
}

/// Decode a decrypted keychain secret into a password string.
///
/// The secret is either raw UTF-8 (the PSK) or a nested binary plist whose
/// dictionary holds the password under a conventional key (`v_Data` again, or a
/// `String`/data value). We try plain UTF-8 first, then fall back to parsing a
/// nested plist and extracting the first plausible string value.
fn decode_secret(plaintext: &[u8]) -> Option<String> {
    // A nested binary plist starts with the "bplist00" magic; only then do we
    // attempt plist parsing (avoids mis-parsing a PSK that happens to be valid
    // UTF-8 but contains odd bytes).
    if plaintext.starts_with(b"bplist00")
        && let Ok(nested) = Value::from_reader(std::io::Cursor::new(plaintext))
            && let Some(s) = extract_nested_password(&nested) {
                return Some(s);
            }
    // Plain UTF-8 PSK.
    std::str::from_utf8(plaintext).ok().map(str::to_string)
}

/// Pull a password string out of a nested keychain-secret plist.
///
/// Looks for common carriers in order; falls back to the first string value in
/// the dict. Returns `None` if nothing string-like is found.
fn extract_nested_password(value: &Value) -> Option<String> {
    let dict = value.as_dictionary()?;
    for key in ["v_Data", "password", "Password", "String"] {
        if let Some(v) = dict.get(key) {
            if let Some(s) = v.as_string() {
                return Some(s.to_string());
            }
            if let Some(d) = v.as_data()
                && let Ok(s) = std::str::from_utf8(d) {
                    return Some(s.to_string());
                }
        }
    }
    // Last resort: any string value in the dict.
    dict.values().find_map(|v| v.as_string().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes_gcm::aead::Aead;
    use aes_kw::KwAes256;
    use plist::{Dictionary, Value};

    /// Wrap `item_key` under `class_key` with RFC 3394 (AES-256 KW).
    fn wrap_item_key(class_key: &[u8; 32], item_key: &[u8; 32]) -> Vec<u8> {
        let kek = KwAes256::new_from_slice(class_key).unwrap();
        let mut buf = vec![0u8; item_key.len() + 8];
        kek.wrap_key(item_key, &mut buf).unwrap();
        buf
    }

    /// AES-256-GCM encrypt `msg` with `item_key` and a fixed nonce, returning
    /// `nonce ‖ ciphertext ‖ tag`.
    fn gcm_seal(item_key: &[u8; 32], nonce: &[u8; 12], msg: &[u8]) -> Vec<u8> {
        let cipher = Aes256Gcm::new_from_slice(item_key).unwrap();
        let ct = cipher
            .encrypt(
                &Nonce::try_from(&nonce[..]).unwrap(),
                Payload { msg, aad: &[] },
            )
            .unwrap();
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(nonce);
        out.extend_from_slice(&ct);
        out
    }

    /// Assemble a full protected blob: class id (LE) ‖ wrapped key ‖ gcm blob.
    fn build_protected_blob(
        class_id: u32,
        class_key: &[u8; 32],
        item_key: &[u8; 32],
        nonce: &[u8; 12],
        secret: &[u8],
    ) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(&class_id.to_le_bytes());
        blob.extend_from_slice(&wrap_item_key(class_key, item_key));
        blob.extend_from_slice(&gcm_seal(item_key, nonce, secret));
        blob
    }

    /// Build a binary plist with a `genp` array of the given attribute dicts.
    fn build_keychain_plist(items: Vec<Dictionary>) -> Vec<u8> {
        let mut root = Dictionary::new();
        let arr: Vec<Value> = items.into_iter().map(Value::Dictionary).collect();
        root.insert(GENP_KEY.to_string(), Value::Array(arr));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();
        buf
    }

    fn airport_item(acct: &str, v_data: Value) -> Dictionary {
        let mut d = Dictionary::new();
        d.insert("svce".into(), Value::String("AirPort".into()));
        d.insert("agrp".into(), Value::String("apple".into()));
        d.insert("acct".into(), Value::String(acct.into()));
        d.insert("v_Data".into(), v_data);
        d
    }

    const CLASS_ID: u32 = 6;
    const CLASS_KEY: [u8; 32] = [0x11; 32];
    const ITEM_KEY: [u8; 32] = [0x22; 32];
    const NONCE: [u8; 12] = [0x33; 12];

    #[test]
    fn round_trip_recovers_ssid_and_password() {
        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, CLASS_KEY.to_vec());

        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, b"hunter2pass");
        let item = airport_item("HomeNetwork", Value::Data(blob));
        let plist = build_keychain_plist(vec![item]);

        let got = extract_wifi(&plist, &keys);
        assert_eq!(
            got,
            vec![WifiCredential {
                ssid: "HomeNetwork".into(),
                password: "hunter2pass".into(),
            }]
        );
    }

    #[test]
    fn round_trip_nested_plist_secret() {
        // Secret is itself a binary plist carrying the password under "v_Data".
        let mut inner = Dictionary::new();
        inner.insert("v_Data".into(), Value::String("nested-psk".into()));
        let mut secret_bytes = Vec::new();
        plist::to_writer_binary(&mut secret_bytes, &Value::Dictionary(inner)).unwrap();

        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, CLASS_KEY.to_vec());
        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, &secret_bytes);
        let item = airport_item("NestedNet", Value::Data(blob));
        let plist = build_keychain_plist(vec![item]);

        let got = extract_wifi(&plist, &keys);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].ssid, "NestedNet");
        assert_eq!(got[0].password, "nested-psk");
    }

    #[test]
    fn missing_class_key_skips_item_without_panic() {
        // Class key for CLASS_ID is absent → item skipped, no panic, empty result.
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, b"secret");
        let item = airport_item("NoKeyNet", Value::Data(blob));
        let plist = build_keychain_plist(vec![item]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }

    #[test]
    fn plaintext_v_data_string_used_directly() {
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        let item = airport_item("PlainNet", Value::String("plaintext-psk".into()));
        let plist = build_keychain_plist(vec![item]);

        let got = extract_wifi(&plist, &keys);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].password, "plaintext-psk");
    }

    #[test]
    fn plaintext_v_data_short_data_used_directly() {
        // Raw bytes that are too short to be a protected blob are read as UTF-8.
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        let item = airport_item("ShortNet", Value::Data(b"rawpsk".to_vec()));
        let plist = build_keychain_plist(vec![item]);

        let got = extract_wifi(&plist, &keys);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].password, "rawpsk");
    }

    #[test]
    fn malformed_blob_skipped() {
        // Long enough to look like a protected blob, but garbage → GCM/unwrap
        // fails → item skipped, no panic.
        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, CLASS_KEY.to_vec());
        let mut junk = CLASS_ID.to_le_bytes().to_vec();
        junk.extend_from_slice(&[0xAB; 80]);
        let item = airport_item("JunkNet", Value::Data(junk));
        let plist = build_keychain_plist(vec![item]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }

    #[test]
    fn wrong_class_key_fails_gcm_and_skips() {
        // Right class id, but the provided class key is wrong → unwrap yields a
        // bogus item key → GCM auth fails → skipped.
        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, [0x99; 32].to_vec());
        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, b"secret");
        let item = airport_item("WrongKeyNet", Value::Data(blob));
        let plist = build_keychain_plist(vec![item]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }

    #[test]
    fn non_wifi_genp_item_ignored() {
        // A generic password that is not AirPort/Wi-Fi must not be returned.
        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, CLASS_KEY.to_vec());
        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, b"secret");
        let mut d = Dictionary::new();
        d.insert("svce".into(), Value::String("com.apple.account".into()));
        d.insert("acct".into(), Value::String("someuser".into()));
        d.insert("v_Data".into(), Value::Data(blob));
        let plist = build_keychain_plist(vec![d]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }

    #[test]
    fn long_plaintext_psk_recovered() {
        // A 60-char plaintext WPA passphrase stored as raw Data is LONGER than a
        // minimal protected blob; decrypt-first (which finds no class id) then the
        // plaintext fallback must still recover it (regression: the old length
        // heuristic dropped plaintext PSKs over ~47 bytes).
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        let psk = "a".repeat(60);
        let item = airport_item("LongNet", Value::Data(psk.clone().into_bytes()));
        let plist = build_keychain_plist(vec![item]);

        let got = extract_wifi(&plist, &keys);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].password, psk);
    }

    #[test]
    fn non_wifi_item_with_wifi_label_ignored() {
        // A generic password for an unrelated service must NOT be exported even
        // when its user-facing label mentions Wi-Fi (we key off svce/agrp, never
        // the free-text labl/desc).
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        let mut d = Dictionary::new();
        d.insert("svce".into(), Value::String("com.example.secret".into()));
        d.insert("agrp".into(), Value::String("com.example".into()));
        d.insert("labl".into(), Value::String("My WiFi password".into()));
        d.insert("acct".into(), Value::String("user".into()));
        d.insert("v_Data".into(), Value::String("should-not-leak".into()));
        let plist = build_keychain_plist(vec![d]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }

    #[test]
    fn empty_or_garbage_plist_yields_empty() {
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        assert!(extract_wifi(b"not a plist at all", &keys).is_empty());
        assert!(extract_wifi(&[], &keys).is_empty());
    }

    #[test]
    fn item_without_acct_skipped() {
        let mut keys = HashMap::new();
        keys.insert(CLASS_ID, CLASS_KEY.to_vec());
        let blob = build_protected_blob(CLASS_ID, &CLASS_KEY, &ITEM_KEY, &NONCE, b"secret");
        let mut d = Dictionary::new();
        d.insert("svce".into(), Value::String("AirPort".into()));
        // no "acct"
        d.insert("v_Data".into(), Value::Data(blob));
        let plist = build_keychain_plist(vec![d]);

        assert!(extract_wifi(&plist, &keys).is_empty());
    }
}
