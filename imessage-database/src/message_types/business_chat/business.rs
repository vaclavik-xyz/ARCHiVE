/*!
 Classification for the Apple Business Chat message family.

 Every business shape shares one balloon bundle ID, so they cannot be told apart
 by bundle ID; [`BusinessMessage::from_map`] inspects the payload instead.
*/

use plist::Value;

use crate::{
    error::plist::PlistParseError,
    message_types::business_chat::{
        form::{FormRequest, FormResponse},
        list_picker::ListPicker,
        quick_reply::QuickReply,
    },
};

/// One of the interactive shapes carried by the business balloon.
#[derive(Debug, PartialEq, Eq)]
pub enum BusinessMessage {
    /// A tappable list of options, and on a reply which one was chosen.
    QuickReply(QuickReply),
    /// A request to fill out an interactive form.
    FormRequest(FormRequest),
    /// A submitted interactive form, with the answers the user gave.
    FormResponse(FormResponse),
    /// A list of items to choose from, and on a reply which were chosen.
    ListPicker(ListPicker),
}

impl BusinessMessage {
    /// Classify a business balloon's resolved `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the payload carries no
    /// shape we render richly (for example a legacy query-string business
    /// message), so callers can fall back to the generic app-card renderer.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        if let Ok(quick_reply) = QuickReply::from_map(payload) {
            return Ok(Self::QuickReply(quick_reply));
        }
        // A submitted form carries answers in `dynamic.selections`; a blank
        // request does not, so the response must be tried before the request.
        if let Ok(response) = FormResponse::from_map(payload) {
            return Ok(Self::FormResponse(response));
        }
        if let Ok(request) = FormRequest::from_map(payload) {
            return Ok(Self::FormRequest(request));
        }
        if let Ok(list_picker) = ListPicker::from_map(payload) {
            return Ok(Self::ListPicker(list_picker));
        }
        Err(PlistParseError::WrongMessageType)
    }
}

#[cfg(test)]
mod tests {
    use std::{env::current_dir, fs::File};

    use plist::Value;

    use crate::{
        error::plist::PlistParseError, message_types::business_chat::BusinessMessage,
        util::plist::parse_ns_keyed_archiver,
    };

    fn classify(filename: &str) -> Result<BusinessMessage, PlistParseError> {
        let path = current_dir()
            .unwrap()
            .join("test_data/app_message")
            .join(filename);
        let plist = Value::from_reader(File::open(path).unwrap()).unwrap();
        BusinessMessage::from_map(&parse_ns_keyed_archiver(&plist).unwrap())
    }

    #[test]
    fn classifies_quick_reply() {
        assert!(matches!(
            classify("BusinessQuickReply.plist"),
            Ok(BusinessMessage::QuickReply(_))
        ));
    }

    #[test]
    fn classifies_form_request() {
        assert!(matches!(
            classify("BusinessFormRequest.plist"),
            Ok(BusinessMessage::FormRequest(_))
        ));
    }

    #[test]
    fn classifies_form_response() {
        assert!(matches!(
            classify("BusinessFormResponse.plist"),
            Ok(BusinessMessage::FormResponse(_))
        ));
    }

    #[test]
    fn classifies_list_picker_prompt() {
        assert!(matches!(
            classify("BusinessListPicker.plist"),
            Ok(BusinessMessage::ListPicker(_))
        ));
    }

    #[test]
    fn classifies_list_picker_reply() {
        assert!(matches!(
            classify("BusinessListPickerResponse.plist"),
            Ok(BusinessMessage::ListPicker(_))
        ));
    }

    #[test]
    fn legacy_business_falls_through() {
        // Legacy query-string business messages have no rich shape and must
        // route to the generic-app fallback.
        assert!(matches!(
            classify("Business.plist"),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
