/*!
 List picker business payloads.

 List pickers store JSON in the archive's `data` field under `listPicker`.
 Prompt payloads list the available items. Reply payloads include
 `replyMessage` and move the selected item(s) into the first section.
*/

use plist::Value;

use crate::{error::plist::PlistParseError, util::plist::get_string_from_dict};

/// One item in a [`ListPicker`].
#[derive(Debug, PartialEq, Eq)]
pub struct ListPickerItem {
    /// Display title, for example `"iPhone"`.
    pub title: String,
    /// Optional secondary label.
    pub subtitle: Option<String>,
    /// Whether this item appears in the selected section of a reply.
    pub selected: bool,
}

/// Apple Business Chat list picker.
///
/// The same model represents prompts and replies. Prompts have no selected
/// [`items`](Self::items); replies mark items from the leading selected section.
#[derive(Debug, PartialEq, Eq)]
pub struct ListPicker {
    /// Template-layout or received-message heading.
    pub summary: Option<String>,
    /// Items flattened across sections in display order.
    pub items: Vec<ListPickerItem>,
}

impl ListPicker {
    /// Parse a [`ListPicker`] from a resolved business `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the payload carries no
    /// `listPicker` block.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = payload
            .as_dictionary()
            .and_then(|dict| dict.get("data"))
            .and_then(Value::as_data)
            .ok_or(PlistParseError::WrongMessageType)?;

        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;
        if !text.contains("\"listPicker\"") {
            return Err(PlistParseError::WrongMessageType);
        }

        let parsed = jzon::parse(text).map_err(|_| PlistParseError::WrongMessageType)?;
        let sections = parsed["listPicker"]["sections"]
            .as_array()
            .ok_or(PlistParseError::WrongMessageType)?;

        // Reply payloads include `replyMessage` and put the selected item(s) in
        // the first section. The section title is localized, so position is the
        // robust signal.
        let is_reply = !parsed["replyMessage"].is_null();

        let mut items = Vec::new();
        for (section_index, section) in sections.iter().enumerate() {
            let selected = is_reply && section_index == 0;
            if let Some(section_items) = section["items"].as_array() {
                for item in section_items {
                    items.push(ListPickerItem {
                        title: item["title"].as_str().unwrap_or_default().to_string(),
                        subtitle: item["subtitle"]
                            .as_str()
                            .filter(|subtitle| !subtitle.is_empty())
                            .map(str::to_string),
                        selected,
                    });
                }
            }
        }

        // Replies use `ldtext` for the selected item. The prompt heading is
        // still useful, so use `receivedMessage.title` when it exists.
        let summary = parsed["receivedMessage"]["title"]
            .as_str()
            .map(str::to_string)
            .or_else(|| get_string_from_dict(payload, "ldtext").map(str::to_string));

        Ok(ListPicker { summary, items })
    }
}

#[cfg(test)]
mod tests {
    use std::{env::current_dir, fs::File};

    use plist::Value;

    use crate::{
        message_types::business_chat::{ListPicker, ListPickerItem},
        util::plist::parse_ns_keyed_archiver,
    };

    fn parse(filename: &str) -> ListPicker {
        let path = current_dir()
            .unwrap()
            .as_path()
            .join("test_data/app_message")
            .join(filename);
        let plist = Value::from_reader(File::open(path).unwrap()).unwrap();
        ListPicker::from_map(&parse_ns_keyed_archiver(&plist).unwrap()).unwrap()
    }

    fn item(title: &str, subtitle: Option<&str>, selected: bool) -> ListPickerItem {
        ListPickerItem {
            title: title.to_string(),
            subtitle: subtitle.map(str::to_string),
            selected,
        }
    }

    #[test]
    fn test_parse_list_picker_prompt() {
        let balloon = parse("BusinessListPicker.plist");
        assert_eq!(
            balloon,
            ListPicker {
                summary: Some("Select a Product".to_string()),
                items: vec![
                    item("iPhone", None, false),
                    item("AirPods", Some("Wireless"), false),
                    item("Apple Watch", None, false),
                ],
            }
        );
    }

    #[test]
    fn test_parse_list_picker_reply() {
        let balloon = parse("BusinessListPickerResponse.plist");
        assert_eq!(
            balloon,
            ListPicker {
                summary: Some("Select a Product".to_string()),
                // In the reply fixture, the first section contains the choice.
                items: vec![
                    item("iPhone", None, true),
                    item("AirPods", Some("Wireless"), false),
                    item("Apple Watch", None, false),
                ],
            }
        );
    }
}
