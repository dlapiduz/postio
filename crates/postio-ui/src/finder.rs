//! What the one box can be asked, and the character that asks it.
//!
//! # Why it is here rather than in a frontend
//!
//! `postio-gtk::finder` is one box with several modes: typing searches mail,
//! and a prefix in an empty box switches to running a command, going to a
//! folder, labelling the selection or finding a correspondent. Which
//! questions the box answers, and which character asks each, are product
//! decisions — not drawing — and ADR 0019 forbids a second frontend
//! re-deriving those. The palette's matcher moved here for that reason in
//! #658 and the search chips in #1157; this is the third instance of the
//! same move, and it leaves `postio-gtk` the rendering.
//!
//! # Why it is a table rather than a `match`
//!
//! The set had no reader outside the widget, so four of the box's five modes
//! appeared in no documentation and nothing at rest said they existed. The
//! constitution's answer to that is one enumerable table — it is what the
//! command registry is — and this is the same answer for the one thing the
//! registry cannot hold. A prefix is not a command: it selects which question
//! is being asked and then the user keeps typing, so it has no invocation, no
//! context predicate of its own and nothing to undo.
//!
//! So: the bar's hint, the cheat sheet and the generated documentation all
//! read [`MODES`]. A sixth mode appears in all three from one edit here.

/// One question the box can be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinderMode {
    /// The character that reaches this mode from an empty box.
    ///
    /// `None` for search: it is what the box already is, so there is nothing
    /// to type to get there.
    pub prefix: Option<char>,
    /// What the field shows once the mode is active, so the mode is visible
    /// at a glance. Search wears the `/` the canvas draws on the field.
    pub marker: &'static str,
    /// What the mode is for, in a user's words. This is the string the hint
    /// and the documentation both show.
    pub purpose: &'static str,
}

/// Every mode, in the order the hint and the cheat sheet list them.
///
/// Search first because it is what the box does with no prefix at all; the
/// rest in the order `postio-gtk::finder` has always listed them.
pub const MODES: &[FinderMode] = &[
    FinderMode {
        prefix: None,
        marker: "/",
        purpose: "Search all mail",
    },
    FinderMode {
        prefix: Some('>'),
        marker: ">",
        purpose: "Run a command",
    },
    FinderMode {
        prefix: Some('#'),
        marker: "#",
        purpose: "Go to a folder",
    },
    FinderMode {
        prefix: Some('@'),
        marker: "@",
        purpose: "Find a correspondent",
    },
    FinderMode {
        prefix: Some('+'),
        marker: "+",
        purpose: "Add a label",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_has_a_marker_and_a_purpose() {
        assert!(
            !MODES.is_empty(),
            "a table nobody filled in explains nothing"
        );
        for mode in MODES {
            assert!(!mode.marker.is_empty(), "{mode:?} has no marker");
            assert!(!mode.purpose.is_empty(), "{mode:?} says nothing it is for");
        }
    }

    #[test]
    fn prefixes_are_unique_and_exactly_one_mode_has_none() {
        let mut prefixes: Vec<char> = MODES.iter().filter_map(|mode| mode.prefix).collect();
        let before = prefixes.len();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert_eq!(
            prefixes.len(),
            before,
            "two modes answer to the same character, so one of them is unreachable"
        );
        assert_eq!(
            MODES.iter().filter(|mode| mode.prefix.is_none()).count(),
            1,
            "exactly one mode is what the box already is; the rest are typed into it"
        );
    }

    #[test]
    fn the_modes_the_box_ships_are_all_here() {
        let purpose_of = |prefix: Option<char>| {
            MODES
                .iter()
                .find(|mode| mode.prefix == prefix)
                .map(|mode| mode.purpose)
        };
        assert_eq!(purpose_of(None), Some("Search all mail"));
        assert_eq!(purpose_of(Some('>')), Some("Run a command"));
        assert_eq!(purpose_of(Some('#')), Some("Go to a folder"));
        assert_eq!(purpose_of(Some('+')), Some("Add a label"));
        assert_eq!(purpose_of(Some('@')), Some("Find a correspondent"));
    }
}
