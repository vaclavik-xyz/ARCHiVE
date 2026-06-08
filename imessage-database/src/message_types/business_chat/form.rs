/*!
 Interactive form business payloads.

 Form requests and responses both store JSON in the archive's `data` field.
 Requests expose their display text through the template layout; responses also
 include submitted answers under `dynamic.selections`.
*/

use plist::Value;

use crate::{
    error::plist::PlistParseError,
    util::plist::{get_data_from_dict, get_string_from_dict},
};

/// One submitted answer group in a [`FormResponse`].
#[derive(Debug, PartialEq, Eq)]
pub struct FormAnswer {
    /// Prompt shown for this answer group.
    pub question: String,
    /// Submitted values, in display order.
    pub answers: Vec<String>,
}

/// Apple Business Chat form request.
///
/// The request can carry a blank form definition. The exporters only need the
/// template-layout title and subtitle.
#[derive(Debug, PartialEq, Eq)]
pub struct FormRequest {
    /// Template-layout title, for example `"Report an Issue"`.
    pub title: Option<String>,
    /// Template-layout subtitle, for example `"Tap to get started"`.
    pub subtitle: Option<String>,
}

/// Submitted Apple Business Chat form.
#[derive(Debug, PartialEq, Eq)]
pub struct FormResponse {
    /// Template-layout summary, for example `"Here's my completed form"`.
    pub summary: Option<String>,
    /// Submitted answer groups, in payload order.
    pub answers: Vec<FormAnswer>,
}

/// Read the `data` field that stores the business JSON payload.
fn form_data(payload: &Value) -> Result<&[u8], PlistParseError> {
    get_data_from_dict(payload, "data").ok_or(PlistParseError::WrongMessageType)
}

impl FormRequest {
    /// Parse a [`FormRequest`] from a resolved business `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the `data` field does
    /// not look like a form request.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        let data = form_data(payload)?;
        let text = std::str::from_utf8(data).map_err(|_| PlistParseError::WrongMessageType)?;

        // Form requests include a `dynamic` marker. Legacy business payloads
        // store query-string/hash data instead.
        if !text.contains("\"dynamic\"") {
            return Err(PlistParseError::WrongMessageType);
        }

        let user_info = payload
            .as_dictionary()
            .and_then(|dict| dict.get("userInfo"));
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
    /// Parse a [`FormResponse`] from a resolved business `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when `dynamic.selections`
    /// is missing or empty.
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
        // A blank request has no submitted answers.
        assert!(matches!(
            FormResponse::from_map(&archive("BusinessFormRequest.plist")),
            Err(PlistParseError::WrongMessageType)
        ));
    }

    #[test]
    fn test_form_request_rejects_legacy() {
        // The legacy fixture stores query-string/hash data, not form JSON.
        assert!(matches!(
            FormRequest::from_map(&archive("Business.plist")),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
