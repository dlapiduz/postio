//! What one fetch of a message's bytes was for, and what asking for a part
//! came to.
//!
//! The engine fetches a message in pieces (ADR 0017): its words on the text
//! axis, its attachments by section on the payload axis. Whoever waits on a
//! fetch waits on one of those pieces, so the news that a fetch landed or
//! failed names the piece ([`Sections`]), and a part wait does not end on its
//! message's text failing (#1996). [`PayloadAsk`] is what a request for parts
//! answers before anything is fetched, so a person told "no" is told why.

use serde::{Deserialize, Serialize};

/// Which of a message's bytes one fetch brought, or tried to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sections {
    /// The sections holding the message's words.
    Text,
    /// Named payload parts, by MIME path (`2`, `2.1`).
    Parts(Vec<String>),
    /// Every byte, which holds every part.
    Whole,
    /// The header block alone.
    Headers,
}

impl Sections {
    /// Whether this fetch carried `part`'s bytes: a part wait listens for
    /// exactly these.
    pub fn holds_part(&self, part: &str) -> bool {
        match self {
            Sections::Parts(parts) => parts.iter().any(|named| named == part),
            Sections::Whole => true,
            Sections::Text | Sections::Headers => false,
        }
    }

    /// Whether this fetch carried the message's words: what the reading
    /// pane's ask for a body waits on.
    pub fn holds_text(&self) -> bool {
        matches!(self, Sections::Text | Sections::Whole)
    }
}

/// What asking for a message's parts came to, before anything was fetched.
///
/// Only [`Queued`](Self::Queued) means bytes are coming soon; every other
/// answer is a different sentence for the person who asked (#1987, #1996).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadAsk {
    /// At the front of the queue: the bytes land as a `BodyLoaded` naming
    /// the parts, or a `BodyFailed` does.
    Queued,
    /// Queued, but the account cannot reach its server now (no network, or
    /// stopped until the person signs in again). The bytes come when it can.
    Unreachable,
    /// Every part named already has its bytes here.
    Local,
    /// The message, or the part, is no longer on this machine.
    Gone,
    /// The message has no place on the server yet, so there is no address to
    /// fetch from: it was moved or saved a moment ago and not synced since.
    NotOnServer,
    /// The person's settings say attachments are never downloaded.
    Never,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_part_is_held_by_its_own_fetch_and_by_a_whole_one_only() {
        let two = Sections::Parts(vec!["2".to_owned(), "3".to_owned()]);
        assert!(two.holds_part("2"));
        assert!(!two.holds_part("1"));
        assert!(!two.holds_part("2.1"));
        assert!(Sections::Whole.holds_part("2"));
        assert!(!Sections::Text.holds_part("2"));
        assert!(!Sections::Headers.holds_part("2"));
    }

    #[test]
    fn the_words_are_held_by_a_text_fetch_and_a_whole_one_only() {
        assert!(Sections::Text.holds_text());
        assert!(Sections::Whole.holds_text());
        assert!(!Sections::Parts(vec!["2".to_owned()]).holds_text());
        assert!(!Sections::Headers.holds_text());
    }
}
