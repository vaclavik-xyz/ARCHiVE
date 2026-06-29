//! Present X.509 certificates recovered from the keychain: parse just enough of
//! each certificate's DER for a human-readable summary (subject / issuer common
//! name, serial, validity window, CA flag) and emit a standard PEM bundle.
//!
//! The decryption and private-key pairing live in `archive_core::keychain`; this
//! module only *reads* the public certificate bytes it is handed. The DER walk is
//! intentionally minimal and total — any field it cannot parse is left empty
//! rather than erroring, so a slightly unusual certificate still exports its PEM
//! and whatever fields were readable.

use archive_core::keychain::CertificateItem;
use serde::Serialize;

/// Human-facing summary of one recovered certificate (no key material).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CertificateInfo {
    /// Keychain item label; empty when unlabelled.
    pub label: String,
    /// Subject common name (CN), or empty when absent/unparseable.
    pub subject: String,
    /// Issuer common name (CN), or empty.
    pub issuer: String,
    /// Serial number as lowercase hex (colon-separated), or empty.
    pub serial: String,
    /// Validity start, ISO 8601 UTC (`YYYY-MM-DDTHH:MM:SSZ`); empty if unparsed.
    pub not_before: String,
    /// Validity end, ISO 8601 UTC; empty if unparsed.
    pub not_after: String,
    /// Whether the certificate asserts the CA basic constraint.
    pub is_ca: bool,
    /// Whether a matching private key is present (the cert is a usable identity).
    pub has_private_key: bool,
}

/// Build the display summary for one recovered certificate.
pub fn describe(item: &CertificateItem) -> CertificateInfo {
    let x = parse_x509(&item.der);
    CertificateInfo {
        label: item.label.clone(),
        subject: x.subject,
        issuer: x.issuer,
        serial: x.serial,
        not_before: x.not_before,
        not_after: x.not_after,
        is_ca: x.is_ca,
        has_private_key: item.has_private_key,
    }
}

/// Concatenate every recovered certificate into one standard PEM bundle (the form
/// `openssl x509`, browsers and keychains all read). Certificates are emitted in
/// input order, each as a base64 `CERTIFICATE` block.
pub fn to_pem_bundle(items: &[CertificateItem]) -> String {
    let mut out = String::new();
    for item in items {
        out.push_str("-----BEGIN CERTIFICATE-----\n");
        out.push_str(&base64_wrapped(&item.der));
        out.push_str("-----END CERTIFICATE-----\n");
    }
    out
}

// --- minimal DER / X.509 ---------------------------------------------------

#[derive(Default)]
struct Parsed {
    subject: String,
    issuer: String,
    serial: String,
    not_before: String,
    not_after: String,
    is_ca: bool,
}

/// Read one DER TLV at `i`: returns `(tag, content, next_index)`. `None` on any
/// truncation or an unsupported (> 4-byte) length — never panics.
fn read_tlv(b: &[u8], i: usize) -> Option<(u8, &[u8], usize)> {
    let tag = *b.get(i)?;
    let mut j = i + 1;
    let first = *b.get(j)?;
    j += 1;
    let len = if first < 0x80 {
        first as usize
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 {
            return None;
        }
        let mut l = 0usize;
        for _ in 0..n {
            l = (l << 8) | (*b.get(j)? as usize);
            j += 1;
        }
        l
    };
    let content = b.get(j..j.checked_add(len)?)?;
    Some((tag, content, j + len))
}

/// Parse the fields we surface from an X.509 certificate DER. Positional walk of
/// `tbsCertificate`; every field is best-effort and defaults empty/false.
fn parse_x509(der: &[u8]) -> Parsed {
    let mut out = Parsed::default();
    // Certificate ::= SEQUENCE { tbsCertificate SEQUENCE, ... }
    let Some((0x30, cert, _)) = read_tlv(der, 0) else { return out };
    let Some((0x30, tbs, _)) = read_tlv(cert, 0) else { return out };

    let mut i = 0;
    // Optional EXPLICIT [0] version.
    if let Some((0xA0, _, next)) = read_tlv(tbs, i) {
        i = next;
    }
    // serialNumber INTEGER.
    if let Some((0x02, serial, next)) = read_tlv(tbs, i) {
        out.serial = hex_colon(serial);
        i = next;
    }
    // signature AlgorithmIdentifier (skip).
    if let Some((_, _, next)) = read_tlv(tbs, i) {
        i = next;
    }
    // issuer Name.
    if let Some((_, issuer, next)) = read_tlv(tbs, i) {
        out.issuer = find_cn(issuer);
        i = next;
    }
    // validity SEQUENCE { notBefore Time, notAfter Time }.
    if let Some((_, validity, next)) = read_tlv(tbs, i) {
        let (nb, na) = parse_validity(validity);
        out.not_before = nb;
        out.not_after = na;
        i = next;
    }
    // subject Name.
    if let Some((_, subject, _next)) = read_tlv(tbs, i) {
        out.subject = find_cn(subject);
    }
    out.is_ca = has_ca_basic_constraint(tbs);
    out
}

/// Find the first Common Name (OID 2.5.4.3) value in a DER `Name`. The CN
/// attribute is `SEQUENCE { OID 55 04 03, DirectoryString }`; scan for the OID and
/// read the value TLV that follows.
fn find_cn(name: &[u8]) -> String {
    const CN_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x04, 0x03];
    let mut i = 0;
    while i + CN_OID.len() <= name.len() {
        if name[i..].starts_with(&CN_OID)
            && let Some((_, value, _)) = read_tlv(name, i + CN_OID.len())
        {
            return String::from_utf8_lossy(value).trim().to_string();
        }
        i += 1;
    }
    String::new()
}

/// Parse a validity `SEQUENCE` into (notBefore, notAfter) ISO strings.
fn parse_validity(v: &[u8]) -> (String, String) {
    let mut times = Vec::new();
    let mut i = 0;
    while let Some((tag, content, next)) = read_tlv(v, i) {
        if tag == 0x17 || tag == 0x18 {
            times.push(parse_asn1_time(tag, content));
        }
        i = next;
        if times.len() == 2 {
            break;
        }
    }
    (times.first().cloned().unwrap_or_default(), times.get(1).cloned().unwrap_or_default())
}

/// Convert an ASN.1 UTCTime (`0x17`, `YYMMDDHHMMSSZ`) or GeneralizedTime (`0x18`,
/// `YYYYMMDDHHMMSSZ`) to `YYYY-MM-DDTHH:MM:SSZ`. Returns empty on a shape it does
/// not recognize.
fn parse_asn1_time(tag: u8, content: &[u8]) -> String {
    let s = String::from_utf8_lossy(content);
    let d: Vec<char> = s.chars().filter(|c| c.is_ascii_digit()).collect();
    let (year, rest) = if tag == 0x17 {
        if d.len() < 10 {
            return String::new();
        }
        let yy: i32 = d[0..2].iter().collect::<String>().parse().unwrap_or(0);
        let full = if yy < 50 { 2000 + yy } else { 1900 + yy };
        (full, &d[2..])
    } else {
        if d.len() < 12 {
            return String::new();
        }
        let yyyy: i32 = d[0..4].iter().collect::<String>().parse().unwrap_or(0);
        (yyyy, &d[4..])
    };
    let g = |a: usize, b: usize| rest.get(a..b).map(|c| c.iter().collect::<String>()).unwrap_or_default();
    let ss = if rest.len() >= 10 { g(8, 10) } else { "00".into() };
    format!("{year:04}-{}-{}T{}:{}:{}Z", g(0, 2), g(2, 4), g(4, 6), g(6, 8), ss)
}

/// Best-effort detection of the `CA:TRUE` basic constraint (OID 2.5.29.19): find
/// the extension OID and look for an encoded `BOOLEAN TRUE` (`01 01 ff`) shortly
/// after it (within the extension's small value). Defaults false.
fn has_ca_basic_constraint(tbs: &[u8]) -> bool {
    const BC_OID: [u8; 5] = [0x06, 0x03, 0x55, 0x1D, 0x13];
    let mut i = 0;
    while i + BC_OID.len() <= tbs.len() {
        if tbs[i..].starts_with(&BC_OID) {
            let window_end = (i + BC_OID.len() + 16).min(tbs.len());
            let window = &tbs[i + BC_OID.len()..window_end];
            return window.windows(3).any(|w| w == [0x01, 0x01, 0xFF]);
        }
        i += 1;
    }
    false
}

/// Lowercase, colon-separated hex of a byte slice (e.g. `01:a2:ff`).
fn hex_colon(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(":")
}

// --- base64 (PEM) ----------------------------------------------------------

/// Standard base64 of `data`, wrapped at 64 characters per line (PEM body),
/// trailing newline included. Dependency-free.
fn base64_wrapped(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut b64 = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        b64.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        b64.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        b64.push(if chunk.len() > 1 { ALPHABET[(n >> 6 & 63) as usize] as char } else { '=' });
        b64.push(if chunk.len() > 2 { ALPHABET[(n & 63) as usize] as char } else { '=' });
    }
    let mut out = String::with_capacity(b64.len() + b64.len() / 64 + 1);
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- DER builders for a synthetic but structurally-valid certificate ----

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        assert!(content.len() < 128, "test DER stays short-form");
        out.push(content.len() as u8);
        out.extend_from_slice(content);
        out
    }
    fn cn_name(cn: &str) -> Vec<u8> {
        // Name = SEQUENCE { SET { SEQUENCE { OID 2.5.4.3, UTF8String cn } } }
        let oid = [0x06, 0x03, 0x55, 0x04, 0x03];
        let mut atv = Vec::new();
        atv.extend(oid);
        atv.extend(tlv(0x0c, cn.as_bytes()));
        let seq = tlv(0x30, &atv);
        let set = tlv(0x31, &seq);
        tlv(0x30, &set)
    }
    fn utctime(s: &str) -> Vec<u8> {
        tlv(0x17, s.as_bytes())
    }

    /// A minimal cert: SEQUENCE{ tbs SEQUENCE{ [0]ver, serial, sigalg, issuer,
    /// validity, subject, ...basicConstraints CA:TRUE }, sigalg, sig }.
    fn build_cert(serial: &[u8], issuer: &str, subject: &str, nb: &str, na: &str, ca: bool) -> Vec<u8> {
        let mut tbs = Vec::new();
        tbs.extend(tlv(0xA0, &tlv(0x02, &[0x02]))); // [0] version v3
        tbs.extend(tlv(0x02, serial)); // serialNumber
        tbs.extend(tlv(0x30, &[0x06, 0x03, 0x2A, 0x03, 0x04])); // sigalg (dummy OID seq)
        tbs.extend(cn_name(issuer));
        let mut validity = Vec::new();
        validity.extend(utctime(nb));
        validity.extend(utctime(na));
        tbs.extend(tlv(0x30, &validity));
        tbs.extend(cn_name(subject));
        tbs.extend(tlv(0x30, &[0x06, 0x01, 0x00])); // dummy SPKI placeholder
        if ca {
            // extensions [3] { SEQUENCE { SEQUENCE { OID 2.5.29.19, OCTET basicConstraints } } }
            let bc_value = tlv(0x04, &tlv(0x30, &[0x01, 0x01, 0xFF])); // OCTET STRING wrapping SEQ{BOOL TRUE}
            let mut ext = vec![0x06, 0x03, 0x55, 0x1D, 0x13];
            ext.extend(bc_value);
            let ext_seq = tlv(0x30, &ext);
            let exts = tlv(0x30, &ext_seq);
            tbs.extend(tlv(0xA3, &exts));
        }
        let tbs_seq = tlv(0x30, &tbs);
        let mut cert = tbs_seq.clone();
        cert.extend(tlv(0x30, &[0x06, 0x03, 0x2A, 0x03, 0x04])); // outer sigalg
        cert.extend(tlv(0x03, &[0x00, 0xDE, 0xAD])); // signature BIT STRING
        tlv(0x30, &cert)
    }

    #[test]
    fn parses_subject_issuer_serial_validity() {
        let der = build_cert(&[0x01, 0xA2, 0xFF], "Apple Root CA", "device@example.com", "230115103000Z", "250115103000Z", false);
        let info = describe(&CertificateItem { label: "Cert 1".into(), der, has_private_key: true });
        assert_eq!(info.label, "Cert 1");
        assert_eq!(info.issuer, "Apple Root CA");
        assert_eq!(info.subject, "device@example.com");
        assert_eq!(info.serial, "01:a2:ff");
        assert_eq!(info.not_before, "2023-01-15T10:30:00Z");
        assert_eq!(info.not_after, "2025-01-15T10:30:00Z");
        assert!(!info.is_ca);
        assert!(info.has_private_key);
    }

    #[test]
    fn detects_ca_basic_constraint() {
        let der = build_cert(&[0x05], "Root", "Root", "200101000000Z", "300101000000Z", true);
        let info = describe(&CertificateItem { label: String::new(), der, has_private_key: false });
        assert!(info.is_ca);
        assert!(!info.has_private_key);
    }

    #[test]
    fn malformed_der_yields_empty_fields_not_panic() {
        for bad in [vec![], vec![0x30], vec![0x30, 0x84, 0xff, 0xff, 0xff, 0xff], vec![0xff; 8]] {
            let info = describe(&CertificateItem { label: "x".into(), der: bad, has_private_key: false });
            assert_eq!(info.subject, "");
            assert_eq!(info.serial, "");
        }
    }

    #[test]
    fn pem_bundle_round_trips_through_base64() {
        let der = vec![0x30, 0x03, 0x02, 0x01, 0x07];
        let bundle = to_pem_bundle(&[CertificateItem { label: "c".into(), der: der.clone(), has_private_key: false }]);
        assert!(bundle.starts_with("-----BEGIN CERTIFICATE-----\n"));
        assert!(bundle.trim_end().ends_with("-----END CERTIFICATE-----"));
        // Extract and decode the base64 body, compare to the original DER.
        let body: String = bundle.lines().filter(|l| !l.starts_with("-----")).collect();
        assert_eq!(b64_decode(&body), der);
    }

    #[test]
    fn base64_matches_known_vector() {
        // "Man" -> "TWFu"; padding cases per RFC 4648.
        assert_eq!(base64_wrapped(b"Man").trim_end(), "TWFu");
        assert_eq!(base64_wrapped(b"Ma").trim_end(), "TWE=");
        assert_eq!(base64_wrapped(b"M").trim_end(), "TQ==");
    }

    fn b64_decode(s: &str) -> Vec<u8> {
        let alpha = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let idx = |c: u8| alpha.iter().position(|&a| a == c);
        let clean: Vec<u8> = s.bytes().filter(|&c| c != b'=' && !c.is_ascii_whitespace()).collect();
        let mut out = Vec::new();
        for chunk in clean.chunks(4) {
            let mut n = 0u32;
            for (k, &c) in chunk.iter().enumerate() {
                n |= (idx(c).unwrap() as u32) << (18 - 6 * k);
            }
            let bytes = chunk.len() - 1;
            for k in 0..bytes {
                out.push((n >> (16 - 8 * k)) as u8);
            }
        }
        out
    }
}
