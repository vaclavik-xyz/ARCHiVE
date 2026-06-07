/*!
[Media](super) Digital Touch effect: a photo or video, optionally with a
drawing on top of it.

The effect carries a media-type discriminator and an `NSKeyedArchiver` archive.
The archive holds an `NSMutableArray` of drawing overlays. Each overlay is an
`NSData` blob containing a complete, nested [`Sketch`](super::sketch) Digital
Touch message, so the drawing is parsed and rendered exactly like a standalone
sketch. The array is empty when nothing was drawn. The photo or video itself is
delivered as a normal message attachment, not embedded here.
*/

use std::io::Cursor;

use plist::Value;
use protobuf::Message;

use crate::{
    error::digital_touch::DigitalTouchError,
    message_types::digital_touch::{
        digital_touch_proto::{BaseMessage, MediaMessage, TouchKind},
        models::{DigitalTouchMessage, pluralize},
        sketch::DigitalTouchSketch,
        svg::Canvas,
    },
};

/// The kind of media a [`DigitalTouchMedia`] effect carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A still image.
    Image,
    /// A video.
    Video,
    /// An unrecognized media-type discriminator.
    Other(u64),
}

impl MediaKind {
    /// Map the `MediaType` discriminator to a [`MediaKind`].
    fn from_type(media_type: u64) -> Self {
        match media_type {
            1 => MediaKind::Video,
            2 => MediaKind::Image,
            other => MediaKind::Other(other),
        }
    }

    /// Human-readable label.
    fn label(self) -> String {
        match self {
            MediaKind::Image => "Image".to_string(),
            MediaKind::Video => "Video".to_string(),
            MediaKind::Other(media_type) => format!("Media (type {media_type})"),
        }
    }
}

/// A photo or video Digital Touch, optionally drawn on top of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalTouchMedia {
    /// Unique identifier for the message.
    pub id: String,
    /// Whether the media is an image or a video.
    pub kind: MediaKind,
    /// Sketches drawn on top of the media; empty when nothing was drawn.
    pub drawings: Vec<DigitalTouchSketch>,
}

impl DigitalTouchMedia {
    /// Parse the [`MediaMessage`] carried by `base` into a [`DigitalTouchMessage`].
    pub(super) fn from_payload(
        base: &BaseMessage,
    ) -> Result<DigitalTouchMessage, DigitalTouchError> {
        let msg = MediaMessage::parse_from_bytes(&base.TouchPayload)
            .map_err(DigitalTouchError::ProtobufError)?;

        Ok(DigitalTouchMessage::Media(DigitalTouchMedia {
            id: base.ID.clone(),
            kind: MediaKind::from_type(msg.MediaType),
            drawings: decode_drawings(&msg.Archive)?,
        }))
    }

    /// One-line summary, e.g. `"Digital Touch Image"` or
    /// `"Digital Touch Image with drawing (5 strokes)"`.
    pub(super) fn summary(&self) -> String {
        let mut summary = format!("Digital Touch {}", self.kind.label());
        let strokes: usize = self.drawings.iter().map(|s| s.strokes.len()).sum();
        if strokes > 0 {
            summary.push_str(&format!(" with drawing ({})", pluralize(strokes, "stroke")));
        }
        summary
    }

    /// Draw the overlay sketches (if any) and label the media kind. The photo or
    /// video itself lives in a separate attachment, so it is not depicted here.
    pub(super) fn append_svg(&self, canvas: &mut Canvas) {
        for drawing in &self.drawings {
            drawing.append_svg(canvas);
        }

        let width = canvas.width();
        let font = width / 16;
        let y = canvas.height() * 9 / 10;
        canvas.push(&format!(
            r#"<text x="{}" y="{y}" fill="white" font-family="-apple-system, Helvetica, Arial, sans-serif" font-size="{font}" text-anchor="middle">{}</text>"#,
            width / 2,
            self.kind.label(),
        ));
    }
}

/// Decode the drawing overlays from the effect's `NSKeyedArchiver` archive.
///
/// The archive is an `NSMutableArray` of `NSData` blobs, each a nested sketch
/// message. A blob that is not a sketch (or fails to parse) is skipped rather
/// than failing the whole message, since the media kind is still meaningful
/// without its overlay.
fn decode_drawings(archive: &[u8]) -> Result<Vec<DigitalTouchSketch>, DigitalTouchError> {
    if archive.is_empty() {
        return Ok(Vec::new());
    }

    let value =
        Value::from_reader(Cursor::new(archive)).map_err(DigitalTouchError::ArchiveError)?;

    let Some(objects) = value
        .as_dictionary()
        .and_then(|dict| dict.get("$objects"))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };

    let mut drawings = Vec::new();
    for object in objects {
        let Some(data) = object
            .as_dictionary()
            .and_then(|dict| dict.get("NS.data"))
            .and_then(Value::as_data)
        else {
            continue;
        };

        // Each overlay is a nested sketch message. Parse it directly as a sketch
        // (rather than recursing through the general dispatcher) so a nested
        // media blob can't drive unbounded recursion.
        let Ok(base) = BaseMessage::parse_from_bytes(data) else {
            continue;
        };
        if base.TouchKind.enum_value_or_default() != TouchKind::Sketch {
            continue;
        }
        if let Ok(DigitalTouchMessage::Sketch(sketch)) = DigitalTouchSketch::from_payload(&base) {
            drawings.push(sketch);
        }
    }

    Ok(drawings)
}
