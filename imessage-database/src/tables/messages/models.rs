/*!
 This module contains Data structures and models that represent message data.
*/

use std::fmt::{Display, Formatter, Result};

use crabstep::deserializer::iter::Property;

use crate::{
    message_types::text_effects::TextEffect,
    tables::messages::message::Message,
    util::typedstream::{as_float, as_nsstring},
};

// MARK: BubbleComponent
/// Defines the parts of a message bubble, i.e. the content that can exist in a single message.
///
/// # Component Types
///
/// A single iMessage contains data that may be represented across multiple bubbles.
/// Each bubble corresponds to one `__kIMMessagePartAttributeName` index in the
/// underlying [`NSAttributedString`](crate::util::typedstream); the
/// [`Run`](Self::Run) groups every attributed range that shares that part index.
#[derive(Debug, PartialEq, Clone)]
pub enum BubbleComponent {
    /// One bubble's worth of attributed body content. Each contained
    /// [`AttributedRange`] models a single `NSAttributedString` attribute run
    /// (a byte range plus its attribute dictionary); adjacent ranges that
    /// share a `__kIMMessagePartAttributeName` index share the bubble.
    ///
    /// A run may interleave text ranges ([`AttributedRange::attachment`] is
    /// `None`) with inline-attachment ranges (e.g. stickers rendered inline
    /// like emoji), preserving their original order.
    Run(Vec<AttributedRange>),
    /// An [app integration](crate::message_types::app)
    App,
    /// A component that was retracted, found by parsing the [`EditedMessage`](crate::message_types::edited::EditedMessage)
    Retracted,
}

// MARK: Service
/// Defines different types of [services](https://support.apple.com/en-us/104972) we can receive messages from.
#[derive(Debug, PartialEq, Eq)]
pub enum Service<'a> {
    /// An iMessage
    #[allow(non_camel_case_types)]
    iMessage,
    /// A message sent as SMS
    SMS,
    /// A message sent as RCS
    RCS,
    /// A message sent via [satellite](https://support.apple.com/en-us/120930)
    Satellite,
    /// Any other type of message
    Other(&'a str),
    /// Used when service field is not set
    Unknown,
}

impl<'a> Service<'a> {
    /// Creates a [`Service`] enum variant based on the provided service name string.
    #[must_use]
    pub fn from_name(service: Option<&'a str>) -> Self {
        if let Some(service_name) = service {
            return match service_name.trim() {
                "iMessage" => Service::iMessage,
                "iMessageLite" => Service::Satellite,
                "SMS" => Service::SMS,
                "rcs" | "RCS" => Service::RCS,
                service_name => Service::Other(service_name),
            };
        }
        Service::Unknown
    }
}

impl Display for Service<'_> {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result {
        match self {
            Service::iMessage => write!(fmt, "iMessage"),
            Service::SMS => write!(fmt, "SMS"),
            Service::RCS => write!(fmt, "RCS"),
            Service::Satellite => write!(fmt, "Satellite"),
            Service::Other(other) => write!(fmt, "{other}"),
            Service::Unknown => write!(fmt, "Unknown"),
        }
    }
}

// MARK: AttributedRange
/// One attribute run of a message's [`NSAttributedString`](crate::util::typedstream)
/// body: a byte range into the [`Message`]'s [`text`](crate::tables::messages::Message::text)
/// plus every attribute applied to it.
///
/// A range is a *text* range when [`attachment`](Self::attachment) is `None` and
/// an *attachment* range (a `\u{FFFC}` placeholder for an inline attachment)
/// when it is `Some`. Effects, styles, and the inline-emoji hint apply to either
/// kind. The `typedstream`` attribute dictionary is a flat bag, so an attachment
/// range can also carry, say, an [`Animated`](TextEffect::Animated) effect.
///
/// Ranges that share a `__kIMMessagePartAttributeName` index are grouped into one
/// [`BubbleComponent::Run`]. For example, message text with a
/// [`Mention`](TextEffect::Mention) like:
///
/// ```
/// let message_text = "What's up, Christopher?";
/// ```
///
/// parses into a single run of 3 ranges:
///
/// ```
/// use imessage_database::message_types::text_effects::TextEffect;
/// use imessage_database::tables::messages::models::{AttributedRange, BubbleComponent};
///
/// let result = vec![BubbleComponent::Run(vec![
///     AttributedRange::text(0, 11, vec![TextEffect::Default]),  // `What's up, `
///     AttributedRange::text(11, 22, vec![TextEffect::Mention("+5558675309".to_string())]), // `Christopher`
///     AttributedRange::text(22, 23, vec![TextEffect::Default])  // `?`
/// ])];
/// ```
#[derive(Debug, PartialEq, Clone)]
pub struct AttributedRange {
    /// The start index of the affected range of message text
    pub start: usize,
    /// The end index of the affected range of message text
    pub end: usize,
    /// The effects applied to the specified range
    pub effects: Vec<TextEffect>,
    /// `Some` when this range is a `\u{FFFC}` placeholder for an attachment.
    /// The attachment's metadata travels here; effects still apply alongside.
    pub attachment: Option<AttachmentMeta>,
    /// `true` when the range carries `__kIMEmojiImageAttributeName`–Apple's
    /// hint to render the attachment inline–like an emoji (observed on
    /// genmoji, Memoji, and custom sticker ranges).
    pub emoji_image: bool,
}

impl AttributedRange {
    /// Creates a text range (no attachment, no inline-emoji hint) with the
    /// specified start index, end index, and text effects.
    #[must_use]
    pub fn text(start: usize, end: usize, effects: Vec<TextEffect>) -> Self {
        Self {
            start,
            end,
            effects,
            attachment: None,
            emoji_image: false,
        }
    }

    /// Creates an attachment range carrying the given [`AttachmentMeta`].
    #[must_use]
    pub fn attachment(start: usize, end: usize, meta: AttachmentMeta) -> Self {
        Self {
            start,
            end,
            effects: vec![],
            attachment: Some(meta),
            emoji_image: false,
        }
    }

    /// `true` when this range stands in for an attachment rather than text.
    #[must_use]
    pub fn is_attachment(&self) -> bool {
        self.attachment.is_some()
    }
}

// MARK: AttachmentMeta
/// Representation of attachment metadata used for rendering message body in a conversation feed.
#[derive(Debug, PartialEq, Default, Clone)]
pub struct AttachmentMeta {
    /// GUID of the attachment in the `attachment` table
    pub guid: Option<String>,
    /// The transcription, if the attachment was an [audio message](https://support.apple.com/guide/iphone/send-and-receive-audio-messages-iph2e42d3117/ios) sent from or received on a [supported platform](https://www.apple.com/ios/feature-availability/#messages-audio-message-transcription).
    pub transcription: Option<String>,
    /// The height of the attachment in points
    pub height: Option<f64>,
    /// The width of the attachment in points
    pub width: Option<f64>,
    /// The attachment's original filename
    pub name: Option<String>,
}

impl AttachmentMeta {
    /// Applies a single typedstream attribute key/value pair to the metadata,
    /// ignoring any key that isn't attachment metadata. Driven per-key by the
    /// body parser's `build_range`, which walks the full attribute dictionary
    /// so non-attachment-meta keys on the same range are still processed.
    pub(crate) fn set_from_key_value<'a>(
        &'a mut self,
        key: &'a str,
        value: &'a mut Property<'a, 'a>,
    ) {
        match key {
            "__kIMFileTransferGUIDAttributeName" => {
                self.guid = as_nsstring(value).map(String::from);
            }
            "IMAudioTranscription" => self.transcription = as_nsstring(value).map(String::from),
            "__kIMInlineMediaHeightAttributeName" => self.height = as_float(value),
            "__kIMInlineMediaWidthAttributeName" => self.width = as_float(value),
            "__kIMFilenameAttributeName" => self.name = as_nsstring(value).map(String::from),
            _ => {}
        }
    }
}

// MARK: SharedLocation
/// Direction of a legacy shared-location event (`item_type == 4` with
/// `group_action_type == 0`). The two cases are mutually exclusive: the
/// underlying `share_status` bool distinguishes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedLocation {
    /// The sender began sharing their location.
    Started,
    /// The sender stopped sharing their location.
    Stopped,
}

// MARK: GroupAction
/// Represents different types of group message actions that can occur in a chat system
#[derive(Debug, PartialEq, Eq)]
pub enum GroupAction<'a> {
    /// A new participant has been added to the group
    ParticipantAdded(i32),
    /// A participant has been removed from the group
    ParticipantRemoved(i32),
    /// The group name has been changed
    NameChange(&'a str),
    /// A participant has voluntarily left the group
    ParticipantLeft,
    /// The group icon/avatar has been updated with a new image
    GroupIconChanged,
    /// The group icon/avatar has been removed, reverting to default
    GroupIconRemoved,
    /// The chat background was changed
    ChatBackgroundChanged,
    /// The chat background was removed
    ChatBackgroundRemoved,
    /// A participant changed their phone number
    PhoneNumberChanged(i32),
}

impl<'a> GroupAction<'a> {
    /// Creates a new `GroupAction` event type based on the provided message's item and group action data.
    #[must_use]
    pub(crate) fn from_message(message: &'a Message) -> Option<Self> {
        match (
            message.item_type,
            message.group_action_type,
            message.other_handle,
            &message.group_title,
        ) {
            // If the handle_id of the message matches the other_handle, the sender changed their own phone number
            (1, 0, Some(who), _) if message.handle_id == Some(who) => {
                Some(Self::PhoneNumberChanged(who))
            }
            (1, 0, Some(who), _) => Some(Self::ParticipantAdded(who)),
            (1, 1, Some(who), _) => Some(Self::ParticipantRemoved(who)),
            (2, _, _, Some(name)) => Some(Self::NameChange(name)),
            (3, 0, _, _) => Some(Self::ParticipantLeft),
            (3, 1, _, _) => Some(Self::GroupIconChanged),
            (3, 2, _, _) => Some(Self::GroupIconRemoved),
            (3, 4, _, _) => Some(Self::ChatBackgroundChanged),
            (3, 6, _, _) => Some(Self::ChatBackgroundRemoved),
            _ => None,
        }
    }
}
