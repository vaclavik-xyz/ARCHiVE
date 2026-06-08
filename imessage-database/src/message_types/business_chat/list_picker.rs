/*!
 List-picker business messages.

 A business extension can send a list of items (grouped into sections) for the
 recipient to choose from; the reply echoes the list with the chosen item(s)
 floated into a leading "selected" section. Both halves carry their state as a
 JSON document in the archive's `data` field, under a `listPicker` key.
*/

use plist::Value;

use crate::{error::plist::PlistParseError, util::plist::get_string_from_dict};

/// One item offered by a [`ListPicker`].
#[derive(Debug, PartialEq, Eq)]
pub struct ListPickerItem {
    /// The item's label, for example `"iPhone"`.
    pub title: String,
    /// An optional secondary label shown beneath the title.
    pub subtitle: Option<String>,
    /// Whether the recipient selected this item. Always `false` on the prompt;
    /// set on the reply for the chosen item(s).
    pub selected: bool,
}

/// An Apple Business Chat list-picker message.
///
/// This represents both halves of the interaction: the **prompt** offering a
/// list of items (none selected), and the **reply** echoing the list with the
/// chosen item(s) marked.
#[derive(Debug, PartialEq, Eq)]
pub struct ListPicker {
    /// Heading describing the list, for example `"Select a Product"`.
    pub summary: Option<String>,
    /// The items offered, flattened across sections, in display order.
    pub items: Vec<ListPickerItem>,
}

impl ListPicker {
    /// Parse a [`ListPicker`] from a balloon's resolved `NSKeyedArchiver`
    /// payload.
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

        // The reply carries `replyMessage` / `receivedMessage` and floats the
        // chosen item(s) into a leading section; the prompt has neither. We mark
        // the selection by that leading position rather than the section's
        // (localized) title.
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

        // The reply's heading lives in `receivedMessage.title` (its `ldtext` is
        // the chosen item); the prompt's heading is its `ldtext`.
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
                // The first ("You Selected") section is the chosen item.
                items: vec![
                    item("iPhone", None, true),
                    item("AirPods", Some("Wireless"), false),
                    item("Apple Watch", None, false),
                ],
            }
        );
    }
}
