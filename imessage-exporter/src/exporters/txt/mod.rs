use std::{collections::HashMap, fs::File, io::BufWriter};

use crate::{
    app::{
        compatibility::attachment_manager::AttachmentManagerMode, error::RuntimeError,
        progress::ExportProgress, runtime::Config,
    },
    exporters::{
        exporter::{ATTACHMENT_NO_FILENAME, Exporter, MessageFormatter},
        shared::{
            announcement::resolve_announcement,
            balloon::dispatch_app_balloon,
            driver::{MessageWriter, get_or_create_file_for, run_export},
            edited::{EditDiff, NormalizedEdit, normalize_edited},
            format::{format_expressive, message_time},
        },
    },
};

use imessage_database::{
    error::{message::MessageError, table::TableError},
    message_types::{
        edited::EditedMessage,
        sticker::StickerDecoration,
        variants::{Tapback, TapbackAction, Variant},
    },
    tables::{
        attachment::{Attachment, MediaType},
        messages::{
            Message,
            models::{AttachmentMeta, BubbleComponent, TextAttributes},
        },
        table::{FITNESS_RECEIVER, ORPHANED, YOU},
    },
};

mod balloons;
mod view_model;

use askama::Template;
use view_model::{
    AnnouncementBody, AnnouncementVM, AttachmentVM, EditedKind, EditedRow, EditedVM, MessagePartVM,
    MessageVM, PartBody, RepliesVM, StickerVM, TapbackKind, TapbackVM, TapbacksVM,
};

pub struct TXT<'a> {
    /// Data that is setup from the application's runtime
    pub config: &'a Config,
    /// Handles to files we want to write messages to
    /// Map of resolved chatroom file location to a buffered writer
    pub files: HashMap<String, BufWriter<File>>,
    /// Writer instance for orphaned messages
    pub orphaned: BufWriter<File>,
    /// Progress Bar model for alerting the user about current export state
    pb: ExportProgress,
}

// MARK: Exporter
impl<'a> Exporter<'a> for TXT<'a> {
    fn new(config: &'a Config) -> Result<Self, RuntimeError> {
        let mut orphaned = config.options.export_path.clone();
        orphaned.push(ORPHANED);
        orphaned.set_extension("txt");

        let file = File::options().append(true).create(true).open(&orphaned)?;

        Ok(TXT {
            config,
            files: HashMap::new(),
            orphaned: BufWriter::new(file),
            pb: ExportProgress::new(),
        })
    }

    fn iter_messages(&mut self) -> Result<(), RuntimeError> {
        run_export(self)
    }

    fn get_or_create_file(
        &mut self,
        message: &Message,
    ) -> Result<&mut BufWriter<File>, RuntimeError> {
        get_or_create_file_for(self, message)
    }
}

// MARK: Driver hooks
impl<'a> MessageWriter<'a> for TXT<'a> {
    const LABEL: &'static str = "txt";
    const BUFFER_CAPACITY: usize = 1024;

    fn config(&self) -> &'a Config {
        self.config
    }

    fn pb(&self) -> &ExportProgress {
        &self.pb
    }

    fn files_mut(&mut self) -> &mut HashMap<String, BufWriter<File>> {
        &mut self.files
    }

    fn orphaned_mut(&mut self) -> &mut BufWriter<File> {
        &mut self.orphaned
    }

    fn write_file_header(_file: &mut BufWriter<File>) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn write_file_footer(_file: &mut BufWriter<File>) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn footer_notice() -> Option<&'static str> {
        None
    }
}

// MARK: Writer
impl<'a> MessageFormatter<'a> for TXT<'a> {
    fn format_attachment(
        &self,
        attachment: &'a mut Attachment,
        message: &Message,
        metadata: &AttachmentMeta,
    ) -> Result<String, &'a str> {
        // When encoding videos, alert the user that the time estimate may be inaccurate
        let will_encode = matches!(attachment.mime_type(), MediaType::Video(_))
            && matches!(
                self.config.options.attachment_manager.mode,
                AttachmentManagerMode::Full
            );

        if will_encode {
            self.pb
                .set_busy_style("Encoding video, estimates paused...".to_string());
        }

        // Copy the file, if requested
        let handle_result = self.config.options.attachment_manager.handle_attachment(
            message,
            attachment,
            self.config,
        );

        if will_encode {
            self.pb.set_default_style();
        }

        handle_result.ok_or(attachment.filename().ok_or(ATTACHMENT_NO_FILENAME)?)?;

        Ok(AttachmentVM {
            embed_path: self.config.message_attachment_path(attachment),
            transcription: metadata.transcription.as_deref(),
        }
        .render()
        .unwrap_or_default())
    }

    fn format_sticker(&self, sticker: &'a mut Attachment, message: &Message) -> String {
        let who = self.config.who(
            message.handle_id,
            message.is_from_me(),
            &message.destination_caller_id,
        );
        let (path, has_source) =
            match self.format_attachment(sticker, message, &AttachmentMeta::default()) {
                Ok(p) => (p, true),
                Err(e) => (e.to_string(), false),
            };

        let decoration = if has_source {
            sticker.get_sticker_decoration(
                self.config.data_source.db(),
                &self.config.options.platform,
                &self.config.options.db_path,
                self.config.options.attachment_root.as_deref(),
            )
        } else {
            None
        };

        let (effect_prefix, suffix) = match decoration {
            Some(StickerDecoration::GenmojiPrompt(prompt)) => {
                (None, Some(format!(" (Genmoji prompt: {prompt})")))
            }
            Some(StickerDecoration::Memoji) => (None, Some(" (App: Memoji)".to_string())),
            Some(StickerDecoration::Effect(effect)) => (Some(format!("{effect} ")), None),
            Some(StickerDecoration::AppName(name)) => (None, Some(format!(" (App: {name})"))),
            None => (None, None),
        };

        StickerVM {
            effect_prefix,
            who,
            path,
            suffix,
        }
        .render()
        .unwrap_or_default()
    }

    fn format_app(
        &self,
        message: &'a Message,
        attachments: &mut Vec<Attachment>,
    ) -> Result<String, MessageError> {
        dispatch_app_balloon(self, message, attachments, self.config)
    }

    fn format_tapback(&self, msg: &Message) -> Result<String, TableError> {
        let Variant::Tapback(_, action, tapback) = msg.variant() else {
            unreachable!()
        };
        if let TapbackAction::Removed = action {
            return Ok(String::new());
        }
        let who = self
            .config
            .who(msg.handle_id, msg.is_from_me(), &msg.destination_caller_id);
        let kind = match tapback {
            Tapback::Sticker => {
                let mut paths = Attachment::from_message(self.config.data_source.db(), msg)?;
                match paths.get_mut(0) {
                    Some(sticker) => TapbackKind::Sticker {
                        text: self.format_sticker(sticker, msg),
                        who,
                    },
                    None => TapbackKind::StickerMissing { who },
                }
            }
            other => TapbackKind::Reaction {
                tapback: other,
                who,
            },
        };
        Ok(TapbackVM { kind }.render().unwrap_or_default())
    }

    fn format_expressive(&self, msg: &'a Message) -> &'a str {
        format_expressive(msg)
    }

    fn format_announcement(&self, msg: &Message) -> String {
        let Some(resolved) = resolve_announcement(msg, self.config, YOU) else {
            return AnnouncementVM {
                kind: AnnouncementBody::Unknown,
            }
            .render()
            .unwrap_or_default();
        };

        AnnouncementVM {
            kind: AnnouncementBody::Action {
                timestamp: resolved.timestamp,
                who: resolved.who,
                announcement: resolved.announcement,
                participant_name: resolved.participant_name,
            },
        }
        .render()
        .unwrap_or_default()
    }

    fn format_shareplay(&self) -> &'static str {
        "SharePlay Message\nEnded"
    }

    fn format_shared_location(&self, msg: &'a Message) -> &'static str {
        // Handle Shared Location
        if msg.started_sharing_location() {
            return "Started sharing location!";
        } else if msg.stopped_sharing_location() {
            return "Stopped sharing location!";
        }
        "Shared location!"
    }

    fn format_edited(
        &self,
        msg: &'a Message,
        edited_message: &'a EditedMessage,
        message_part_idx: usize,
    ) -> Option<String> {
        let normalized = normalize_edited(msg, edited_message, message_part_idx, self.config)?;

        let kind = match normalized {
            NormalizedEdit::Edited(events) => {
                let rows = events
                    .into_iter()
                    .map(|event| {
                        let timestamp_prefix = match event.diff_since_previous {
                            EditDiff::First => format!("{} ", event.absolute_time),
                            // Diff calculation failed; suppress the prefix to match legacy behavior.
                            EditDiff::Failed => String::new(),
                            EditDiff::Computed(diff) => format!("Edited {diff} later: "),
                        };
                        EditedRow {
                            timestamp_prefix,
                            text: event.text,
                        }
                    })
                    .collect();
                EditedKind::Edited { rows }
            }
            NormalizedEdit::Unsent { diff } => {
                let who = if msg.is_from_me() {
                    self.config.options.custom_name.as_deref().unwrap_or(YOU)
                } else {
                    self.config
                        .who(msg.handle_id, msg.is_from_me(), &msg.destination_caller_id)
                };
                match diff {
                    Some(diff) => EditedKind::UnsentWithDiff { who, diff },
                    None => EditedKind::Unsent { who },
                }
            }
        };

        Some(EditedVM { kind }.render().unwrap_or_default())
    }

    fn format_attributes(&'a self, text: &'a str, attributes: &'a [TextAttributes]) -> String {
        let mut formatted_text = String::with_capacity(text.len());
        let mut prev_start = 0;
        let mut prev_end = 0;

        for effect in attributes {
            if prev_start == effect.start && prev_end == effect.end {
                continue;
            }
            if let Some(message_content) = text.get(effect.start..effect.end) {
                prev_start = effect.start;
                prev_end = effect.end;
                // There isn't really a way to represent formatted text in a plain text export
                formatted_text.push_str(message_content);
            }
        }
        formatted_text
    }

    /// Render `message` directly into `out`. The trait's `format_message`
    /// wraps this, allocating a fresh `String` per call; hot callers
    /// (`run_export`, `build_replies`) reuse a single buffer instead so
    /// each message doesn't pay for a heap allocation.
    fn format_message_into(
        &self,
        message: &Message,
        indent_size: usize,
        out: &mut String,
    ) -> Result<(), TableError> {
        let indent = (0..indent_size).map(|_| " ").collect::<String>();
        let mut attachments = Attachment::from_message(self.config.data_source.db(), message)?;
        let mut replies_map = message.get_replies(self.config.data_source.db())?;
        let mut attachment_index: usize = 0;

        let mut parts = Vec::with_capacity(message.components.len());
        for (idx, message_part) in message.components.iter().enumerate() {
            let body = self.build_part_body(
                message,
                idx,
                message_part,
                &mut attachments,
                &mut attachment_index,
            );
            parts.push(MessagePartVM {
                indent: &indent,
                body,
                expressive: if message.expressive_send_style_id.is_some() {
                    let e = self.format_expressive(message);
                    if e.is_empty() { None } else { Some(e) }
                } else {
                    None
                },
                tapbacks: self.build_tapbacks(message, idx, &indent)?,
                replies: self.build_replies(replies_map.get_mut(&idx))?,
            });
        }

        let vm = MessageVM {
            indent: &indent,
            timestamp: self.get_time(message),
            sender: self.config.who(
                message.handle_id,
                message.is_from_me(),
                &message.destination_caller_id,
            ),
            is_deleted: message.is_deleted(),
            subject: message.subject.as_deref(),
            shareplay: if message.is_shareplay() {
                Some(self.format_shareplay())
            } else {
                None
            },
            shared_location: if message.started_sharing_location()
                || message.stopped_sharing_location()
            {
                Some(self.format_shared_location(message))
            } else {
                None
            },
            parts,
            trailing_reply_context: message.is_reply() && indent.is_empty(),
            top_level: indent.is_empty(),
        };
        let _ = vm.render_into(out);
        Ok(())
    }
}

// MARK: Impl
impl TXT<'_> {
    fn get_time(&self, message: &Message) -> String {
        let (mut date, read_receipt) = message_time(self.config, message);
        if read_receipt.is_empty() {
            date
        } else {
            date.push(' ');
            date.push_str(&read_receipt);
            date
        }
    }

    fn build_part_body(
        &self,
        message: &Message,
        idx: usize,
        message_part: &BubbleComponent,
        attachments: &mut Vec<Attachment>,
        attachment_index: &mut usize,
    ) -> PartBody {
        match message_part {
            BubbleComponent::Text(text_attrs) => {
                let Some(text) = &message.text else {
                    return PartBody::Empty;
                };
                if message.is_part_edited(idx) {
                    return match &message.edited_parts {
                        Some(edited_parts) => {
                            match self.format_edited(message, edited_parts, idx) {
                                Some(edited) => PartBody::Line { text: edited },
                                None => PartBody::Empty,
                            }
                        }
                        None => PartBody::Empty,
                    };
                }

                let mut formatted_text = self.format_attributes(text, text_attrs);
                if formatted_text.is_empty() {
                    formatted_text.push_str(text);
                }

                if self.config.translated_messages.contains(&message.guid)
                    && let Ok(Some(translation)) =
                        message.get_translation(self.config.data_source.db())
                {
                    PartBody::Translated {
                        translated: translation.translated_text,
                        original: formatted_text,
                    }
                } else if formatted_text.starts_with(FITNESS_RECEIVER) {
                    PartBody::Line {
                        text: formatted_text.replace(FITNESS_RECEIVER, YOU),
                    }
                } else {
                    PartBody::Line {
                        text: formatted_text,
                    }
                }
            }
            BubbleComponent::Attachment(metadata) => {
                let Some(attachment) = attachments.get_mut(*attachment_index) else {
                    return PartBody::Line {
                        text: "Attachment missing!".to_string(),
                    };
                };
                if attachment.is_sticker {
                    return PartBody::Line {
                        text: self.format_sticker(attachment, message),
                    };
                }
                let body = match self.format_attachment(attachment, message, metadata) {
                    Ok(result) => PartBody::Line { text: result },
                    Err(result) => PartBody::Line {
                        text: result.to_string(),
                    },
                };
                *attachment_index += 1;
                body
            }
            BubbleComponent::App => match self.format_app(message, attachments) {
                Ok(ok_bubble) => PartBody::Line { text: ok_bubble },
                Err(why) => PartBody::Line {
                    text: format!("Unable to format app message: {why}"),
                },
            },
            BubbleComponent::Retracted => match &message.edited_parts {
                Some(edited_parts) => match self.format_edited(message, edited_parts, idx) {
                    Some(edited) => PartBody::Line { text: edited },
                    None => PartBody::Empty,
                },
                None => PartBody::Empty,
            },
        }
    }

    fn build_tapbacks<'b>(
        &self,
        message: &Message,
        idx: usize,
        indent: &'b str,
    ) -> Result<Option<TapbacksVM<'b>>, TableError> {
        let Some(tapbacks) = self
            .config
            .tapbacks
            .get(&message.guid)
            .and_then(|m| m.get(&idx))
        else {
            return Ok(None);
        };

        let mut rendered = Vec::new();
        for tapback in tapbacks {
            let f = self.format_tapback(tapback)?;
            if !f.is_empty() {
                rendered.push(f);
            }
        }

        if rendered.is_empty() {
            Ok(None)
        } else {
            Ok(Some(TapbacksVM {
                indent,
                tapbacks: rendered,
            }))
        }
    }

    fn build_replies(
        &self,
        replies: Option<&mut Vec<Message>>,
    ) -> Result<Option<RepliesVM>, TableError> {
        let Some(replies) = replies else {
            return Ok(None);
        };
        let mut rendered = Vec::new();
        for reply in replies.iter_mut() {
            if let Ok(body) = reply.parse_body(self.config.data_source.db()) {
                reply.apply_body(body);
            }
            if !reply.is_tapback() {
                let mut reply_buf = String::new();
                self.format_message_into(reply, 4, &mut reply_buf)?;
                rendered.push(reply_buf);
            }
        }
        if rendered.is_empty() {
            Ok(None)
        } else {
            Ok(Some(RepliesVM { replies: rendered }))
        }
    }
}

// MARK: Tests

/// Test-only convenience: allocate a buffer and forward to
/// `format_message_into`. Production paths (`iter_messages`, `build_replies`)
/// use the buffer-reusing API directly.
#[cfg(test)]
fn format_message(
    exporter: &TXT<'_>,
    message: &Message,
    indent_size: usize,
) -> Result<String, TableError> {
    let mut out = String::with_capacity(1024);
    exporter.format_message_into(message, indent_size, &mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::env::current_dir;

    use crate::exporters::txt::format_message;

    use crate::{
        Config, Exporter, Options, TXT,
        app::{
            compatibility::attachment_manager::AttachmentManagerMode, contacts::Name,
            export_type::ExportType,
        },
        exporters::exporter::MessageFormatter,
    };
    use imessage_database::{
        message_types::text_effects::TextEffect,
        tables::{
            messages::models::{AttachmentMeta, BubbleComponent, TextAttributes},
            table::ME,
        },
        util::platform::Platform,
    };

    #[test]
    fn can_create() {
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();
        assert_eq!(exporter.files.len(), 0);
    }

    #[test]
    fn can_get_time_valid() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        // Create fake message
        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        // May 17, 2022  8:29:42 PM
        message.date_delivered = 674526582885055488;
        // May 17, 2022  9:30:31 PM
        message.date_read = 674530231992568192;

        assert_eq!(
            exporter.get_time(&message),
            "May 17, 2022  5:29:42 PM (Read by you after 1 hour, 49 seconds)"
        );
    }

    #[test]
    fn can_get_time_invalid() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        // Create fake message
        let mut message = Config::fake_message();
        // May 17, 2022  9:30:31 PM
        message.date = 674530231992568192;
        // May 17, 2022  9:30:31 PM
        message.date_delivered = 674530231992568192;
        // Wed May 18 2022 02:36:24 GMT+0000
        message.date_read = 674526582885055488;
        assert_eq!(exporter.get_time(&message), "May 17, 2022  6:30:31 PM");
    }

    #[test]
    fn can_format_txt_from_me_normal() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("Hello world".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_from_me_normal_deleted() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.text = Some("Hello world".to_string());
        message.date = 674526582885055488;
        message.is_from_me = true;
        message.deleted_from = Some(0);
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nThis message was deleted from the conversation!\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_from_me_normal_read() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.text = Some("Hello world".to_string());
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        // May 17, 2022  9:30:31 PM
        message.date_delivered = 674530231992568192;
        message.is_from_me = true;
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected =
            "May 17, 2022  5:29:42 PM (Read by them after 1 hour, 49 seconds)\nMe\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_from_them_normal() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("Hello world".to_string());
        message.handle_id = Some(999999);
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nSample Contact\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_from_them_normal_read() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.handle_id = Some(999999);
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("Hello world".to_string());
        // May 17, 2022  8:29:42 PM
        message.date_delivered = 674526582885055488;
        // May 17, 2022  9:30:31 PM
        message.date_read = 674530231992568192;
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM (Read by you after 1 hour, 49 seconds)\nSample Contact\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_from_them_custom_name_read() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.custom_name = Some("Name".to_string());
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.handle_id = Some(999999);
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("Hello world".to_string());
        // May 17, 2022  8:29:42 PM
        message.date_delivered = 674526582885055488;
        // May 17, 2022  9:30:31 PM
        message.date_read = 674530231992568192;
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM (Read by Name after 1 hour, 49 seconds)\nSample Contact\nHello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_shareplay() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.item_type = 6;

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nSharePlay Message\nEnded\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_announcement() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 2;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You renamed the conversation to Hello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_announcement_custom_name() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.custom_name = Some("Name".to_string());
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.item_type = 2;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM Name renamed the conversation to Hello world\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn format_message_into_appends_to_existing_buffer() {
        // Mirrors the production hot path in `iter_messages`, which reuses a
        // single `String` across messages via `clear()` + `format_message_into`.
        // Tests must protect that invariant: the helper appends, not overwrites.
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.date = 674526582885055488;
        message.text = Some("hello".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let standalone = format_message(&exporter, &message, 0).unwrap();

        let prefix = "PREV MSG\n\n";
        let mut buf = String::with_capacity(1024);
        buf.push_str(prefix);
        let cap_before = buf.capacity();

        exporter.format_message_into(&message, 0, &mut buf).unwrap();

        assert!(
            buf.starts_with(prefix),
            "format_message_into must not overwrite existing buffer content"
        );
        assert_eq!(&buf[prefix.len()..], standalone);
        assert!(buf.capacity() >= cap_before);
    }

    #[test]
    fn can_format_txt_announcement_unknown() {
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.date = 674526582885055488;

        let actual = exporter.format_announcement(&message);
        let expected = "Unable to format announcement!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_removed() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.participants.insert(1, Name::fake_name("Other"));
        config.real_participants.insert(0, 0);
        config.real_participants.insert(1, 1);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 1;
        message.group_action_type = 1;
        message.other_handle = Some(1);

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You removed Other from the conversation.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_removed_other() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.participants.insert(1, Name::fake_name("Other"));
        config.participants.insert(2, Name::fake_name("Second"));
        config.real_participants.insert(0, 0);
        config.real_participants.insert(1, 1);
        config.real_participants.insert(2, 2);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = false;
        message.handle_id = Some(1);
        message.item_type = 1;
        message.group_action_type = 1;
        message.other_handle = Some(2);

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM Other removed Second from the conversation.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_changed_number() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.participants.insert(1, Name::fake_name("Other"));
        config.real_participants.insert(0, 0);
        config.real_participants.insert(1, 1);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = false;
        message.handle_id = Some(1);
        message.item_type = 1;
        message.group_action_type = 0;
        message.other_handle = Some(1);

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM Other changed their phone number.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_added() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.participants.insert(1, Name::fake_name("Other"));
        config.real_participants.insert(0, 0);
        config.real_participants.insert(1, 1);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 1;
        message.group_action_type = 0;
        message.other_handle = Some(1);

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You added Other to the conversation.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_left() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 3;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You left the conversation.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_icon_removed() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 3;
        message.group_action_type = 2;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You removed the group photo.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_group_icon_added() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 3;
        message.group_action_type = 1;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You changed the group photo.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_chat_background_removed() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 3;
        message.group_action_type = 6;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You removed the chat background.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_chat_background_added() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.group_title = Some("Hello world".to_string());
        message.is_from_me = true;
        message.item_type = 3;
        message.group_action_type = 4;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You changed the chat background.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_audio_message_kept() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.is_from_me = true;
        message.item_type = 5;

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You kept an audio message.\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_me() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(2000);
        message.associated_message_guid = Some("fake_guid".to_string());

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "Loved by Me";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_them() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(2000);
        message.associated_message_guid = Some("fake_guid".to_string());
        message.handle_id = Some(999999);

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "Loved by Sample Contact";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_custom_emoji() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(2006);
        message.associated_message_guid = Some("fake_guid".to_string());
        message.handle_id = Some(999999);
        message.associated_message_emoji = Some("☕️".to_string());

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "☕️ by Sample Contact";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_custom_sticker() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(2007);
        message.associated_message_guid = Some("fake_guid".to_string());
        message.handle_id = Some(999999);
        message.num_attachments = 1;

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "Sticker from Sample Contact not found!";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_custom_sticker_exists() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(2007);
        message.associated_message_guid = Some("fake_guid".to_string());
        message.handle_id = Some(999999);
        message.num_attachments = 1;
        message.rowid = 452567;

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "Sticker from Sample Contact: /Users/chris/Library/Messages/StickerCache/8e682c381ab52ec2-289D9E83-33EE-4153-AF13-43DB31792C6F/289D9E83-33EE-4153-AF13-43DB31792C6F.heic (App: Free People) from Sample Contact";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_tapback_custom_sticker_removed() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.associated_message_type = Some(3007);
        message.associated_message_guid = Some("fake_guid".to_string());
        message.handle_id = Some(999999);
        message.num_attachments = 1;
        message.rowid = 452567;

        let actual = exporter.format_tapback(&message).unwrap();
        let expected = "";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_started_sharing_location_me() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.is_from_me = false;
        message.other_handle = Some(2);
        message.share_status = false;
        message.share_direction = Some(false);
        message.item_type = 4;

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "Dec 31, 2000  4:00:00 PM\nMe\nStarted sharing location!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_stopped_sharing_location_me() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.is_from_me = false;
        message.other_handle = Some(2);
        message.share_status = true;
        message.share_direction = Some(false);
        message.item_type = 4;

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "Dec 31, 2000  4:00:00 PM\nMe\nStopped sharing location!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_started_sharing_location_them() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.handle_id = None;
        message.is_from_me = false;
        message.other_handle = Some(0);
        message.share_status = false;
        message.share_direction = Some(false);
        message.item_type = 4;

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "Dec 31, 2000  4:00:00 PM\nUnknown\nStarted sharing location!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_stopped_sharing_location_them() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.handle_id = None;
        message.is_from_me = false;
        message.other_handle = Some(0);
        message.share_status = true;
        message.share_direction = Some(false);
        message.item_type = 4;

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "Dec 31, 2000  4:00:00 PM\nUnknown\nStopped sharing location!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_attachment_macos() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();

        let actual = exporter
            .format_attachment(&mut attachment, &message, &AttachmentMeta::default())
            .unwrap();

        assert_eq!(actual, "a/b/c/d.jpg");
    }

    #[test]
    fn can_format_txt_attachment_macos_invalid_disabled() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.filename = None;
        attachment.transfer_name = None;

        let actual =
            exporter.format_attachment(&mut attachment, &message, &AttachmentMeta::default());

        assert_eq!(actual, Err("Attachment missing name metadata!"));
    }

    #[test]
    fn can_format_txt_attachment_macos_invalid_clone() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.attachment_manager.mode = AttachmentManagerMode::Clone;

        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.filename = None;
        attachment.transfer_name = None;

        let actual =
            exporter.format_attachment(&mut attachment, &message, &AttachmentMeta::default());

        assert_eq!(actual, Err("Attachment missing name metadata!"));
    }

    #[test]
    fn can_format_txt_attachment_ios() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.options.platform = Platform::iOS;
        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();

        let actual = exporter
            .format_attachment(&mut attachment, &message, &AttachmentMeta::default())
            .unwrap();

        assert!(actual.ends_with("33/33c81da8ae3194fc5a0ea993ef6ffe0b048baedb"));
    }

    #[test]
    fn can_format_txt_attachment_ios_invalid_disabled() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let mut config = Config::fake_app(options);
        config.options.platform = Platform::iOS;

        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.filename = None;
        attachment.transfer_name = None;

        let actual =
            exporter.format_attachment(&mut attachment, &message, &AttachmentMeta::default());

        assert_eq!(actual, Err("Attachment missing name metadata!"));
    }

    #[test]
    fn can_format_txt_attachment_ios_invalid_clone() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.attachment_manager.mode = AttachmentManagerMode::Clone;

        let mut config = Config::fake_app(options);
        config.options.platform = Platform::iOS;

        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.filename = None;
        attachment.transfer_name = None;

        let actual =
            exporter.format_attachment(&mut attachment, &message, &AttachmentMeta::default());

        assert_eq!(actual, Err("Attachment missing name metadata!"));
    }

    #[test]
    fn can_format_txt_attachment_sticker() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.export_path = current_dir().unwrap().parent().unwrap().to_path_buf();

        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.rowid = 3;
        attachment.is_sticker = true;
        let sticker_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("imessage-database/test_data/stickers/outline.heic");
        attachment.filename = Some(sticker_path.to_string_lossy().to_string());
        attachment.copied_path = Some(sticker_path);

        let actual = exporter.format_sticker(&mut attachment, &message);

        assert_eq!(
            actual,
            "Outline Sticker from Me: imessage-database/test_data/stickers/outline.heic"
        );

        // Remove the file created by the constructor for this test
        let orphaned_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("orphaned.txt");
        let _ = std::fs::remove_file(orphaned_path);
    }

    #[test]
    fn can_format_txt_attachment_sticker_genmoji() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.export_path = current_dir().unwrap().parent().unwrap().to_path_buf();

        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.rowid = 2;
        attachment.is_sticker = true;
        attachment.emoji_description = Some("Example description".to_string());
        let sticker_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("imessage-database/test_data/stickers/outline.heic");
        attachment.filename = Some(sticker_path.to_string_lossy().to_string());
        attachment.copied_path = Some(sticker_path);

        let actual = exporter.format_sticker(&mut attachment, &message);

        assert_eq!(
            actual,
            "Sticker from Me: imessage-database/test_data/stickers/outline.heic (Genmoji prompt: Example description)"
        );

        // Remove the file created by the constructor for this test
        let orphaned_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("orphaned.txt");
        let _ = std::fs::remove_file(orphaned_path);
    }

    #[test]
    fn can_format_txt_attachment_sticker_app() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.export_path = current_dir().unwrap().parent().unwrap().to_path_buf();

        let mut config = Config::fake_app(options);
        config.participants.insert(0, Name::fake_name(ME));
        config.real_participants.insert(0, 0);

        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.rowid = 1;
        attachment.is_sticker = true;
        let sticker_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("imessage-database/test_data/stickers/outline.heic");
        attachment.filename = Some(sticker_path.to_string_lossy().to_string());
        attachment.copied_path = Some(sticker_path);

        let actual = exporter.format_sticker(&mut attachment, &message);

        assert_eq!(
            actual,
            "Sticker from Me: imessage-database/test_data/stickers/outline.heic (App: Free People)"
        );

        // Remove the file created by the constructor for this test
        let orphaned_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("orphaned.txt");
        let _ = std::fs::remove_file(orphaned_path);
    }

    #[test]
    fn can_format_txt_attachment_audio_transcript() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let message = Config::fake_message();

        let mut attachment = Config::fake_attachment();
        attachment.uti = Some("com.apple.coreaudio-format".to_string());
        attachment.transfer_name = Some("Audio Message.caf".to_string());
        attachment.filename = Some("Audio Message.caf".to_string());
        attachment.mime_type = None;

        let meta = AttachmentMeta {
            transcription: Some("Test".to_string()),
            ..Default::default()
        };

        let actual = exporter
            .format_attachment(&mut attachment, &message, &meta)
            .unwrap();

        assert_eq!(actual, "Audio Message.caf\nTranscription: Test");
    }

    #[test]
    fn can_format_txt_single_url_no_bundle_id() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();

        // Use test message payload from test database
        // May 17, 2022  8:29:42 PM
        message.guid = "FAKEGUID-D0C8-4212-AA87-DD8AE4FD1203".to_string();
        message.rowid = 123445;

        message.date = 674526582885055488;
        // Set the message components to a single url
        message.text = Some("https://example.com".to_string());
        message.components = vec![BubbleComponent::Text(vec![
                TextAttributes::new(
                    0,
                    84,
                    vec![
                        TextEffect::Link("https://www.ghacks.net/2020/01/23/lastpass-no-longer-listed-on-the-chrome-web-store/".to_string()),
                    ]
                ),
            ]),];

        let body = message.parse_body(config.data_source.db()).unwrap();
        message.apply_body(body);

        let actual = format_message(&exporter, &message, 0).unwrap();

        assert_eq!(
            actual,
            "May 17, 2022  5:29:42 PM\nUnknown\nhttps://www.ghacks.net/2020/01/23/lastpass-no-longer-listed-on-the-chrome-web-store/\nLastPass no longer listed on the Chrome Web Store - gHacks Tech News\nLastPass customers and new users searching for password managers on Google's Chrome Web Store may have noticed that the LastPass extension for Google Chrome is currently no longer listed on the store.\n\n"
        );
    }

    #[test]
    fn can_format_txt_translated_message() {
        // Create exporter
        let mut options = Options::fake_options(ExportType::Txt);
        options.attachment_manager.mode = AttachmentManagerMode::Clone;

        let mut config = Config::fake_app(options);
        config
            .translated_messages
            .insert("56FE94B9-2345-4A3C-A57F-949BDDDDF9FF".to_string());

        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.guid = "56FE94B9-2345-4A3C-A57F-949BDDDDF9FF".to_string();
        message.rowid = 548216;
        message
            .generate_text_legacy(config.data_source.db())
            .unwrap();

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "Dec 31, 2000  4:00:00 PM\nUnknown\nOh, il a traduit ce que j'ai envoyé !\nTranslated from:\nOh it translated what I sent!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn balloon_indent_matches_header_in_reply_context() {
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut msg = Config::fake_message();
        msg.date = 674526582885055488;
        msg.is_from_me = true;
        msg.chat_id = Some(0);
        msg.balloon_bundle_id = Some("com.apple.messages.URLBalloonProvider".to_string());
        msg.text = Some("https://example.com".to_string());
        msg.rowid = i32::MAX;
        msg.components = vec![BubbleComponent::App];

        let mut out = String::new();
        exporter.format_message_into(&msg, 4, &mut out).unwrap();

        // Every non-blank line should start with exactly four spaces.
        for line in out.lines().filter(|l| !l.is_empty()) {
            let leading = line.chars().take_while(|c| *c == ' ').count();
            assert_eq!(
                leading, 4,
                "expected 4-space indent on every non-blank line, got {leading} on: {line:?}\nfull output:\n{out}"
            );
        }
    }

    #[test]
    fn can_format_txt_url_message_without_payload_uses_text_fallback() {
        // Defensive path in dispatch_app_balloon: when a URL-balloon message
        // has no payload row but does carry `text`, the normal `format_url`
        // pipeline still produces the raw URL via its msg.text fallback.
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        message.date = 674526582885055488;
        message.rowid = i32::MAX;
        message.is_from_me = true;
        message.chat_id = Some(0);
        message.balloon_bundle_id = Some("com.apple.messages.URLBalloonProvider".to_string());
        message.text = Some("https://example.com".to_string());
        message.components = vec![BubbleComponent::App];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nhttps://example.com\n\n";

        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod balloon_format_tests {
    use std::{collections::HashMap, env::current_dir, fs::File, io::Read};

    use crate::{
        Config, Exporter, Options, TXT, app::export_type::ExportType::Txt,
        exporters::exporter::BalloonFormatter,
    };
    use imessage_database::message_types::{
        app::AppMessage,
        app_store::AppStoreMessage,
        collaboration::CollaborationMessage,
        digital_touch::DigitalTouch,
        handwriting::HandwrittenMessage,
        music::MusicMessage,
        placemark::{Placemark, PlacemarkMessage},
        polls::{Poll, PollOption, PollOptionID, PollVote},
        url::URLMessage,
    };

    #[test]
    fn can_format_txt_url() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = URLMessage {
            title: Some("title"),
            summary: Some("summary"),
            url: Some("url"),
            original_url: Some("original_url"),
            item_type: Some("item_type"),
            images: vec!["images"],
            icons: vec!["icons"],
            site_name: Some("site_name"),
            placeholder: false,
        };

        let expected = exporter.format_url(&Config::fake_message(), &balloon);
        let actual = "url\ntitle\nsummary";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_music() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = MusicMessage {
            url: Some("url"),
            preview: Some("preview"),
            artist: Some("artist"),
            album: Some("album"),
            track_name: Some("track_name"),
            lyrics: None,
        };

        let expected = exporter.format_music(&balloon);
        let actual = "track_name\nalbum\nartist\nurl";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_music_lyrics() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = MusicMessage {
            url: Some("url"),
            preview: None,
            artist: Some("artist"),
            album: Some("album"),
            track_name: Some("track_name"),
            lyrics: Some(vec!["a", "b"]),
        };

        let expected = exporter.format_music(&balloon);
        let actual = "Lyrics:\na\nb\n\n\ntrack_name\nalbum\nartist\nurl";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_collaboration() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = CollaborationMessage {
            original_url: Some("original_url"),
            url: Some("url"),
            title: Some("title"),
            creation_date: Some(0.),
            bundle_id: Some("bundle_id"),
            app_name: Some("app_name"),
        };

        let expected = exporter.format_collaboration(&balloon);
        let actual = "app_name message:\ntitle\nurl";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_apple_pay() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: Some("image"),
            url: Some("url"),
            title: Some("title"),
            subtitle: Some("subtitle"),
            caption: Some("caption"),
            subcaption: Some("subcaption"),
            trailing_caption: Some("trailing_caption"),
            trailing_subcaption: Some("trailing_subcaption"),
            app_name: Some("app_name"),
            ldtext: Some("ldtext"),
        };

        let expected = exporter.format_apple_pay(&balloon);
        let actual = "caption transaction: ldtext";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_fitness() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: Some("image"),
            url: Some("url"),
            title: Some("title"),
            subtitle: Some("subtitle"),
            caption: Some("caption"),
            subcaption: Some("subcaption"),
            trailing_caption: Some("trailing_caption"),
            trailing_subcaption: Some("trailing_subcaption"),
            app_name: Some("app_name"),
            ldtext: Some("ldtext"),
        };

        let expected = exporter.format_fitness(&balloon);
        let actual = "app_name message: ldtext";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_slideshow() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: Some("image"),
            url: Some("url"),
            title: Some("title"),
            subtitle: Some("subtitle"),
            caption: Some("caption"),
            subcaption: Some("subcaption"),
            trailing_caption: Some("trailing_caption"),
            trailing_subcaption: Some("trailing_subcaption"),
            app_name: Some("app_name"),
            ldtext: Some("ldtext"),
        };

        let expected = exporter.format_slideshow(&balloon);
        let actual = "Photo album: ldtext url";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_find_my() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: Some("image"),
            url: Some("url"),
            title: Some("title"),
            subtitle: Some("subtitle"),
            caption: Some("caption"),
            subcaption: Some("subcaption"),
            trailing_caption: Some("trailing_caption"),
            trailing_subcaption: Some("trailing_subcaption"),
            app_name: Some("app_name"),
            ldtext: Some("ldtext"),
        };

        let expected = exporter.format_find_my(&balloon);
        let actual = "app_name:  ldtext";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_check_in_timer() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1&interfaceVersion=1&sendDate=1697316869.688709"),
            title: None,
            subtitle: None,
            caption: Some("Check In: Timer Started"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In: Timer Started"),
        };

        let expected = exporter.format_check_in(&balloon);
        let actual = "Check\u{a0}In: Timer Started\nChecked in at Oct 14, 2023  1:54:29 PM";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_check_in_timer_late() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1&interfaceVersion=1&sendDate=1697316869.688709"),
            title: None,
            subtitle: None,
            caption: Some("Check In: Has not checked in when expected, location shared"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In: Has not checked in when expected, location shared"),
        };

        let expected = exporter.format_check_in(&balloon);
        let actual = "Check\u{a0}In: Has not checked in when expected, location shared\nChecked in at Oct 14, 2023  1:54:29 PM";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_accepted_check_in() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1&interfaceVersion=1&sendDate=1697316869.688709"),
            title: None,
            subtitle: None,
            caption: Some("Check In: Fake Location"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In: Fake Location"),
        };

        let expected = exporter.format_check_in(&balloon);
        let actual = "Check\u{a0}In: Fake Location\nChecked in at Oct 14, 2023  1:54:29 PM";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_app_store() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppStoreMessage {
            url: Some("url"),
            app_name: Some("app_name"),
            original_url: Some("original_url"),
            description: Some("description"),
            platform: Some("platform"),
            genre: Some("genre"),
        };

        let expected = exporter.format_app_store(&balloon);
        let actual = "app_name\ndescription\nplatform\ngenre\nurl";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_placemark() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = PlacemarkMessage {
            url: Some("url"),
            original_url: Some("original_url"),
            place_name: Some("Name"),
            placemark: Placemark {
                name: Some("name"),
                address: Some("address"),
                state: Some("state"),
                city: Some("city"),
                iso_country_code: Some("iso_country_code"),
                postal_code: Some("postal_code"),
                country: Some("country"),
                street: Some("street"),
                sub_administrative_area: Some("sub_administrative_area"),
                sub_locality: Some("sub_locality"),
            },
        };

        let expected = exporter.format_placemark(&balloon);
        let actual = "Name\nurl\nname\naddress\nstate\ncity\niso_country_code\npostal_code\ncountry\nstreet\nsub_administrative_area\nsub_locality";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_poll() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut poll_options: HashMap<PollOptionID, PollOption> = HashMap::new();

        let id1: PollOptionID = "1".to_string();
        let id2: PollOptionID = "2".to_string();
        let id3: PollOptionID = "3".to_string();

        poll_options.insert(
            id1.clone(),
            PollOption {
                text: "Rust".to_string(),
                creator: "alice".to_string(),
                votes: vec![PollVote {
                    voter: "carol".to_string(),
                    option_id: id1.clone(),
                }],
            },
        );

        poll_options.insert(
            id2.clone(),
            PollOption {
                text: "Go".to_string(),
                creator: "bob".to_string(),
                votes: vec![
                    PollVote {
                        voter: "alice".to_string(),
                        option_id: id2.clone(),
                    },
                    PollVote {
                        voter: "bob".to_string(),
                        option_id: id2.clone(),
                    },
                ],
            },
        );

        poll_options.insert(
            id3.clone(),
            PollOption {
                text: "Python".to_string(),
                creator: "carol".to_string(),
                votes: vec![PollVote {
                    voter: "dave".to_string(),
                    option_id: id3.clone(),
                }],
            },
        );

        let poll = Poll {
            options: poll_options,
            order: vec![id1, id2, id3],
        };

        let expected = exporter.format_poll(&poll);
        let actual = "- Rust (1)\n  - carol\n- Go (2)\n  - alice\n  - bob\n- Python (1)\n  - dave";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_generic_app() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: Some("image"),
            url: Some("url"),
            title: Some("title"),
            subtitle: Some("subtitle"),
            caption: Some("caption"),
            subcaption: Some("subcaption"),
            trailing_caption: Some("trailing_caption"),
            trailing_subcaption: Some("trailing_subcaption"),
            app_name: Some("app_name"),
            ldtext: Some("ldtext"),
        };

        let expected = exporter.format_generic_app(
            &balloon,
            "bundle_id",
            &mut vec![],
            &Config::fake_message(),
        );
        let actual = "app_name message:\ntitle\nsubtitle\ncaption\nsubcaption\ntrailing_caption\ntrailing_subcaption";

        assert_eq!(expected, actual);
    }

    #[test]
    fn can_format_txt_digital_touch_kiss() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let msg = Config::fake_message();
        let actual = exporter.format_digital_touch(&msg, &DigitalTouch::Kiss);
        let expected = "Digital Touch Message: Kiss";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_handwriting_disabled_mode() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let payload_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("imessage-database/test_data/handwritten_message/handwriting.bin");
        let mut payload = vec![];
        File::open(payload_path)
            .unwrap()
            .read_to_end(&mut payload)
            .unwrap();
        let balloon = HandwrittenMessage::from_payload(&payload).unwrap();

        let mut expected = String::new();
        let expected_path = current_dir()
            .unwrap()
            .parent()
            .unwrap()
            .join("imessage-database/test_data/handwritten_message/handwriting.ascii");
        File::open(expected_path)
            .unwrap()
            .read_to_string(&mut expected)
            .unwrap();

        let msg = Config::fake_message();
        let actual = exporter.format_handwriting(&msg, &balloon);

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_check_in_estimated_end_time() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1&interfaceVersion=1&estimatedEndTime=1697316869.688709"),
            title: None,
            subtitle: None,
            caption: Some("Check In: Timer Started"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In: Timer Started"),
        };

        let actual = exporter.format_check_in(&balloon);
        let expected = "Check In: Timer Started\nExpected at Oct 14, 2023  1:54:29 PM";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_check_in_trigger_time() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1&interfaceVersion=1&triggerTime=1697316869.688709"),
            title: None,
            subtitle: None,
            caption: Some("Check In: Timer Started"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In: Timer Started"),
        };

        let actual = exporter.format_check_in(&balloon);
        let expected = "Check In: Timer Started\nWas expected at Oct 14, 2023  1:54:29 PM";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_check_in_no_recognized_metadata() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = AppMessage {
            image: None,
            url: Some("?messageType=1"),
            title: None,
            subtitle: None,
            caption: Some("Check In"),
            subcaption: None,
            trailing_caption: None,
            trailing_subcaption: None,
            app_name: Some("Check In"),
            ldtext: Some("Check In"),
        };

        // No timestamp metadata → footer is None → only the caption renders.
        let actual = exporter.format_check_in(&balloon);
        let expected = "Check In";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_poll_empty_options() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let poll = Poll {
            options: HashMap::new(),
            order: vec![],
        };

        // Empty poll: the for-loop body doesn't execute, so the template
        // emits nothing at all.
        let actual = exporter.format_poll(&poll);
        let expected = "";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_url_no_url_falls_back_to_msg_text() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let balloon = URLMessage {
            title: None,
            summary: None,
            url: None,
            original_url: None,
            item_type: None,
            images: vec![],
            icons: vec![],
            site_name: None,
            placeholder: false,
        };

        let mut msg = Config::fake_message();
        msg.text = Some("https://example.com/from-text".to_string());

        let actual = exporter.format_url(&msg, &balloon);
        let expected = "https://example.com/from-text";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_collaboration_no_url_with_original_url() {
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        // url=None + original_url=Some: TXT's `url` field is balloon.get_url()
        // which falls back to original_url, so the URL line still renders.
        let balloon = CollaborationMessage {
            original_url: Some("https://example.com/original"),
            url: None,
            title: Some("Doc title"),
            creation_date: None,
            bundle_id: Some("bundle"),
            app_name: Some("App"),
        };

        let actual = exporter.format_collaboration(&balloon);
        let expected = "App message:\nDoc title\nhttps://example.com/original";

        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod text_effect_tests {
    use imessage_database::{
        message_types::text_effects::{Animation, Style, TextEffect, Unit},
        tables::messages::models::{BubbleComponent, TextAttributes},
    };

    use crate::{
        Config, Exporter, Options, TXT, app::export_type::ExportType,
        exporters::txt::format_message,
    };

    #[test]
    fn can_format_txt_text_styles_mixed_end_to_end() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("Underline normal jitter normal".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);

        message.components = vec![BubbleComponent::Text(vec![
            TextAttributes::new(0, 9, vec![TextEffect::Styles(vec![Style::Underline])]),
            TextAttributes::new(9, 17, vec![TextEffect::Default]),
            TextAttributes::new(17, 23, vec![TextEffect::Animated(Animation::Jitter)]),
            TextAttributes::new(23, 30, vec![TextEffect::Default]),
        ])];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nUnderline normal jitter normal\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_text_styled_plain_link() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text =
            Some("https://github.com/ReagentX/imessage-exporter/discussions/553".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);

        message.components = vec![BubbleComponent::Text(vec![TextAttributes::new(
            0,
            61,
            vec![
                TextEffect::Animated(Animation::Big),
                TextEffect::Link(
                    "https://github.com/ReagentX/imessage-exporter/discussions/553".to_string(),
                ),
            ],
        )])];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nhttps://github.com/ReagentX/imessage-exporter/discussions/553\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_text_styled_emoji_bold_underline() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("🅱️Bold_Underline".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);

        message.components = vec![BubbleComponent::Text(vec![
            TextAttributes::new(0, 7, vec![TextEffect::Default]),
            TextAttributes::new(7, 11, vec![TextEffect::Styles(vec![Style::Bold])]),
            TextAttributes::new(11, 12, vec![TextEffect::Default]),
            TextAttributes::new(12, 21, vec![TextEffect::Styles(vec![Style::Underline])]),
        ])];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\n🅱️Bold_Underline\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_text_styled_overlapping_ranges() {
        // Create exporter
        let options = Options::fake_options(ExportType::Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some("8:00 pm".to_string());
        message.is_from_me = true;
        message.chat_id = Some(0);

        message.components = vec![BubbleComponent::Text(vec![
            TextAttributes::new(
                0,
                1,
                vec![
                    TextEffect::Conversion(Unit::Timezone),
                    TextEffect::Styles(vec![Style::Bold]),
                ],
            ),
            TextAttributes::new(1, 2, vec![TextEffect::Conversion(Unit::Timezone)]),
            TextAttributes::new(
                2,
                4,
                vec![
                    TextEffect::Conversion(Unit::Timezone),
                    TextEffect::Styles(vec![Style::Underline]),
                ],
            ),
            TextAttributes::new(4, 5, vec![TextEffect::Conversion(Unit::Timezone)]),
            TextAttributes::new(
                5,
                7,
                vec![
                    TextEffect::Conversion(Unit::Timezone),
                    TextEffect::Styles(vec![Style::Italic]),
                ],
            ),
        ])];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\n8:00 pm\n\n";

        assert_eq!(actual, expected);
    }
}

#[cfg(test)]
mod edited_tests {
    use imessage_database::{
        message_types::{
            edited::{EditStatus, EditedMessage, EditedMessagePart},
            text_effects::TextEffect,
        },
        tables::messages::models::{AttachmentMeta, BubbleComponent, TextAttributes},
    };

    use crate::{
        Config, Exporter, Options, TXT,
        app::{contacts::Name, export_type::ExportType::Txt},
        exporters::{exporter::MessageFormatter, txt::format_message},
    };

    #[test]
    fn can_format_txt_conversion_final_unsent() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.date_edited = 674530231992568192;
        message.text = Some(
            "From arbitrary byte stream:\r\u{FFFC}To native Rust data structures:\r".to_string(),
        );
        message.is_from_me = true;
        message.chat_id = Some(0);
        message.edited_parts = Some(EditedMessage {
            parts: vec![
                EditedMessagePart {
                    status: EditStatus::Original,
                    edit_history: vec![],
                },
                EditedMessagePart {
                    status: EditStatus::Original,
                    edit_history: vec![],
                },
                EditedMessagePart {
                    status: EditStatus::Original,
                    edit_history: vec![],
                },
                EditedMessagePart {
                    status: EditStatus::Unsent,
                    edit_history: vec![],
                },
            ],
        });

        message.components = vec![
            BubbleComponent::Text(vec![TextAttributes::new(0, 28, vec![TextEffect::Default])]),
            BubbleComponent::Attachment(AttachmentMeta {
                guid: Some("D0551D89-4E11-43D0-9A0E-06F19704E97B".to_string()),
                transcription: None,
                height: None,
                width: None,
                name: None,
            }),
            BubbleComponent::Text(vec![TextAttributes::new(31, 63, vec![TextEffect::Default])]),
            BubbleComponent::Retracted,
        ];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nFrom arbitrary byte stream:\r\nAttachment missing!\nTo native Rust data structures:\r\nYou unsent this message part 1 hour, 49 seconds after sending!\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_unsent_from_them_resolves_contact_name() {
        // Regression: when someone else unsends a message part, TXT must
        // render the resolved contact name rather than the literal "They".
        // This matches HTML's behavior and is consistent with TXT's own
        // tapback, announcement, and message-header rendering.
        let options = Options::fake_options(Txt);
        let mut config = Config::fake_app(options);
        config
            .participants
            .insert(999999, Name::fake_name("Sample Contact"));
        config.real_participants.insert(999999, 999999);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.date_edited = 674530231992568192;
        message.handle_id = Some(999999);
        message.text = Some("hello".to_string());
        message.edited_parts = Some(EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Unsent,
                edit_history: vec![],
            }],
        });
        message.components = vec![BubbleComponent::Retracted];

        let actual = format_message(&exporter, &message, 0).unwrap();
        assert!(
            actual.contains(
                "Sample Contact unsent this message part 1 hour, 49 seconds after sending!"
            ),
            "expected resolved contact name in unsent notice, got: {actual}"
        );
        assert!(
            !actual.contains("They unsent"),
            "unsent notice should not fall back to the literal \"They\", got: {actual}"
        );
    }

    #[test]
    fn can_format_txt_conversion_no_edits() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.text = Some(
            "From arbitrary byte stream:\r\u{FFFC}To native Rust data structures:\r".to_string(),
        );
        message.is_from_me = true;
        message.chat_id = Some(0);

        message.components = vec![
            BubbleComponent::Text(vec![TextAttributes::new(0, 28, vec![TextEffect::Default])]),
            BubbleComponent::Attachment(AttachmentMeta {
                guid: Some("D0551D89-4E11-43D0-9A0E-06F19704E97B".to_string()),
                transcription: None,
                height: None,
                width: None,
                name: None,
            }),
            BubbleComponent::Text(vec![TextAttributes::new(31, 63, vec![TextEffect::Default])]),
            BubbleComponent::Retracted,
        ];

        let actual = format_message(&exporter, &message, 0).unwrap();
        let expected = "May 17, 2022  5:29:42 PM\nMe\nFrom arbitrary byte stream:\r\nAttachment missing!\nTo native Rust data structures:\r\n\n";

        assert_eq!(actual, expected);
    }

    #[test]
    fn can_format_txt_conversion_fully_unsent() {
        // Create exporter
        let options = Options::fake_options(Txt);
        let config = Config::fake_app(options);
        let exporter = TXT::new(&config).unwrap();

        let mut message = Config::fake_message();
        // May 17, 2022  8:29:42 PM
        message.date = 674526582885055488;
        message.date_edited = 674530231992568192;
        message.text = None;
        message.is_from_me = true;
        message.chat_id = Some(0);
        message.edited_parts = Some(EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Unsent,
                edit_history: vec![],
            }],
        });

        message.components = vec![BubbleComponent::Retracted];

        let actual = exporter.format_announcement(&message);
        let expected = "May 17, 2022  5:29:42 PM You unsent a message!\n\n";

        assert_eq!(actual, expected);
    }
}
