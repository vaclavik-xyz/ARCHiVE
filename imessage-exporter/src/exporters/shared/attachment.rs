use imessage_database::tables::{
    attachment::{Attachment, MediaType},
    messages::Message,
};

use crate::{
    app::{compatibility::attachment_manager::AttachmentManagerMode, runtime::Config},
    exporters::{formatter::AttachmentRender, shared::driver::ExportState},
};

/// Run the per-attachment side effects every exporter needs before it can
/// emit a reference to a file on disk: surface a busy indicator on the
/// progress bar for conversions that spawn ffmpeg (video transcoding and
/// animated-sticker decoding), then ask the
/// [`AttachmentManager`](crate::app::compatibility::attachment_manager::AttachmentManager)
/// to copy or convert the file.
///
/// Returns `Ok(())` when the attachment has a filename and
/// `handle_attachment` succeeded; otherwise returns the
/// [`AttachmentRender`] fallback the caller should propagate.
pub(crate) fn prepare_attachment(
    config: &Config,
    state: &ExportState,
    attachment: &mut Attachment,
    message: &Message,
) -> Result<(), AttachmentRender> {
    // Surface the busy bar only on the `ffmpeg`-spawning conversion paths:
    //   - animated sticker (image MIME, e.g. HEICS) in Basic or Full
    //     → `sticker_copy_convert`
    //   - animated sticker (video MIME, e.g. video Memoji) in Full only
    //     → `video_copy_convert`
    //   - any non-sticker video in Full → `video_copy_convert`
    let mode = &config.options.attachment_manager.mode;
    let is_video_full = matches!(attachment.mime_type(), MediaType::Video(_))
        && matches!(mode, AttachmentManagerMode::Full);
    let is_animated_sticker_encoding = attachment.is_animated_sticker()
        && match attachment.mime_type() {
            // HEICS etc. → `sticker_copy_convert`, which runs in Basic or Full.
            MediaType::Image(_) => {
                matches!(
                    mode,
                    AttachmentManagerMode::Basic | AttachmentManagerMode::Full
                )
            }
            // Video Memoji etc. → `video_copy_convert`, which runs in Full only.
            _ => matches!(mode, AttachmentManagerMode::Full),
        };

    let busy_label = if is_animated_sticker_encoding {
        Some("Encoding animated sticker, estimates paused...")
    } else if is_video_full {
        Some("Encoding video, estimates paused...")
    } else {
        None
    };

    if let Some(label) = busy_label {
        state.pb.set_busy_style(label.to_string());
    }

    let handle_result = config
        .options
        .attachment_manager
        .handle_attachment(message, attachment, config);

    if busy_label.is_some() {
        state.pb.set_default_style();
    }

    let Some(filename) = attachment.filename() else {
        return Err(AttachmentRender::MissingFilename);
    };
    if let Err(why) = handle_result {
        state.pb.println(why);
        return Err(AttachmentRender::NamedFile(filename.to_string()));
    }
    Ok(())
}
