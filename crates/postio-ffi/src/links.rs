//! Postio's own `postio://message/<id>` links over `postio_ui::links`, for the Mac's URL routing;
//! opening one is `Session::focus_open_link`.

use postio_model::MessageId;

/// The link to `message`: `postio://message/42`.
#[uniffi::export]
pub fn message_link(message: i64) -> String {
    postio_ui::links::message_uri(MessageId::new(message))
}

/// The message `uri` names, when it is a `postio://message/<id>` link with
/// an id above zero; `None` for anything else.
#[uniffi::export]
pub fn parse_message_link(uri: String) -> Option<i64> {
    postio_ui::links::message(&uri).map(|message| message.get())
}

#[uniffi::export]
pub fn link_unknown() -> String {
    postio_ui::links::UNKNOWN.to_owned()
}

#[uniffi::export]
pub fn link_gone() -> String {
    postio_ui::links::GONE.to_owned()
}
