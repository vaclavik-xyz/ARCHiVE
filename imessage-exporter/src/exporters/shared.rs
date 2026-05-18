use askama::Template;

use imessage_database::{
    error::{message::MessageError, plist::PlistParseError},
    message_types::{
        app::AppMessage,
        digital_touch,
        expressives::{BubbleEffect, Expressive, ScreenEffect},
        handwriting::HandwrittenMessage,
        url::URLMessage,
        variants::{BalloonProvider, CustomBalloon, URLOverride, Variant},
    },
    tables::{attachment::Attachment, messages::Message},
    util::{
        dates::{TIMESTAMP_FACTOR, format, get_local_time},
        plist::parse_ns_keyed_archiver,
    },
};

use crate::{app::runtime::Config, exporters::exporter::BalloonFormatter};

// MARK: Expressive

/// Format the expressive send style for a message. This is identical
/// across all export formats.
pub(super) fn format_expressive(msg: &Message) -> &str {
    match msg.get_expressive() {
        Expressive::Screen(effect) => match effect {
            ScreenEffect::Confetti => "Sent with Confetti",
            ScreenEffect::Echo => "Sent with Echo",
            ScreenEffect::Fireworks => "Sent with Fireworks",
            ScreenEffect::Balloons => "Sent with Balloons",
            ScreenEffect::Heart => "Sent with Heart",
            ScreenEffect::Lasers => "Sent with Lasers",
            ScreenEffect::ShootingStar => "Sent with Shooting Star",
            ScreenEffect::Sparkles => "Sent with Sparkles",
            ScreenEffect::Spotlight => "Sent with Spotlight",
        },
        Expressive::Bubble(effect) => match effect {
            BubbleEffect::Slam => "Sent with Slam",
            BubbleEffect::Loud => "Sent with Loud",
            BubbleEffect::Gentle => "Sent with Gentle",
            BubbleEffect::InvisibleInk => "Sent with Invisible Ink",
        },
        Expressive::Unknown(effect) => effect,
        Expressive::None => "",
    }
}

// MARK: Time

/// Compute the formatted timestamp and read receipt for a message.
/// Returns `(formatted_date, read_receipt)` where `read_receipt` is
/// empty if there is no read receipt data.
pub(super) fn message_time(config: &Config, message: &Message) -> (String, String) {
    let date = match message.date(config.offset) {
        Ok(d) => format(&d),
        Err(why) => why.to_string(),
    };
    let mut read_receipt = String::new();
    if let Some(time) = message.time_until_read(config.offset)
        && !time.is_empty()
    {
        let who = if message.is_from_me() {
            "them"
        } else {
            config.options.custom_name.as_deref().unwrap_or("you")
        };
        read_receipt = format!("(Read by {who} after {time})");
    }
    (date, read_receipt)
}

// MARK: Templates

/// Render an Askama template and strip a single trailing newline, if present.
/// TXT balloon templates emit a `\n` after their final block so they can be
/// chained, but callers embed them mid-stream and don't want that newline.
pub(super) fn render_trimmed<T: Template>(template: &T) -> String {
    let mut out = template.render().unwrap_or_default();
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// MARK: Check In

/// Parse a Check In timestamp from a `parse_query_string` value and render it
/// with the given prefix (e.g. `"Checked in at "`). Returns `None` if the
/// value is unparseable.
pub(super) fn format_check_in_caption(date_str: &str, prefix: &str) -> Option<String> {
    let date_stamp = date_str.parse::<f64>().unwrap_or(0.) as i64 * TIMESTAMP_FACTOR;
    let date_time = get_local_time(date_stamp, 0).ok()?;
    Some(format!("{prefix}{}", format(&date_time)))
}

// MARK: App Dispatch

/// Drive the App-balloon decision tree shared by both exporters: pick the
/// right payload source (raw vs keyed-archiver), parse it, and dispatch to
/// the matching [`BalloonFormatter`] method.
///
/// Returns `Ok(None)` when the message has no payload data; callers handle
/// that case themselves (HTML wraps the message text in an `<a>` card, TXT
/// just emits the text).
pub(super) fn dispatch_app_balloon<T, F>(
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

#[cfg(test)]
mod tests {
    use crate::{
        Config, Options,
        app::export_type::ExportType,
        exporters::shared::{format_expressive, message_time},
    };

    fn make_config_with_custom_name(custom_name: Option<&str>) -> Config {
        let mut options = Options::fake_options(ExportType::Html);
        options.custom_name = custom_name.map(str::to_string);
        Config::fake_app(options)
    }

    // MARK: format_expressive

    #[test]
    fn format_expressive_returns_empty_when_none() {
        let mut msg = Config::fake_message();
        msg.expressive_send_style_id = None;
        assert_eq!(format_expressive(&msg), "");
    }

    #[test]
    fn format_expressive_screen_effects() {
        let cases = [
            (
                "com.apple.messages.effect.CKConfettiEffect",
                "Sent with Confetti",
            ),
            ("com.apple.messages.effect.CKEchoEffect", "Sent with Echo"),
            (
                "com.apple.messages.effect.CKFireworksEffect",
                "Sent with Fireworks",
            ),
            (
                "com.apple.messages.effect.CKHappyBirthdayEffect",
                "Sent with Balloons",
            ),
            ("com.apple.messages.effect.CKHeartEffect", "Sent with Heart"),
            (
                "com.apple.messages.effect.CKLasersEffect",
                "Sent with Lasers",
            ),
            (
                "com.apple.messages.effect.CKShootingStarEffect",
                "Sent with Shooting Star",
            ),
            (
                "com.apple.messages.effect.CKSparklesEffect",
                "Sent with Sparkles",
            ),
            (
                "com.apple.messages.effect.CKSpotlightEffect",
                "Sent with Spotlight",
            ),
        ];
        for (style_id, expected) in cases {
            let mut msg = Config::fake_message();
            msg.expressive_send_style_id = Some(style_id.to_string());
            assert_eq!(format_expressive(&msg), expected, "for {style_id}");
        }
    }

    #[test]
    fn format_expressive_bubble_effects() {
        let cases = [
            (
                "com.apple.MobileSMS.expressivesend.gentle",
                "Sent with Gentle",
            ),
            (
                "com.apple.MobileSMS.expressivesend.impact",
                "Sent with Slam",
            ),
            (
                "com.apple.MobileSMS.expressivesend.invisibleink",
                "Sent with Invisible Ink",
            ),
            ("com.apple.MobileSMS.expressivesend.loud", "Sent with Loud"),
        ];
        for (style_id, expected) in cases {
            let mut msg = Config::fake_message();
            msg.expressive_send_style_id = Some(style_id.to_string());
            assert_eq!(format_expressive(&msg), expected, "for {style_id}");
        }
    }

    #[test]
    fn format_expressive_unknown_returns_raw_id() {
        let mut msg = Config::fake_message();
        msg.expressive_send_style_id = Some("com.apple.messages.effect.NotARealEffect".to_string());
        assert_eq!(
            format_expressive(&msg),
            "com.apple.messages.effect.NotARealEffect"
        );
    }

    // MARK: message_time

    #[test]
    fn message_time_no_read_receipt() {
        let config = make_config_with_custom_name(None);
        let mut msg = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        msg.date = 674526582885055488;
        // date_read=0 yields no read receipt
        let (date, read) = message_time(&config, &msg);
        assert_eq!(date, "May 17, 2022  5:29:42 PM");
        assert!(read.is_empty(), "expected empty read receipt, got {read:?}");
    }

    #[test]
    fn message_time_read_from_them_uses_default_you() {
        let config = make_config_with_custom_name(None);
        let mut msg = Config::fake_message();
        msg.date = 674526582885055488;
        msg.date_delivered = 674526582885055488;
        msg.date_read = 674530231992568192;
        // is_from_me defaults to false → reader is "you"
        let (date, read) = message_time(&config, &msg);
        assert_eq!(date, "May 17, 2022  5:29:42 PM");
        assert_eq!(read, "(Read by you after 1 hour, 49 seconds)");
    }

    #[test]
    fn message_time_read_from_them_uses_custom_name() {
        let config = make_config_with_custom_name(Some("Chris"));
        let mut msg = Config::fake_message();
        msg.date = 674526582885055488;
        msg.date_delivered = 674526582885055488;
        msg.date_read = 674530231992568192;
        let (_, read) = message_time(&config, &msg);
        assert_eq!(read, "(Read by Chris after 1 hour, 49 seconds)");
    }

    #[test]
    fn message_time_read_from_me_uses_them() {
        let config = make_config_with_custom_name(Some("Chris"));
        let mut msg = Config::fake_message();
        // Sent at 8:29:42 PM, delivered ~1 hour later. For sent messages,
        // time_until_read measures sent → delivered (not date_read).
        msg.date = 674526582885055488;
        msg.date_delivered = 674530231992568192;
        msg.is_from_me = true;
        // When the message is from me, the reader on the other side is "them"
        // regardless of custom_name (custom_name renames you, not the recipient).
        let (_, read) = message_time(&config, &msg);
        assert_eq!(read, "(Read by them after 1 hour, 49 seconds)");
    }
}
