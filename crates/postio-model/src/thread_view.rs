//! What reading a thread gives: each member's own line, and the unsent reply
//! (spec 013, research R6).
//!
//! Plain data, serialisable, so the store's one owner can answer a frontend
//! in another process, as [`crate::listing`] does. `postio-storage` fills
//! them in one read.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{DraftId, MessageId};

/// What a member's row says of the message, with no body fetched to say it
/// (FR-028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Line {
    /// The sender's own words, cut to a line.
    Own(String),
    /// The message is all quote: there is nothing of its own to say.
    Quoted,
    /// No body is here yet: the stored preview, cut where the attribution
    /// begins. May be empty, and the frontend then draws a placeholder.
    Preview(String),
}

/// One message of a thread, as a child row draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMember {
    /// Local id.
    pub id: MessageId,
    /// Who it is from.
    pub from: Option<EmailAddress>,
    /// Sent by one of the account's identities: "You".
    pub own: bool,
    /// When the server received it.
    pub at: DateTime<Utc>,
    /// Whether it has been read.
    pub seen: bool,
    /// Whether it has an attachment.
    pub has_attachments: bool,
    /// What the row says.
    pub line: Line,
    /// The index (in date order) of the member its `In-Reply-To` names, only
    /// when that is not the member just before it: the "to Ana" tag.
    pub answers: Option<u32>,
    /// Whether it has an open action marker.
    pub marker: bool,
}

/// An unsent reply in a thread, shown after its members.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDraft {
    /// Local id.
    pub id: DraftId,
    /// The start of what it says, cut as a preview is.
    pub first_words: String,
    /// When it was last saved.
    pub updated_at: DateTime<Utc>,
}

/// `preview` cut where an attribution line ("On Mon, Ana wrote:") begins.
///
/// A stored preview is flattened onto one line, so the line cannot be found
/// as a line: this looks for the cue that ends one ("wrote:", "a écrit :",
/// "schrieb:", "escribió:") and steps back to where the introducing words
/// ("On", "Le", "Am", "El") start. A cue with no such start loses the whole
/// sentence it ends.
pub fn before_attribution(preview: &str) -> &str {
    const CUES: [&str; 5] = ["wrote:", "a écrit :", "a écrit:", "schrieb:", "escribió:"];
    const STARTS: [&str; 4] = ["On ", "Le ", "Am ", "El "];
    let Some(cue) = CUES.iter().filter_map(|cue| preview.find(cue)).min() else {
        return preview;
    };
    let head = &preview[..cue];
    let start = STARTS
        .iter()
        .filter_map(|word| {
            head.match_indices(word)
                .map(|(at, _)| at)
                .filter(|&at| at == 0 || head[..at].ends_with(char::is_whitespace))
                .last()
        })
        .max()
        .or_else(|| head.rfind(['.', '!', '?']).map(|at| at + 1).or(Some(0)))
        .unwrap_or(0);
    head[..start].trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_attribution_and_what_it_introduces_are_cut() {
        assert_eq!(
            before_attribution("Sounds good. On Mon, 21 Sep 2026, Ana Ruiz wrote: > The draft"),
            "Sounds good."
        );
        assert_eq!(
            before_attribution("On Mon, Ana wrote: > The draft"),
            "",
            "an attribution alone is all of it"
        );
        assert_eq!(
            before_attribution("Merci. Le 21 sept., Ana a écrit : > Le brouillon"),
            "Merci."
        );
    }

    #[test]
    fn a_preview_that_introduces_nothing_stays_whole() {
        assert_eq!(
            before_attribution("On the other hand, ship it Friday."),
            "On the other hand, ship it Friday."
        );
    }
}
