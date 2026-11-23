//! What a composer asks before it sends: the questions and their words, shared by every frontend.

use postio_model::Draft;

pub const NO_RECIPIENTS: &str = "not sent — add a recipient first";

pub const ALREADY_QUEUED: &str = "not sent again — this draft is already on its way";

/// Its Undo cancels the send (#1752).
pub const QUEUED_TO_SEND: &str = "Message queued to send";

pub const SEND_SCHEDULED: &str = "Send scheduled";

/// Each entry is a clause, since they are joined into one.
pub fn send_concerns(draft: &Draft) -> Vec<String> {
    let mut concerns = Vec::new();
    // Asked, never refused: sending with no subject is the user's call.
    if draft.subject.trim().is_empty() {
        concerns.push("this message has no subject".to_owned());
    }
    if postio_model::mention::mentions_an_attachment(draft) {
        concerns.push("it mentions an attachment and does not carry one".to_owned());
    }
    concerns
}

pub fn join_with_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
