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
        out.push_str(&format!("N:{};{};;;\r\n", c.last, c.first));
        let full_name = format!("{} {}", c.first, c.last);
        out.push_str(&format!("FN:{}\r\n", full_name.trim()));
        if !c.organization.is_empty() {
            out.push_str(&format!("ORG:{}\r\n", c.organization));
        }
        for p in &c.phones {
            out.push_str(&format!("TEL;TYPE={}:{}\r\n", p.label, p.value));
        }
        for e in &c.emails {
            out.push_str(&format!("EMAIL;TYPE={}:{}\r\n", e.label, e.value));
        }
        if !c.note.is_empty() {
            out.push_str(&format!("NOTE:{}\r\n", c.note));
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
}
