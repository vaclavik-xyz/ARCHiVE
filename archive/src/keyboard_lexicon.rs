//! The user's **custom keyboard words** — the entries added via *Settings →
//! General → Keyboard* ("Add to Dictionary") that iOS keeps in
//! `Library/Keyboard/LocalDictionary`. Unlike the *learned* typing model in
//! `user_model_database.sqlite` (whose keys are internal markers, not a readable
//! vocabulary — deliberately not parsed), this file is a clean list of words the
//! user themselves added, so it is worth recovering as-is.
//!
//! The on-disk shape has varied across iOS versions — a binary/XML property list
//! (an array of strings, or a dict) or a plain newline-delimited UTF-8 list — so
//! the parser is **format-tolerant**: it tries a plist first, then plain text,
//! and as a last resort carves printable word runs from the bytes. Any input that
//! yields no plausible words returns an empty list rather than erroring.
//!
//! Availability: the file is present in ordinary backups but is **empty on a
//! device whose owner never added a custom word**, so an honest empty result is
//! common.

use std::io::Cursor;

use plist::Value;
use serde::Serialize;

/// Backup domain + candidate relative paths of the custom-words file, probed in
/// order (the keyboard data has lived under both `KeyboardDomain` and `HomeDomain`).
pub const SOURCES: &[(&str, &str)] = &[
    ("KeyboardDomain", "Library/Keyboard/LocalDictionary"),
    ("HomeDomain", "Library/Keyboard/LocalDictionary"),
];

/// One recovered custom keyboard word.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LexiconWord {
    /// The word/phrase the user added to their keyboard dictionary.
    pub word: String,
}

/// Parse custom words from a `LocalDictionary` file's bytes, de-duplicated and
/// sorted (case-insensitive). Tolerant of the plist and plain-text layouts.
pub fn parse(bytes: &[u8]) -> Vec<LexiconWord> {
    let mut words = from_plist(bytes)
        .or_else(|| from_plain_text(bytes))
        .unwrap_or_else(|| carve_words(bytes));

    // Normalize: trim, drop empties, de-duplicate (case-insensitive), sort.
    words.iter_mut().for_each(|w| *w = w.trim().to_string());
    words.retain(|w| is_word(w));
    words.sort_by_key(|w| w.to_lowercase());
    words.dedup_by_key(|w| w.to_lowercase());
    words.into_iter().map(|word| LexiconWord { word }).collect()
}

/// Extract the word strings from a property list. The canonical `LocalDictionary`
/// shape is an **array of strings**; a wrapper dict (e.g. `{ "words": [...] }`) is
/// also handled by descending into its **values**. Dictionary **keys** are never
/// treated as words — they are container/metadata names (`words`, language codes,
/// schema fields), not user-added vocabulary. Returns `Some` (possibly empty)
/// whenever the bytes parse as a plist, so a property list is never re-scanned by
/// the binary carve; `None` only when the bytes are not a plist at all.
fn from_plist(bytes: &[u8]) -> Option<Vec<String>> {
    let value = Value::from_reader(Cursor::new(bytes)).ok()?;
    let mut out = Vec::new();
    collect_word_strings(&value, &mut out);
    Some(out)
}

/// Recursively gather strings from arrays and dict **values** (never keys).
fn collect_word_strings(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.push(s.clone()),
        Value::Array(a) => a.iter().for_each(|e| collect_word_strings(e, out)),
        // Descend into values only — keys are container/metadata names, not words.
        Value::Dictionary(d) => d.values().for_each(|val| collect_word_strings(val, out)),
        _ => {}
    }
}

/// Treat the bytes as a plain newline-delimited UTF-8 word list. `None` when the
/// bytes are not predominantly printable text (so binary files fall through to
/// carving).
fn from_plain_text(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let printable = text.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\r' || *c == '\t').count();
    if printable * 100 < text.chars().count().max(1) * 90 {
        return None; // too much control content to be a text list
    }
    Some(text.lines().map(|l| l.trim().to_string()).collect())
}

/// Last-resort carve: extract maximal runs of letter-like characters (handling
/// both UTF-8 and UTF-16LE text embedded in a binary file).
fn carve_words(bytes: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    // UTF-8 runs.
    out.extend(carve_runs(&String::from_utf8_lossy(bytes)));
    // UTF-16LE runs (every other byte is often 0 for ASCII-in-UTF-16).
    if bytes.len() >= 2 {
        let u16s: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        out.extend(carve_runs(&String::from_utf16_lossy(&u16s)));
    }
    out
}

/// Split a string into runs of letters (plus internal apostrophes/hyphens),
/// keeping those of length ≥ 2.
fn carve_runs(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in s.chars() {
        if c.is_alphabetic() || ((c == '\'' || c == '-') && !cur.is_empty()) {
            cur.push(c);
        } else {
            if cur.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
    }
    if cur.chars().filter(|c| c.is_alphabetic()).count() >= 2 {
        out.push(cur);
    }
    out
}

/// Whether a trimmed candidate is a plausible word: non-empty, not absurdly long,
/// and mostly non-control.
fn is_word(w: &str) -> bool {
    if w.is_empty() || w.chars().count() > 64 {
        return false;
    }
    let control = w.chars().filter(|c| c.is_control() || *c == '\u{fffd}').count();
    control == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(v: &[LexiconWord]) -> Vec<&str> {
        v.iter().map(|w| w.word.as_str()).collect()
    }

    #[test]
    fn parses_plist_array_layout() {
        let arr = Value::Array(vec![
            Value::String("ARCHiVE".into()),
            Value::String("Václavík".into()),
            Value::String("".into()),
            Value::String("ARCHiVE".into()), // duplicate
        ]);
        let mut buf = Vec::new();
        arr.to_writer_xml(&mut buf).unwrap();
        let got = parse(&buf);
        // Empty dropped, duplicate collapsed, sorted case-insensitively.
        assert_eq!(words(&got), vec!["ARCHiVE", "Václavík"]);
    }

    #[test]
    fn parses_plist_wrapper_dict_values_not_keys() {
        // A wrapper dict { "words": [...] }: the array values are recovered, but the
        // container key "words" (metadata, not a user word) must NOT be.
        let mut d = plist::Dictionary::new();
        d.insert("words".into(), Value::Array(vec![Value::String("foo".into()), Value::String("bar".into())]));
        d.insert("schemaVersion".into(), Value::Integer(3.into()));
        let mut buf = Vec::new();
        Value::Dictionary(d).to_writer_xml(&mut buf).unwrap();
        let got: Vec<String> = parse(&buf).into_iter().map(|w| w.word).collect();
        assert!(got.contains(&"foo".to_string()) && got.contains(&"bar".to_string()));
        assert!(!got.contains(&"words".to_string()), "container key must not be a word");
        assert!(!got.contains(&"schemaVersion".to_string()));
    }

    #[test]
    fn parses_plain_text_lines() {
        let got = parse(b"hello\nworld\n\n  spaced  \nhello\n");
        assert_eq!(words(&got), vec!["hello", "spaced", "world"]);
    }

    #[test]
    fn carves_words_from_binary() {
        // A binary blob with embedded ASCII words and noise bytes.
        let mut blob = vec![0x00, 0x01, 0xff];
        blob.extend_from_slice(b"Kortex");
        blob.extend_from_slice(&[0x00, 0x00, 0x10]);
        blob.extend_from_slice(b"Probe");
        blob.push(0xfe);
        let got: Vec<String> = parse(&blob).into_iter().map(|w| w.word).collect();
        assert!(got.contains(&"Kortex".to_string()));
        assert!(got.contains(&"Probe".to_string()));
    }

    #[test]
    fn empty_and_malformed_never_panic() {
        assert!(parse(b"").is_empty());
        assert!(parse(&[0x00, 0x01, 0x02, 0xff, 0xfe]).is_empty());
    }
}
