use askama::Template;

#[derive(Template)]
#[template(path = "apple_pay.txt")]
pub(super) struct ApplePayVM<'a> {
    pub indent: &'a str,
    pub caption: Option<&'a str>,
    pub ldtext: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "fitness.txt")]
pub(super) struct FitnessVM<'a> {
    pub indent: &'a str,
    pub app_name: Option<&'a str>,
    pub ldtext: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "slideshow.txt")]
pub(super) struct SlideshowVM<'a> {
    pub indent: &'a str,
    pub ldtext: Option<&'a str>,
    pub url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "find_my.txt")]
pub(super) struct FindMyVM<'a> {
    pub indent: &'a str,
    pub app_name: Option<&'a str>,
    pub ldtext: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "digital_touch.txt")]
pub(super) struct DigitalTouchVM<'a> {
    pub indent: &'a str,
    pub debug: String,
}

#[derive(Template)]
#[template(path = "check_in.txt")]
pub(super) struct CheckInVM<'a> {
    pub indent: &'a str,
    /// Resolved label: `balloon.caption.unwrap_or("Check In")`.
    pub caption: &'a str,
    /// Pre-formatted footer line (e.g., `"Checked in at Oct 14, …"`). `None`
    /// when the metadata yields no recognized timestamp.
    pub footer: Option<String>,
}

#[derive(Template)]
#[template(path = "poll.txt")]
pub(super) struct PollVM<'a> {
    pub indent: &'a str,
    pub options: Vec<PollOptionVM<'a>>,
}

pub(super) struct PollOptionVM<'a> {
    pub text: &'a str,
    pub vote_count: usize,
    pub voters: Vec<&'a str>,
}

#[derive(Template)]
#[template(path = "generic_app.txt")]
pub(super) struct GenericAppVM<'a> {
    pub indent: &'a str,
    /// `app_name` falling back to `bundle_id`.
    pub name: &'a str,
    /// True when `indent` or `name` is non-empty (matches the legacy
    /// `if !out_s.is_empty()` guard around the `" message:"` suffix).
    pub has_label: bool,
    pub title: Option<&'a str>,
    pub subtitle: Option<&'a str>,
    pub caption: Option<&'a str>,
    pub subcaption: Option<&'a str>,
    pub trailing_caption: Option<&'a str>,
    pub trailing_subcaption: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "url.txt")]
pub(super) struct UrlVM<'a> {
    pub indent: &'a str,
    /// Resolved primary line: `balloon.get_url()` falling back to `msg.text`.
    pub primary: Option<&'a str>,
    pub title: Option<&'a str>,
    pub summary: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "music.txt")]
pub(super) struct MusicVM<'a> {
    pub indent: &'a str,
    pub lyrics: Option<&'a [&'a str]>,
    pub track_name: Option<&'a str>,
    pub album: Option<&'a str>,
    pub artist: Option<&'a str>,
    pub url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "collaboration.txt")]
pub(super) struct CollaborationVM<'a> {
    pub indent: &'a str,
    /// `app_name` falling back to `bundle_id`.
    pub name: Option<&'a str>,
    pub has_label: bool,
    pub title: Option<&'a str>,
    /// `balloon.get_url()`.
    pub url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "app_store.txt")]
pub(super) struct AppStoreVM<'a> {
    pub indent: &'a str,
    pub app_name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub platform: Option<&'a str>,
    pub genre: Option<&'a str>,
    pub url: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "placemark.txt")]
pub(super) struct PlacemarkVM<'a> {
    pub indent: &'a str,
    pub place_name: Option<&'a str>,
    pub url: Option<&'a str>,
    pub name: Option<&'a str>,
    pub address: Option<&'a str>,
    pub state: Option<&'a str>,
    pub city: Option<&'a str>,
    pub iso_country_code: Option<&'a str>,
    pub postal_code: Option<&'a str>,
    pub country: Option<&'a str>,
    pub street: Option<&'a str>,
    pub sub_administrative_area: Option<&'a str>,
    pub sub_locality: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "attachment.txt")]
pub(super) struct AttachmentVM<'a> {
    pub embed_path: String,
    pub transcription: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "sticker.txt")]
pub(super) struct StickerVM<'a> {
    /// e.g. `"Outline "` (trailing space) for `UserGenerated` effects;
    /// `None` for the other variants.
    pub effect_prefix: Option<String>,
    pub who: &'a str,
    /// Path returned by `format_attachment` (or its error string on failure).
    pub path: String,
    /// Pre-formatted parenthesized suffix like `" (App: Memoji)"`. `None` for
    /// `UserGenerated` (which uses `effect_prefix`) and on attachment errors.
    pub suffix: Option<String>,
}

#[derive(Template)]
#[template(path = "tapback.txt")]
pub(super) struct TapbackVM<'a> {
    pub kind: TapbackKind<'a>,
}

pub(super) enum TapbackKind<'a> {
    /// Standard reaction — `tapback` is the rendered Display name (e.g. `"Loved"`).
    Reaction { tapback: String, who: &'a str },
    /// Sticker tapback whose attachment was found and rendered.
    Sticker {
        /// Pre-rendered sticker text.
        text: String,
        who: &'a str,
    },
    /// Sticker tapback whose attachment is missing.
    StickerMissing { who: &'a str },
}

#[derive(Template)]
#[template(path = "edited.txt")]
pub(super) struct EditedVM<'a> {
    pub indent: &'a str,
    pub kind: EditedKind<'a>,
}

pub(super) enum EditedKind<'a> {
    Edited { rows: Vec<EditedRow<'a>> },
    UnsentWithDiff { who: &'a str, diff: String },
    Unsent { who: &'a str },
}

pub(super) struct EditedRow<'a> {
    /// Either `"{absolute_timestamp} "` for the first edit, or
    /// `"{indent}Edited {diff} later: "` for subsequent edits.
    pub timestamp_prefix: String,
    pub text: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "announcement.txt")]
pub(super) struct AnnouncementVM<'a> {
    pub kind: AnnouncementBody<'a>,
}

pub(super) enum AnnouncementBody<'a> {
    Action {
        timestamp: String,
        who: &'a str,
        action_text: String,
    },
    Unknown,
}

#[derive(Template)]
#[template(path = "message.txt")]
pub(super) struct MessageVM<'a> {
    pub indent: &'a str,
    pub timestamp: String,
    pub sender: &'a str,
    pub is_deleted: bool,
    pub subject: Option<&'a str>,
    /// Static SharePlay marker (`"SharePlay Message\nEnded"`).
    pub shareplay: Option<&'a str>,
    /// Static shared-location marker.
    pub shared_location: Option<&'a str>,
    pub parts: Vec<MessagePartVM<'a>>,
    pub trailing_reply_context: bool,
    /// Top-level messages get an extra blank-line separator at the end.
    pub top_level: bool,
}

#[derive(Template)]
#[template(path = "message_part.txt")]
pub(super) struct MessagePartVM<'a> {
    pub indent: &'a str,
    pub body: PartBody,
    /// Single-line expressive marker (e.g. `"🌧️ Background Rain"`).
    pub expressive: Option<&'a str>,
    pub tapbacks: Option<TapbacksVM<'a>>,
    pub replies: Option<RepliesVM>,
}

#[derive(Template)]
#[template(path = "tapbacks.txt")]
pub(super) struct TapbacksVM<'a> {
    pub indent: &'a str,
    /// Each entry is one fully-rendered tapback line (e.g. `"Loved by Me"`).
    pub tapbacks: Vec<String>,
}

#[derive(Template)]
#[template(path = "replies.txt")]
pub(super) struct RepliesVM {
    /// Each entry is a fully-rendered nested message (multi-line, with its
    /// own internal indent already applied).
    pub replies: Vec<String>,
}

/// Each variant becomes a single `add_line` style emission in the part body
/// (i.e., one indent prefix + content + trailing newline). `Translated` is
/// the only multi-line variant; the rest collapse to a single line.
pub(super) enum PartBody {
    Empty,
    Line {
        text: String,
    },
    Translated {
        translated: String,
        original: String,
    },
}
