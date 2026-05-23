use imessage_database::message_types::variants::Tapback;

/// Format-agnostic shape for tapback rendering. The `payload` carried by
/// [`Sticker`] is generic so each exporter can use its own pre-rendered type
/// (HTML uses [`Html`](crate::exporters::html::safe::Html), TXT uses
/// [`String`]).
///
/// [`Sticker`]: TapbackKind::Sticker
pub enum TapbackKind<'a, S> {
    /// Standard reaction.
    Reaction { tapback: Tapback<'a>, who: &'a str },
    /// Sticker tapback whose attachment was found and rendered.
    Sticker { payload: S, who: &'a str },
    /// Sticker tapback whose attachment is missing.
    StickerMissing { who: &'a str },
}
