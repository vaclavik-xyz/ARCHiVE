use std::{
    collections::{
        HashMap,
        hash_map::Entry::{Occupied, Vacant},
    },
    fs::File,
    io::{BufWriter, Write},
};

use imessage_database::{
    error::table::TableError,
    tables::{messages::Message, table::Table},
};

use crate::{
    app::{error::RuntimeError, progress::ExportProgress, runtime::Config},
    exporters::exporter::MessageFormatter,
};

/// Format-specific hooks consumed by [`run_export`] and
/// [`get_or_create_file_for`]. Implementers hold the shared per-export state
/// (the file cache, the orphaned writer, the progress bar) and expose
/// accessors so the shared loop can drive them.
pub trait MessageWriter<'a>: MessageFormatter<'a> {
    /// Format label substituted into the `"Exporting to … as <label>…"`
    /// status line (e.g. the file-extension name of the output format).
    const LABEL: &'static str;
    /// Initial capacity for the per-message buffer reused across iterations.
    const BUFFER_CAPACITY: usize;

    /// Return a reference to the config for this export, used to resolve conversation routes
    /// and database access across the shared export code.
    fn config(&self) -> &'a Config;

    /// Return a reference to the progress bar
    fn pb(&self) -> &ExportProgress;

    /// Return a mutable reference to the file cache, mapping chatroom names to open [`BufWriter<File>`]s.
    fn files_mut(&mut self) -> &mut HashMap<String, BufWriter<File>>;

    /// Return a mutable reference to the shared orphaned writer for messages without a conversation.
    fn orphaned_mut(&mut self) -> &mut BufWriter<File>;

    /// Write a per-file header. Called once for the orphaned file at the
    /// start of [`run_export`] and once when each chat file is first opened
    /// (skipped if the file already exists on disk, to avoid duplicate
    /// headers on group-name collisions). Return `Ok(())` to emit nothing.
    fn write_file_header(file: &mut BufWriter<File>) -> Result<(), RuntimeError>;

    /// Write a per-file footer. Called for every cached chat file and the
    /// orphaned file after iteration ends. Return `Ok(())` to emit nothing.
    fn write_file_footer(file: &mut BufWriter<File>) -> Result<(), RuntimeError>;

    /// Optional notice printed once before per-file footers are written.
    /// Return `None` to suppress the notice.
    fn footer_notice() -> Option<&'static str>;
}

/// Resolve the `BufWriter` for `message`, creating the chat file (and writing
/// its header) on first sight. Messages without a conversation route to the
/// shared orphaned writer.
pub fn get_or_create_file_for<'a, 'b, W>(
    writer: &'b mut W,
    message: &Message,
) -> Result<&'b mut BufWriter<File>, RuntimeError>
where
    W: MessageWriter<'a>,
{
    match writer.config().conversation(message) {
        Some((chatroom, _)) => {
            let filename = writer.config().filename(chatroom);
            let mut path = writer.config().options.export_path.clone();
            path.push(&filename);
            match writer.files_mut().entry(filename) {
                Occupied(entry) => Ok(entry.into_mut()),
                Vacant(entry) => {
                    // If the file already exists, don't write the headers again.
                    // This can happen if multiple chats use the same group name.
                    let file_exists = path.exists();
                    let file = File::options().append(true).create(true).open(&path)?;
                    let mut buf = BufWriter::new(file);
                    if !file_exists {
                        let _ = W::write_file_header(&mut buf);
                    }
                    Ok(entry.insert(buf))
                }
            }
        }
        None => Ok(writer.orphaned_mut()),
    }
}

/// Stream every message in the database, dispatching announcements and
/// regular messages to `writer.format_announcement` /
/// `writer.format_message_into`. Tapbacks, poll votes and poll updates are
/// rendered in context by their parent messages, so they're skipped here.
/// Duplicate ROWIDs are dropped (see [issue #135]).
///
/// [issue #135]: https://github.com/ReagentX/imessage-exporter/issues/135
pub fn run_export<'a, W>(writer: &mut W) -> Result<(), RuntimeError>
where
    W: MessageWriter<'a>,
{
    eprintln!(
        "Exporting to {} as {}...",
        writer.config().options.export_path.display(),
        W::LABEL,
    );

    W::write_file_header(writer.orphaned_mut())?;

    let mut current_message_row = -1;
    let mut current_message = 0;
    let total_messages = Message::get_count(
        writer.config().data_source.db(),
        &writer.config().options.query_context,
    )?;
    writer.pb().start(total_messages);

    let mut statement = Message::stream_rows(
        writer.config().data_source.db(),
        &writer.config().options.query_context,
    )?;
    let messages = statement
        .query_map([], |row| Ok(Message::from_row(row)))
        .map_err(|err| RuntimeError::DatabaseError(TableError::QueryError(err)))?;

    // Reused across iterations so each message doesn't allocate a fresh
    // output buffer. Capacity grows naturally to fit the largest message
    // and `clear()` retains it.
    let mut msg_buf = String::with_capacity(W::BUFFER_CAPACITY);
    for message in messages {
        let mut msg = Message::extract(message)?;

        // Early escape if we try and render the same message GUID twice
        // See https://github.com/ReagentX/imessage-exporter/issues/135
        if msg.rowid == current_message_row {
            current_message += 1;
            continue;
        }
        current_message_row = msg.rowid;

        if let Ok(body) = msg.parse_body(writer.config().data_source.db()) {
            msg.apply_body(body);
        }

        if msg.is_announcement() {
            let announcement = writer.format_announcement(&msg);
            let file = get_or_create_file_for(writer, &msg)?;
            file.write_all(announcement.as_bytes())
                .map_err(RuntimeError::DiskError)?;
        }
        // Message tapbacks and poll votes are rendered in context, so no need to render them separately
        else if !msg.is_tapback() && !msg.is_poll_vote() && !msg.is_poll_update() {
            msg_buf.clear();
            writer.format_message_into(&msg, 0, &mut msg_buf)?;
            let file = get_or_create_file_for(writer, &msg)?;
            file.write_all(msg_buf.as_bytes())
                .map_err(RuntimeError::DiskError)?;
        }
        current_message += 1;
        if current_message % 99 == 0 {
            writer.pb().set_position(current_message);
        }
    }
    writer.pb().finish();

    if let Some(notice) = W::footer_notice() {
        eprintln!("{notice}");
    }
    for file in writer.files_mut().values_mut() {
        W::write_file_footer(file)?;
    }
    W::write_file_footer(writer.orphaned_mut())?;

    Ok(())
}
