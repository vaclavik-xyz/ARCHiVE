use imessage_database::{
    error::{message::MessageError, plist::PlistParseError},
    message_types::{
        app::AppMessage,
        digital_touch,
        handwriting::HandwrittenMessage,
        url::URLMessage,
        variants::{BalloonProvider, CustomBalloon, URLOverride, Variant},
    },
    tables::{
        attachment::Attachment,
        messages::Message,
        table::{FITNESS_RECEIVER, YOU},
    },
    util::{
        dates::{TIMESTAMP_FACTOR, format, get_local_time},
        plist::parse_ns_keyed_archiver,
    },
};

use crate::{app::runtime::Config, exporters::exporter::BalloonFormatter};

// MARK: Dispatch

/// Drive the App-balloon decision tree: pick the right payload source
/// (raw vs keyed-archiver), parse it, and dispatch to the matching
/// [`BalloonFormatter`] method.
pub fn dispatch_app_balloon<F: BalloonFormatter>(
    formatter: &F,
    message: &Message,
    attachments: &mut Vec<Attachment>,
    config: &Config,
) -> Result<String, MessageError> {
    // First, determine if is a balloon message; if it is not, bail out early
    let Variant::App(balloon) = message.variant() else {
        return Err(MessageError::PlistParseError(
            PlistParseError::WrongMessageType,
        ));
    };

    // Handwritten messages use a different payload type
    if message.is_handwriting()
        && let Some(payload) = message.raw_payload_data(config.data_source.db())
    {
        return match HandwrittenMessage::from_payload(&payload) {
            Ok(bubble) => Ok(formatter.format_handwriting(message, &bubble)),
            Err(why) => Err(MessageError::PlistParseError(
                PlistParseError::HandwritingError(why),
            )),
        };
    }

    // Digital touch messages use a different payload type
    if message.is_digital_touch()
        && let Some(payload) = message.raw_payload_data(config.data_source.db())
    {
        return match digital_touch::from_payload(&payload) {
            Some(bubble) => Ok(formatter.format_digital_touch(message, &bubble)),
            None => Err(MessageError::PlistParseError(
                PlistParseError::DigitalTouchError,
            )),
        };
    }

    // Poll messages use a different payload type
    if message.is_poll() {
        let poll = message.as_poll(config.data_source.db())?;
        return match poll {
            Some(poll) => Ok(formatter.format_poll(&poll)),
            None => Err(MessageError::PlistParseError(PlistParseError::PollError)),
        };
    }

    // Otherwise, we expect an NSKeyedArchiver payload
    let Some(payload) = message.payload_data(config.data_source.db()) else {
        // URL message may omit the relevant payload. Defensively we reuse the normal
        // URL renderer with an empty balloon.
        if message.is_url() && message.text.is_some() {
            return Ok(formatter.format_url(message, &URLMessage::default()));
        }
        return Err(MessageError::PlistParseError(PlistParseError::NoPayload));
    };

    let parsed = parse_ns_keyed_archiver(&payload)?;

    let rendered = if message.is_url() {
        let bubble = URLMessage::get_url_message_override(&parsed)?;
        match bubble {
            URLOverride::Normal(b) => formatter.format_url(message, &b),
            URLOverride::AppleMusic(b) => formatter.format_music(&b),
            URLOverride::Collaboration(b) => formatter.format_collaboration(&b),
            URLOverride::AppStore(b) => formatter.format_app_store(&b),
            URLOverride::SharedPlacemark(b) => formatter.format_placemark(&b),
        }
    } else {
        match AppMessage::from_map(&parsed) {
            Ok(bubble) => match balloon {
                CustomBalloon::Application(bundle_id) => {
                    formatter.format_generic_app(&bubble, bundle_id, attachments, message)
                }
                CustomBalloon::ApplePay => formatter.format_apple_pay(&bubble),
                CustomBalloon::Fitness => formatter.format_fitness(&bubble),
                CustomBalloon::Slideshow => formatter.format_slideshow(&bubble),
                CustomBalloon::CheckIn => formatter.format_check_in(&bubble),
                CustomBalloon::FindMy => formatter.format_find_my(&bubble),
                CustomBalloon::Polls
                | CustomBalloon::Handwriting
                | CustomBalloon::DigitalTouch
                | CustomBalloon::URL => unreachable!(),
            },
            Err(why) => return Err(MessageError::PlistParseError(why)),
        }
    };

    Ok(rendered)
}

// MARK: Fitness

/// Replace the leading [`FITNESS_RECEIVER`] sentinel emitted by Fitness app
/// messages with [`YOU`] so the rendered string reads in first person.
/// Returns the input unchanged if the sentinel isn't present.
pub fn rewrite_fitness_receiver(text: String) -> String {
    if let Some(rest) = text.strip_prefix(FITNESS_RECEIVER) {
        format!("{YOU}{rest}")
    } else {
        text
    }
}

// MARK: Check In

/// Parse a Check In timestamp from a `parse_query_string` value and render it
/// with the given prefix (e.g. `"Checked in at "`). Returns `None` if the
/// value is unparseable.
pub fn format_check_in_caption(date_str: &str, prefix: &str) -> Option<String> {
    let date_stamp = date_str.parse::<f64>().unwrap_or(0.) as i64 * TIMESTAMP_FACTOR;
    let date_time = get_local_time(date_stamp, 0).ok()?;
    Some(format!("{prefix}{}", format(&date_time)))
}
