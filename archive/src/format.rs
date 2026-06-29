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
    /// PDF rendered from the same HTML the `Html` variant produces.
    Pdf,
}

impl Format {
    pub fn from_cli(value: &str) -> Option<Format> {
        match value.to_lowercase().as_str() {
            "csv" => Some(Format::Csv),
            "json" => Some(Format::Json),
            "vcf" | "vcard" => Some(Format::Vcf),
            "html" => Some(Format::Html),
            "pdf" => Some(Format::Pdf),
            _ => None,
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Vcf => "vcf",
            Format::Html => "html",
            Format::Pdf => "pdf",
        }
    }
}

fn join_addresses(addresses: &[crate::contacts::Address]) -> String {
    addresses
        .iter()
        .map(|a| {
            let line = a.display_line();
            if a.label.is_empty() { line } else { format!("{}: {}", a.label, line) }
        })
        .collect::<Vec<_>>()
        .join("; ")
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
    wtr.write_record(["first", "last", "organization", "phones", "emails", "addresses", "note"])
        .unwrap();
    for c in contacts {
        wtr.write_record([
            &c.first,
            &c.last,
            &c.organization,
            &join_labeled(&c.phones),
            &join_labeled(&c.emails),
            &join_addresses(&c.addresses),
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
        for a in &c.addresses {
            out.push_str(&format!(
                "ADR;TYPE={}:;;{};{};{};{};{}\r\n",
                vcard_param(&a.label),
                vcard_escape(&a.street),
                vcard_escape(&a.city),
                vcard_escape(&a.state),
                vcard_escape(&a.zip),
                vcard_escape(&a.country),
            ));
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

pub fn accounts_csv(items: &[crate::accounts::Account]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["account_type", "type_identifier", "description", "username", "bundle_id", "date", "active"])
        .unwrap();
    for a in items {
        wtr.write_record([
            a.account_type.clone(),
            a.type_identifier.clone(),
            a.description.clone(),
            a.username.clone(),
            a.bundle_id.clone(),
            a.date.clone(),
            a.active.map(|v| v.to_string()).unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn accounts_json(items: &[crate::accounts::Account]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "accounts.html")]
struct AccountsTemplate<'a> {
    accounts: &'a [crate::accounts::Account],
}

pub fn accounts_html(items: &[crate::accounts::Account]) -> String {
    AccountsTemplate { accounts: items }.render().unwrap()
}

pub fn known_networks_csv(items: &[crate::known_networks::KnownNetwork]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["ssid", "bssid", "last_joined", "hidden", "security"]).unwrap();
    for n in items {
        wtr.write_record([
            n.ssid.clone(),
            n.bssid.clone(),
            n.last_joined.clone(),
            n.hidden.map(|v| v.to_string()).unwrap_or_default(),
            n.security.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn known_networks_json(items: &[crate::known_networks::KnownNetwork]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "known-networks.html")]
struct KnownNetworksTemplate<'a> {
    networks: &'a [crate::known_networks::KnownNetwork],
}

pub fn known_networks_html(items: &[crate::known_networks::KnownNetwork]) -> String {
    KnownNetworksTemplate { networks: items }.render().unwrap()
}

pub fn voicemail_csv(items: &[crate::voicemail::Voicemail]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "sender", "date", "duration_seconds", "trashed", "trashed_at", "expiration", "flags",
        "audio_file",
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
            v.audio_file.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn voicemail_json(items: &[crate::voicemail::Voicemail]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "voicemail.html")]
struct VoicemailTemplate<'a> {
    voicemails: &'a [crate::voicemail::Voicemail],
}

pub fn voicemail_html(items: &[crate::voicemail::Voicemail]) -> String {
    VoicemailTemplate { voicemails: items }.render().unwrap()
}

pub fn voice_memos_csv(items: &[crate::voice_memos::VoiceMemo]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["title", "date", "duration_seconds", "source_file", "audio_file"])
        .unwrap();
    for v in items {
        wtr.write_record([
            v.title.clone(),
            v.date.clone(),
            v.duration_seconds.to_string(),
            v.source_file.clone(),
            v.audio_file.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn voice_memos_json(items: &[crate::voice_memos::VoiceMemo]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "voice-memos.html")]
struct VoiceMemosTemplate<'a> {
    memos: &'a [crate::voice_memos::VoiceMemo],
}

pub fn voice_memos_html(items: &[crate::voice_memos::VoiceMemo]) -> String {
    VoiceMemosTemplate { memos: items }.render().unwrap()
}

pub fn safari_history_csv(items: &[crate::safari::HistoryVisit]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["url", "title", "date", "visit_count"]).unwrap();
    for v in items {
        wtr.write_record([v.url.clone(), v.title.clone(), v.date.clone(), v.visit_count.to_string()])
            .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn safari_history_json(items: &[crate::safari::HistoryVisit]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "safari-history.html")]
struct SafariHistoryTemplate<'a> {
    visits: &'a [crate::safari::HistoryVisit],
}

pub fn safari_history_html(items: &[crate::safari::HistoryVisit]) -> String {
    SafariHistoryTemplate { visits: items }.render().unwrap()
}

pub fn safari_bookmarks_csv(items: &[crate::safari::Bookmark]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["title", "url", "folder"]).unwrap();
    for b in items {
        wtr.write_record([b.title.clone(), b.url.clone(), b.folder.clone()]).unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn safari_bookmarks_json(items: &[crate::safari::Bookmark]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "safari-bookmarks.html")]
struct SafariBookmarksTemplate<'a> {
    bookmarks: &'a [crate::safari::Bookmark],
}

pub fn safari_bookmarks_html(items: &[crate::safari::Bookmark]) -> String {
    SafariBookmarksTemplate { bookmarks: items }.render().unwrap()
}

pub fn calendar_csv(items: &[crate::calendar::CalendarEvent]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["summary", "start", "end", "all_day", "calendar"]).unwrap();
    for e in items {
        wtr.write_record([
            e.summary.clone(),
            e.start.clone(),
            e.end.clone(),
            e.all_day.to_string(),
            e.calendar.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn calendar_json(items: &[crate::calendar::CalendarEvent]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "calendar.html")]
struct CalendarTemplate<'a> {
    events: &'a [crate::calendar::CalendarEvent],
}

pub fn calendar_html(items: &[crate::calendar::CalendarEvent]) -> String {
    CalendarTemplate { events: items }.render().unwrap()
}

pub fn notes_csv(items: &[crate::notes::Note]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["title", "folder", "created", "modified", "body_source", "body"]).unwrap();
    for n in items {
        wtr.write_record([
            n.title.clone(),
            n.folder.clone(),
            n.created.clone(),
            n.modified.clone(),
            n.body_source.clone(),
            n.body.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn notes_json(items: &[crate::notes::Note]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "notes.html")]
struct NotesTemplate<'a> {
    notes: &'a [crate::notes::Note],
}

pub fn notes_html(items: &[crate::notes::Note]) -> String {
    NotesTemplate { notes: items }.render().unwrap()
}

fn opt_num<T: ToString>(v: Option<T>) -> String {
    v.map(|x| x.to_string()).unwrap_or_default()
}

pub fn photos_csv(items: &[crate::photos::Photo]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "filename", "kind", "created", "favorite", "trashed", "width", "height",
        "latitude", "longitude", "duration_seconds", "file",
        "hidden", "edited", "live_photo", "modified", "added",
        "original_filename", "title", "burst_id", "albums",
    ])
    .unwrap();
    for p in items {
        wtr.write_record([
            p.filename.clone(),
            p.kind.clone(),
            p.created.clone(),
            p.favorite.to_string(),
            p.trashed.to_string(),
            p.width.to_string(),
            p.height.to_string(),
            opt_num(p.latitude),
            opt_num(p.longitude),
            opt_num(p.duration_seconds),
            p.file.clone().unwrap_or_default(),
            p.hidden.to_string(),
            p.edited.to_string(),
            p.live_photo.to_string(),
            p.modified.clone(),
            p.added.clone(),
            p.original_filename.clone(),
            p.title.clone(),
            p.burst_id.clone().unwrap_or_default(),
            p.albums.join("; "),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn photos_json(items: &[crate::photos::Photo]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "photos.html")]
struct PhotosTemplate<'a> {
    photos: &'a [crate::photos::Photo],
}

pub fn photos_html(items: &[crate::photos::Photo]) -> String {
    PhotosTemplate { photos: items }.render().unwrap()
}

pub fn attachments_csv(items: &[crate::attachments::Attachment]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["name", "mime_type", "created", "total_bytes", "file"]).unwrap();
    for a in items {
        wtr.write_record([
            a.name.clone(),
            a.mime_type.clone(),
            a.created.clone(),
            a.total_bytes.to_string(),
            a.file.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn attachments_json(items: &[crate::attachments::Attachment]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "attachments.html")]
struct AttachmentsTemplate<'a> {
    attachments: &'a [crate::attachments::Attachment],
}

pub fn attachments_html(items: &[crate::attachments::Attachment]) -> String {
    AttachmentsTemplate { attachments: items }.render().unwrap()
}

pub fn whatsapp_csv(items: &[crate::whatsapp::WaMessage]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["chat", "sender", "from_me", "date", "text", "media_file"]).unwrap();
    for m in items {
        wtr.write_record([
            m.chat.clone(),
            m.sender.clone(),
            m.from_me.to_string(),
            m.date.clone(),
            m.text.clone(),
            m.media_file.clone().unwrap_or_default(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn whatsapp_json(items: &[crate::whatsapp::WaMessage]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "whatsapp.html")]
struct WhatsappTemplate<'a> {
    messages: &'a [crate::whatsapp::WaMessage],
}

pub fn whatsapp_html(items: &[crate::whatsapp::WaMessage]) -> String {
    WhatsappTemplate { messages: items }.render().unwrap()
}

// --- Health ---------------------------------------------------------------

pub fn health_workouts_csv(items: &[crate::health::Workout]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "activity_type", "activity_type_id", "start", "end",
        "duration_seconds", "total_distance", "total_energy_burned",
    ])
    .unwrap();
    for w in items {
        wtr.write_record([
            w.activity_type.clone().unwrap_or_default(),
            opt_num(w.activity_type_id),
            w.start.clone(),
            w.end.clone(),
            opt_num(w.duration_seconds),
            opt_num(w.total_distance),
            opt_num(w.total_energy_burned),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn health_quantity_csv(items: &[crate::health::QuantitySummary]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["name", "data_type_id", "count", "sum", "min", "avg", "max", "first", "last"])
        .unwrap();
    for q in items {
        wtr.write_record([
            q.name.clone(),
            q.data_type_id.to_string(),
            q.count.to_string(),
            opt_num(q.sum),
            opt_num(q.min),
            opt_num(q.avg),
            opt_num(q.max),
            q.first.clone(),
            q.last.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn health_json(data: &crate::health::HealthData) -> String {
    serde_json::to_string_pretty(data).unwrap()
}

#[derive(Template)]
#[template(path = "health.html")]
struct HealthTemplate<'a> {
    data: &'a crate::health::HealthData,
}

pub fn health_html(data: &crate::health::HealthData) -> String {
    HealthTemplate { data }.render().unwrap()
}

// --- Reminders ------------------------------------------------------------

pub fn reminders_csv(items: &[crate::reminders::Reminder]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record([
        "list", "title", "notes", "due", "completed", "completed_date", "priority", "created", "flagged",
    ])
    .unwrap();
    for r in items {
        wtr.write_record([
            r.list.clone(),
            r.title.clone(),
            r.notes.clone(),
            r.due.clone().unwrap_or_default(),
            r.completed.to_string(),
            r.completed_date.clone().unwrap_or_default(),
            r.priority.to_string(),
            r.created.clone().unwrap_or_default(),
            r.flagged.to_string(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn reminders_json(items: &[crate::reminders::Reminder]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "reminders.html")]
struct RemindersTemplate<'a> {
    reminders: &'a [crate::reminders::Reminder],
}

pub fn reminders_html(items: &[crate::reminders::Reminder]) -> String {
    RemindersTemplate { reminders: items }.render().unwrap()
}

// --- Mail -----------------------------------------------------------------

pub fn mail_csv(items: &[crate::mail::MailMessage]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["date", "from", "to", "subject", "snippet"]).unwrap();
    for m in items {
        wtr.write_record([
            m.date.clone().unwrap_or_default(),
            m.from.clone(),
            m.to.clone(),
            m.subject.clone(),
            m.snippet.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn mail_json(items: &[crate::mail::MailMessage]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "mail.html")]
struct MailTemplate<'a> {
    messages: &'a [crate::mail::MailMessage],
}

pub fn mail_html(items: &[crate::mail::MailMessage]) -> String {
    MailTemplate { messages: items }.render().unwrap()
}

// --- Installed apps -------------------------------------------------------

pub fn apps_csv(items: &[String]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["bundle_id"]).unwrap();
    for id in items {
        wtr.write_record([id]).unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn apps_json(items: &[String]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "apps.html")]
struct AppsTemplate<'a> {
    apps: &'a [String],
}

pub fn apps_html(items: &[String]) -> String {
    AppsTemplate { apps: items }.render().unwrap()
}

// --- Timeline -------------------------------------------------------------

pub fn timeline_csv(items: &[crate::timeline::Event]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["timestamp", "kind", "summary"]).unwrap();
    for e in items {
        wtr.write_record([e.timestamp.clone(), e.kind.clone(), e.summary.clone()]).unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn timeline_json(items: &[crate::timeline::Event]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "timeline.html")]
struct TimelineTemplate<'a> {
    events: &'a [crate::timeline::Event],
}

pub fn timeline_html(items: &[crate::timeline::Event]) -> String {
    TimelineTemplate { events: items }.render().unwrap()
}

// --- Recovered deleted records --------------------------------------------

pub fn deleted_csv(items: &[crate::recover_deleted::DeletedRecord]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["store", "source", "rowid", "date", "summary"]).unwrap();
    for d in items {
        wtr.write_record([
            d.store.clone(),
            d.source.clone(),
            d.rowid.map(|r| r.to_string()).unwrap_or_default(),
            d.date.clone().unwrap_or_default(),
            d.summary.clone(),
        ])
        .unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn deleted_json(items: &[crate::recover_deleted::DeletedRecord]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "deleted.html")]
struct DeletedTemplate<'a> {
    records: &'a [crate::recover_deleted::DeletedRecord],
}

pub fn deleted_html(items: &[crate::recover_deleted::DeletedRecord]) -> String {
    DeletedTemplate { records: items }.render().unwrap()
}

// --- Wi-Fi credentials ----------------------------------------------------

pub fn wifi_csv(items: &[archive_core::keychain::WifiCredential]) -> String {
    let mut wtr = csv::Writer::from_writer(Vec::new());
    wtr.write_record(["ssid", "password"]).unwrap();
    for w in items {
        wtr.write_record([w.ssid.clone(), w.password.clone()]).unwrap();
    }
    String::from_utf8(wtr.into_inner().unwrap()).unwrap()
}

pub fn wifi_json(items: &[archive_core::keychain::WifiCredential]) -> String {
    serde_json::to_string_pretty(items).unwrap()
}

#[derive(Template)]
#[template(path = "wifi.html")]
struct WifiTemplate<'a> {
    networks: &'a [archive_core::keychain::WifiCredential],
}

pub fn wifi_html(items: &[archive_core::keychain::WifiCredential]) -> String {
    WifiTemplate { networks: items }.render().unwrap()
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
            addresses: vec![],
        }]
    }

    #[test]
    fn format_from_cli_parses_known() {
        assert_eq!(Format::from_cli("csv"), Some(Format::Csv));
        assert_eq!(Format::from_cli("VCF"), Some(Format::Vcf));
        assert_eq!(Format::from_cli("json"), Some(Format::Json));
        assert_eq!(Format::from_cli("html"), Some(Format::Html));
        assert_eq!(Format::from_cli("PDF"), Some(Format::Pdf));
        assert_eq!(Format::from_cli("vcard"), Some(Format::Vcf)); // lowercase alias
        assert_eq!(Format::from_cli("nope"), None);
        assert_eq!(Format::Pdf.extension(), "pdf");
    }

    #[test]
    fn csv_has_header_and_row() {
        let out = contacts_csv(&sample());
        assert!(out.starts_with("first,last,organization,phones,emails,addresses,note"));
        assert!(out.contains("Jan,Novák,Acme,Mobile: +420776452878,Home: jan@example.cz,,kamarád"));
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
            addresses: vec![],
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
            rowid: 3,
            sender: "+420776452878".into(),
            date: "2020-09-13T12:26:40+00:00".into(),
            duration_seconds: 30,
            trashed: false,
            trashed_at: None,
            expiration: None,
            flags: 0,
            audio_file: None,
        }]
    }

    #[test]
    fn voicemail_csv_has_header_and_row() {
        let out = voicemail_csv(&sample_voicemails());
        assert!(out.starts_with(
            "sender,date,duration_seconds,trashed,trashed_at,expiration,flags,audio_file"
        ));
        // The sample has no audio, so the row ends with an empty audio_file cell.
        assert!(out.contains("+420776452878,2020-09-13T12:26:40+00:00,30,false,,,0,"));
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

    #[test]
    fn voicemail_html_renders_audio_player_when_present() {
        let mut items = sample_voicemails();
        items[0].audio_file = Some("voicemail_audio/2020-09-13_122640_+420_3.m4a".into());
        let out = voicemail_html(&items);
        assert!(out.contains(
            "<audio controls src=\"voicemail_audio/2020-09-13_122640_+420_3.m4a\"></audio>"
        ));
    }

    #[test]
    fn voicemail_html_escapes_audio_file_attribute() {
        // A crafted audio_file must not break out of the src attribute.
        let mut items = sample_voicemails();
        items[0].audio_file = Some("\"><script>alert(1)</script>".into());
        let out = voicemail_html(&items);
        assert!(!out.contains("\"><script>"));
        // askama 0.16 escapes <, >, " as numeric entities.
        assert!(out.contains("&#60;script&#62;"));
    }

    #[test]
    fn voicemail_json_includes_rowid_and_audio_file() {
        let mut items = sample_voicemails();
        items[0].audio_file = Some("voicemail_audio/x_3.amr".into());
        let out = voicemail_json(&items);
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["rowid"], 3);
        assert_eq!(back[0]["audio_file"], "voicemail_audio/x_3.amr");
    }

    fn sample_voice_memos() -> Vec<crate::voice_memos::VoiceMemo> {
        vec![crate::voice_memos::VoiceMemo {
            title: "Schůzka".into(),
            date: "2020-01-06T10:40:00+00:00".into(),
            duration_seconds: 13,
            source_file: "A1B2C3.m4a".into(),
            audio_file: None,
        }]
    }

    #[test]
    fn voice_memos_csv_has_header_and_row() {
        let out = voice_memos_csv(&sample_voice_memos());
        assert!(out.starts_with("title,date,duration_seconds,source_file,audio_file"));
        // No audio extracted → trailing empty audio_file cell.
        assert!(out.contains("Schůzka,2020-01-06T10:40:00+00:00,13,A1B2C3.m4a,"));
    }

    #[test]
    fn voice_memos_json_roundtrips() {
        let mut items = sample_voice_memos();
        items[0].audio_file = Some("voice_memos/2020-01-06_104000_Schuzka_1.m4a".into());
        let out = voice_memos_json(&items);
        let back: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(back[0]["title"], "Schůzka");
        assert_eq!(back[0]["audio_file"], "voice_memos/2020-01-06_104000_Schuzka_1.m4a");
    }

    #[test]
    fn voice_memos_html_renders_audio_player_when_present() {
        let mut items = sample_voice_memos();
        items[0].audio_file = Some("voice_memos/2020-01-06_104000_Schuzka_1.m4a".into());
        let out = voice_memos_html(&items);
        assert!(out.contains("<html"));
        assert!(out.contains(
            "<audio controls src=\"voice_memos/2020-01-06_104000_Schuzka_1.m4a\"></audio>"
        ));
    }

    #[test]
    fn voice_memos_html_escapes_audio_file_attribute() {
        let mut items = sample_voice_memos();
        items[0].audio_file = Some("\"><script>alert(1)</script>".into());
        let out = voice_memos_html(&items);
        assert!(!out.contains("\"><script>"));
        assert!(out.contains("&#60;script&#62;"));
    }

    #[test]
    fn safari_history_csv_json_and_html_escape() {
        let v = vec![crate::safari::HistoryVisit {
            url: "https://apple.com".into(),
            title: "Apple".into(),
            date: "2020-01-06T10:40:00+00:00".into(),
            visit_count: 5,
        }];
        let csv = safari_history_csv(&v);
        assert!(csv.starts_with("url,title,date,visit_count"));
        assert!(csv.contains("https://apple.com,Apple,2020-01-06T10:40:00+00:00,5"));
        let back: serde_json::Value = serde_json::from_str(&safari_history_json(&v)).unwrap();
        assert_eq!(back[0]["visit_count"], 5);

        let mut x = v.clone();
        x[0].title = "<script>alert(1)</script>".into();
        let html = safari_history_html(&x);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn safari_bookmarks_csv_and_json() {
        let b = vec![crate::safari::Bookmark {
            title: "Apple".into(),
            url: "https://apple.com".into(),
            folder: "Favorites".into(),
        }];
        assert!(safari_bookmarks_csv(&b).starts_with("title,url,folder"));
        let back: serde_json::Value = serde_json::from_str(&safari_bookmarks_json(&b)).unwrap();
        assert_eq!(back[0]["folder"], "Favorites");
        assert!(safari_bookmarks_html(&b).contains("https://apple.com"));

        let mut x = b.clone();
        x[0].title = "<script>alert(1)</script>".into();
        let html = safari_bookmarks_html(&x);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn calendar_csv_json_and_html() {
        let e = vec![crate::calendar::CalendarEvent {
            summary: "Standup".into(),
            start: "2020-01-06T10:40:00+00:00".into(),
            end: "2020-01-06T11:10:00+00:00".into(),
            all_day: false,
            calendar: "Work".into(),
        }];
        let csv = calendar_csv(&e);
        assert!(csv.starts_with("summary,start,end,all_day,calendar"));
        assert!(csv.contains("Standup,2020-01-06T10:40:00+00:00,2020-01-06T11:10:00+00:00,false,Work"));
        let back: serde_json::Value = serde_json::from_str(&calendar_json(&e)).unwrap();
        assert_eq!(back[0]["all_day"], false);
        assert!(calendar_html(&e).contains("Standup"));

        let mut x = e.clone();
        x[0].summary = "<script>alert(1)</script>".into();
        let html = calendar_html(&x);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn notes_csv_json_and_html_escape() {
        let n = vec![crate::notes::Note {
            title: "Nákup".into(),
            folder: "Práce".into(),
            created: "2020-01-06T10:40:00+00:00".into(),
            modified: "2020-01-06T10:48:20+00:00".into(),
            body: "mléko\nchléb".into(),
            body_source: "decoded".into(),
        }];
        let csv = notes_csv(&n);
        assert!(csv.starts_with("title,folder,created,modified,body_source,body"));
        let back: serde_json::Value = serde_json::from_str(&notes_json(&n)).unwrap();
        assert_eq!(back[0]["body_source"], "decoded");
        assert_eq!(back[0]["body"], "mléko\nchléb");

        let mut x = n.clone();
        x[0].body = "<script>alert(1)</script>".into();
        let html = notes_html(&x);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    fn sample_photo() -> crate::photos::Photo {
        crate::photos::Photo {
            filename: "IMG_0001.HEIC".into(),
            kind: "image".into(),
            created: "2020-01-06T10:40:00+00:00".into(),
            modified: "2020-01-06T10:40:50+00:00".into(),
            added: "2020-01-06T10:40:10+00:00".into(),
            favorite: true,
            hidden: false,
            trashed: false,
            edited: true,
            live_photo: true,
            kind_subtype: Some(2),
            width: 4032,
            height: 3024,
            latitude: Some(50.087),
            longitude: Some(14.42),
            duration_seconds: None,
            burst_id: None,
            original_filename: "IMG_E0001.HEIC".into(),
            title: "Západ".into(),
            albums: vec!["Dovolená".into(), "Rodina".into()],
            source_path: "Media/DCIM/100APPLE/IMG_0001.HEIC".into(),
            file: Some("photos/1_IMG_0001.HEIC".into()),
        }
    }

    #[test]
    fn photos_csv_json_and_html() {
        let p = vec![sample_photo()];
        let csv = photos_csv(&p);
        assert!(csv.starts_with(
            "filename,kind,created,favorite,trashed,width,height,latitude,longitude,duration_seconds,file"
        ));
        assert!(csv.contains("IMG_0001.HEIC,image,2020-01-06T10:40:00+00:00,true,false,4032,3024,50.087,14.42,,photos/1_IMG_0001.HEIC"));
        // Enriched columns appended.
        let header = csv.lines().next().unwrap();
        assert!(header.ends_with("hidden,edited,live_photo,modified,added,original_filename,title,burst_id,albums"));
        assert!(csv.contains("IMG_E0001.HEIC,Západ,,Dovolená; Rodina")); // original,title,burst(empty),albums
        let back: serde_json::Value = serde_json::from_str(&photos_json(&p)).unwrap();
        assert_eq!(back[0]["latitude"], 50.087);
        assert_eq!(back[0]["duration_seconds"], serde_json::Value::Null);
        assert_eq!(back[0]["live_photo"], true);
        assert_eq!(back[0]["albums"][0], "Dovolená");
        let html = photos_html(&p);
        assert!(html.contains("<img src=\"photos/1_IMG_0001.HEIC\""));
        assert!(html.contains("◉Live")); // Live marker
        assert!(html.contains("Dovolená")); // album shown
        assert!(html.contains("Západ")); // title shown
    }

    #[test]
    fn photos_html_escapes_album_title() {
        let mut p = sample_photo();
        p.albums = vec!["<script>alert(1)</script>".into()];
        let html = photos_html(&[p]);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn photos_html_shows_burst_marker() {
        let mut p = sample_photo();
        p.burst_id = Some("BURST1".into());
        let html = photos_html(&[p]);
        assert!(html.contains("🔥BURST1"));
    }

    #[test]
    fn photos_html_escapes_filename() {
        let mut p = sample_photo();
        p.file = None;
        p.filename = "<script>alert(1)</script>".into();
        let html = photos_html(&[p]);
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    fn sample_attachment() -> crate::attachments::Attachment {
        crate::attachments::Attachment {
            name: "photo.jpg".into(),
            mime_type: "image/jpeg".into(),
            created: "2020-01-06T10:40:00+00:00".into(),
            total_bytes: 102400,
            source_path: "Library/SMS/Attachments/ab/12/G/photo.jpg".into(),
            file: Some("attachments/1_photo.jpg".into()),
        }
    }

    #[test]
    fn attachments_csv_json_and_html() {
        let a = vec![sample_attachment()];
        let csv = attachments_csv(&a);
        assert!(csv.starts_with("name,mime_type,created,total_bytes,file"));
        assert!(csv.contains("photo.jpg,image/jpeg,2020-01-06T10:40:00+00:00,102400,attachments/1_photo.jpg"));
        let back: serde_json::Value = serde_json::from_str(&attachments_json(&a)).unwrap();
        assert_eq!(back[0]["mime_type"], "image/jpeg");
        // image mime → inline <img> in the gallery.
        assert!(attachments_html(&a).contains("<img src=\"attachments/1_photo.jpg\""));
    }

    #[test]
    fn attachments_html_links_non_images_and_escapes() {
        let mut a = sample_attachment();
        a.mime_type = "video/quicktime".into();
        a.name = "<script>alert(1)</script>".into();
        let html = attachments_html(&[a]);
        assert!(!html.contains("<img")); // non-image → link, not inline image
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    fn sample_wa() -> crate::whatsapp::WaMessage {
        crate::whatsapp::WaMessage {
            chat: "Jana".into(),
            sender: "420@s.whatsapp.net".into(),
            from_me: false,
            date: "2020-01-06T10:40:00+00:00".into(),
            text: "Ahoj".into(),
            source_path: "Message/Media/x/photo.jpg".into(),
            media_file: Some("whatsapp_media/2_photo.jpg".into()),
        }
    }

    #[test]
    fn whatsapp_csv_json_and_html_inline_image() {
        let m = vec![sample_wa()];
        let csv = whatsapp_csv(&m);
        assert!(csv.starts_with("chat,sender,from_me,date,text,media_file"));
        assert!(csv.contains("Jana,420@s.whatsapp.net,false,2020-01-06T10:40:00+00:00,Ahoj,whatsapp_media/2_photo.jpg"));
        let back: serde_json::Value = serde_json::from_str(&whatsapp_json(&m)).unwrap();
        assert_eq!(back[0]["chat"], "Jana");
        assert_eq!(back[0]["from_me"], false);
        // image media → inline <img>.
        assert!(whatsapp_html(&m).contains("<img src=\"whatsapp_media/2_photo.jpg\""));
    }

    #[test]
    fn whatsapp_html_escapes_text_and_links_non_image() {
        let mut m = sample_wa();
        m.media_file = Some("whatsapp_media/3_clip.mp4".into());
        m.text = "<script>alert(1)</script>".into();
        let html = whatsapp_html(&[m]);
        assert!(!html.contains("<img")); // non-image → link
        assert!(html.contains("&#60;script&#62;"));
        assert!(!html.contains("<script>alert"));
    }

    #[test]
    fn photos_html_escapes_file_path_in_src_attribute() {
        // A crafted file path must not break out of the src/href attribute.
        let mut p = sample_photo();
        p.file = Some("photos/1_a\"><script>.jpg".into());
        let html = photos_html(&[p]);
        assert!(!html.contains("\"><script>"));
        assert!(html.contains("&#34;") || html.contains("&#62;"));
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

    fn sample_with_address() -> Vec<Contact> {
        vec![Contact {
            first: "Jan".into(),
            last: "Novák".into(),
            organization: String::new(),
            phones: vec![],
            emails: vec![],
            note: String::new(),
            addresses: vec![crate::contacts::Address {
                label: "Work".into(),
                street: "Hlavní 1".into(),
                city: "Praha".into(),
                state: String::new(),
                zip: "11000".into(),
                country: "Czechia".into(),
                country_code: String::new(),
            }],
        }]
    }

    #[test]
    fn contacts_csv_includes_addresses_column() {
        let out = contacts_csv(&sample_with_address());
        assert!(out.starts_with("first,last,organization,phones,emails,addresses,note"));
        assert!(out.contains("Work: Hlavní 1, Praha, 11000, Czechia"));
    }

    #[test]
    fn vcard_emits_escaped_adr() {
        let out = contacts_vcard(&sample_with_address());
        assert!(out.contains("ADR;TYPE=Work:;;Hlavní 1;Praha;;11000;Czechia"));
    }

    #[test]
    fn vcard_adr_resists_injection() {
        let contacts = vec![Contact {
            first: "X".into(),
            last: String::new(),
            organization: String::new(),
            phones: vec![],
            emails: vec![],
            note: String::new(),
            addresses: vec![crate::contacts::Address {
                label: "Home".into(),
                street: "A;B\nADR:evil".into(),
                city: String::new(),
                state: String::new(),
                zip: String::new(),
                country: String::new(),
                country_code: String::new(),
            }],
        }];
        let out = contacts_vcard(&contacts);
        assert!(out.contains("A\\;B\\nADR:evil"));
        assert!(!out.contains("\nADR:evil")); // the injected newline did not start a property
    }

    #[test]
    fn contacts_html_shows_address() {
        let out = contacts_html(&sample_with_address());
        assert!(out.contains("Hlavní 1"));
        assert!(out.contains("Work"));
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
            addresses: vec![],
        }];
        let out = contacts_vcard(&contacts);
        assert!(out.contains("FN:Firma s.r.o."));
        assert!(!out.contains("FN:\r\n"));
    }

    #[test]
    fn renders_health_reminders_mail() {
        let data = crate::health::HealthData {
            workouts: vec![crate::health::Workout {
                activity_type_id: Some(37),
                activity_type: Some("running".into()),
                start: "2024-01-01T10:00:00Z".into(),
                end: "2024-01-01T10:30:00Z".into(),
                duration_seconds: Some(1800),
                total_distance: Some(5000.0),
                total_energy_burned: Some(300.0),
            }],
            quantity_summary: vec![crate::health::QuantitySummary {
                data_type_id: 7,
                name: "step_count".into(),
                count: 3,
                sum: Some(12000.0),
                min: None,
                avg: None,
                max: None,
                first: "2024-01-01T00:00:00Z".into(),
                last: "2024-01-03T00:00:00Z".into(),
            }],
        };
        let html = health_html(&data);
        assert!(html.contains("running") && html.contains("step_count"));
        assert!(health_workouts_csv(&data.workouts).contains("running"));
        assert!(health_quantity_csv(&data.quantity_summary).contains("step_count"));
        assert!(health_json(&data).contains("\"workouts\""));

        let reminders = vec![crate::reminders::Reminder {
            list: "Nákup".into(),
            title: "Mléko".into(),
            notes: "2 l".into(),
            due: Some("2024-01-02T09:00:00Z".into()),
            completed: false,
            completed_date: None,
            priority: 1,
            created: Some("2024-01-01T08:00:00Z".into()),
            flagged: true,
        }];
        assert!(reminders_html(&reminders).contains("Mléko"));
        assert!(reminders_csv(&reminders).contains("list,title,notes"));
        assert!(reminders_json(&reminders).contains("\"title\""));

        let mail = vec![crate::mail::MailMessage {
            from: "a@x.cz".into(),
            to: "b@y.cz".into(),
            subject: "Ahoj".into(),
            date: Some("2024-01-01T12:00:00Z".into()),
            snippet: "text".into(),
        }];
        assert!(mail_html(&mail).contains("Ahoj"));
        assert!(mail_csv(&mail).contains("date,from,to,subject,snippet"));
        assert!(mail_json(&mail).contains("\"subject\""));

        let apps = vec!["com.acme.app".to_string(), "net.example.tool".to_string()];
        assert!(apps_html(&apps).contains("com.acme.app"));
        assert!(apps_csv(&apps).contains("bundle_id"));
        assert!(apps_json(&apps).contains("net.example.tool"));

        let events = vec![crate::timeline::Event {
            timestamp: "2021-01-01T00:00:00+00:00".into(),
            kind: "call".into(),
            summary: "incoming +420 (5s)".into(),
        }];
        assert!(timeline_html(&events).contains("call"));
        assert!(timeline_csv(&events).contains("timestamp,kind,summary"));
        assert!(timeline_json(&events).contains("\"kind\""));

        let deleted = vec![crate::recover_deleted::DeletedRecord {
            store: "messages".into(),
            source: "wal".into(),
            rowid: Some(12),
            date: Some("2020-01-06T10:40:00+00:00".into()),
            summary: "ahoj".into(),
        }];
        assert!(deleted_html(&deleted).contains("ahoj"));
        assert!(deleted_csv(&deleted).contains("store,source,rowid,date,summary"));
        assert!(deleted_json(&deleted).contains("\"store\""));

        let wifi = vec![archive_core::keychain::WifiCredential {
            ssid: "HomeNet".into(),
            password: "s3cr3t-pass".into(),
        }];
        assert!(wifi_html(&wifi).contains("HomeNet") && wifi_html(&wifi).contains("s3cr3t-pass"));
        assert!(wifi_csv(&wifi).contains("ssid,password"));
        assert!(wifi_json(&wifi).contains("\"ssid\""));
    }
}
