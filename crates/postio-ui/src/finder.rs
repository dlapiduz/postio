//! What the one box can be asked, and the character that asks it.
//!
//! # Why it is here rather than in a frontend
//!
//! The classic app's finder is one box with several modes: typing searches mail,
//! and a prefix in an empty box switches to running a command, going to a
//! folder, labelling the selection or finding a correspondent. Which
//! questions the box answers, and which character asks each, are product
//! decisions — not drawing — and ADR 0019 forbids a second frontend
//! re-deriving those. The palette's matcher moved here for that reason in
//! #658 and the search chips in #1157; this is the third instance of the
//! same move, and it leaves the frontend the rendering.
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
//!
//! # The blended mode
//!
//! Focus's command bar (spec 007 FR-060, research R5) answers typed text
//! with three groups at once: commands, places, and one search row. The
//! classic box keeps its modes and never blends, so [`blend`] is a new mode
//! beside them rather than a change to any of them, and it is in no row of
//! [`MODES`]. Its `>` is the command mode's prefix, so the character means
//! one thing in both boxes.

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
/// rest in the order the classic app's finder always listed them.
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

// ---------------------------------------------------------------------------
// The bar's blended mode (spec 007 FR-060, FR-061; research R5)
// ---------------------------------------------------------------------------

/// What sort of place a [`Place`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceKind {
    /// A mailbox with a role: Inbox, Drafts, Sent, Archive.
    Mailbox,
    /// A folder the user made.
    Folder,
    /// A label.
    Label,
    /// A saved search: a query with a name.
    SavedSearch,
}

/// Where going to a place takes the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A mailbox or a folder.
    Mailbox(postio_model::ids::MailboxId),
    /// A label's conversations.
    Label(postio_model::ids::LabelId),
    /// A saved search's query, in the one query language.
    Search(String),
    /// An account's Outbox: its drafts whose send is under way. A view over
    /// the Drafts folder, not a folder of its own (spec 003).
    Outbox(postio_model::ids::AccountId),
}

/// One place the bar can go, as the frontend knows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// What sort of place.
    pub kind: PlaceKind,
    /// Its name, as the rest of the app calls it.
    pub name: String,
    /// How many conversations it holds, when that is known.
    pub count: Option<u32>,
    /// The command that goes there directly, whose key its row shows.
    pub go: Option<postio_core::ActionId>,
    /// Where it takes the list.
    pub destination: Destination,
}

/// One place the bar matched, matched as the palette matches a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceHit<'a> {
    /// The place.
    pub place: &'a Place,
    /// The key that goes there directly, as it is bound now.
    pub binding: Option<String>,
    /// Byte indices in the place's name the query matched.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// The character that narrows the blended bar to commands: the command
/// mode's own prefix in [`MODES`].
pub const COMMANDS_ONLY: char = '>';

/// What the bar offers for what was typed: three groups, in the order
/// screen 09 draws them, each ranked within itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blend<'a> {
    /// The commands the text matches, best first, each with its key: the
    /// palette's own rows ([`crate::palette::entries`]), so a command ranks
    /// here as it does there.
    pub commands: Vec<crate::palette::Entry>,
    /// The places the text names, best first, and between equals in
    /// [`PlaceKind`]'s order.
    pub places: Vec<PlaceHit<'a>>,
    /// The one "Search mail for …" row: the words typed, as the query it
    /// runs. `None` when nothing was typed, or when `>` asked for commands
    /// only.
    pub search: Option<String>,
}

/// The bar's blended answer to `text` (spec 007 FR-060): the commands
/// reachable in `context` that it matches, the `places` it names, and a
/// search for it, or the commands alone after [`COMMANDS_ONLY`].
///
/// Whatever it runs acts on what the bar opened over, not on what the list
/// shows by then: see [`Held`].
pub fn blend<'a>(
    text: &str,
    places: &'a [Place],
    keymap: &postio_core::Keymap,
    context: postio_core::Context,
    state: postio_core::Availability,
) -> Blend<'a> {
    if let Some(commands) = text.strip_prefix(COMMANDS_ONLY) {
        return Blend {
            commands: crate::palette::entries(keymap, context, state, commands),
            ..Blend::default()
        };
    }
    let query = text.trim();
    if query.is_empty() {
        return Blend::default();
    }
    Blend {
        commands: crate::palette::entries(keymap, context, state, query),
        places: place_hits(places, query, keymap),
        search: Some(query.to_owned()),
    }
}

/// The places matching `query`, best first.
fn place_hits<'a>(
    places: &'a [Place],
    query: &str,
    keymap: &postio_core::Keymap,
) -> Vec<PlaceHit<'a>> {
    let mut found: Vec<PlaceHit<'a>> = places
        .iter()
        .filter_map(|place| {
            let matched = crate::palette::score(query, &place.name)?;
            Some(PlaceHit {
                place,
                binding: place
                    .go
                    .and_then(|go| keymap.binding(go))
                    .map(str::to_owned),
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();
    found.sort_by_key(|hit| (std::cmp::Reverse(hit.score), hit.place.kind));
    found.truncate(crate::palette::MAX_ROWS);
    found
}

/// What the bar opened over: the aim a command run from it acts on (spec
/// 007 FR-061, US4 scenario 3).
///
/// Taken when the bar opens, from the list's selection and cursor, and
/// kept while it is open: typing in the bar must not change what "Archive"
/// archives, whatever the list does meanwhile. Its [`aim`](Self::aim) is
/// what a frontend hands `postio_core::aim::mirror` and `refine` when a
/// command row runs, in place of the list's state at that moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    /// The view a whole-view selection is relative to.
    pub scope: Option<postio_core::state::ViewScope>,
    /// What the user had marked.
    pub selection: postio_core::Selection,
    /// Where the keyboard was.
    pub cursor: Option<postio_model::ids::MessageId>,
}

impl Held {
    /// The held aim, over the frontend's rows.
    pub fn aim<'a>(
        &'a self,
        rows: &'a dyn postio_core::aim::RowFacts,
    ) -> postio_core::aim::Aim<'a> {
        postio_core::aim::Aim {
            scope: self.scope.clone(),
            selection: &self.selection,
            cursor: self.cursor,
            rows,
        }
    }

    /// `command`, aimed at what the bar opened over: a conversation row
    /// marked or under the cursor is the conversation, as a key pressed in
    /// the list would have it (`postio_core::aim::refine`).
    pub fn aimed(
        &self,
        command: postio_core::Command,
        rows: &dyn postio_core::aim::RowFacts,
    ) -> postio_core::Command {
        postio_core::aim::refine(command, &self.aim(rows))
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

    // -- the blended mode (spec 007 T085) --------------------------------

    use postio_core::aim::{RowFacts, RowKind};
    use postio_core::{
        ActionId, Availability, Command, CommandId, Context, Keymap, MessageTarget, Scope,
        Selection,
    };
    use postio_model::ids::{AccountId, LabelId, MailboxId, MessageId, ThreadId};

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    fn an_account() -> Availability {
        Availability::open(Scope::Account(AccountId::new(1)))
    }

    /// The places screen 09's bar can go to.
    fn places() -> Vec<Place> {
        let place = |kind, name: &str, count, go, destination| Place {
            kind,
            name: name.to_owned(),
            count: Some(count),
            go,
            destination,
        };
        vec![
            place(
                PlaceKind::Mailbox,
                "Inbox",
                312,
                Some(ActionId::Builtin(CommandId::GoToInbox)),
                Destination::Mailbox(MailboxId::new(1)),
            ),
            place(
                PlaceKind::Mailbox,
                "Archive",
                18_204,
                None,
                Destination::Mailbox(MailboxId::new(2)),
            ),
            place(
                PlaceKind::Folder,
                "Receipts",
                214,
                None,
                Destination::Mailbox(MailboxId::new(3)),
            ),
            place(
                PlaceKind::Label,
                "Travel",
                12,
                None,
                Destination::Label(LabelId::new(1)),
            ),
            place(
                PlaceKind::SavedSearch,
                "Waiting on reply",
                5,
                None,
                Destination::Search("is:unread".to_owned()),
            ),
        ]
    }

    fn named<'a>(hits: &'a [PlaceHit<'_>]) -> Vec<&'a str> {
        hits.iter().map(|hit| hit.place.name.as_str()).collect()
    }

    /// Screen 09: "arch" gives three groups, in the order it draws them --
    /// the commands it matches with their keys, the places it names with
    /// their counts, and one row that searches mail for the words typed.
    #[test]
    fn typing_arch_gives_the_three_groups_screen_09_draws() {
        let places = places();
        let found = blend("arch", &places, &defaults(), Context::List, an_account());

        let first = found.commands.first().expect("a command matches arch");
        assert_eq!(first.id, ActionId::Builtin(CommandId::Archive));
        assert_eq!(first.binding.as_deref(), Some("a"));
        assert_eq!(
            found.commands,
            crate::palette::entries(&defaults(), Context::List, an_account(), "arch"),
            "the commands are the palette's own, ranked as it ranks them"
        );

        assert_eq!(named(&found.places), ["Archive"]);
        assert_eq!(found.places[0].place.count, Some(18_204));

        assert_eq!(found.search.as_deref(), Some("arch"));
    }

    /// Places rank among themselves, and each shows the key that goes
    /// there as it is bound now.
    #[test]
    fn places_are_ranked_and_carry_the_key_that_goes_there() {
        let places = places();
        let found = blend("in", &places, &defaults(), Context::List, an_account());
        assert_eq!(named(&found.places), ["Inbox", "Waiting on reply"]);
        assert_eq!(found.places[0].binding.as_deref(), Some("g i"));
        assert_eq!(found.places[1].binding, None);
    }

    /// `>`, the finder's own prefix for commands, narrows the bar to them.
    #[test]
    fn a_leading_chevron_narrows_the_bar_to_commands() {
        let places = places();
        let only = blend(">arch", &places, &defaults(), Context::List, an_account());
        assert!(!only.commands.is_empty(), "no command matched >arch");
        assert_eq!(
            only.commands,
            blend("arch", &places, &defaults(), Context::List, an_account()).commands
        );
        assert!(only.places.is_empty(), "{:?}", only.places);
        assert_eq!(only.search, None);
        assert_eq!(
            Some(COMMANDS_ONLY),
            MODES
                .iter()
                .find(|mode| mode.purpose == "Run a command")
                .and_then(|mode| mode.prefix),
            "the bar's prefix for commands is the finder's"
        );
    }

    /// Nothing typed is nothing to blend: the bar shows its saved searches
    /// then, which are not this function's.
    #[test]
    fn an_empty_bar_blends_nothing() {
        let places = places();
        assert_eq!(
            blend("  ", &places, &defaults(), Context::List, an_account()),
            Blend::default()
        );
        assert_ne!(
            blend("in", &places, &defaults(), Context::List, an_account()),
            Blend::default(),
            "and something typed is something to blend"
        );
    }

    /// Every row on screen 09's list is a conversation: `m<n>` is thread
    /// `t<n>`'s row.
    struct Conversations;

    impl RowFacts for Conversations {
        fn row_kind(&self, message: MessageId) -> RowKind {
            RowKind::Thread(ThreadId::new(message.get()))
        }
    }

    /// Spec 007 US4 scenario 3 (FR-061): three rows selected, the bar
    /// opened, "Archive" run from it, and exactly those three archived --
    /// whatever the list does while the bar is open.
    #[test]
    fn a_command_from_the_bar_acts_on_what_it_opened_over() {
        let held = Held {
            scope: None,
            selection: Selection::These(vec![
                MessageId::new(1),
                MessageId::new(2),
                MessageId::new(3),
            ]),
            cursor: Some(MessageId::new(4)),
        };
        let places = places();
        let run = blend("arch", &places, &defaults(), Context::List, an_account()).commands[0].id;
        let ActionId::Builtin(id) = run else {
            panic!("{run:?} is not a built-in command");
        };
        assert_eq!(
            held.aimed(Command::default_for(id), &Conversations),
            Command::Archive {
                target: MessageTarget::Threads(vec![
                    ThreadId::new(1),
                    ThreadId::new(2),
                    ThreadId::new(3),
                ]),
            }
        );
        // Nothing marked: the row the cursor was on.
        let cursor = Held {
            selection: Selection::default(),
            ..held
        };
        assert_eq!(
            cursor.aimed(Command::default_for(id), &Conversations),
            Command::Archive {
                target: MessageTarget::Thread(ThreadId::new(4)),
            }
        );
    }
}
