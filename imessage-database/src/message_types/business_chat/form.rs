/*!
 Interactive form business messages.

 A business extension can ask the recipient to fill out a multi-page form
 ([`FormRequest`]); submitting it sends back the answers ([`FormResponse`]). Both
 carry their state as a JSON document in the archive's `data` field, under a
 `dynamic` block.
*/

use plist::Value;

use crate::{error::plist::PlistParseError, util::plist::get_string_from_dict};

/// One answered question in a [`FormResponse`].
#[derive(Debug, PartialEq, Eq)]
pub struct FormAnswer {
    /// The question that was asked.
    pub question: String,
    /// The answer(s) the user gave, in order. A single-selection question has
    /// one; a multiple-selection question may have several.
    pub answers: Vec<String>,
}

/// A request to fill out an Apple Business Chat interactive form.
///
/// The request also carries the entire blank form template, which we do not
/// surface; only the title and subtitle are rendered.
#[derive(Debug, PartialEq, Eq)]
pub struct FormRequest {
    /// The form's title, for example `"Report an Issue"`.
    pub title: Option<String>,
    /// The form's subtitle, for example `"Tap to get started"`.
    pub subtitle: Option<String>,
}

/// A submitted Apple Business Chat interactive form.
#[derive(Debug, PartialEq, Eq)]
pub struct FormResponse {
    /// Heading describing the submission, for example
    /// `"Here's my completed form"`.
    pub summary: Option<String>,
    /// The questions asked and the answers the user gave, in order.
    pub answers: Vec<FormAnswer>,
}

/// Read the `data` field a business balloon stores its JSON payload in.
fn form_data(payload: &Value) -> Result<&[u8], PlistParseError> {
    payload
        .as_dictionary()
        .and_then(|dict| dict.get("data"))
        .and_then(Value::as_data)
        .ok_or(PlistParseError::WrongMessageType)
}

impl FormRequest {
    /// Parse a [`FormRequest`] from a balloon's resolved `NSKeyedArchiver`
    /// payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the payload carries no
    /// `dynamic` form block (for example a legacy business message).
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = form_data(payload)?;
        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;

        // A `dynamic` block is what makes a payload a form; legacy business
        // messages have none.
        if !text.contains("\"dynamic\"") {
            return Err(PlistParseError::WrongMessageType);
        }

        let user_info = payload.as_dictionary().and_then(|dict| dict.get("userInfo"));
        let title = user_info
            .and_then(|info| get_string_from_dict(info, "caption"))
            .or_else(|| get_string_from_dict(payload, "ldtext"))
            .map(str::to_string);
        let subtitle = user_info
            .and_then(|info| get_string_from_dict(info, "subcaption"))
            .map(str::to_string);

        Ok(FormRequest { title, subtitle })
    }
}

impl FormResponse {
    /// Parse a [`FormResponse`] from a balloon's resolved `NSKeyedArchiver`
    /// payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the payload has no
    /// submitted answers in `dynamic.selections` (for example a blank
    /// [`FormRequest`]).
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = form_data(payload)?;
        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;

        if !text.contains("\"selections\"") {
            return Err(PlistParseError::WrongMessageType);
        }

        let parsed = jzon::parse(text).map_err(|_| PlistParseError::WrongMessageType)?;
        let selections = parsed["dynamic"]["selections"]
            .as_array()
            .filter(|selections| !selections.is_empty())
            .ok_or(PlistParseError::WrongMessageType)?;

        let answers = selections
            .iter()
            .map(|page| FormAnswer {
                question: page["subtitle"].as_str().unwrap_or_default().to_string(),
                answers: page["items"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| item["title"].as_str().unwrap_or_default().to_string())
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();

        let summary = get_string_from_dict(payload, "ldtext").map(str::to_string);

        Ok(FormResponse { summary, answers })
    }
}

#[cfg(test)]
mod tests {
    use std::{env::current_dir, fs::File};

    use plist::Value;

    use crate::{
        error::plist::PlistParseError,
        message_types::business_chat::{FormAnswer, FormRequest, FormResponse},
        util::plist::parse_ns_keyed_archiver,
    };

    fn archive(filename: &str) -> Value {
        let path = current_dir()
            .unwrap()
            .as_path()
            .join("test_data/app_message")
            .join(filename);
        let plist = Value::from_reader(File::open(path).unwrap()).unwrap();
        parse_ns_keyed_archiver(&plist).unwrap()
    }

    fn answer(question: &str, value: &str) -> FormAnswer {
        FormAnswer {
            question: question.to_string(),
            answers: vec![value.to_string()],
        }
    }

    #[test]
    fn test_parse_form_request() {
        let request = FormRequest::from_map(&archive("BusinessFormRequest.plist")).unwrap();
        assert_eq!(
            request,
            FormRequest {
                title: Some("Report an Issue".to_string()),
                subtitle: Some("Tap to get started".to_string()),
            }
        );
    }

    #[test]
    fn test_parse_form_response() {
        let response = FormResponse::from_map(&archive("BusinessFormResponse.plist")).unwrap();
        assert_eq!(
            response,
            FormResponse {
                summary: Some("Here's my completed form".to_string()),
                answers: vec![
                    answer(
                        "Which option best describes your request?",
                        "The first example option"
                    ),
                    answer("When did this happen?", "01/01/2024"),
                    answer("Anything else to add?", "Example free-text response."),
                ],
            }
        );
    }

    #[test]
    fn test_form_response_requires_selections() {
        // A blank request carries no answers, so it is not a response.
        assert!(matches!(
            FormResponse::from_map(&archive("BusinessFormRequest.plist")),
            Err(PlistParseError::WrongMessageType)
        ));
    }

    #[test]
    fn test_form_request_rejects_legacy() {
        // Legacy business messages have no `dynamic` form block.
        assert!(matches!(
            FormRequest::from_map(&archive("Business.plist")),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
