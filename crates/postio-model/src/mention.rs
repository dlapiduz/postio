//! Did they say "attached" and attach nothing? (spec 002, FR-057)
//!
//! The one composer check whose whole value is catching a mistake the person
//! has already made and cannot see. It costs a confirmation when it is wrong
//! and saves a second email when it is right, so the bar for firing is "the
//! user said so, in their own words" rather than anything cleverer.
//!
//! # Whose words
//!
//! Two exclusions do most of the work, and both are about authorship rather
//! than about vocabulary.
//!
//! A **quoted original** is somebody else's writing. Replying to "here is the
//! report, attached" must not ask whether *you* forgot something — and that
//! is not a rare case, it is most of the replies anyone sends to a message
//! with a file on it. Firing there teaches people to dismiss the dialog
//! unread, which costs the check everything it was for.
//!
//! A **signature** is boilerplate. Somebody whose sign-off mentions
//! attachments would otherwise be asked on every message they ever send.
//!
//! # Why plain text
//!
//! The draft's text alternative is the same prose as its HTML and needs no
//! parser, which keeps this in `postio-model` where the composer, a future
//! frontend and a rule engine can all reach it. Quoting is `> ` by the same
//! convention every mail client uses, and a signature starts at the RFC 3676
//! separator — both already spelled in this crate.

use crate::Draft;
use crate::signature;

/// The words people use. Matched whole, case-insensitively.
///
/// Short on purpose. Every addition is a new way to be wrong at somebody who
/// did nothing wrong, and the cost of missing one is that the check stays
/// quiet — which is the failure it is allowed to have.
const WORDS: [&str; 6] = [
    "attached",
    "attachment",
    "attachments",
    "attaching",
    "enclosed",
    "enclosure",
];

/// Whether this draft claims to carry something it does not.
///
/// `false` when it carries any part at all, inline images included: a pasted
/// screenshot is visibly there, and asking "did you forget an attachment?"
/// about a message with a picture in it is the app not looking at what the
/// person can plainly see.
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

/// Whether one line of the user's own prose says it.
fn says_so(line: &str) -> bool {
    line.split(|character: char| !character.is_alphanumeric())
        .any(|word| {
            WORDS
                .iter()
                .any(|candidate| word.eq_ignore_ascii_case(candidate))
        })
}
