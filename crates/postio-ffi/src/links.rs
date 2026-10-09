//! Postio's own links at the boundary (specs/009-focus-macos T111, for the
//! Mac's T117): `postio://message/<id>`, over `postio_ui::links`, so the
//! Mac reads and writes a link exactly as the line a capture writes it.
//!
//! Opening one is `Session::focus_open_link`, which looks the message up and
//! opens it, or says `link_unknown`/`link_gone` as a toast; these are for
//! the Mac's URL routing, which wants to know before it asks.

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

/// What a link that is not one of Postio's says.
#[uniffi::export]
pub fn link_unknown() -> String {
    postio_ui::links::UNKNOWN.to_owned()
}

/// What a link to a message no longer on this computer says.
#[uniffi::export]
pub fn link_gone() -> String {
    postio_ui::links::GONE.to_owned()
}
