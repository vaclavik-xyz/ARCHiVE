use std::collections::HashMap;

use imessage_database::tables::{
    attachment::Attachment,
    messages::{
        Message,
        models::{AttributedRange, BubbleComponent},
    },
};

use crate::exporters::formatter::{MessageFormatter, PartBodyBuilder};

/// Resolve each attachment range in `ranges` to an index into `attachments`,
/// matching by file-transfer GUID and falling back to positional order
/// (starting at `positional_start`) for ranges that carry no GUID (the legacy
/// (non-typedstream) parse path).
///
/// The message body lists attachment placeholders in display order, but
/// [`Attachment::from_message`](imessage_database::tables::attachment::Attachment::from_message)
/// returns rows in the join's (unspecified) order, so pairing placeholders to
/// attachments by position can mis-order a message with several attachments.
/// Pairing by GUID keeps every placeholder bound to its own attachment.
///
/// Returns one index per attachment range, in range order.
pub(crate) fn resolve_run_attachment_indices(
    ranges: &[AttributedRange],
    attachments: &[Attachment],
    positional_start: usize,
) -> Vec<usize> {
    let guid_to_idx: HashMap<&str, usize> = attachments
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.guid.as_deref().map(|g| (g, i)))
        .collect();

    let mut out = Vec::new();
    let mut positional = positional_start;
    for range in ranges {
        if let Some(meta) = &range.attachment {
            let idx = meta
                .guid
                .as_deref()
                .and_then(|g| guid_to_idx.get(g).copied())
                .unwrap_or(positional);
            out.push(idx);
            positional += 1;
        }
    }
    out
}

/// Walks `message_part` and produces the format's part-body. Owns the
/// format-agnostic control flow:
///
///  - run-vs-app-vs-retracted branching
///  - the part-edited / Retracted edit-history dance
///
/// A plain (non-edited) [`Run`](BubbleComponent::Run)–a bubble's worth of
/// attributed ranges–is delegated to the format's
/// [`MessageFormatter::render_run`], which interleaves text and inline
/// attachments, advances `attachment_index`, and applies translation. App and
/// Retracted leaves are wrapped via the format's [`PartBodyBuilder`] impl.
pub(crate) fn dispatch_part_body<'a, F>(
    formatter: &'a F,
    message: &'a Message,
    idx: usize,
    message_part: &'a BubbleComponent,
    attachments: &'a mut Vec<Attachment>,
    attachment_index: &mut usize,
) -> F::Body
where
    F: MessageFormatter<'a> + PartBodyBuilder,
{
    match message_part {
        BubbleComponent::Run(ranges) => {
            // An edited part renders its edit history in place of the live body.
            if message.is_part_edited(idx) {
                return match &message.edited_parts {
                    Some(edited_parts) => match formatter.format_edited(message, edited_parts, idx)
                    {
                        Some(rendered) => formatter.body_text_edited(rendered),
                        None => formatter.body_empty(),
                    },
                    None => formatter.body_empty(),
                };
            }
            formatter.render_run(message, ranges, attachments, attachment_index)
        }
        BubbleComponent::App => match formatter.format_app(message, attachments) {
            Ok(content) => formatter.body_app(content),
            Err(why) => formatter.body_app_error(message, why.to_string()),
        },
        BubbleComponent::Retracted => match &message.edited_parts {
            Some(edited_parts) => match formatter.format_edited(message, edited_parts, idx) {
                Some(content) => formatter.body_retracted(content),
                None => formatter.body_empty(),
            },
            None => formatter.body_empty(),
        },
    }
}
