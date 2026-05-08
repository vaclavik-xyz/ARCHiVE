use imessage_database::{
    message_types::expressives::{BubbleEffect, Expressive, ScreenEffect},
    tables::messages::Message,
    util::dates::format,
};

use crate::app::runtime::Config;

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
            ("com.apple.messages.effect.CKConfettiEffect", "Sent with Confetti"),
            ("com.apple.messages.effect.CKEchoEffect", "Sent with Echo"),
            ("com.apple.messages.effect.CKFireworksEffect", "Sent with Fireworks"),
            ("com.apple.messages.effect.CKHappyBirthdayEffect", "Sent with Balloons"),
            ("com.apple.messages.effect.CKHeartEffect", "Sent with Heart"),
            ("com.apple.messages.effect.CKLasersEffect", "Sent with Lasers"),
            ("com.apple.messages.effect.CKShootingStarEffect", "Sent with Shooting Star"),
            ("com.apple.messages.effect.CKSparklesEffect", "Sent with Sparkles"),
            ("com.apple.messages.effect.CKSpotlightEffect", "Sent with Spotlight"),
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
            ("com.apple.MobileSMS.expressivesend.gentle", "Sent with Gentle"),
            ("com.apple.MobileSMS.expressivesend.impact", "Sent with Slam"),
            ("com.apple.MobileSMS.expressivesend.invisibleink", "Sent with Invisible Ink"),
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
