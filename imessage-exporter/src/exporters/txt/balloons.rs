use std::collections::HashMap;

use askama::Template;

use imessage_database::{
    message_types::{
        app::AppMessage,
        app_store::AppStoreMessage,
        collaboration::CollaborationMessage,
        digital_touch::DigitalTouch,
        handwriting::HandwrittenMessage,
        music::MusicMessage,
        placemark::PlacemarkMessage,
        polls::Poll,
        url::URLMessage,
    },
    tables::{attachment::Attachment, messages::Message},
    util::dates::{TIMESTAMP_FACTOR, format, get_local_time},
};

use crate::{
    app::compatibility::attachment_manager::AttachmentManagerMode,
    exporters::exporter::BalloonFormatter,
};

use super::{
    TXT,
    view_model::{
        ApplePayVM, AppStoreVM, CheckInVM, CollaborationVM, DigitalTouchVM, FindMyVM, FitnessVM,
        GenericAppVM, MusicVM, PlacemarkVM, PollOptionVM, PollVM, SlideshowVM, UrlVM,
    },
};

// MARK: Balloon
impl<'a> BalloonFormatter<&'a str> for TXT<'a> {
    fn format_url(&self, msg: &Message, balloon: &URLMessage, indent: &str) -> String {
        let mut out = UrlVM {
            indent,
            primary: balloon.get_url().or(msg.text.as_deref()),
            title: balloon.title,
            summary: balloon.summary,
        }
        .render()
        .unwrap_or_default();
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn format_music(&self, balloon: &MusicMessage, indent: &str) -> String {
        MusicVM {
            indent,
            lyrics: balloon.lyrics.as_deref(),
            track_name: balloon.track_name,
            album: balloon.album,
            artist: balloon.artist,
            url: balloon.url,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_collaboration(&self, balloon: &CollaborationMessage, indent: &str) -> String {
        let name = balloon.app_name.or(balloon.bundle_id);
        let has_label = !indent.is_empty() || name.is_some_and(|n| !n.is_empty());
        let mut out = CollaborationVM {
            indent,
            name,
            has_label,
            title: balloon.title,
            url: balloon.get_url(),
        }
        .render()
        .unwrap_or_default();
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn format_app_store(&self, balloon: &AppStoreMessage, indent: &'a str) -> String {
        let mut out = AppStoreVM {
            indent,
            app_name: balloon.app_name,
            description: balloon.description,
            platform: balloon.platform,
            genre: balloon.genre,
            url: balloon.url,
        }
        .render()
        .unwrap_or_default();
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn format_placemark(&self, balloon: &PlacemarkMessage, indent: &'a str) -> String {
        let mut out = PlacemarkVM {
            indent,
            place_name: balloon.place_name,
            url: balloon.get_url(),
            name: balloon.placemark.name,
            address: balloon.placemark.address,
            state: balloon.placemark.state,
            city: balloon.placemark.city,
            iso_country_code: balloon.placemark.iso_country_code,
            postal_code: balloon.placemark.postal_code,
            country: balloon.placemark.country,
            street: balloon.placemark.street,
            sub_administrative_area: balloon.placemark.sub_administrative_area,
            sub_locality: balloon.placemark.sub_locality,
        }
        .render()
        .unwrap_or_default();
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }

    fn format_handwriting(
        &self,
        msg: &Message,
        balloon: &HandwrittenMessage,
        indent: &str,
    ) -> String {
        match self.config.options.attachment_manager.mode {
            AttachmentManagerMode::Disabled => balloon
                .render_ascii(40)
                .replace('\n', &format!("{indent}\n")),
            _ => self
                .config
                .options
                .attachment_manager
                .handle_handwriting(msg, balloon, self.config)
                .map(|filepath| self.config.relative_path(&filepath))
                .map(|filepath| format!("{indent}{filepath}"))
                .unwrap_or_else(|| {
                    balloon
                        .render_ascii(40)
                        .replace('\n', &format!("{indent}\n"))
                }),
        }
    }

    fn format_digital_touch(&self, _: &Message, balloon: &DigitalTouch, indent: &str) -> String {
        DigitalTouchVM {
            indent,
            debug: format!("{balloon:?}"),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_apple_pay(&self, balloon: &AppMessage, indent: &str) -> String {
        ApplePayVM {
            indent,
            caption: balloon.caption,
            ldtext: balloon.ldtext,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_fitness(&self, balloon: &AppMessage, indent: &str) -> String {
        FitnessVM {
            indent,
            app_name: balloon.app_name,
            ldtext: balloon.ldtext,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_slideshow(&self, balloon: &AppMessage, indent: &str) -> String {
        SlideshowVM {
            indent,
            ldtext: balloon.ldtext,
            url: balloon.url,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_find_my(&self, balloon: &AppMessage, indent: &'a str) -> String {
        FindMyVM {
            indent,
            app_name: balloon.app_name,
            ldtext: balloon.ldtext,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_check_in(&self, balloon: &AppMessage, indent: &'a str) -> String {
        let metadata: HashMap<&str, &str> = balloon.parse_query_string();
        let footer = if let Some(date_str) = metadata.get("estimatedEndTime") {
            format_check_in_caption(date_str, "Expected at ")
        } else if let Some(date_str) = metadata.get("triggerTime") {
            format_check_in_caption(date_str, "Was expected at ")
        } else if let Some(date_str) = metadata.get("sendDate") {
            format_check_in_caption(date_str, "Checked in at ")
        } else {
            None
        };

        CheckInVM {
            indent,
            caption: balloon.caption.unwrap_or("Check In"),
            footer,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_poll(&self, poll: &Poll, indent: &'a str) -> String {
        let options = poll
            .order
            .iter()
            .filter_map(|id| poll.options.get(id))
            .map(|opt| PollOptionVM {
                text: &opt.text,
                vote_count: opt.votes.len(),
                voters: opt.votes.iter().map(|v| v.voter.as_str()).collect(),
            })
            .collect();

        PollVM { indent, options }.render().unwrap_or_default()
    }

    fn format_generic_app(
        &self,
        balloon: &AppMessage,
        bundle_id: &str,
        _: &mut Vec<Attachment>,
        indent: &str,
    ) -> String {
        let name = balloon.app_name.unwrap_or(bundle_id);
        let has_label = !indent.is_empty() || !name.is_empty();
        let mut out = GenericAppVM {
            indent,
            name,
            has_label,
            title: balloon.title,
            subtitle: balloon.subtitle,
            caption: balloon.caption,
            subcaption: balloon.subcaption,
            trailing_caption: balloon.trailing_caption,
            trailing_subcaption: balloon.trailing_subcaption,
        }
        .render()
        .unwrap_or_default();
        // Match legacy `out_s.strip_suffix('\n').unwrap_or(&out_s).to_string()`.
        if out.ends_with('\n') {
            out.pop();
        }
        out
    }
}

fn format_check_in_caption(date_str: &str, prefix: &str) -> Option<String> {
    let date_stamp = date_str.parse::<f64>().unwrap_or(0.) as i64 * TIMESTAMP_FACTOR;
    let date_time = get_local_time(date_stamp, 0).ok()?;
    Some(format!("{prefix}{}", format(&date_time)))
}
