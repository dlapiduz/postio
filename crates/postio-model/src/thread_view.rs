//! Plain serialisable data (like `listing`), so the store's one owner can answer a frontend in
//! another process; `postio-storage` fills it in one read.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::draft::Draft;
use crate::ids::{DraftId, MessageId, ThreadId};
use crate::label::Label;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Line {
    Own(String),
    Quoted,
    /// No body is here yet: the stored preview, cut where the attribution
    /// begins. May be empty, and the frontend then draws a placeholder.
    Preview(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMember {
    pub id: MessageId,
    pub from: Option<EmailAddress>,
    pub own: bool,
    pub at: DateTime<Utc>,
    pub seen: bool,
    pub has_attachments: bool,
    pub line: Line,
    /// The index (in date order) of the member its `In-Reply-To` names, only
    /// when that is not the member just before it: the "to Ana" tag.
    pub answers: Option<u32>,
    pub marker: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadDraft {
    pub id: DraftId,
    pub first_words: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadView {
    pub thread: ThreadId,
    pub subject: Option<String>,
    pub members: Vec<ThreadMember>,
    pub draft: Option<ThreadDraft>,
    pub muted: Option<u32>,
    pub labels: Vec<Label>,
}

/// A stored preview is one line, so this looks for the cue that ends an attribution ("wrote:", "a
/// écrit :", "schrieb:", "escribió:") and steps back to where its introducing words start. A cue
/// with no such start loses the whole sentence it ends.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ForwardForm {
    #[default]
    This,
    Thread,
    Attachments,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Forwarded {
    Draft(Box<Draft>),
    /// Not on this machine (a body, raw source or attachment bytes), so the form was refused and
    /// nothing was fetched.
    NotLocal(Vec<MessageId>),
}
