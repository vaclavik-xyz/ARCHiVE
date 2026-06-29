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
use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
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

/// Item format version 3 uses AES-GCM (the modern format on every backup tested);
/// versions 1 and 2 use legacy AES-CBC (very old iOS).
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

/// One recovered **network credential** — a VPN or enterprise-Wi-Fi (802.1X/EAP)
/// password/secret from the keychain.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NetworkCredential {
    /// What kind of credential the markers indicate: `vpn` or `eap`.
    pub kind: String,
    /// Service identifier (`svce`) the credential belongs to; may be empty.
    pub service: String,
    /// Account / username (`acct`); may be empty.
    pub account: String,
    /// The recovered secret (VPN password/shared-secret or EAP password) in
    /// plaintext. Sensitive — callers must not log it.
    pub password: String,
}

/// Classify a generic-password item as a VPN or enterprise-Wi-Fi (EAP) credential
/// from its service/access-group markers, or `None` when it is neither. Personal
/// Wi-Fi PSKs (`svce == "AirPort"`) are excluded — those are recovered by
/// [`extract_wifi`].
///
/// To avoid mislabelling unrelated secrets, **distinctive** markers
/// (`ipsec`, `l2tp`, `ikev2`, `eapol`, …) match as substrings, but the short
/// ambiguous marker `eap` matches only as a whole **token** (so `cheap`/`leap` do
/// not trip it). The over-broad word `enterprise` is intentionally not a marker.
/// Best-effort: an unusual marker may still be missed.
fn classify_network_marker(svce: &str, agrp: &str) -> Option<&'static str> {
    if svce.eq_ignore_ascii_case("AirPort") {
        return None;
    }
    let hay = format!("{} {}", svce.to_lowercase(), agrp.to_lowercase());
    // Tokens are maximal alphanumeric runs (split on '.', '-', '_', space, '/', …).
    let has_token = |needle: &str| {
        hay.split(|c: char| !c.is_ascii_alphanumeric()).any(|t| t == needle)
    };
    // Multi-character, distinctive markers are safe to match as substrings — they
    // will not appear inside unrelated app/service names.
    const VPN_SUBSTR: &[&str] = &["ipsec", "l2tp", "pptp", "ikev2", "racoon", "openvpn", "wireguard", "anyconnect"];
    const EAP_SUBSTR: &[&str] = &["eapol", "8021x", "802.1x"];
    // The short markers `vpn` and `eap` match only as whole tokens, so an
    // unrelated `com.vendor.vpnclient` / `notavpn-token` / `cheap` secret is not
    // mislabelled and its plaintext over-exported.
    if VPN_SUBSTR.iter().any(|m| hay.contains(m)) || has_token("vpn") {
        Some("vpn")
    } else if EAP_SUBSTR.iter().any(|m| hay.contains(m)) || has_token("eap") {
        Some("eap")
    } else {
        None
    }
}

/// Recover **VPN / enterprise-Wi-Fi (802.1X/EAP) credentials** from the keychain
/// `genp` array. Each item decrypts via the same pipeline as the other
/// extractors; items whose service/access-group markers indicate a VPN or EAP
/// credential (and that carry a non-empty secret) are returned. Personal Wi-Fi
/// PSKs and ordinary app/website logins are out of scope (see `extract_wifi` /
/// `extract_passwords`). Total and panic-free.
///
/// **Best-effort:** the marker set is documented but could not be validated
/// against a device that actually has such credentials, so coverage is not
/// guaranteed. Sensitive: returned `password` fields are plaintext.
pub fn extract_network_credentials(plist_bytes: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Vec<NetworkCredential> {
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items_array(&root, GENP_KEY) {
        let Some(attrs) = item_attributes(item, class_keys) else {
            continue;
        };
        let service = attr(&attrs, "svce").and_then(utf8_nonempty).unwrap_or_default();
        let agrp = attr(&attrs, "agrp").and_then(utf8_nonempty).unwrap_or_default();
        let Some(kind) = classify_network_marker(&service, &agrp) else {
            continue;
        };
        let Some(password) = attr(&attrs, "v_Data").and_then(utf8_nonempty) else {
            continue;
        };
        let account = attr(&attrs, "acct").and_then(utf8_nonempty).unwrap_or_default();
        out.push(NetworkCredential { kind: kind.to_string(), service, account, password });
    }
    out
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
            // A decrypt that yields no parseable attributes (e.g. a wrong-format
            // legacy-CBC item) is not counted as decrypted — consistent with
            // `item_attributes` and keeping the census honest.
            let attrs = decrypt_item(blob, class_keys)
                .map(|p| parse_der_attributes(&p))
                .filter(|a| !a.is_empty());
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

/// One recovered X.509 **certificate** from the keychain `cert` array. Carries the
/// raw certificate DER (a public artifact — no private key material) plus whether
/// a matching private key is present on the device (i.e. this cert is an
/// *identity*). The display-level X.509 metadata (subject/issuer/validity) is
/// parsed by the consumer from `der`; this layer only decrypts and pairs.
#[derive(Debug, Clone, PartialEq)]
pub struct CertificateItem {
    /// The keychain item label (`labl`); may be empty.
    pub label: String,
    /// The full X.509 certificate, DER-encoded.
    pub der: Vec<u8>,
    /// True when a private key whose public-key hash matches this certificate's
    /// `pkhh` exists in the keychain `keys` array — the cert is then a usable
    /// identity (the private key itself is **not** exported here).
    pub has_private_key: bool,
}

/// Extract X.509 certificates from the keychain `cert` array. Each cert item
/// decrypts (same pipeline as the other extractors) to DER attributes; the
/// certificate bytes live in the decrypted `v_Data` attribute. A cert is paired
/// with a private key when its public-key hash (`pkhh`) matches the label
/// (`klbl`) of a private-key item in the `keys` array. Total and panic-free: an
/// item that does not decrypt or carries no certificate bytes is skipped. No
/// private key material is ever returned.
pub fn extract_certificates(plist_bytes: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Vec<CertificateItem> {
    let Ok(root) = Value::from_reader(std::io::Cursor::new(plist_bytes)) else {
        return Vec::new();
    };

    // Public-key hashes of every private key in the keychain (`kcls` == 1), used to
    // flag which recovered certificates are usable identities.
    let mut private_key_hashes: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
    for item in items_array(&root, KEYS_KEY) {
        let Some(attrs) = item_attributes(item, class_keys) else { continue };
        let is_private = attr(&attrs, "kcls").map(attr_u32) == Some(1);
        if is_private && let Some(klbl) = attr(&attrs, "klbl") {
            private_key_hashes.insert(klbl.to_vec());
        }
    }

    let mut out = Vec::new();
    for item in items_array(&root, CERT_KEY) {
        // Certificates are commonly stored under a *ThisDeviceOnly* protection
        // class whose key is not transferable in a portable backup; those items
        // fail to unwrap here and are skipped (only transferable certs decrypt).
        let Some(attrs) = item_attributes(item, class_keys) else { continue };
        let Some(der) = attr(&attrs, "v_Data").filter(|d| !d.is_empty()).map(<[u8]>::to_vec) else {
            continue;
        };
        let label = attr(&attrs, "labl").and_then(utf8_nonempty).unwrap_or_default();
        let has_private_key = attr(&attrs, "pkhh").is_some_and(|h| private_key_hashes.contains(h));
        out.push(CertificateItem { label, der, has_private_key });
    }
    out
}

/// Read a keychain numeric attribute (stored little-endian in the DER value
/// bytes) as a u32; short/empty slices read as 0.
fn attr_u32(b: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    let n = b.len().min(4);
    buf[..n].copy_from_slice(&b[..n]);
    u32::from_le_bytes(buf)
}

/// The (version, protection-class) header of a keychain item blob, without
/// decrypting. `None` when the blob is too short.
fn item_header(blob: &[u8]) -> Option<(u32, u32)> {
    let version = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?);
    let class = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
    Some((version, class))
}

/// The decrypted DER attributes of one keychain item dict, or `None` when the
/// item has no protected `v_Data` blob, it cannot be decrypted, or the decrypted
/// bytes do not parse into **any** attributes. The last case is what makes the
/// legacy-CBC path safe: a wrong key/format yields plaintext that is not a valid
/// `SET OF SEQUENCE`, so it produces zero attributes and the item is treated as
/// *undecrypted* — never a successful decrypt with empty/garbage fields.
fn item_attributes(item: &Value, class_keys: &HashMap<u32, Vec<u8>>) -> Option<Vec<(String, Vec<u8>)>> {
    let blob = item.as_dictionary()?.get("v_Data")?.as_data()?;
    let plaintext = decrypt_item(blob, class_keys)?;
    let attrs = parse_der_attributes(&plaintext);
    (!attrs.is_empty()).then_some(attrs)
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
///
/// Version 3 items (every backup tested) use AES-GCM. Versions 1/2 (very old iOS)
/// use AES-CBC — handled **best-effort/experimental**: the header layout and the
/// CBC convention (zero IV, PKCS#7 padding) are reconstructed from limited
/// documentation and could not be validated against a real v1/2 backup, so a
/// format mismatch safely yields garbage that the strict DER parser then rejects
/// (the item is skipped, never mis-decoded into fabricated attributes). The v3
/// path is unchanged.
fn decrypt_item(blob: &[u8], class_keys: &HashMap<u32, Vec<u8>>) -> Option<Vec<u8>> {
    let version = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?);
    let class = u32::from_le_bytes(blob.get(4..8)?.try_into().ok()?);
    let wrap_len = u32::from_le_bytes(blob.get(8..12)?.try_into().ok()?) as usize;
    let class_key = class_keys.get(&class)?;
    let wrapped = blob.get(12..12usize.checked_add(wrap_len)?)?;
    let ct = blob.get(12 + wrap_len..)?;
    let item_key = aes_kw_unwrap(class_key, wrapped)?;
    match version {
        ITEM_VERSION_GCM => apple_gcm_decrypt(&item_key, ct),
        1 | 2 => apple_cbc_decrypt(&item_key, ct),
        _ => None,
    }
}

/// Decrypt legacy (version 1/2) AES-256-CBC keychain ciphertext with a zero IV and
/// PKCS#7 padding — the best-effort convention for old keychain items (see
/// [`decrypt_item`]). `None` when the ciphertext is not a positive multiple of the
/// block size or the padding is invalid (a wrong key/format then yields no item
/// rather than garbage). Never panics.
fn apple_cbc_decrypt(key32: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    if ct.is_empty() || !ct.len().is_multiple_of(16) {
        return None;
    }
    let cipher = Aes256::new_from_slice(key32).ok()?;
    let mut prev = [0u8; 16]; // zero IV
    let mut out = Vec::with_capacity(ct.len());
    for chunk in ct.chunks_exact(16) {
        let mut block = aes::cipher::array::Array::from(<[u8; 16]>::try_from(chunk).ok()?);
        cipher.decrypt_block(&mut block);
        for (b, p) in block.0.iter().zip(prev.iter()) {
            out.push(b ^ p);
        }
        prev.copy_from_slice(chunk);
    }
    // PKCS#7 unpad: last byte = pad length in 1..=16, every padding byte equal.
    let pad = *out.last()? as usize;
    if pad == 0 || pad > 16 || pad > out.len() {
        return None;
    }
    if out[out.len() - pad..].iter().any(|&b| b as usize != pad) {
        return None;
    }
    out.truncate(out.len() - pad);
    Some(out)
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

    // --- Certificate extraction --------------------------------------------

    /// DER value carrying raw bytes as an OCTET STRING (tag 0x04); `parse_der_
    /// attributes` reads the content generically regardless of tag.
    fn der_octets(b: &[u8]) -> Vec<u8> {
        let mut t = vec![0x04];
        t.extend(der_len(b.len()));
        t.extend(b);
        t
    }
    /// Build the attribute SET from mixed string/byte values.
    fn build_der_bytes(attrs: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (k, v) in attrs {
            let mut seq_body = der_utf8(k);
            seq_body.extend(der_octets(v));
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
    /// Encrypt a binary-valued attribute set into a keychain item the way iOS does.
    fn make_item_bytes(class_id: u32, class_key: &[u8], item_key: &[u8], attrs: &[(&str, Vec<u8>)]) -> Value {
        let der = build_der_bytes(attrs);
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

    #[test]
    fn extracts_certificate_and_flags_identity() {
        let cert_der = vec![0x30, 0x82, 0x01, 0x02, 0xAB, 0xCD]; // stand-in X.509 DER
        let pubkey_hash = vec![0xDE; 20];
        let cert = make_item_bytes(CLASS, &CK, &IK, &[
            ("labl", b"My Client Cert".to_vec()),
            ("pkhh", pubkey_hash.clone()),
            ("v_Data", cert_der.clone()),
        ]);
        // A matching private key (kcls == 1) whose klbl equals the cert's pkhh.
        let priv_key = make_item_bytes(CLASS, &CK, &IK, &[
            ("kcls", vec![1]),
            ("klbl", pubkey_hash.clone()),
            ("v_Data", vec![0x11, 0x22]),
        ]);

        let mut root = Dictionary::new();
        root.insert(CERT_KEY.into(), Value::Array(vec![cert]));
        root.insert(KEYS_KEY.into(), Value::Array(vec![priv_key]));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();

        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_certificates(&buf, &keys);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].label, "My Client Cert");
        assert_eq!(got[0].der, cert_der);
        assert!(got[0].has_private_key, "cert with a matching private key is an identity");
    }

    #[test]
    fn certificate_without_matching_key_is_not_identity() {
        let cert = make_item_bytes(CLASS, &CK, &IK, &[
            ("labl", b"CA Root".to_vec()),
            ("pkhh", vec![0x01; 20]),
            ("v_Data", vec![0x30, 0x03, 0x02, 0x01, 0x05]),
        ]);
        // A public key (kcls == 0) is NOT a private key, so it must not flag identity.
        let pub_key = make_item_bytes(CLASS, &CK, &IK, &[("kcls", vec![0]), ("klbl", vec![0x01; 20]), ("v_Data", vec![0x09])]);
        let mut root = Dictionary::new();
        root.insert(CERT_KEY.into(), Value::Array(vec![cert]));
        root.insert(KEYS_KEY.into(), Value::Array(vec![pub_key]));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();

        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_certificates(&buf, &keys);
        assert_eq!(got.len(), 1);
        assert!(!got[0].has_private_key);
    }

    #[test]
    fn certificate_under_nontransferable_class_is_skipped() {
        // A cert under a class whose key is absent (ThisDeviceOnly in a real
        // backup) cannot be unwrapped — it is skipped, never errors.
        let cert = make_item_bytes(11, &[0x99; 32], &IK, &[("labl", b"Device Cert".to_vec()), ("v_Data", vec![0x30, 0x01, 0x00])]);
        let mut root = Dictionary::new();
        root.insert(CERT_KEY.into(), Value::Array(vec![cert]));
        let mut buf = Vec::new();
        plist::to_writer_binary(&mut buf, &Value::Dictionary(root)).unwrap();
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec()); // class 11 key not provided
        assert!(extract_certificates(&buf, &keys).is_empty());
    }

    #[test]
    fn certificates_malformed_inputs_never_panic() {
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        assert!(extract_certificates(b"not a plist", &keys).is_empty());
        assert!(extract_certificates(b"", &keys).is_empty());
    }

    // --- Network (VPN / EAP) credentials ------------------------------------

    #[test]
    fn classify_network_marker_buckets_vpn_eap_and_excludes_wifi() {
        assert_eq!(classify_network_marker("com.apple.vpn.managed", "apple"), Some("vpn"));
        assert_eq!(classify_network_marker("MyCompany IPsec", ""), Some("vpn"));
        assert_eq!(classify_network_marker("eapol:en0", "apple"), Some("eap"));
        assert_eq!(classify_network_marker("Corp 802.1X", ""), Some("eap"));
        assert_eq!(classify_network_marker("eap", "apple"), Some("eap")); // bare token
        assert_eq!(classify_network_marker("auth.eap.network", ""), Some("eap")); // token between dots
        // Personal Wi-Fi PSKs are out of scope (handled by extract_wifi).
        assert_eq!(classify_network_marker("AirPort", "apple"), None);
        // An ordinary app secret is neither.
        assert_eq!(classify_network_marker("com.example.token", "com.example"), None);
    }

    #[test]
    fn classify_network_marker_no_substring_false_positives() {
        // "eap" must NOT trip on words that merely contain it.
        assert_eq!(classify_network_marker("com.cheapskate.deals", "com.cheapskate"), None);
        assert_eq!(classify_network_marker("LeapYear Planner", "com.leapyear.app"), None);
        // "enterprise" is intentionally not a marker (too broad).
        assert_eq!(classify_network_marker("Acme Enterprise Suite", "com.acme.enterprise"), None);
        // A third-party token store is left alone.
        assert_eq!(classify_network_marker("session-token", "com.vendor.app"), None);
        // "vpn" only matches as a whole token, so these unrelated secrets are NOT
        // exported as VPN credentials.
        assert_eq!(classify_network_marker("notavpn-token", "com.vendor.app"), None);
        assert_eq!(classify_network_marker("auth", "com.vendor.vpnclient"), None);
        // A genuine bare `vpn` token is still detected.
        assert_eq!(classify_network_marker("corp.vpn.gateway", ""), Some("vpn"));
    }

    #[test]
    fn extracts_vpn_and_eap_credentials_only() {
        let vpn = make_item(CLASS, &CK, &IK, &[
            ("svce", "com.apple.vpn.managed"), ("acct", "road-warrior"), ("agrp", "apple"), ("v_Data", "vpnSecret!"),
        ]);
        let eap = make_item(CLASS, &CK, &IK, &[
            ("svce", "CorpWiFi-EAP"), ("acct", "jane.doe"), ("agrp", "apple"), ("v_Data", "eapPass"),
        ]);
        let airport = make_item(CLASS, &CK, &IK, &[
            ("svce", "AirPort"), ("acct", "HomeNet"), ("agrp", "apple"), ("v_Data", "psk12345"),
        ]);
        let app = make_item(CLASS, &CK, &IK, &[
            ("svce", "com.example.token"), ("acct", "u"), ("agrp", "com.example"), ("v_Data", "tok"),
        ]);

        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_network_credentials(&keychain_plist(vec![vpn, eap, airport, app]), &keys);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], NetworkCredential { kind: "vpn".into(), service: "com.apple.vpn.managed".into(), account: "road-warrior".into(), password: "vpnSecret!".into() });
        assert_eq!(got[1].kind, "eap");
        assert_eq!(got[1].account, "jane.doe");
        // AirPort PSK and the ordinary app secret are excluded.
        assert!(got.iter().all(|c| c.service != "AirPort" && c.service != "com.example.token"));
    }

    #[test]
    fn network_credential_without_secret_is_skipped() {
        let item = make_item(CLASS, &CK, &IK, &[("svce", "com.apple.vpn"), ("agrp", "apple"), ("v_Data", "")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert!(extract_network_credentials(&keychain_plist(vec![item]), &keys).is_empty());
    }

    #[test]
    fn network_credentials_malformed_never_panic() {
        let keys: HashMap<u32, Vec<u8>> = HashMap::new();
        assert!(extract_network_credentials(b"not a plist", &keys).is_empty());
        assert!(extract_network_credentials(b"", &keys).is_empty());
    }

    // --- Legacy (version 1/2) AES-CBC items ---------------------------------

    /// AES-256-CBC encrypt with a zero IV and PKCS#7 padding — the inverse of
    /// `apple_cbc_decrypt`, used to build synthetic legacy items.
    fn cbc_encrypt(key32: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let cipher = Aes256::new_from_slice(key32).unwrap();
        let pad = 16 - (plaintext.len() % 16);
        let mut padded = plaintext.to_vec();
        padded.extend(std::iter::repeat_n(pad as u8, pad));
        let mut prev = [0u8; 16];
        let mut out = Vec::with_capacity(padded.len());
        for chunk in padded.chunks_exact(16) {
            let mut blk = [0u8; 16];
            for i in 0..16 {
                blk[i] = chunk[i] ^ prev[i];
            }
            let mut b = aes::cipher::array::Array::from(blk);
            cipher.encrypt_block(&mut b);
            out.extend_from_slice(&b.0);
            prev.copy_from_slice(&b.0);
        }
        out
    }

    /// Build a legacy `version` (1 or 2) CBC item with the v3-style header layout.
    fn make_cbc_item(version: u32, class_id: u32, class_key: &[u8], item_key: &[u8], attrs: &[(&str, &str)]) -> Value {
        let der = build_der(attrs);
        let ct = cbc_encrypt(item_key, &der);
        let wrapped = kw_wrap(class_key, item_key);
        let mut blob = Vec::new();
        blob.extend(version.to_le_bytes());
        blob.extend(class_id.to_le_bytes());
        blob.extend((wrapped.len() as u32).to_le_bytes());
        blob.extend(&wrapped);
        blob.extend(&ct);
        let mut d = Dictionary::new();
        d.insert("v_Data".into(), Value::Data(blob));
        Value::Dictionary(d)
    }

    #[test]
    fn legacy_cbc_item_round_trips_through_wifi() {
        // A version-2 AirPort item decrypts via the CBC path and surfaces like any
        // other Wi-Fi credential — exercising decrypt_item end-to-end.
        let item = make_cbc_item(2, CLASS, &CK, &IK, &[
            ("svce", "AirPort"), ("acct", "OldNet"), ("agrp", "apple"), ("v_Data", "legacyPass"),
        ]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        let got = extract_wifi(&keychain_plist(vec![item]), &keys);
        assert_eq!(got, vec![WifiCredential { ssid: "OldNet".into(), password: "legacyPass".into() }]);
    }

    #[test]
    fn legacy_version_1_also_decrypts() {
        let item = make_cbc_item(1, CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "V1Net"), ("agrp", "apple"), ("v_Data", "p1")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert_eq!(extract_wifi(&keychain_plist(vec![item]), &keys).len(), 1);
    }

    #[test]
    fn legacy_cbc_wrong_key_yields_no_item_not_garbage() {
        // A wrong class key unwraps to the wrong item key; the CBC plaintext is then
        // garbage whose PKCS#7 padding/DER almost never validates, so the item is
        // safely skipped rather than mis-decoded into fabricated attributes.
        let item = make_cbc_item(2, CLASS, &CK, &IK, &[("svce", "AirPort"), ("acct", "Net"), ("agrp", "apple"), ("v_Data", "pw")]);
        let mut keys = HashMap::new();
        keys.insert(CLASS, [0x99; 32].to_vec()); // wrong KEK → wrong item key
        assert!(extract_wifi(&keychain_plist(vec![item]), &keys).is_empty());
    }

    /// Build a legacy CBC item whose plaintext is arbitrary (not necessarily DER).
    fn make_cbc_item_raw(version: u32, class_id: u32, class_key: &[u8], item_key: &[u8], plaintext: &[u8]) -> Value {
        let ct = cbc_encrypt(item_key, plaintext);
        let wrapped = kw_wrap(class_key, item_key);
        let mut blob = Vec::new();
        blob.extend(version.to_le_bytes());
        blob.extend(class_id.to_le_bytes());
        blob.extend((wrapped.len() as u32).to_le_bytes());
        blob.extend(&wrapped);
        blob.extend(&ct);
        let mut d = Dictionary::new();
        d.insert("v_Data".into(), Value::Data(blob));
        Value::Dictionary(d)
    }

    #[test]
    fn cbc_decrypt_to_non_der_is_not_a_successful_item() {
        // CBC succeeds (valid padding) but the plaintext is not a DER SET, so it
        // parses to zero attributes: the item is neither extracted nor counted as
        // decrypted in the inventory census.
        let item = make_cbc_item_raw(2, CLASS, &CK, &IK, b"this is plainly not a DER attribute set");
        let mut keys = HashMap::new();
        keys.insert(CLASS, CK.to_vec());
        assert!(extract_wifi(&keychain_plist(vec![item.clone()]), &keys).is_empty());

        // In the inventory the same item is present but `decrypted: false`.
        let inv = inventory(&keychain_plist(vec![item]), &keys);
        assert_eq!(inv.len(), 1);
        assert!(!inv[0].decrypted, "a no-attribute decrypt must not count as decrypted");
    }

    #[test]
    fn apple_cbc_decrypt_rejects_bad_length() {
        // Non-block-multiple and empty ciphertext are rejected outright.
        assert!(apple_cbc_decrypt(&IK, &[0u8; 17]).is_none());
        assert!(apple_cbc_decrypt(&IK, &[0u8; 1]).is_none());
        assert!(apple_cbc_decrypt(&IK, &[]).is_none());
        // A valid round-trip is recovered (sanity for the helper itself).
        let ct = cbc_encrypt(&IK, b"hello world");
        assert_eq!(apple_cbc_decrypt(&IK, &ct).unwrap(), b"hello world");
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
