use imessage_database::tables::{
    attachment::{Attachment, MediaType},
    messages::Message,
};

use crate::{
    app::{compatibility::attachment_manager::AttachmentManagerMode, runtime::Config},
    exporters::{formatter::ATTACHMENT_NO_FILENAME, shared::driver::ExportState},
};

/// Run the per-attachment side effects every exporter needs before it can
/// emit a reference to a file on disk: toggle the progress bar's "encoding
/// video" indicator if the attachment will be transcoded, then ask the
/// [`AttachmentManager`](crate::app::compatibility::attachment_manager::AttachmentManager)
/// to copy or convert the file. Returns the filename (or
/// [`ATTACHMENT_NO_FILENAME`] when missing) on failure so the caller can
/// render a missing-attachment notice without re-deriving the source.
///
/// The filename check fires regardless of `handle_attachment`'s result:
/// when `AttachmentManagerMode::Disabled` is in effect, `handle_attachment`
/// returns `Some(())` without touching `filename`, but the caller still
/// can't render anything useful downstream without one.
pub(crate) fn prepare_attachment(
    config: &Config,
    state: &ExportState,
    attachment: &mut Attachment,
    message: &Message,
) -> Result<(), String> {
    let will_encode = matches!(attachment.mime_type(), MediaType::Video(_))
        && matches!(
            config.options.attachment_manager.mode,
            AttachmentManagerMode::Full
        );

    if will_encode {
        state
            .pb
            .set_busy_style("Encoding video, estimates paused...".to_string());
    }

    let handle_result = config
        .options
        .attachment_manager
        .handle_attachment(message, attachment, config);

    if will_encode {
        state.pb.set_default_style();
    }

    let Some(filename) = attachment.filename() else {
        return Err(ATTACHMENT_NO_FILENAME.to_string());
    };
    handle_result.ok_or_else(|| filename.to_string())?;
    Ok(())
}
