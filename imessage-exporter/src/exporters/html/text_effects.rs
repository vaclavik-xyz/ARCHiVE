use std::borrow::Cow;

use imessage_database::message_types::text_effects::{Animation, Style, TextEffect, Unit};

use crate::{app::sanitizers::sanitize_html, exporters::exporter::TextEffectFormatter};

use super::HTML;

// MARK: Text Effects
impl<'a> TextEffectFormatter<'a> for HTML<'a> {
    fn format_effect(&'a self, text: &'a str, effect: &'a TextEffect) -> Cow<'a, str> {
        match effect {
            TextEffect::Default => Cow::Borrowed(text),
            TextEffect::Mention(mentioned) => Cow::Owned(self.format_mention(text, mentioned)),
            TextEffect::Link(url) => Cow::Owned(self.format_link(text, url)),
            TextEffect::OTP => Cow::Owned(self.format_otp(text)),
            TextEffect::Styles(styles) => Cow::Owned(self.format_styles(text, styles)),
            TextEffect::Animated(animation) => Cow::Owned(self.format_animated(text, animation)),
            TextEffect::Conversion(unit) => Cow::Owned(self.format_conversion(text, unit)),
        }
    }

    fn format_mention(&self, text: &str, mentioned: &str) -> String {
        format!(
            "<span title=\"{}\"><b>{text}</b></span>",
            sanitize_html(mentioned)
        )
    }

    fn format_link(&self, text: &str, url: &str) -> String {
        format!("<a href=\"{}\">{text}</a>", sanitize_html(url))
    }

    fn format_otp(&self, text: &str) -> String {
        format!("<u>{text}</u>")
    }

    fn format_conversion(&self, text: &str, _: &Unit) -> String {
        format!("<u>{text}</u>")
    }

    fn format_styles(&self, text: &str, styles: &[Style]) -> String {
        let (prefix, suffix): (String, String) = styles.iter().rev().fold(
            (String::new(), String::new()),
            |(mut prefix, mut suffix), style| {
                let (open, close) = match style {
                    Style::Bold => ("<b>", "</b>"),
                    Style::Italic => ("<i>", "</i>"),
                    Style::Strikethrough => ("<s>", "</s>"),
                    Style::Underline => ("<u>", "</u>"),
                };
                prefix.push_str(open);
                suffix.insert_str(0, close);
                (prefix, suffix)
            },
        );

        format!("{prefix}{text}{suffix}")
    }

    fn format_animated(&self, text: &str, animation: &Animation) -> String {
        // animation:? is a Rust enum variant name (safe), text is pre-sanitized
        format!("<span class=\"animation{animation:?}\">{text}</span>")
    }
}
