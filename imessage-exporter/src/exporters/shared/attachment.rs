use imessage_database::tables::{
    attachment::{Attachment, MediaType},
    messages::Message,
};

use crate::{
    app::{compatibility::attachment_manager::AttachmentManagerMode, runtime::Config},
    exporters::shared::driver::ExportState,
};

/// Failure mode reported by [`prepare_attachment`]. Owned (no borrow from
/// `attachment`) so the caller can keep using `attachment` after `?`.
pub(crate) enum AttachmentError {
    /// [`Attachment::filename`] returned `None`. The caller should surface
    /// [`ATTACHMENT_NO_FILENAME`](crate::exporters::formatter::ATTACHMENT_NO_FILENAME)
    /// in this case.
    NoFilename,
    /// [`AttachmentManager::handle_attachment`](crate::app::compatibility::attachment_manager::AttachmentManager::handle_attachment)
    /// returned `None`. The caller should surface [`Attachment::filename`].
    HandleFailed,
}

/// Run the per-attachment side effects every exporter needs before it can
/// emit a reference to a file on disk: toggle the progress bar's "encoding
/// video" indicator if the attachment will be transcoded, then ask the
/// [`AttachmentManager`](crate::app::compatibility::attachment_manager::AttachmentManager)
/// to copy or convert the file.
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
) -> Result<(), AttachmentError> {
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

    if attachment.filename().is_none() {
        return Err(AttachmentError::NoFilename);
    }
    handle_result.ok_or(AttachmentError::HandleFailed)?;
    Ok(())
}
