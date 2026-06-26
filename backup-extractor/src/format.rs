//! Render contacts to the supported output formats.

use askama::Template;

use crate::contacts::Contact;

/// Output format chosen with `-f`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Csv,
    Json,
    Vcf,
    Html,
}

impl Format {
    pub fn from_cli(value: &str) -> Option<Format> {
        match value.to_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "vcf" | "vcard" => Some(Format::Vcf),
            "html" => Some(Format::Html),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Vcf => "vcf",
            Format::Html => "html",
        }
    }
}

fn join_labeled(items: &[crate::contacts::Labeled]) -> String {
    items
        .iter()
        .map(|l| format!("{}: {}", l.label, l.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Escape a vCard 3.0 (RFC 2426) text VALUE: backslash, semicolon, comma, newlines.
/// Prevents malformed cards or property injection from arbitrary contact data.
fn vcard_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\r', "")
        .replace('\n', "\\n")
}

/// Sanitize a vCard PARAMETER value (e.g. a TYPE label): drop characters that
/// would break the parameter or inject structure.
fn vcard_param(value: &str) -> String {
    value
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n' | ';' | ':' | ',' | '"' | '\\'))
        .collect()
}

pub fn contacts_csv(contacts: &[Contact]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["first", "last", "organization", "phones", "emails", "note"])
        .unwrap();
    for c in contacts {
        wtr.write_record([
            &c.first,
            &c.last,
            &c.organization,
            &join_labeled(&c.phones),
            &join_labeled(&c.emails),
            &c.note,
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn contacts_json(contacts: &[Contact]) -> String {
    serde_json::to_string_pretty(contacts).unwrap()
}

pub fn contacts_vcard(contacts: &[Contact]) -> String {
    let mut out = String::new();
    for c in contacts {
        out.push_str("BEGIN:VCARD\r\nVERSION:3.0\r\n");
        out.push_str(&format!("N:{};{};;;\r\n", vcard_escape(&c.last), vcard_escape(&c.first)));
        let full_name = format!("{} {}", c.first, c.last);
        let full_name = full_name.trim();
        let fn_value = if full_name.is_empty() { c.organization.as_str() } else { full_name };
        out.push_str(&format!("FN:{}\r\n", vcard_escape(fn_value)));
        if !c.organization.is_empty() {
            out.push_str(&format!("ORG:{}\r\n", vcard_escape(&c.organization)));
        }
        for p in &c.phones {
            out.push_str(&format!("TEL;TYPE={}:{}\r\n", vcard_param(&p.label), vcard_escape(&p.value)));
        }
        for e in &c.emails {
            out.push_str(&format!("EMAIL;TYPE={}:{}\r\n", vcard_param(&e.label), vcard_escape(&e.value)));
        }
        if !c.note.is_empty() {
            out.push_str(&format!("NOTE:{}\r\n", vcard_escape(&c.note)));
        }
        out.push_str("END:VCARD\r\n");
    }
    out
}

#[derive(Template)]
#[template(path = "contacts.html")]
struct ContactsTemplate<'a> {
    contacts: &'a [Contact],
}

pub fn contacts_html(contacts: &[Contact]) -> String {
    ContactsTemplate { contacts }.render().unwrap()
}

pub fn calls_csv(calls: &[crate::calls::Call]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "number", "date", "duration_seconds", "direction", "answered", "service",
        "video", "call_type", "location", "country",
    ])
    .unwrap();
    for c in calls {
        wtr.write_record([
            c.number.clone(),
            c.date.clone(),
            c.duration_seconds.to_string(),
            c.direction.clone(),
            c.answered.to_string(),
            c.service.clone(),
            c.video.map(|v| v.to_string()).unwrap_or_default(),
            c.call_type.map(|v| v.to_string()).unwrap_or_default(),
            c.location.clone().unwrap_or_default(),
            c.country.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn calls_json(calls: &[crate::calls::Call]) -> String {
    serde_json::to_string_pretty(calls).unwrap()
}

#[derive(Template)]
#[template(path = "calls.html")]
struct CallsTemplate<'a> {
    calls: &'a [crate::calls::Call],
}

pub fn calls_html(calls: &[crate::calls::Call]) -> String {
    CallsTemplate { calls }.render().unwrap()
}

#[allow(dead_code)]
pub fn voicemail_csv(items: &[crate::voicemail::Voicemail]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "sender", "date", "duration_seconds", "trashed", "trashed_at", "expiration", "flags",
    ])
    .unwrap();
    for v in items {
        wtr.write_record([
            v.sender.clone(),
            v.date.clone(),
            v.duration_seconds.to_string(),
            v.trashed.to_string(),
            v.trashed_at.clone().unwrap_or_default(),
            v.expiration.clone().unwrap_or_default(),
            v.flags.to_string(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

#[allow(dead_code)]
pub fn voicemail_json(items: &[crate::voicemail::Voicemail]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "voicemail.html")]
struct VoicemailTemplate<'a> {
    voicemails: &'a [crate::voicemail::Voicemail],
}

#[allow(dead_code)]
pub fn voicemail_html(items: &[crate::voicemail::Voicemail]) -> String {
    VoicemailTemplate { voicemails: items }.render().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contacts::{Contact, Labeled};

    fn sample() -> Vec<Contact> {
        vec![Contact {
            first: "Jan".into(),
            last: "Novák".into(),
            organization: "Acme".into(),
            phones: vec![Labeled { label: "Mobile".into(), value: "+420776452878".into() }],
            emails: vec![Labeled { label: "Home".into(), value: "jan@example.cz".into() }],
            note: "kamarád".into(),
        }]
    }

    #[test]
    fn format_from_cli_parses_known() {
        assert_eq!(Format::from_cli("csv"), Some(Format::Csv));
        assert_eq!(Format::from_cli("VCF"), Some(Format::Vcf));
        assert_eq!(Format::from_cli("json"), Some(Format::Json));
        assert_eq!(Format::from_cli("html"), Some(Format::Html));
        assert_eq!(Format::from_cli("vcard"), Some(Format::Vcf)); // lowercase alias
        assert_eq!(Format::from_cli("nope"), None);
    }

    #[test]
    fn csv_has_header_and_row() {
        let out = contacts_csv(&sample());
        assert!(out.starts_with("first,last,organization,phones,emails,note"));
        assert!(out.contains("Jan,Novák,Acme,Mobile: +420776452878,Home: jan@example.cz,kamarád"));
    }

    #[test]
    fn json_roundtrips() {
        let out = contacts_json(&sample());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["first"], "Jan");
        assert_eq!(back[0]["phones"][0]["value"], "+420776452878");
    }

    #[test]
    fn vcard_is_wellformed() {
        let out = contacts_vcard(&sample());
        assert!(out.contains("BEGIN:VCARD"));
        assert!(out.contains("BEGIN:VCARD\r\n"));
        assert!(out.contains("VERSION:3.0"));
        assert!(out.contains("FN:Jan Novák"));
        assert!(out.contains("N:Novák;Jan;;;"));
        assert!(out.contains("ORG:Acme"));
        assert!(out.contains("TEL;TYPE=Mobile:+420776452878"));
        assert!(out.contains("EMAIL;TYPE=Home:jan@example.cz"));
        assert!(out.trim_end().ends_with("END:VCARD"));
    }

    #[test]
    fn html_lists_the_contact() {
        let out = contacts_html(&sample());
        assert!(out.contains("<html"));
        assert!(out.contains("Jan Novák"));
        assert!(out.contains("+420776452878"));
    }

    #[test]
    fn vcard_escapes_special_chars_and_resists_injection() {
        let contacts = vec![Contact {
            first: "A;B,C".into(),
            last: "D\\E".into(),
            organization: "Org\nInjected".into(),
            phones: vec![Labeled { label: "Mobile".into(), value: "123".into() }],
            emails: vec![],
            note: "line1\nTEL:evil@inject".into(),
        }];
        let out = contacts_vcard(&contacts);
        assert!(out.contains("N:D\\\\E;A\\;B\\,C;;;"));
        assert!(out.contains("ORG:Org\\nInjected"));
        assert!(out.contains("NOTE:line1\\nTEL:evil@inject"));
        // The injected newline did NOT create a real standalone property line.
        assert!(!out.contains("\nTEL:evil@inject"));
    }

    fn sample_voicemails() -> Vec<crate::voicemail::Voicemail> {
        vec![crate::voicemail::Voicemail {
            sender: "+420776452878".into(),
            date: "2020-09-13T12:26:40+00:00".into(),
            duration_seconds: 30,
            trashed: false,
            trashed_at: None,
            expiration: None,
            flags: 0,
        }]
    }

    #[test]
    fn voicemail_csv_has_header_and_row() {
        let out = voicemail_csv(&sample_voicemails());
        assert!(out.starts_with(
            "sender,date,duration_seconds,trashed,trashed_at,expiration,flags"
        ));
        assert!(out.contains("+420776452878,2020-09-13T12:26:40+00:00,30,false,,,0"));
    }

    #[test]
    fn voicemail_json_roundtrips() {
        let out = voicemail_json(&sample_voicemails());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["sender"], "+420776452878");
        assert_eq!(back[0]["trashed"], false);
    }

    #[test]
    fn voicemail_html_lists_items() {
        let out = voicemail_html(&sample_voicemails());
        assert!(out.contains("<html"));
        assert!(out.contains("+420776452878"));
    }

    fn sample_calls() -> Vec<crate::calls::Call> {
        vec![crate::calls::Call {
            number: "+420776452878".into(),
            date: "2026-06-20T14:33:05+00:00".into(),
            duration_seconds: 42,
            direction: "outgoing".into(),
            answered: true,
            service: "phone".into(),
            video: None,
            call_type: Some(1),
            location: None,
            country: Some("CZ".into()),
        }]
    }

    #[test]
    fn calls_csv_has_header_and_row() {
        let out = calls_csv(&sample_calls());
        assert!(out.starts_with(
            "number,date,duration_seconds,direction,answered,service,video,call_type,location,country"
        ));
        assert!(out.contains("+420776452878"));
        assert!(out.contains(",outgoing,true,phone,,1,,CZ"));
    }

    #[test]
    fn calls_json_roundtrips() {
        let out = calls_json(&sample_calls());
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["number"], "+420776452878");
        assert_eq!(back[0]["direction"], "outgoing");
        assert_eq!(back[0]["country"], "CZ");
    }

    #[test]
    fn calls_html_lists_calls() {
        let out = calls_html(&sample_calls());
        assert!(out.contains("<html"));
        assert!(out.contains("+420776452878"));
        assert!(out.contains("outgoing"));
    }

    #[test]
    fn calls_html_escapes_html_in_data() {
        let mut calls = sample_calls();
        calls[0].number = "<script>alert(1)</script>".into();
        let out = calls_html(&calls);
        // askama's HTML escaper emits numeric character references.
        assert!(out.contains("&#60;script&#62;"));
        assert!(!out.contains("<script>alert"));
    }

    #[test]
    fn vcard_fn_falls_back_to_organization() {
        let contacts = vec![Contact {
            first: String::new(),
            last: String::new(),
            organization: "Firma s.r.o.".into(),
            phones: vec![],
            emails: vec![],
            note: String::new(),
        }];
        let out = contacts_vcard(&contacts);
        assert!(out.contains("FN:Firma s.r.o."));
        assert!(!out.contains("FN:\r\n"));
    }
}
