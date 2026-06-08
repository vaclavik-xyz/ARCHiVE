/*!
 Classification for Apple Business Chat payloads.

 The business extension uses one balloon bundle ID for several payload schemas.
 The bundle ID gets us into this module; the decoded payload determines the
 concrete renderer.
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

/// Parsed business payload supported by the exporters.
#[derive(Debug, PartialEq, Eq)]
pub enum BusinessMessage {
    /// [`QuickReply`] prompt, optionally with the selected option on replies.
    QuickReply(QuickReply),
    /// [`FormRequest`] asking the user to fill out an interactive form.
    FormRequest(FormRequest),
    /// [`FormResponse`] with the answers the user gave.
    FormResponse(FormResponse),
    /// [`ListPicker`] prompt or reply, with selected items marked on replies.
    ListPicker(ListPicker),
}

impl BusinessMessage {
    /// Classify a resolved business `NSKeyedArchiver` payload.
    ///
    /// Returns [`PlistParseError::WrongMessageType`] when the payload carries no
    /// supported business schema. Callers use that to preserve the generic app
    /// card fallback for older query-string payloads.
    pub fn from_map(payload: &Value) -> Result<Self, PlistParseError> {
        if let Ok(quick_reply) = QuickReply::from_map(payload) {
            return Ok(Self::QuickReply(quick_reply));
        }
        // Form responses carry `dynamic.selections`. Requests also carry
        // `dynamic`, so the more specific parser has to run first.
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
        // Legacy query-string payloads have no supported business schema.
        assert!(matches!(
            classify("Business.plist"),
            Err(PlistParseError::WrongMessageType)
        ));
    }
}
