use imessage_database::{
    message_types::edited::{EditStatus, EditedMessage},
    tables::messages::{Message, models::BubbleComponent},
    util::dates::{format, get_local_time, readable_diff},
};

use crate::app::runtime::Config;

/// Where an edit event sits relative to the previous one in the history.
pub enum EditDiff {
    /// First event in the history; no prior to diff against.
    First,
    /// Diff couldn't be computed (invalid timestamps or `readable_diff`
    /// returned `None`). Renderers typically suppress the prefix in this case.
    Failed,
    /// Human-readable diff between this event and the previous one
    /// (e.g. `"3 minutes"`).
    Computed(String),
}

/// Pre-computed view of one entry in an edit history. Carries both the raw
/// fields a renderer needs (`text`, `components`) and the derived values
/// callers would otherwise have to compute themselves (`diff_since_previous`,
/// `absolute_time`, `is_last`).
///
/// Events whose underlying `EditedEvent.text` is `None` are filtered out by
/// [`normalize_edited`] — they carry nothing renderable, and emitting a bare
/// timestamp row produced broken output in TXT. Their timestamps still
/// advance the diff bookkeeping for subsequent events, so chronology is
/// preserved across the gap.
pub struct NormalizedEditEvent<'a> {
    /// `true` for the final emitted event in the (filtered) history. HTML
    /// uses this to swap `<tbody>` for `<tfoot>`.
    pub is_last: bool,
    /// `event.text.as_deref()`, guaranteed `Some` since no-text events are
    /// filtered out of [`NormalizedEdit::Edited`].
    pub text: &'a str,
    /// `event.components` — passed through for renderers that need to format
    /// the original text attributes.
    pub components: &'a [BubbleComponent],
    /// Position of this event relative to the previous one in the history.
    /// See [`EditDiff`].
    pub diff_since_previous: EditDiff,
    /// Absolute time string for this event (formatted date or the timestamp
    /// error's `Display` output on failure).
    pub absolute_time: String,
}

/// Outcome of normalizing one [`EditedMessagePart`]. `None` from
/// [`normalize_edited`] means the part's status is [`EditStatus::Original`]
/// (i.e., nothing to render).
pub enum NormalizedEdit<'a> {
    /// [`EditStatus::Edited`] — at least one entry in `edit_history`.
    Edited(Vec<NormalizedEditEvent<'a>>),
    /// [`EditStatus::Unsent`]. `diff` is `Some(s)` when both `msg.date` and
    /// `msg.date_edited` are parseable AND `readable_diff` returns `Some`.
    /// Resolving the actor's display name is left to the caller, since the
    /// `is_from_me` / `custom_name` substitution rules are format-agnostic
    /// but live alongside each renderer.
    Unsent { diff: Option<String> },
}

pub fn normalize_edited<'a>(
    msg: &Message,
    edited: &'a EditedMessage,
    part_idx: usize,
    config: &Config,
) -> Option<NormalizedEdit<'a>> {
    let part = edited.part(part_idx)?;
    match part.status {
        EditStatus::Original => None,
        EditStatus::Edited => {
            let mut events = Vec::with_capacity(part.edit_history.len());
            let mut previous_timestamp: Option<i64> = None;
            for event in &part.edit_history {
                let diff_since_previous = match previous_timestamp {
                    None => EditDiff::First,
                    Some(prev) => match (
                        get_local_time(prev, config.offset),
                        get_local_time(event.date, config.offset),
                    ) {
                        (Ok(start), Ok(end)) => readable_diff(&start, &end)
                            .map(EditDiff::Computed)
                            .unwrap_or(EditDiff::Failed),
                        _ => EditDiff::Failed,
                    },
                };
                previous_timestamp = Some(event.date);

                let Some(text) = event.text.as_deref() else {
                    // No-text events carry nothing to render but their date
                    // still anchors the next event's diff.
                    continue;
                };
                let absolute_time = match get_local_time(event.date, config.offset) {
                    Ok(d) => format(&d),
                    Err(why) => why.to_string(),
                };
                events.push(NormalizedEditEvent {
                    is_last: false,
                    text,
                    components: &event.components,
                    diff_since_previous,
                    absolute_time,
                });
            }
            if let Some(last) = events.last_mut() {
                last.is_last = true;
            }
            Some(NormalizedEdit::Edited(events))
        }
        EditStatus::Unsent => {
            let diff = msg
                .date(config.offset)
                .ok()
                .zip(msg.date_edited(config.offset).ok())
                .and_then(|(s, e)| readable_diff(&s, &e));
            Some(NormalizedEdit::Unsent { diff })
        }
    }
}

#[cfg(test)]
mod tests {
    use imessage_database::message_types::edited::{
        EditStatus, EditedEvent, EditedMessage, EditedMessagePart,
    };

    use super::{EditDiff, NormalizedEdit, normalize_edited};
    use crate::{Config, Options, app::export_type::ExportType};

    // May 17, 2022  8:29:42 PM
    const DATE_A: i64 = 674526582885055488;
    // 1 hour, 49 seconds later
    const DATE_B: i64 = 674530231992568192;

    fn make_config() -> Config {
        Config::fake_app(Options::fake_options(ExportType::Txt))
    }

    fn event(date: i64, text: Option<&str>) -> EditedEvent {
        EditedEvent {
            date,
            text: text.map(str::to_string),
            components: vec![],
            guid: None,
        }
    }

    #[test]
    fn normalize_returns_none_for_original_status() {
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Original,
                edit_history: vec![],
            }],
        };
        assert!(normalize_edited(&msg, &edited, 0, &config).is_none());
    }

    #[test]
    fn normalize_returns_none_for_out_of_range_part_index() {
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage { parts: vec![] };
        assert!(normalize_edited(&msg, &edited, 0, &config).is_none());
    }

    #[test]
    fn normalize_unsent_carries_readable_diff() {
        let config = make_config();
        let mut msg = Config::fake_message();
        msg.date = DATE_A;
        msg.date_edited = DATE_B;
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Unsent,
                edit_history: vec![],
            }],
        };
        match normalize_edited(&msg, &edited, 0, &config) {
            Some(NormalizedEdit::Unsent { diff: Some(diff) }) => {
                assert_eq!(diff, "1 hour, 49 seconds");
            }
            Some(NormalizedEdit::Unsent { diff: None }) => {
                panic!("expected a computed diff, got None")
            }
            _ => panic!("expected Unsent"),
        }
    }

    #[test]
    fn normalize_unsent_diff_is_none_when_date_edited_missing() {
        let config = make_config();
        let mut msg = Config::fake_message();
        msg.date = DATE_A;
        // date_edited defaults to 0; readable_diff can't compute a diff
        // against the iMessage epoch.
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Unsent,
                edit_history: vec![],
            }],
        };
        assert!(matches!(
            normalize_edited(&msg, &edited, 0, &config),
            Some(NormalizedEdit::Unsent { diff: None })
        ));
    }

    #[test]
    fn normalize_edited_single_event_is_first_and_last() {
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Edited,
                edit_history: vec![event(DATE_A, Some("hi"))],
            }],
        };
        match normalize_edited(&msg, &edited, 0, &config) {
            Some(NormalizedEdit::Edited(events)) => {
                assert_eq!(events.len(), 1);
                assert!(events[0].is_last, "single event must be is_last");
                assert!(matches!(events[0].diff_since_previous, EditDiff::First));
                assert_eq!(events[0].text, "hi");
            }
            _ => panic!("expected Edited"),
        }
    }

    #[test]
    fn normalize_edited_multiple_events_diffs_against_previous() {
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Edited,
                edit_history: vec![event(DATE_A, Some("first")), event(DATE_B, Some("second"))],
            }],
        };
        match normalize_edited(&msg, &edited, 0, &config) {
            Some(NormalizedEdit::Edited(events)) => {
                assert_eq!(events.len(), 2);
                assert!(!events[0].is_last);
                assert!(events[1].is_last);
                assert!(matches!(events[0].diff_since_previous, EditDiff::First));
                match &events[1].diff_since_previous {
                    EditDiff::Computed(diff) => assert_eq!(diff, "1 hour, 49 seconds"),
                    _ => panic!("expected Computed diff for second event"),
                }
            }
            _ => panic!("expected Edited"),
        }
    }

    #[test]
    fn normalize_edited_skips_no_text_events_but_diffs_through_them() {
        // No-text events render as empty rows (in HTML) or bare timestamp
        // prefixes (in TXT, which was outright broken — no newline emitted),
        // so we drop them at the normalization layer. Their dates still
        // anchor the diff for the *next* text-bearing event so chronology
        // across the gap is preserved.
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Edited,
                edit_history: vec![
                    event(DATE_A, None),
                    event(DATE_B, Some("after the gap")),
                ],
            }],
        };
        match normalize_edited(&msg, &edited, 0, &config) {
            Some(NormalizedEdit::Edited(events)) => {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0].text, "after the gap");
                // Diff must be against DATE_A (the skipped event), not First.
                match &events[0].diff_since_previous {
                    EditDiff::Computed(diff) => assert_eq!(diff, "1 hour, 49 seconds"),
                    _ => panic!("expected Computed diff against the skipped event"),
                }
                assert!(events[0].is_last);
            }
            _ => panic!("expected Edited"),
        }
    }

    #[test]
    fn normalize_edited_all_no_text_yields_empty_events() {
        // Defensive edge case: every history entry parses to text=None.
        // Renderers should get an empty rows list and emit nothing
        // structural for it (no empty table / no stray prefixes).
        let config = make_config();
        let msg = Config::fake_message();
        let edited = EditedMessage {
            parts: vec![EditedMessagePart {
                status: EditStatus::Edited,
                edit_history: vec![event(DATE_A, None), event(DATE_B, None)],
            }],
        };
        match normalize_edited(&msg, &edited, 0, &config) {
            Some(NormalizedEdit::Edited(events)) => assert!(events.is_empty()),
            _ => panic!("expected Edited with empty events"),
        }
    }
}
