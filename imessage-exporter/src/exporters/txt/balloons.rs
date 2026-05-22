use askama::Template;

use imessage_database::{
    message_types::{
        app::AppMessage, app_store::AppStoreMessage, collaboration::CollaborationMessage,
        digital_touch::DigitalTouch, handwriting::HandwrittenMessage, music::MusicMessage,
        placemark::PlacemarkMessage, polls::Poll, url::URLMessage,
    },
    tables::{attachment::Attachment, messages::Message},
};

use crate::{
    app::compatibility::attachment_manager::AttachmentManagerMode,
    exporters::{
        exporter::BalloonFormatter,
        shared::balloon::{CheckInState, resolve_check_in_footer},
    },
};

use super::{
    TXT,
    view_model::{
        AppStoreVM, ApplePayVM, CheckInVM, CollaborationVM, DigitalTouchVM, FindMyVM, FitnessVM,
        GenericAppVM, MusicVM, PlacemarkVM, PollOptionVM, PollVM, SlideshowVM, UrlVM,
    },
};

/// Render an Askama template and strip a single trailing newline, if present.
/// TXT balloon templates emit a `\n` after their final block (so they can be
/// chained) but the call site embeds them mid-stream, so the newline has to
/// come off.
fn render_trimmed<T: Template>(template: &T) -> String {
    let mut out = template.render().unwrap_or_default();
    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// MARK: Balloon
impl BalloonFormatter for TXT<'_> {
    fn format_url(&self, msg: &Message, balloon: &URLMessage) -> String {
        render_trimmed(&UrlVM {
            primary: balloon.get_url().or(msg.text.as_deref()).into(),
            title: balloon.title.into(),
            summary: balloon.summary.into(),
        })
    }

    fn format_music(&self, balloon: &MusicMessage) -> String {
        render_trimmed(&MusicVM {
            lyrics: balloon.lyrics.as_deref(),
            track_name: balloon.track_name.into(),
            album: balloon.album.into(),
            artist: balloon.artist.into(),
            url: balloon.url.into(),
        })
    }

    fn format_collaboration(&self, balloon: &CollaborationMessage) -> String {
        let name = balloon.app_name.or(balloon.bundle_id);
        let has_label = name.is_some_and(|n| !n.is_empty());
        render_trimmed(&CollaborationVM {
            name: name.into(),
            has_label,
            title: balloon.title.into(),
            url: balloon.get_url().into(),
        })
    }

    fn format_app_store(&self, balloon: &AppStoreMessage) -> String {
        render_trimmed(&AppStoreVM {
            app_name: balloon.app_name.into(),
            description: balloon.description.into(),
            platform: balloon.platform.into(),
            genre: balloon.genre.into(),
            url: balloon.url.into(),
        })
    }

    fn format_placemark(&self, balloon: &PlacemarkMessage) -> String {
        render_trimmed(&PlacemarkVM {
            place_name: balloon.place_name.into(),
            url: balloon.get_url().into(),
            name: balloon.placemark.name.into(),
            address: balloon.placemark.address.into(),
            state: balloon.placemark.state.into(),
            city: balloon.placemark.city.into(),
            iso_country_code: balloon.placemark.iso_country_code.into(),
            postal_code: balloon.placemark.postal_code.into(),
            country: balloon.placemark.country.into(),
            street: balloon.placemark.street.into(),
            sub_administrative_area: balloon.placemark.sub_administrative_area.into(),
            sub_locality: balloon.placemark.sub_locality.into(),
        })
    }

    fn format_handwriting(&self, msg: &Message, balloon: &HandwrittenMessage) -> String {
        match self.config.options.attachment_manager.mode {
            AttachmentManagerMode::Disabled => balloon.render_ascii(40),
            _ => self
                .config
                .options
                .attachment_manager
                .handle_handwriting(msg, balloon, self.config)
                .map(|filepath| self.config.relative_path(&filepath))
                .unwrap_or_else(|| balloon.render_ascii(40)),
        }
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
            caption: balloon.caption.into(),
            ldtext: balloon.ldtext.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_fitness(&self, balloon: &AppMessage) -> String {
        FitnessVM {
            app_name: balloon.app_name.into(),
            ldtext: balloon.ldtext.into(),
        }
        .render()
        .unwrap_or_default()
    }

    fn format_slideshow(&self, balloon: &AppMessage) -> String {
        SlideshowVM {
            ldtext: balloon.ldtext.into(),
            url: balloon.url.into(),
        }
        .render()
        .unwrap_or_default()
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
            CheckInState::Expected(at) => format!("Expected at {at}"),
            CheckInState::WasExpected(at) => format!("Was expected at {at}"),
            CheckInState::CheckedIn(at) => format!("Checked in at {at}"),
        });

        CheckInVM {
            caption: balloon.caption.unwrap_or("Check In"),
            footer,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_poll(&self, poll: &Poll) -> String {
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

        render_trimmed(&PollVM { options })
    }

    fn format_generic_app(
        &self,
        balloon: &AppMessage,
        bundle_id: &str,
        _: &mut Vec<Attachment>,
        _: &Message,
    ) -> String {
        render_trimmed(&GenericAppVM {
            name: balloon.app_name.unwrap_or(bundle_id),
            title: balloon.title.into(),
            subtitle: balloon.subtitle.into(),
            caption: balloon.caption.into(),
            subcaption: balloon.subcaption.into(),
            trailing_caption: balloon.trailing_caption.into(),
            trailing_subcaption: balloon.trailing_subcaption.into(),
        })
    }
}
