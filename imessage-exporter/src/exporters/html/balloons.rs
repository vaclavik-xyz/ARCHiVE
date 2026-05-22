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
    shared::balloon::{CheckInState, resolve_check_in_footer},
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
            wrapper_url: balloon_url.or(msg_text).into(),
            name: balloon.site_name.or(balloon_url).or(msg_text).into(),
            images: &balloon.images,
            lazy: !self.config.options.no_lazy,
            title: balloon.title.into(),
            summary: balloon.summary.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_music(&self, balloon: &MusicMessage) -> String {
        MusicVM {
            track_name: balloon.track_name.into(),
            preview: balloon.preview.into(),
            lyrics: balloon.lyrics.as_deref(),
            url: balloon.url.into(),
            artist: balloon.artist.into(),
            album: balloon.album.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_collaboration(&self, balloon: &CollaborationMessage) -> String {
        CollaborationVM {
            name: balloon.app_name.or(balloon.bundle_id).into(),
            wrapper_url: balloon.url.into(),
            title: balloon.title.into(),
            footer_url: balloon.get_url().into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_app_store(&self, balloon: &AppStoreMessage) -> String {
        AppStoreVM {
            app_name: balloon.app_name.into(),
            url: balloon.url.into(),
            description: balloon.description.into(),
            platform: balloon.platform.into(),
            genre: balloon.genre.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_placemark(&self, balloon: &PlacemarkMessage) -> String {
        let url = balloon.get_url();
        PlacemarkVM {
            url: url.into(),
            name: balloon.place_name.or(url).into(),
            address: balloon.placemark.address.into(),
            postal_code: balloon.placemark.postal_code.into(),
            country: balloon.placemark.country.into(),
            sub_administrative_area: balloon.placemark.sub_administrative_area.into(),
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
            app_name: balloon.app_name.into(),
            ldtext: balloon.ldtext.into(),
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
            app_name: balloon.app_name.into(),
            ldtext: balloon.ldtext.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_check_in(&self, balloon: &AppMessage) -> String {
        let footer = resolve_check_in_footer(balloon).map(|f| match f {
            CheckInState::Expected(at) => format!("Expected around {at}"),
            CheckInState::WasExpected(at) => format!("Was expected around {at}"),
            CheckInState::CheckedIn(at) => format!("Checked in at {at}"),
        });

        CheckInVM {
            name: balloon.app_name.unwrap_or("Check In"),
            ldtext: balloon.ldtext.into(),
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
            url: balloon.url.into(),
            image: balloon.image.into(),
            attachment_html,
            name: balloon.app_name.unwrap_or(bundle_id),
            title: balloon.title.into(),
            subtitle: balloon.subtitle.into(),
            ldtext: balloon.ldtext.into(),
            caption: balloon.caption.into(),
            subcaption: balloon.subcaption.into(),
            trailing_caption: balloon.trailing_caption.into(),
            trailing_subcaption: balloon.trailing_subcaption.into(),
        }
        .render()
        .unwrap_or_default()
    }
}
