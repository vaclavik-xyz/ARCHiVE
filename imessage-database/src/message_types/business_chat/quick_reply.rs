/*!
 Quick reply business payloads.

 Quick replies store JSON in the archive's `data` field. Prompt payloads list
 the available options. Reply payloads add `selectedIndex`.
*/

use plist::Value;

use crate::{error::plist::PlistParseError, util::plist::get_string_from_dict};

/// One option in a [`QuickReply`] prompt.
#[derive(Debug, PartialEq, Eq)]
pub struct QuickReplyOption {
    /// Display title, for example `"Yes"`.
    pub title: String,
}

/// Apple Business Chat quick reply.
///
/// The same model represents prompts and replies. Prompts have
/// [`options`](Self::options) and no [`selected_index`](Self::selected_index);
/// replies carry the same option list plus a selected index.
#[derive(Debug, PartialEq, Eq)]
pub struct QuickReply {
    /// Prompt summary or template-layout fallback text.
    pub summary_text: Option<String>,
    /// Options in display order.
    pub options: Vec<QuickReplyOption>,
    /// Index into [`options`](Self::options) selected by a reply.
    pub selected_index: Option<usize>,
}

impl QuickReply {
    /// Parse a [`QuickReply`] from a resolved business `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when `data` does not carry
    /// a quick reply schema.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = payload
            .as_dictionary()
            .and_then(|dict| dict.get("data"))
            .and_then(Value::as_data)
            .ok_or(PlistParseError::WrongMessageType)?;

        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;

        // Forms and legacy business payloads reuse this bundle ID. This parser
        // should only parse quick reply JSON.
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
            // Replies in this fixture have no `summaryText`.
            // `ldtext` is the fallback.
            summary_text: Some("Replied to a question".to_string()),
            options: vec![option("Yes"), option("No")],
            selected_index: Some(0),
        };
        assert_eq!(balloon, expected);
    }

    #[test]
    fn test_legacy_business_is_not_quick_reply() {
        // The legacy fixture stores hash data, not quick reply JSON.
        assert!(matches!(
            parse("Business.plist"),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
