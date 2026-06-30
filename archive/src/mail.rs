//! Parse Apple Mail `.emlx` messages from an iOS backup (best-effort).
//!
//! Feasibility caveat: unlike macOS, iOS device backups normally do **not**
//! contain mail message bodies — Apple treats IMAP/Exchange mail as
//! server-side and excludes it, backing up only the account *settings*. The one
//! realistic source is locally-stored mail (POP3 accounts or mailboxes the user
//! moved "On My iPhone"), which lands under `MailDomain` as Apple `.emlx`
//! files. The controller wires this as an optional, best-effort extractor that
//! reports "no mail store found" (count 0) when nothing is present.
//!
//! An `.emlx` file is three concatenated parts:
//!   1. an ASCII decimal **byte count** on the first line (terminated by `\n`),
//!   2. exactly that many bytes of the **RFC 822 message** (headers, blank
//!      line, body),
//!   3. a trailing Apple **XML plist** with per-message flags (ignored here).
//!
//! This module hand-rolls a minimal header/body split and a text-part snippet
//! extractor; it pulls in no email/MIME crate (only std + chrono). It never
//! panics on malformed input — every parse path returns `None` or empty fields.

use chrono::DateTime;
use serde::Serialize;

/// Backup domain holding locally-stored Apple Mail (`MailDomain`). Within it,
/// individual messages live as `.emlx` files under per-account/per-mailbox
/// directories (the controller enumerates them; the exact subtree is
/// account-dependent). Mail is frequently absent — see the module docs.
pub const MAIL_DOMAIN: &str = "MailDomain";

/// Maximum length (in characters) of the extracted body `snippet`. Bounds output
/// so a large message body cannot bloat the record.
pub const SNIPPET_CAP: usize = 280;

/// One parsed Apple Mail message. Header fields keep their raw values when MIME
/// encoded-words cannot be decoded (see [`parse_emlx`]).
#[derive(Debug, Clone, Serialize)]
pub struct MailMessage {
    /// `From:` header (decoded display value); empty when absent.
    pub from: String,
    /// `To:` header (decoded display value); empty when absent.
    pub to: String,
    /// `Subject:` header (decoded display value); empty when absent.
    pub subject: String,
    /// `Date:` header parsed (RFC 2822) to ISO 8601 UTC; `None` when absent or
    /// unparseable.
    pub date: Option<String>,
    /// First `text/plain` body part (or the whole body when not multipart),
    /// whitespace-collapsed and capped at [`SNIPPET_CAP`] characters.
    pub snippet: String,
}

/// Parse one `.emlx` file's raw bytes into a [`MailMessage`]. Returns `None` on
/// malformed input (missing/invalid byte-count line, or a count that overruns
/// the buffer); never panics.
///
/// Decoding decisions:
/// - Header names are matched case-insensitively; folded headers (continuation
///   lines starting with space/tab) are unfolded.
/// - MIME encoded-words (`=?charset?B/Q?...?=`) in `From`/`To`/`Subject` are
///   decoded best-effort for UTF-8 / ASCII-compatible charsets; any word that
///   cannot be decoded is left verbatim, so the raw header is never lost.
/// - `Date` is parsed with `chrono::DateTime::parse_from_rfc2822` and rendered
///   as ISO 8601 UTC, else `None`.
/// - The snippet comes from the first `text/plain` part of a `multipart/*`
///   message (matched by its boundary), else the raw body. No transfer-encoding
///   (quoted-printable / base64) decoding is applied to the body — the snippet
///   is a human-readable preview, not a faithful body reconstruction.
pub fn parse_emlx(bytes: &[u8]) -> Option<MailMessage> {
    let message = rfc822_slice(bytes)?;

    let (header_block, body) = split_headers_body(message);
    let headers = parse_headers(header_block);

    let from = header_value(&headers, "from").map(|v| decode_words(&v)).unwrap_or_default();
    let to = header_value(&headers, "to").map(|v| decode_words(&v)).unwrap_or_default();
    let subject = header_value(&headers, "subject").map(|v| decode_words(&v)).unwrap_or_default();
    let date = header_value(&headers, "date").and_then(|v| parse_date(&v));

    let content_type = header_value(&headers, "content-type").unwrap_or_default();
    let snippet = snippet_from_body(body, &content_type);

    Some(MailMessage { from, to, subject, date, snippet })
}

/// Slice out the RFC 822 message using the leading ASCII byte-count line. The
/// first line (up to `\n`) is a decimal byte count; the message is exactly that
/// many bytes immediately after it. Returns `None` when the count line is
/// missing/non-numeric or the count overruns the buffer.
fn rfc822_slice(bytes: &[u8]) -> Option<&[u8]> {
    let newline = bytes.iter().position(|&b| b == b'\n')?;
    // The count line may carry a trailing `\r`; trim ASCII whitespace either way.
    let count_str = std::str::from_utf8(&bytes[..newline]).ok()?;
    let count: usize = count_str.trim().parse().ok()?;
    let start = newline + 1;
    let end = start.checked_add(count)?;
    if end > bytes.len() {
        return None;
    }
    Some(&bytes[start..end])
}

/// Split a message into its header block and body at the first blank line
/// (`\r\n\r\n` or `\n\n`). When no blank line exists, treat the whole input as
/// headers and the body as empty. Operates on raw bytes, decoding lazily later.
fn split_headers_body(message: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = find_subslice(message, b"\r\n\r\n") {
        (&message[..pos], &message[pos + 4..])
    } else if let Some(pos) = find_subslice(message, b"\n\n") {
        (&message[..pos], &message[pos + 2..])
    } else {
        (message, &[])
    }
}

/// All `(lowercased-name, unfolded-value)` header pairs, in order. Continuation
/// lines (starting with a space or tab) are appended to the preceding header's
/// value with the folding whitespace collapsed to a single space. Lines without
/// a colon (and stray leading continuation lines) are skipped.
fn parse_headers(block: &[u8]) -> Vec<(String, String)> {
    // Lossy UTF-8 is fine: header *names* are ASCII; non-ASCII bytes in values
    // are either MIME-encoded (decoded later) or raw 8-bit we surface as-is.
    let text = String::from_utf8_lossy(block);
    let mut headers: Vec<(String, String)> = Vec::new();
    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() {
            continue;
        }
        if line.starts_with([' ', '\t']) {
            // Folded continuation: append to the current header's value.
            if let Some(last) = headers.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim_start());
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    headers
}

/// First value for a (lowercase) header `name`, or `None` when absent.
fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
}

/// Parse an RFC 2822 `Date:` value into ISO 8601 UTC, or `None` when unparseable.
fn parse_date(value: &str) -> Option<String> {
    DateTime::parse_from_rfc2822(value.trim())
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc).to_rfc3339())
}

/// Build the snippet: the first `text/plain` part of a multipart message (found
/// via the `Content-Type` boundary), else the whole body. The chosen text is
/// whitespace-collapsed and capped at [`SNIPPET_CAP`] characters.
fn snippet_from_body(body: &[u8], content_type: &str) -> String {
    let text = if let Some(boundary) = multipart_boundary(content_type) {
        first_text_plain_part(body, &boundary).unwrap_or_else(|| body.to_vec())
    } else {
        body.to_vec()
    };
    let decoded = String::from_utf8_lossy(&text);
    collapse_and_cap(&decoded, SNIPPET_CAP)
}

/// The `boundary=` parameter of a `multipart/*` content type, or `None` when the
/// type is not multipart or carries no boundary. Handles quoted and bare values.
fn multipart_boundary(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.contains("multipart/") {
        return None;
    }
    // Find `boundary=` (case-insensitive) and read its value up to the next `;`.
    let idx = lower.find("boundary=")?;
    let rest = &content_type[idx + "boundary=".len()..];
    let rest = rest.trim_start();
    let value = if let Some(stripped) = rest.strip_prefix('"') {
        // Quoted: up to the closing quote.
        stripped.split('"').next().unwrap_or("")
    } else {
        // Bare: up to the next `;` or whitespace.
        rest.split([';', ' ', '\t', '\r', '\n']).next().unwrap_or("")
    };
    if value.is_empty() { None } else { Some(value.to_string()) }
}

/// The body bytes of the first `text/plain` part within a multipart body, split
/// on `--<boundary>`. Each part is itself header-block + blank line + content;
/// the first part whose `Content-Type` is `text/plain` (or that has no explicit
/// type, defaulting to text/plain per RFC 2045) wins. `None` when none match.
fn first_text_plain_part(body: &[u8], boundary: &str) -> Option<Vec<u8>> {
    let delimiter = format!("--{boundary}");
    let text = String::from_utf8_lossy(body);
    // `split` yields the preamble first (everything before the opening
    // delimiter); skip it with `.skip(1)` so it is never mistaken for a part.
    for part in text.split(delimiter.as_str()).skip(1) {
        // Skip the epilogue and the trailing `--` close delimiter.
        let part = part.trim_start_matches(['\r', '\n']);
        if part.is_empty() || part.starts_with("--") {
            continue;
        }
        let (part_headers, part_body) = split_headers_body(part.as_bytes());
        let headers = parse_headers(part_headers);
        let ctype = header_value(&headers, "content-type").unwrap_or_default().to_ascii_lowercase();
        // Default content type is text/plain when unspecified (RFC 2045 §5.2).
        if ctype.is_empty() || ctype.contains("text/plain") {
            return Some(part_body.to_vec());
        }
    }
    None
}

/// Collapse all runs of ASCII whitespace to single spaces, trim the ends, and
/// cap at `max` characters (by `char`, so multibyte UTF-8 is never split).
fn collapse_and_cap(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        collapsed
    } else {
        collapsed.chars().take(max).collect()
    }
}

/// Find the first index of `needle` in `haystack` (byte-wise), or `None`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode MIME encoded-words (`=?charset?enc?text?=`) in a header value,
/// best-effort. Recognizes `B` (base64) and `Q` (quoted-printable) encodings for
/// UTF-8 / US-ASCII / Latin-1 charsets; any word that fails to decode (unknown
/// charset, bad payload) is left verbatim so the raw header is never lost.
/// Per RFC 2047, whitespace separating two adjacent encoded-words is dropped.
fn decode_words(value: &str) -> String {
    if !value.contains("=?") {
        return value.to_string();
    }
    let mut out = String::new();
    let mut rest = value;
    // Tracks whether the previous emitted token was a decoded encoded-word, so
    // the whitespace separating two adjacent encoded-words can be collapsed away.
    let mut prev_was_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, tail) = rest.split_at(start);
        // Emit literal text before the encoded-word. If it is only the
        // whitespace between two encoded-words, RFC 2047 says to drop it.
        if !(prev_was_word && before.trim().is_empty() && !before.is_empty()) {
            out.push_str(before);
        }
        // An encoded-word is `=?charset?enc?payload?=`.
        let after = &tail[2..];
        if let Some((decoded, consumed)) = decode_one_word(after) {
            out.push_str(&decoded);
            rest = &after[consumed..];
            prev_was_word = true;
        } else {
            // Not a valid encoded-word: emit the `=?` literally and move on.
            out.push_str("=?");
            rest = after;
            prev_was_word = false;
        }
    }
    out.push_str(rest);
    out
}

/// Decode a single encoded-word whose `charset?enc?payload?=` text begins at the
/// start of `s` (the leading `=?` already stripped). Returns the decoded string
/// and the number of bytes consumed up to and including the closing `?=`, or
/// `None` when the syntax/charset/payload is unsupported.
fn decode_one_word(s: &str) -> Option<(String, usize)> {
    let charset_end = s.find('?')?;
    let charset = s[..charset_end].to_ascii_lowercase();
    let after_charset = &s[charset_end + 1..];
    let enc_end = after_charset.find('?')?;
    let enc = after_charset[..enc_end].to_ascii_uppercase();
    let after_enc = &after_charset[enc_end + 1..];
    let payload_end = after_enc.find("?=")?;
    let payload = &after_enc[..payload_end];
    // Total consumed: charset + '?' + enc + '?' + payload + '?='.
    let consumed = charset_end + 1 + enc_end + 1 + payload_end + 2;

    let raw: Vec<u8> = match enc.as_str() {
        "B" => decode_base64(payload)?,
        "Q" => decode_q(payload),
        _ => return None,
    };
    let decoded = match charset.as_str() {
        "utf-8" | "utf8" | "us-ascii" | "ascii" => String::from_utf8_lossy(&raw).into_owned(),
        // Latin-1 maps each byte directly to the same Unicode scalar value.
        "iso-8859-1" | "latin1" | "iso8859-1" => raw.iter().map(|&b| b as char).collect(),
        _ => return None, // unsupported charset → caller keeps the raw word
    };
    Some((decoded, consumed))
}

/// Decode the quoted-printable payload of a `Q`-encoded word: `_` → space and
/// `=XX` → the byte `0xXX`; other bytes pass through. Malformed `=XX` sequences
/// are emitted verbatim.
fn decode_q(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'_' => {
                out.push(b' ');
                i += 1;
            }
            b'=' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                if let (Some(hi), Some(lo)) = (hi, lo) {
                    out.push((hi * 16 + lo) as u8);
                    i += 3;
                } else {
                    out.push(b'=');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Decode standard base64 (`A–Z a–z 0–9 + /`, `=` padding). Whitespace is
/// ignored; any other character makes the whole word undecodable (`None`).
fn decode_base64(payload: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut acc = 0u32;
    let mut bits = 0u32;
    let mut out = Vec::new();
    for &c in payload.as_bytes() {
        if c == b'=' || c.is_ascii_whitespace() {
            continue; // padding / folding whitespace
        }
        let v = val(c)? as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Build a customer-facing summary of the recovered mail.
pub fn summary(items: &[MailMessage]) -> crate::summary::Summary {
    use crate::summary::{iso_range, tally, year_rows, Summary};
    use std::collections::HashSet;

    let dated = items.iter().filter(|m| m.date.is_some()).count();
    let undated = items.iter().filter(|m| m.date.is_none()).count();
    let distinct_senders = items
        .iter()
        .map(|m| m.from.as_str())
        .filter(|f| !f.is_empty())
        .collect::<HashSet<_>>()
        .len();
    let no_subject = items.iter().filter(|m| m.subject.is_empty()).count();

    let by_sender: Vec<(String, usize)> =
        tally(items.iter().map(|m| m.from.clone()).filter(|f| !f.is_empty()))
            .into_iter()
            .take(15)
            .collect();
    // Domain = substring after the last `@` of `from`, lowercased; addresses
    // without an `@` are skipped. `from` may carry display-name syntax
    // (`Name <a@b.com>`), so the domain can include a trailing `>`.
    let by_domain: Vec<(String, usize)> = tally(
        items
            .iter()
            .filter_map(|m| m.from.rsplit_once('@').map(|(_, d)| d.to_ascii_lowercase())),
    )
    .into_iter()
    .take(15)
    .collect();

    Summary::new("mail", "Pošta", "e-mailů", items.len())
        .count("S datem", dated)
        .count("Bez data", undated)
        .count("Unikátních odesílatelů", distinct_senders)
        .count("Bez předmětu", no_subject)
        .period_from(iso_range(items.iter().map(|m| m.date.as_deref().unwrap_or(""))))
        .breakdown("Po letech", year_rows(items.iter().map(|m| m.date.as_deref().unwrap_or(""))))
        .breakdown("Podle odesílatele", by_sender)
        .breakdown("Podle domény odesílatele", by_domain)
        .note("Obvykle jen lokální/POP3 schránky; těla IMAP/Exchange nebývají v záloze.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrap an RFC 822 message into a full `.emlx` buffer: `<byte-count>\n` +
    /// message + a trailing Apple plist (which `parse_emlx` must ignore).
    fn emlx(message: &str) -> Vec<u8> {
        let plist = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
            \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
            <plist version=\"1.0\"><dict><key>flags</key><integer>0</integer></dict></plist>\n";
        let mut buf = format!("{}\n", message.len()).into_bytes();
        buf.extend_from_slice(message.as_bytes());
        buf.extend_from_slice(plist.as_bytes());
        buf
    }

    fn mail(from: &str, subject: &str, date: Option<&str>) -> MailMessage {
        MailMessage {
            from: from.into(),
            to: "me@x.cz".into(),
            subject: subject.into(),
            date: date.map(Into::into),
            snippet: "...".into(),
        }
    }

    #[test]
    fn summary_counts_breakdowns_and_period() {
        let msgs = vec![
            mail("alice@example.com", "Ahoj", Some("2023-05-01T10:00:00+00:00")),
            mail("alice@example.com", "", Some("2024-06-01T10:00:00+00:00")),
            mail("", "Bez odesílatele", None),
        ];
        let s = summary(&msgs);
        assert_eq!(s.total, 3);
        assert_eq!(s.total_label, "e-mailů");
        let get = |label: &str| s.counts.iter().find(|(l, _)| l == label).map(|(_, n)| *n);
        assert_eq!(get("S datem"), Some(2));
        assert_eq!(get("Bez data"), Some(1));
        assert_eq!(get("Unikátních odesílatelů"), Some(1)); // two messages, one sender
        assert_eq!(get("Bez předmětu"), Some(1));
        let dom = s.breakdowns.iter().find(|b| b.title == "Podle domény odesílatele").unwrap();
        assert_eq!(dom.rows[0], ("example.com".to_string(), 2));
        assert!(s.period.is_some()); // derived from the two dated messages
    }

    #[test]
    fn parses_simple_message() {
        let msg = "From: Alice <alice@example.com>\r\n\
                   To: Bob <bob@example.com>\r\n\
                   Subject: Hello there\r\n\
                   Date: Mon, 6 Jan 2020 10:40:00 +0000\r\n\
                   \r\n\
                   This is the body of the message.\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.from, "Alice <alice@example.com>");
        assert_eq!(m.to, "Bob <bob@example.com>");
        assert_eq!(m.subject, "Hello there");
        assert_eq!(m.date.as_deref(), Some("2020-01-06T10:40:00+00:00"));
        assert_eq!(m.snippet, "This is the body of the message.");
    }

    #[test]
    fn unfolds_folded_headers() {
        let msg = "From: alice@example.com\r\n\
                   Subject: This is a very long subject\r\n\
                   \tthat continues on the next line\r\n\
                   Date: Tue, 7 Jan 2020 08:00:00 +0100\r\n\
                   \r\n\
                   Body.\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.subject, "This is a very long subject that continues on the next line");
        // +0100 → 07:00 UTC.
        assert_eq!(m.date.as_deref(), Some("2020-01-07T07:00:00+00:00"));
    }

    #[test]
    fn snippet_from_multipart_text_plain_part() {
        let msg = "From: a@example.com\r\n\
                   To: b@example.com\r\n\
                   Subject: Multipart\r\n\
                   Content-Type: multipart/alternative; boundary=\"SEP\"\r\n\
                   \r\n\
                   This is the preamble, ignore it.\r\n\
                   --SEP\r\n\
                   Content-Type: text/plain; charset=utf-8\r\n\
                   \r\n\
                   Plain text part wins.\r\n\
                   --SEP\r\n\
                   Content-Type: text/html\r\n\
                   \r\n\
                   <p>HTML part should be skipped.</p>\r\n\
                   --SEP--\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.snippet, "Plain text part wins.");
    }

    #[test]
    fn malformed_buffers_return_none() {
        // No newline at all → no byte-count line.
        assert!(parse_emlx(b"no newline here").is_none());
        // Non-numeric count line.
        assert!(parse_emlx(b"abc\nrest of data").is_none());
        // Count overruns the buffer.
        assert!(parse_emlx(b"9999\nshort").is_none());
        // Empty input.
        assert!(parse_emlx(b"").is_none());
    }

    #[test]
    fn unparseable_date_yields_none_but_keeps_other_fields() {
        let msg = "From: alice@example.com\r\n\
                   To: bob@example.com\r\n\
                   Subject: No date\r\n\
                   Date: not a real date\r\n\
                   \r\n\
                   Still has a body.\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.date, None);
        assert_eq!(m.from, "alice@example.com");
        assert_eq!(m.to, "bob@example.com");
        assert_eq!(m.subject, "No date");
        assert_eq!(m.snippet, "Still has a body.");
    }

    #[test]
    fn missing_date_header_yields_none() {
        let msg = "From: alice@example.com\r\nSubject: x\r\n\r\nbody\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.date, None);
    }

    #[test]
    fn snippet_is_whitespace_collapsed_and_capped() {
        let long_body = "word ".repeat(200); // 1000 chars before collapse
        let msg = format!(
            "From: a@example.com\r\nSubject: long\r\n\r\n  multiple   spaces\tand\n\nnewlines {long_body}"
        );
        let m = parse_emlx(&emlx(&msg)).expect("parses");
        assert_eq!(m.snippet.chars().count(), SNIPPET_CAP);
        assert!(!m.snippet.contains('\n'));
        assert!(!m.snippet.contains("  "), "runs of whitespace collapsed");
        assert!(m.snippet.starts_with("multiple spaces and newlines"));
    }

    #[test]
    fn header_names_are_case_insensitive() {
        let msg = "FROM: alice@example.com\r\nSUBJECT: Yelling\r\n\r\nbody\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.from, "alice@example.com");
        assert_eq!(m.subject, "Yelling");
    }

    #[test]
    fn decodes_mime_encoded_word_subject_utf8_q_and_b() {
        // Q-encoded UTF-8 "Schůzka" (ů = U+016F → UTF-8 C5 AF).
        let q = "Subject: =?utf-8?Q?Sch=C5=AFzka?=\r\n";
        // B-encoded UTF-8 "Příliš".
        let pristly = "Příliš";
        let b64 = super_encode_base64(pristly.as_bytes());
        let b = format!("Subject: =?UTF-8?B?{b64}?=\r\n");
        let msg = format!("From: a@example.com\r\n{q}\r\nbody\r\n");
        let m = parse_emlx(&emlx(&msg)).expect("parses");
        assert_eq!(m.subject, "Schůzka");

        let msg2 = format!("From: a@example.com\r\n{b}\r\nbody\r\n");
        let m2 = parse_emlx(&emlx(&msg2)).expect("parses");
        assert_eq!(m2.subject, pristly);
    }

    #[test]
    fn keeps_raw_header_on_unsupported_encoded_word() {
        // Unknown charset → the word is left verbatim, never lost.
        let msg = "From: a@example.com\r\nSubject: =?shift_jis?B?gqA=?=\r\n\r\nbody\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.subject, "=?shift_jis?B?gqA=?=");
    }

    #[test]
    fn non_multipart_uses_raw_body() {
        let msg = "From: a@example.com\r\nContent-Type: text/plain\r\n\r\nJust plain.\r\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.snippet, "Just plain.");
    }

    #[test]
    fn lf_only_line_endings_are_handled() {
        // Some stores use bare LF rather than CRLF.
        let msg = "From: a@example.com\nSubject: LF only\n\nLF body.\n";
        let m = parse_emlx(&emlx(msg)).expect("parses");
        assert_eq!(m.subject, "LF only");
        assert_eq!(m.snippet, "LF body.");
    }

    /// Minimal base64 encoder, test-only, to build `B`-encoded fixtures.
    fn super_encode_base64(input: &[u8]) -> String {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in input.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | (b[2] as u32);
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
            out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
        }
        out
    }
}
