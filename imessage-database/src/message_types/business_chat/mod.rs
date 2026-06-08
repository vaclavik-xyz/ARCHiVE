/*!
 Apple Business Chat message family.

 Business extensions send several distinct interactive shapes that all share the
 `com.apple.icloud.apps.messages.business.extension` balloon bundle ID. Each shape
 has its own type in this module, and [`BusinessMessage`] classifies a payload
 into one of them.
*/

mod business;
mod form;
mod list_picker;
mod quick_reply;

pub use business::BusinessMessage;
pub use form::{FormAnswer, FormRequest, FormResponse};
pub use list_picker::{ListPicker, ListPickerItem};
pub use quick_reply::{QuickReply, QuickReplyOption};
