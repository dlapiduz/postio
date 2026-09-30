//! What a composer asks before it sends: the questions and their words.
//!
//! Shared so the desktop's dialog and the terminal's status line ask the same
//! things about the same message.

use postio_model::Draft;

/// What the status line says when `ctrl+Return` is pressed on a draft that is
/// addressed to nobody.
pub const NO_RECIPIENTS: &str = "not sent — add a recipient first";

/// What the status line says when `ctrl+Return` is pressed on a draft that has
/// already been handed over: it is the queue's now, not the composer's.
pub const ALREADY_QUEUED: &str = "not sent again — this draft is already on its way";

/// What is odd about this message, in the words the dialog uses.
///
/// Empty for a message with nothing odd about it, which is almost all of
/// them. Each entry is a clause rather than a sentence, because they are
/// joined into one.
pub fn send_concerns(draft: &Draft) -> Vec<String> {
    let mut concerns = Vec::new();
    // FR-018. Asked, never refused: a message with no subject is a perfectly
    // ordinary thing to send on purpose, and refusing it would be the app
    // having an opinion about someone else's correspondence.
    if draft.subject.trim().is_empty() {
        concerns.push("this message has no subject".to_owned());
    }
    // FR-057.
    if postio_model::mention::mentions_an_attachment(draft) {
        concerns.push("it mentions an attachment and does not carry one".to_owned());
    }
    concerns
}

/// `a`, `a and b`, `a, b and c`.
pub fn join_with_and(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
