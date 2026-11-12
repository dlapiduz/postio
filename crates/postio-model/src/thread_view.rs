//! What reading a thread gives: each member's own line, and the unsent reply
//! (spec 013, research R6).
//!
//! Plain data, serialisable, so the store's one owner can answer a frontend
//! in another process, as [`crate::listing`] does. `postio-storage` fills
//! them in one read.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::draft::Draft;
use crate::ids::{DraftId, MessageId, ThreadId};
use crate::label::Label;

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

/// A thread as a window or an expansion reads it (`Req::Thread`): its
/// members in date order, the unsent reply after them, and its labels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadView {
    /// The thread.
    pub thread: ThreadId,
    /// Its subject, when it has one.
    pub subject: Option<String>,
    /// Every member, oldest first.
    pub members: Vec<ThreadMember>,
    /// An unsent reply, shown after the members.
    pub draft: Option<ThreadDraft>,
    /// Replies since the thread was muted, when it is. Always `None` until
    /// mute arrives (spec 013, US5).
    pub muted: Option<u32>,
    /// Every label any member carries, each once.
    pub labels: Vec<Label>,
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

/// What a forward carries (spec 013 FR-023): the chooser's three cards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ForwardForm {
    /// One message, quoted as a forward always has been. The default.
    #[default]
    This,
    /// Every message, oldest first, in one readable message, with every
    /// attachment once.
    Thread,
    /// Each message as an `.eml` attachment.
    Attachments,
}

/// What building a forward answered (`Req::ForwardThread`,
/// `Req::SetForwardForm`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forwarded {
    /// The draft, not yet saved: the composer's autosave gives it an id, as
    /// for any reply.
    Draft(Box<Draft>),
    /// These messages are not on this machine (a body, a raw source or an
    /// attachment's bytes), so the form was refused. Nothing was fetched
    /// (FR-028).
    NotLocal(Vec<MessageId>),
}
