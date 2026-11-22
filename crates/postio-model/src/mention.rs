//! Did they say "attached" and attach nothing? Fires only on the user's own words: a quoted
//! original is somebody else's writing and a signature is boilerplate. Plain text, so the check
//! stays in `postio-model` where every frontend reaches it.

use crate::Draft;
use crate::signature;

/// Short on purpose: every addition is a new way to be wrong at somebody who did nothing wrong; a
/// miss only leaves the check quiet.
const WORDS: [&str; 6] = [
    "attached",
    "attachment",
    "attachments",
    "attaching",
    "enclosed",
    "enclosure",
];

/// `false` when it carries any part, inline images included: a pasted screenshot is visibly there.
pub fn mentions_an_attachment(draft: &Draft) -> bool {
    if !draft.attachments.is_empty() {
        return false;
    }
    let Some(body) = draft.body.text.as_deref() else {
        return false;
    };
    // The signature goes first, so a sign-off that quotes something cannot
    // sneak back in through the line filter below.
    let written = signature::split(body).0;
    written
        .lines()
        .filter(|line| !line.trim_start().starts_with('>'))
        .any(says_so)
}

fn says_so(line: &str) -> bool {
    line.split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            WORDS
                .iter()
                .any(|candidate| word.eq_ignore_ascii_case(candidate))
        })
}
