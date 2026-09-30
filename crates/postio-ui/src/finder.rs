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

/// One label the `+` box can offer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelHit {
    /// The label to apply.
    pub id: postio_model::ids::LabelId,
    /// Its name, which is also what travels as an IMAP keyword.
    pub name: String,
    /// Byte indices in `name` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// The labels matching `query`, best first.
///
/// The same matcher `folders` and the command palette use, so `wk` finds
/// `Work` here exactly as `cp` finds "Command palette" there.
pub fn labels(labels: &[postio_model::Label], query: &str) -> Vec<LabelHit> {
    let query = query.trim();
    let mut found: Vec<LabelHit> = labels
        .iter()
        .filter_map(|label| {
            let matched = crate::palette::score(query, &label.name)?;
            Some(LabelHit {
                id: label.id,
                name: label.name.clone(),
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();
    // Stable, so an empty query leaves the repository's own order -- by name
    // -- alone, which is what makes the list scannable.
    found.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    found.truncate(crate::palette::MAX_ROWS);
    found
}

/// One correspondent the box matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactHit {
    /// What to call them: the name the user set, then the last name seen on
    /// the address, then the address itself.
    pub name: String,
    /// The addr-spec, which is what `from:` will be given.
    pub address: String,
    /// How many messages this address has been seen on, for the cap beside
    /// the row — a correspondent you write to daily reads differently from
    /// one who mailed you once.
    pub times_seen: u32,
    /// Byte indices in `name` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// The correspondents matching `query`, best first.
///
/// Scored with the palette's own matcher over the *name*, and again over the
/// address when the name did not match — people look for `grace`, and they
/// look for `gh`, and they look for `@example.org`. One matcher across the
/// whole box, so `gh` finds Grace Hopper here exactly as `wd` finds
/// `wayland-devel` one mode over.
///
/// Ties break on how often the correspondent has been seen, which is the
/// reason an empty query offers the people you actually write to.
///
/// This is deliberately *not* the order `ContactRepository::search` returns
/// any more. #424 put recency first there, because composing to somebody is
/// about who you are writing to now. Finding is a different question — it
/// asks whose mail to go and read — and the people worth offering for that
/// are the ones there is a lot of mail from. The two surfaces answer
/// differently on purpose; if that ever stops being true, this is the comment
/// that was wrong.
pub fn contacts(contacts: &[postio_model::Contact], query: &str) -> Vec<ContactHit> {
    let query = query.trim();
    let mut found: Vec<ContactHit> = contacts
        .iter()
        .filter_map(|contact| {
            let name = contact_name(contact);
            let address = contact.address.address.clone();
            // The name first, so the highlight lands on what the row shows.
            // Falling back to the address means `@example.org` still finds
            // people, and costs nothing when the name already matched.
            let matched = match crate::palette::score(query, &name) {
                Some(matched) => matched,
                None => {
                    crate::palette::score(query, &address).map(|matched| crate::palette::Match {
                        score: matched.score,
                        positions: Vec::new(),
                    })?
                }
            };
            Some(ContactHit {
                name,
                address,
                times_seen: contact.times_seen,
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();
    found.sort_by_key(|hit| {
        (
            std::cmp::Reverse(hit.score),
            std::cmp::Reverse(hit.times_seen),
        )
    });
    found.truncate(crate::palette::MAX_ROWS);
    found
}

/// What to call a correspondent: the name the user set, then the last display
/// name seen on the address, then the address itself. Never empty, so a row
/// always has something to say.
fn contact_name(contact: &postio_model::Contact) -> String {
    contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone())
        .unwrap_or_else(|| contact.address.address.clone())
}

/// The query picking `hit` puts in the box.
///
/// A `from:` chip, quoted if the address could not survive being typed back
/// in. Deliberately *the query*, not a search that has already run: the point
/// of landing back in search is that the user can go on building on it.
pub fn contact_query(hit: &ContactHit) -> String {
    if hit.address.chars().any(char::is_whitespace) {
        format!("from:\"{}\"", hit.address.replace('"', ""))
    } else {
        format!("from:{}", hit.address)
    }
}

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
