/*!
 Quick-reply business messages.

 A business extension can send an interactive prompt that offers the recipient a
 short list of options to tap (for example `"Yes"` / `"No"`). Tapping one sends a
 reply back that records which option was chosen. Both halves carry their state
 as a JSON document in the archive's `data` field.
*/

use plist::Value;

use crate::{error::plist::PlistParseError, util::plist::get_string_from_dict};

/// One tappable option offered by a [`QuickReply`] prompt.
#[derive(Debug, PartialEq, Eq)]
pub struct QuickReplyOption {
    /// The user-facing label for the option, for example `"Yes"`.
    pub title: String,
}

/// An Apple Business Chat quick-reply message.
///
/// This represents both halves of the interaction:
/// - the **prompt** a business sends (a [`summary_text`](Self::summary_text)
///   and a list of [`options`](Self::options), with no selection), and
/// - the **reply** the recipient sends back by tapping an option (the same
///   options plus a [`selected_index`](Self::selected_index)).
#[derive(Debug, PartialEq, Eq)]
pub struct QuickReply {
    /// Heading describing the prompt, for example `"Choose an option"`. Sent
    /// replies carry no `summaryText`, so this falls back to the layout's
    /// `ldtext` (for example `"Replied to a question"`).
    pub summary_text: Option<String>,
    /// The options offered, in display order.
    pub options: Vec<QuickReplyOption>,
    /// The index into [`options`](Self::options) that was selected. Present
    /// only on the sent reply; `None` on the original prompt.
    pub selected_index: Option<usize>,
}

impl QuickReply {
    /// Parse a [`QuickReply`] from a balloon's resolved `NSKeyedArchiver`
    /// payload.
    ///
    /// The interactive content is a JSON document stored in the archive's
    /// `data` field. Returns [`PlistParseError::WrongMessageType`] when the
    /// payload is some other `business.extension` shape (an interactive form or
    /// a legacy hash) that carries no quick reply.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = payload
            .as_dictionary()
            .and_then(|dict| dict.get("data"))
            .and_then(Value::as_data)
            .ok_or(PlistParseError::WrongMessageType)?;

        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;

        // Other business balloons (forms, legacy hashes) reuse this bundle ID
        // without a quick reply. This guard keeps those (sometimes very large)
        // payloads off the JSON path.
        if !text.contains("\"quick-reply\"") {
            return Err(PlistParseError::WrongMessageType);
        }

        let parsed = jzon::parse(text).map_err(|_| PlistParseError::WrongMessageType)?;
        let quick_reply = &parsed["quick-reply"];

        let options = quick_reply["items"]
            .as_array()
            .ok_or(PlistParseError::WrongMessageType)?
            .iter()
            .map(|item| QuickReplyOption {
                title: item["title"].as_str().unwrap_or_default().to_string(),
            })
            .collect();

        let selected_index = quick_reply["selectedIndex"].as_usize();

        let summary_text = quick_reply["summaryText"]
            .as_str()
            .map(str::to_string)
            .or_else(|| get_string_from_dict(payload, "ldtext").map(str::to_string));

        Ok(QuickReply {
            summary_text,
            options,
            selected_index,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{env::current_dir, fs::File};

    use plist::Value;

    use crate::{
        error::plist::PlistParseError,
        message_types::business_chat::{QuickReply, QuickReplyOption},
        util::plist::parse_ns_keyed_archiver,
    };

    fn parse(filename: &str) -> Result<QuickReply, PlistParseError> {
        let plist_path = current_dir()
            .unwrap()
            .as_path()
            .join("test_data/app_message")
            .join(filename);
        let plist_data = File::open(plist_path).unwrap();
        let plist = Value::from_reader(plist_data).unwrap();
        let parsed = parse_ns_keyed_archiver(&plist).unwrap();
        QuickReply::from_map(&parsed)
    }

    fn option(title: &str) -> QuickReplyOption {
        QuickReplyOption {
            title: title.to_string(),
        }
    }

    #[test]
    fn test_parse_business_quick_reply_prompt() {
        let balloon = parse("BusinessQuickReply.plist").unwrap();
        let expected = QuickReply {
            summary_text: Some("Choose an option".to_string()),
            options: vec![option("Yes"), option("No")],
            selected_index: None,
        };
        assert_eq!(balloon, expected);
    }

    #[test]
    fn test_parse_business_quick_reply_response() {
        let balloon = parse("BusinessQuickReplyResponse.plist").unwrap();
        let expected = QuickReply {
            // The reply has no `summaryText`; the heading comes from `ldtext`.
            summary_text: Some("Replied to a question".to_string()),
            options: vec![option("Yes"), option("No")],
            selected_index: Some(0),
        };
        assert_eq!(balloon, expected);
    }

    #[test]
    fn test_legacy_business_is_not_quick_reply() {
        // The legacy query-string business format stores a hash in `data`, not a
        // quick reply, so it must be rejected and routed to the generic-app
        // fallback rather than rendered as an interactive message.
        assert!(matches!(
            parse("Business.plist"),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
