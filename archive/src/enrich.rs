//! Cross-cutting contact enrichment: resolve phone numbers / emails / WhatsApp
//! JIDs that appear in other extractors (calls, voicemail, WhatsApp) to a
//! display name from the address book.
//!
//! Matching is best-effort and conservative: phone numbers are reduced to their
//! last 9 significant digits (so `+420 123 456 789`, `00420123456789`, and
//! `123 456 789` all match the same contact across country-code and formatting
//! differences), emails are matched case-insensitively, and WhatsApp JIDs
//! (`<number>@s.whatsapp.net`) are matched on their numeric local part. A handle
//! that resolves to no contact is simply left unenriched.

use std::collections::HashMap;

use crate::contacts::Contact;

/// Minimum digit count for a value to be treated as a phone number (avoids
/// matching short codes / partial strings).
const MIN_PHONE_DIGITS: usize = 7;

/// Number of trailing digits used as the phone match key.
const PHONE_KEY_DIGITS: usize = 9;

/// A reverse index from handle → contact display name.
#[derive(Debug, Default)]
pub struct ContactIndex {
    phone: HashMap<String, String>,
    email: HashMap<String, String>,
}

/// A contact's display name: "First Last", else the organization; empty when
/// none of those are set.
fn display_name(c: &Contact) -> String {
    let full = format!("{} {}", c.first.trim(), c.last.trim());
    let full = full.trim();
    if !full.is_empty() {
        full.to_string()
    } else {
        c.organization.trim().to_string()
    }
}

/// Reduce a phone-ish string to its match key: the last [`PHONE_KEY_DIGITS`]
/// digits (or all digits when shorter). `None` when there are too few digits.
fn phone_key(s: &str) -> Option<String> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < MIN_PHONE_DIGITS {
        return None;
    }
    Some(if digits.len() > PHONE_KEY_DIGITS {
        digits[digits.len() - PHONE_KEY_DIGITS..].to_string()
    } else {
        digits
    })
}

impl ContactIndex {
    /// Build the index from the address book. The first contact seen wins a key
    /// collision, keeping the result deterministic for a given input order.
    pub fn build(contacts: &[Contact]) -> Self {
        let mut idx = ContactIndex::default();
        for c in contacts {
            let name = display_name(c);
            if name.is_empty() {
                continue;
            }
            for p in &c.phones {
                if let Some(k) = phone_key(&p.value) {
                    idx.phone.entry(k).or_insert_with(|| name.clone());
                }
            }
            for e in &c.emails {
                let k = e.value.trim().to_lowercase();
                if !k.is_empty() {
                    idx.email.entry(k).or_insert_with(|| name.clone());
                }
            }
        }
        idx
    }

    /// Whether the index holds no resolvable handles.
    pub fn is_empty(&self) -> bool {
        self.phone.is_empty() && self.email.is_empty()
    }

    /// Resolve one handle (phone, email, or WhatsApp JID) to a contact name.
    pub fn resolve(&self, handle: &str) -> Option<&str> {
        let handle = handle.trim();
        if handle.is_empty() {
            return None;
        }
        if let Some(at) = handle.find('@') {
            let (local, domain) = (&handle[..at], &handle[at + 1..]);
            // A messaging JID (e.g. `123@s.whatsapp.net`, group `…@g.us`) carries
            // the number in its local part; everything else is an email.
            if domain.contains("whatsapp") || domain.contains("g.us") {
                return phone_key(local).and_then(|k| self.phone.get(&k).map(String::as_str));
            }
            return self.email.get(&handle.to_lowercase()).map(String::as_str);
        }
        phone_key(handle).and_then(|k| self.phone.get(&k).map(String::as_str))
    }
}

/// Fill `contact_name` on each call from its `number`, leaving already-set or
/// unresolved entries untouched.
pub fn enrich_calls(idx: &ContactIndex, calls: &mut [crate::calls::Call]) {
    for c in calls {
        if c.contact_name.is_empty()
            && let Some(name) = idx.resolve(&c.number)
        {
            c.contact_name = name.to_string();
        }
    }
}

/// Fill `contact_name` on each voicemail from its `sender`.
pub fn enrich_voicemail(idx: &ContactIndex, items: &mut [crate::voicemail::Voicemail]) {
    for v in items {
        if v.contact_name.is_empty()
            && let Some(name) = idx.resolve(&v.sender)
        {
            v.contact_name = name.to_string();
        }
    }
}

/// Fill `contact_name` on each WhatsApp message from its `sender` JID, falling
/// back to the chat peer's JID (`chat_jid`) when `sender` is empty — which it is
/// for the owner's own (`from_me`) messages, where the resolvable party is the
/// conversation peer.
pub fn enrich_whatsapp(idx: &ContactIndex, items: &mut [crate::whatsapp::WaMessage]) {
    for m in items {
        if !m.contact_name.is_empty() {
            continue;
        }
        let handle = if m.sender.is_empty() { m.chat_jid.as_str() } else { m.sender.as_str() };
        if let Some(name) = idx.resolve(handle) {
            m.contact_name = name.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{Contact, Labeled};

    fn contact(first: &str, last: &str, phones: &[&str], emails: &[&str]) -> Contact {
        Contact {
            first: first.into(),
            last: last.into(),
            organization: String::new(),
            phones: phones.iter().map(|p| Labeled { label: "mobile".into(), value: p.to_string() }).collect(),
            emails: emails.iter().map(|e| Labeled { label: "home".into(), value: e.to_string() }).collect(),
            note: String::new(),
            addresses: Vec::new(),
        }
    }

    #[test]
    fn resolves_phone_across_formatting_and_country_code() {
        let idx = ContactIndex::build(&[contact("Jan", "Novák", &["+420 123 456 789"], &[])]);
        assert_eq!(idx.resolve("00420123456789"), Some("Jan Novák"));
        assert_eq!(idx.resolve("123456789"), Some("Jan Novák"));
        assert_eq!(idx.resolve("+420 123 456 789"), Some("Jan Novák"));
        assert_eq!(idx.resolve("987654321"), None);
    }

    #[test]
    fn resolves_email_case_insensitively_and_whatsapp_jid() {
        let idx = ContactIndex::build(&[contact("Eva", "Malá", &["776112233"], &["Eva@Example.COM"])]);
        assert_eq!(idx.resolve("eva@example.com"), Some("Eva Malá"));
        // WhatsApp JID resolves on its numeric local part.
        assert_eq!(idx.resolve("420776112233@s.whatsapp.net"), Some("Eva Malá"));
        // A non-messaging email that isn't in the book stays unresolved.
        assert_eq!(idx.resolve("nobody@example.com"), None);
    }

    #[test]
    fn enrich_whatsapp_uses_sender_or_chat_jid_fallback() {
        let idx = ContactIndex::build(&[contact("Eva", "Malá", &["776112233"], &[])]);
        let wa = |from_me: bool, sender: &str, chat_jid: &str| crate::whatsapp::WaMessage {
            chat: "Eva".into(),
            chat_jid: chat_jid.into(),
            sender: sender.into(),
            from_me,
            date: String::new(),
            text: String::new(),
            source_path: String::new(),
            media_file: None,
            contact_name: String::new(),
        };
        let mut msgs = vec![
            // Incoming: resolved from the sender JID.
            wa(false, "420776112233@s.whatsapp.net", "420776112233@s.whatsapp.net"),
            // Outgoing (own): sender empty, resolved from the chat peer's JID.
            wa(true, "", "420776112233@s.whatsapp.net"),
        ];
        enrich_whatsapp(&idx, &mut msgs);
        assert_eq!(msgs[0].contact_name, "Eva Malá");
        assert_eq!(msgs[1].contact_name, "Eva Malá");
    }

    #[test]
    fn organization_fallback_and_empty_names_skipped() {
        let mut org = contact("", "", &["111222333"], &[]);
        org.organization = "Acme s.r.o.".into();
        let nameless = contact("", "", &["444555666"], &[]); // no name, no org → skipped
        let idx = ContactIndex::build(&[org, nameless]);
        assert_eq!(idx.resolve("111222333"), Some("Acme s.r.o."));
        assert_eq!(idx.resolve("444555666"), None);
    }

    #[test]
    fn short_numbers_and_empty_handles_never_match() {
        let idx = ContactIndex::build(&[contact("X", "Y", &["12345"], &[])]); // too short to index
        assert!(idx.is_empty());
        assert_eq!(idx.resolve(""), None);
        assert_eq!(idx.resolve("123"), None);
    }

    #[test]
    fn enrich_calls_sets_names_best_effort() {
        let idx = ContactIndex::build(&[contact("Jan", "Novák", &["123456789"], &[])]);
        let mut calls = vec![
            crate::calls::Call {
                number: "+420123456789".into(),
                date: String::new(),
                duration_seconds: 0,
                direction: "incoming".into(),
                answered: true,
                service: "phone".into(),
                video: None,
                call_type: None,
                location: None,
                country: None,
                contact_name: String::new(),
            },
            crate::calls::Call {
                number: "555000111".into(),
                date: String::new(),
                duration_seconds: 0,
                direction: "outgoing".into(),
                answered: false,
                service: "phone".into(),
                video: None,
                call_type: None,
                location: None,
                country: None,
                contact_name: String::new(),
            },
        ];
        enrich_calls(&idx, &mut calls);
        assert_eq!(calls[0].contact_name, "Jan Novák");
        assert_eq!(calls[1].contact_name, ""); // unknown number stays empty
    }
}
