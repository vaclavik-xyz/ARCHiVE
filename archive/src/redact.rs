//! Redact personal identifiers from the one-line text the unified reports build,
//! so a `timeline` or `search` export can be shared (with a colleague, in a
//! filing) without leaking every phone number and email in full. It masks the two
//! strongest direct identifiers while keeping the surrounding structure readable:
//!
//! - **Phone numbers / long digit runs** — any run of ≥5 consecutive digits keeps
//!   only its last two, the rest become `•` (so `+420776452878` → `+••••••••••78`).
//! - **Email local parts** — everything before the `@` except the first character
//!   (so `jan.novak@example.com` → `j••••••••@example.com`).
//!
//! Names are deliberately kept — a redacted report still has to read as "X called
//! Y" to be useful — so this is identifier masking, not full anonymisation.

/// Mask a long digit run (`len >= 5`): keep the last two digits, replace the rest.
fn mask_digit_run(run: &[char]) -> String {
    if run.len() < 5 {
        return run.iter().collect();
    }
    let mut s: String = std::iter::repeat_n('•', run.len() - 2).collect();
    s.extend(&run[run.len() - 2..]);
    s
}

/// Mask an email local part: keep the first character, replace the rest with `•`.
fn mask_local(local: &str) -> String {
    let mut chars = local.chars();
    match chars.next() {
        Some(first) => {
            let mut s = String::from(first);
            s.extend(std::iter::repeat_n('•', chars.count()));
            s
        }
        None => String::new(),
    }
}

/// Redact one whitespace-delimited word: an email's local part, else any embedded
/// long digit run.
fn redact_word(w: &str) -> String {
    // Email: an `@` with a dotted domain after it. Mask the local part only.
    if let Some(at) = w.find('@')
        && w[at + 1..].contains('.')
    {
        let (local, rest) = w.split_at(at); // rest starts at '@'
        return format!("{}{rest}", mask_local(local));
    }
    // Otherwise mask long digit runs anywhere in the word.
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            out.push_str(&mask_digit_run(&chars[start..i]));
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// Redact phone numbers and email local parts in `s`, preserving word structure.
pub fn redact_pii(s: &str) -> String {
    s.split_whitespace().map(redact_word).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_phone_keeping_last_two() {
        assert_eq!(redact_pii("outgoing +420776452878 (42s)"), "outgoing +••••••••••78 (42s)");
        // A short number (< 5 digits) is left alone.
        assert_eq!(redact_pii("apt 4321"), "apt 4321");
        assert_eq!(redact_pii("code 12345"), "code •••45");
    }

    #[test]
    fn masks_email_local_part_only() {
        assert_eq!(redact_pii("from jan.novak@example.com today"), "from j••••••••@example.com today");
        // A bare '@' with no dotted domain is not treated as an email.
        assert_eq!(redact_pii("rate 5@ each"), "rate 5@ each");
    }

    #[test]
    fn keeps_names_and_words() {
        assert_eq!(redact_pii("Jan Novák called Eva"), "Jan Novák called Eva");
    }

    #[test]
    fn handles_combined_identifiers() {
        let out = redact_pii("Jan +420123456789 jan@firma.cz");
        assert_eq!(out, "Jan +••••••••••89 j••@firma.cz");
    }

    #[test]
    fn redacts_a_bare_query_value() {
        // search --redact masks the echoed query too, so a bare identifier query
        // does not leak into the report title or the JSON envelope.
        assert_eq!(redact_pii("+420776452878"), "+••••••••••78");
        assert_eq!(redact_pii("jan@firma.cz"), "j••@firma.cz");
    }
}
