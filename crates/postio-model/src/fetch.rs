//! What a fetch of a message's bytes was for (ADR 0017): its text, or attachment parts by section.
//! A part wait must not end on its message's text failing (#1996).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sections {
    Text,
    Parts(Vec<String>),
    Whole,
    Headers,
}

impl Sections {
    pub fn holds_part(&self, part: &str) -> bool {
        match self {
            Sections::Parts(parts) => parts.iter().any(|named| named == part),
            Sections::Whole => true,
            Sections::Text | Sections::Headers => false,
        }
    }

    pub fn holds_text(&self) -> bool {
        matches!(self, Sections::Text | Sections::Whole)
    }
}

/// Only `Queued` means bytes are coming soon; every other answer tells the person why not (#1987,
/// #1996).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadAsk {
    /// At the front of the queue: the bytes land as a `BodyLoaded` naming
    /// the parts, or a `BodyFailed` does.
    Queued,
    /// Queued, but the account cannot reach its server now (no network, or
    /// stopped until the person signs in again). The bytes come when it can.
    Unreachable,
    Local,
    Gone,
    /// The message has no place on the server yet, so there is no address to
    /// fetch from: it was moved or saved a moment ago and not synced since.
    NotOnServer,
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
