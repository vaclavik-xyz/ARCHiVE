use askama::Template;

use imessage_database::{
    message_types::{
        expressives::Expressive,
        sticker::StickerDecoration,
        variants::{Announcement, Tapback},
    },
    tables::messages::models::{GroupAction, Service},
};

#[derive(Template)]
#[template(path = "balloons/digital_touch.html")]
pub(super) struct DigitalTouchVM {
    pub debug: String,
}

#[derive(Template)]
#[template(path = "balloons/find_my.html")]
pub(super) struct FindMyVM<'a> {
    pub app_name: Option<&'a str>,
    pub ldtext: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/apple_pay.html")]
pub(super) struct ApplePayVM<'a> {
    pub app_name: Option<&'a str>,
    pub ldtext: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/app_card.html")]
pub(super) struct AppCardVM<'a> {
    pub url: Option<&'a str>,
    pub image: Option<&'a str>,
    /// Pre-rendered HTML for an attachment, used when `image` is `None` and an
    /// attachment is present. Already escaped — emit with `{{ ... |safe }}`.
    pub attachment_html: Option<String>,
    pub name: &'a str,
    pub title: Option<&'a str>,
    pub subtitle: Option<&'a str>,
    pub ldtext: Option<&'a str>,
    pub caption: Option<&'a str>,
    pub subcaption: Option<&'a str>,
    pub trailing_caption: Option<&'a str>,
    pub trailing_subcaption: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/url.html")]
pub(super) struct UrlVM<'a> {
    /// Resolved outer-link target: `balloon.get_url()` falling back to `msg.text`.
    pub wrapper_url: Option<&'a str>,
    /// Resolved label: `site_name` falling back to URL then `msg.text`.
    pub name: Option<&'a str>,
    pub images: Vec<&'a str>,
    pub lazy: bool,
    pub title: Option<&'a str>,
    pub summary: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/music.html")]
pub(super) struct MusicVM<'a> {
    pub track_name: Option<&'a str>,
    pub preview: Option<&'a str>,
    pub lyrics: Option<&'a [&'a str]>,
    pub url: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub album: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/collaboration.html")]
pub(super) struct CollaborationVM<'a> {
    pub name: Option<&'a str>,
    /// `<a>` wrapper opens only when `balloon.url` is set (not the `original_url` fallback).
    pub wrapper_url: Option<&'a str>,
    pub title: Option<&'a str>,
    /// Subcaption uses the resolved URL (`url` or `original_url`).
    pub footer_url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/app_store.html")]
pub(super) struct AppStoreVM<'a> {
    pub app_name: Option<&'a str>,
    pub url: Option<&'a str>,
    pub description: Option<&'a str>,
    pub platform: Option<&'a str>,
    pub genre: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/placemark.html")]
pub(super) struct PlacemarkVM<'a> {
    pub url: Option<&'a str>,
    pub name: Option<&'a str>,
    pub address: Option<&'a str>,
    pub postal_code: Option<&'a str>,
    pub country: Option<&'a str>,
    pub sub_administrative_area: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "balloons/check_in.html")]
pub(super) struct CheckInVM<'a> {
    pub name: &'a str,
    pub ldtext: Option<&'a str>,
    /// Pre-formatted footer text (e.g. "Checked in at Oct 14, 2023…"). `None`
    /// when the balloon's metadata yields no recognized timestamp.
    pub footer: Option<String>,
}

#[derive(Template)]
#[template(path = "balloons/poll.html")]
pub(super) struct PollVM<'a> {
    pub options: Vec<PollOptionVM<'a>>,
}

pub(super) struct PollOptionVM<'a> {
    pub text: &'a str,
    pub vote_count: usize,
    /// Pre-rendered `<div class="vote-bar" style="…">` element. Rendered in
    /// Rust because keeping `{{ }}` inside a `style=` attribute trips the
    /// CSS linter no matter what value-shape we use; emitting the entire
    /// element as a `|safe` string keeps the template's source clean.
    pub bar_html: String,
    pub voters: Vec<&'a str>,
}

#[derive(Template)]
#[template(path = "attachments/attachment.html")]
pub(super) struct AttachmentVM<'a> {
    pub lazy: bool,
    pub variant: AttachmentVariant<'a>,
}

pub(super) enum AttachmentVariant<'a> {
    Image {
        embed_path: String,
    },
    Video {
        embed_path: String,
        media_type: &'a str,
    },
    Audio {
        embed_path: String,
        media_type: &'a str,
    },
    AudioTranscription {
        embed_path: String,
        media_type: &'a str,
        transcription: &'a str,
    },
    /// `text/*` and `application/*` types share this layout.
    Download {
        embed_path: String,
        filename: &'a str,
        file_size: String,
    },
    UnknownFolder {
        embed_path: String,
        filename: &'a str,
        file_size: String,
    },
    UnknownOther {
        embed_path: String,
        file_size: String,
    },
    Other {
        embed_path: String,
        media_type: &'a str,
    },
}

#[derive(Template)]
#[template(path = "attachments/sticker_suffix.html")]
pub(super) struct StickerSuffixVM {
    pub kind: StickerDecoration,
}

#[derive(Template)]
#[template(path = "tapback.html")]
pub(super) struct TapbackVM<'a> {
    pub kind: TapbackKind<'a>,
}

pub(super) enum TapbackKind<'a> {
    /// Standard reaction
    Reaction { tapback: Tapback<'a>, who: String },
    /// Sticker tapback whose attachment was found and rendered.
    Sticker {
        /// Pre-rendered sticker HTML (already escaped).
        html: String,
        who: String,
    },
    /// Sticker tapback whose attachment is missing.
    StickerMissing { who: String },
}

#[derive(Template)]
#[template(path = "edited.html")]
pub(super) struct EditedVM {
    pub kind: EditedKind,
}

pub(super) enum EditedKind {
    Edited { rows: Vec<EditedRow> },
    UnsentWithDiff { who: String, diff: String },
    Unsent { who: String },
}

pub(super) struct EditedRow {
    /// `"tbody"` for non-final edits, `"tfoot"` for the last edit.
    pub tag: &'static str,
    pub timestamp: String,
    /// Pre-rendered HTML for the edited text (text-effect output or escaped plain text).
    pub text: String,
}

#[derive(Template)]
#[template(path = "announcement_inner.html")]
pub(super) struct AnnouncementInnerVM<'a> {
    pub kind: AnnouncementBody<'a>,
}

pub(super) enum AnnouncementBody<'a> {
    Action {
        timestamp: String,
        who: &'a str,
        announcement: Announcement<'a>,
        /// Resolved display name for the participant in `ParticipantAdded`
        /// / `ParticipantRemoved`. `None` for every other variant.
        participant_name: Option<&'a str>,
    },
    Unknown,
}

#[derive(Template)]
#[template(path = "message.html")]
pub(super) struct MessageVM<'a> {
    pub guid: &'a str,
    /// Render `id="r-{guid}"` on the outer wrapper when this is a top-level reply.
    pub anchor_id: bool,
    /// True for `<div class="sent {service}">`, false for `<div class="received">`.
    pub is_from_me: bool,
    pub service: Service<'a>,
    pub date: String,
    pub read_after: String,
    pub reply_anchor: Option<ReplyAnchorKind>,
    pub sender: &'a str,
    pub is_deleted: bool,
    pub subject: Option<&'a str>,
    /// SharePlay marker (`<hr>SharePlay …`) — currently always a `'static` literal.
    pub shareplay: Option<&'a str>,
    /// Shared-location marker — currently always a `'static` literal.
    pub shared_location: Option<&'a str>,
    /// Rendered directly into the outer buffer via [`MessagePartVM`]'s `Display`
    /// impl, avoiding a per-part `String` allocation.
    pub parts: Vec<MessagePartVM<'a>>,
    pub trailing_reply_context: bool,
}

pub(super) enum ReplyAnchorKind {
    /// Reply rendered inline within a thread; link points to the top-level message.
    InThread,
    /// Top-level message that has descendants; link jumps to its in-thread copy.
    TopLevel,
}

#[derive(Template)]
#[template(path = "message_part.html")]
pub(super) struct MessagePartVM<'a> {
    pub body: PartBody,
    pub expressive: Option<Expressive<'a>>,
    /// Pre-rendered tapbacks block (already includes its trailing newline).
    pub tapbacks: Option<String>,
    /// Pre-rendered replies block (already includes its trailing newline).
    pub replies: Option<String>,
}

/// Each `String` is pre-rendered HTML — emitted with `|safe`. Wrapping per
/// variant happens in `message_part.html`, so leaf renderers can move their
/// HTML in without an extra `format!()` allocation.
pub(super) enum PartBody {
    /// Empty body (no text, edited content missing, etc.) — emits nothing.
    Empty,
    TextBubble {
        html: String,
    },
    TextTranslated {
        translated: String,
        original: String,
    },
    TextEdited {
        html: String,
    },
    Attachment {
        html: String,
    },
    AttachmentError {
        error: String,
    },
    AttachmentMissing,
    Sticker {
        html: String,
    },
    App {
        html: String,
    },
    AppError {
        html: String,
    },
    Retracted {
        html: String,
    },
}
