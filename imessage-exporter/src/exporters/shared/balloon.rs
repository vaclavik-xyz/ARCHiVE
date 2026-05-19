use imessage_database::{
    error::{message::MessageError, plist::PlistParseError},
    message_types::{
        app::AppMessage,
        digital_touch,
        handwriting::HandwrittenMessage,
        url::URLMessage,
        variants::{BalloonProvider, CustomBalloon, URLOverride, Variant},
    },
    tables::{attachment::Attachment, messages::Message},
    util::plist::parse_ns_keyed_archiver,
};

use crate::{app::runtime::Config, exporters::exporter::BalloonFormatter};

/// Drive the App-balloon decision tree: pick the right payload source
/// (raw vs keyed-archiver), parse it, and dispatch to the matching
/// [`BalloonFormatter`] method.
///
/// Returns `Ok(None)` when the message has no payload data; callers handle
/// that case themselves (typically by falling back to the message's text).
pub fn dispatch_app_balloon<T, F>(
    formatter: &F,
    message: &Message,
    attachments: &mut Vec<Attachment>,
    context: T,
    config: &Config,
) -> Result<Option<String>, MessageError>
where
    T: Copy,
    F: BalloonFormatter<T>,
{
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
            Ok(bubble) => Ok(Some(
                formatter.format_handwriting(message, &bubble, context),
            )),
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
            Some(bubble) => Ok(Some(
                formatter.format_digital_touch(message, &bubble, context),
            )),
            None => Err(MessageError::PlistParseError(
                PlistParseError::DigitalTouchError,
            )),
        };
    }

    // Poll messages use a different payload type
    if message.is_poll() {
        let poll = message.as_poll(config.data_source.db())?;
        return match poll {
            Some(poll) => Ok(Some(formatter.format_poll(&poll, context))),
            None => Err(MessageError::PlistParseError(PlistParseError::PollError)),
        };
    }

    let Some(payload) = message.payload_data(config.data_source.db()) else {
        return Ok(None);
    };

    let parsed = parse_ns_keyed_archiver(&payload)?;

    let rendered = if message.is_url() {
        let bubble = URLMessage::get_url_message_override(&parsed)?;
        match bubble {
            URLOverride::Normal(b) => formatter.format_url(message, &b, context),
            URLOverride::AppleMusic(b) => formatter.format_music(&b, context),
            URLOverride::Collaboration(b) => formatter.format_collaboration(&b, context),
            URLOverride::AppStore(b) => formatter.format_app_store(&b, context),
            URLOverride::SharedPlacemark(b) => formatter.format_placemark(&b, context),
        }
    } else {
        match AppMessage::from_map(&parsed) {
            Ok(bubble) => match balloon {
                CustomBalloon::Application(bundle_id) => {
                    formatter.format_generic_app(&bubble, bundle_id, attachments, context)
                }
                CustomBalloon::ApplePay => formatter.format_apple_pay(&bubble, context),
                CustomBalloon::Fitness => formatter.format_fitness(&bubble, context),
                CustomBalloon::Slideshow => formatter.format_slideshow(&bubble, context),
                CustomBalloon::CheckIn => formatter.format_check_in(&bubble, context),
                CustomBalloon::FindMy => formatter.format_find_my(&bubble, context),
                CustomBalloon::Polls
                | CustomBalloon::Handwriting
                | CustomBalloon::DigitalTouch
                | CustomBalloon::URL => unreachable!(),
            },
            Err(why) => return Err(MessageError::PlistParseError(why)),
        }
    };

    Ok(Some(rendered))
}
