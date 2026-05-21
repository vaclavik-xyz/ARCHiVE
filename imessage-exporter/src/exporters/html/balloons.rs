use std::collections::HashMap;

use imessage_database::{
    message_types::{
        app::AppMessage, app_store::AppStoreMessage, collaboration::CollaborationMessage,
        digital_touch::DigitalTouch, handwriting::HandwrittenMessage, music::MusicMessage,
        placemark::PlacemarkMessage, polls::Poll, url::URLMessage,
    },
    tables::{
        attachment::Attachment,
        messages::{Message, models::AttachmentMeta},
    },
};

use askama::Template;

use crate::exporters::{
    exporter::{BalloonFormatter, MessageFormatter},
    shared::balloon::format_check_in_caption,
};

use super::{
    HTML,
    view_model::{
        AppCardVM, AppStoreVM, ApplePayVM, CheckInVM, CollaborationVM, DigitalTouchVM, FindMyVM,
        MusicVM, PlacemarkVM, PollOptionVM, PollVM, UrlVM,
    },
};

// MARK: Balloons
impl BalloonFormatter for HTML<'_> {
    fn format_url(&self, msg: &Message, balloon: &URLMessage) -> String {
        let balloon_url = balloon.get_url();
        let msg_text = msg.text.as_deref();
        UrlVM {
            wrapper_url: balloon_url.or(msg_text),
            name: balloon.site_name.or(balloon_url).or(msg_text),
            images: &balloon.images,
            lazy: !self.config.options.no_lazy,
            title: balloon.title,
            summary: balloon.summary,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_music(&self, balloon: &MusicMessage) -> String {
        MusicVM {
            track_name: balloon.track_name,
            preview: balloon.preview,
            lyrics: balloon.lyrics.as_deref(),
            url: balloon.url,
            artist: balloon.artist,
            album: balloon.album,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_collaboration(&self, balloon: &CollaborationMessage) -> String {
        CollaborationVM {
            name: balloon.app_name.or(balloon.bundle_id),
            wrapper_url: balloon.url,
            title: balloon.title,
            footer_url: balloon.get_url(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_app_store(&self, balloon: &AppStoreMessage) -> String {
        AppStoreVM {
            app_name: balloon.app_name,
            url: balloon.url,
            description: balloon.description,
            platform: balloon.platform,
            genre: balloon.genre,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_placemark(&self, balloon: &PlacemarkMessage) -> String {
        let url = balloon.get_url();
        PlacemarkVM {
            url,
            name: balloon.place_name.or(url),
            address: balloon.placemark.address,
            postal_code: balloon.placemark.postal_code,
            country: balloon.placemark.country,
            sub_administrative_area: balloon.placemark.sub_administrative_area,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_handwriting(&self, _: &Message, balloon: &HandwrittenMessage) -> String {
        balloon.render_svg()
    }

    fn format_digital_touch(&self, _: &Message, balloon: &DigitalTouch) -> String {
        DigitalTouchVM {
            debug: format!("{balloon:?}"),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_apple_pay(&self, balloon: &AppMessage) -> String {
        ApplePayVM {
            app_name: balloon.app_name,
            ldtext: balloon.ldtext,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_fitness(&self, balloon: &AppMessage) -> String {
        self.balloon_to_html(balloon, "Fitness", None)
    }

    fn format_slideshow(&self, balloon: &AppMessage) -> String {
        self.balloon_to_html(balloon, "Slideshow", None)
    }

    fn format_find_my(&self, balloon: &AppMessage) -> String {
        FindMyVM {
            app_name: balloon.app_name,
            ldtext: balloon.ldtext,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_check_in(&self, balloon: &AppMessage) -> String {
        let metadata: HashMap<&str, &str> = balloon.parse_query_string();
        let footer = if let Some(date_str) = metadata.get("estimatedEndTime") {
            format_check_in_caption(date_str, "Expected around ")
        } else if let Some(date_str) = metadata.get("triggerTime") {
            format_check_in_caption(date_str, "Was expected around ")
        } else if let Some(date_str) = metadata.get("sendDate") {
            format_check_in_caption(date_str, "Checked in at ")
        } else {
            None
        };

        CheckInVM {
            name: balloon.app_name.unwrap_or("Check In"),
            ldtext: balloon.ldtext,
            footer,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_poll(&self, poll: &Poll) -> String {
        let max_votes = poll
            .order
            .iter()
            .filter_map(|option_id| poll.options.get(option_id))
            .map(|option| option.votes.len())
            .max()
            .unwrap_or(0);

        let options = poll
            .order
            .iter()
            .filter_map(|id| poll.options.get(id))
            .map(|opt| {
                let vote_count = opt.votes.len();
                let bar_width = (vote_count * 100).checked_div(max_votes).unwrap_or(0);
                PollOptionVM {
                    text: &opt.text,
                    vote_count,
                    bar_html: format!(
                        "<div class=\"vote-bar\" style=\"width: {bar_width}%;\"></div>"
                    ),
                    voters: opt.votes.iter().map(|v| v.voter.as_str()).collect(),
                }
            })
            .collect();

        PollVM { options }.render().unwrap_or_default()
    }

    fn format_generic_app(
        &self,
        balloon: &AppMessage,
        bundle_id: &str,
        attachments: &mut Vec<Attachment>,
        msg: &Message,
    ) -> String {
        let attachment_html = if balloon.image.is_none() {
            attachments.get_mut(0).map(|attachment| {
                self.format_attachment(attachment, msg, &AttachmentMeta::default())
                    .unwrap_or_default()
            })
        } else {
            None
        };
        self.balloon_to_html(balloon, bundle_id, attachment_html)
    }
}

impl HTML<'_> {
    fn balloon_to_html(
        &self,
        balloon: &AppMessage,
        bundle_id: &str,
        attachment_html: Option<String>,
    ) -> String {
        AppCardVM {
            url: balloon.url,
            image: balloon.image,
            attachment_html,
            name: balloon.app_name.unwrap_or(bundle_id),
            title: balloon.title,
            subtitle: balloon.subtitle,
            ldtext: balloon.ldtext,
            caption: balloon.caption,
            subcaption: balloon.subcaption,
            trailing_caption: balloon.trailing_caption,
            trailing_subcaption: balloon.trailing_subcaption,
        }
        .render()
        .unwrap_or_default()
    }
}
