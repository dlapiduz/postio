//! What the one box can be asked and the character that asks each, as a table so the hint,
//! cheat sheet and docs read one source (ADR 0019). The blended bar mode (`blend`) is in no row.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinderMode {
    /// `None` for search: it is what the box already is.
    pub prefix: Option<char>,
    pub marker: &'static str,
    pub purpose: &'static str,
}

/// Search first: it is what the box does with no prefix.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelHit {
    pub id: postio_model::ids::LabelId,
    pub name: String,
    pub positions: Vec<usize>,
    pub score: i32,
}

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
    // Stable, so an empty query keeps the repository's order (by name).
    found.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    found.truncate(crate::palette::MAX_ROWS);
    found
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactHit {
    pub name: String,
    pub address: String,
    pub addresses: Vec<String>,
    pub times_seen: u32,
    pub positions: Vec<usize>,
    pub score: i32,
}

/// Scored over the name, then the address when the name did not match. Ties break on how
/// often the correspondent was seen, so an empty query offers people you write to; not
/// `ContactRepository::search`'s recency order (#424).
pub fn contacts(contacts: &[postio_model::Contact], query: &str) -> Vec<ContactHit> {
    let query = query.trim();
    let mut found: Vec<ContactHit> = contacts
        .iter()
        .filter_map(|contact| {
            let name = contact_name(contact);
            let addresses: Vec<String> = contact
                .addresses
                .iter()
                .map(|owned| owned.address.address.clone())
                .collect();
            // Name first so the highlight lands on what the row shows; the best-scoring address
            // stands for the person.
            let matched = match crate::palette::score(query, &name) {
                Some(matched) => matched,
                None => addresses
                    .iter()
                    .filter_map(|address| crate::palette::score(query, address))
                    .max_by_key(|matched| matched.score)
                    .map(|matched| crate::palette::Match {
                        score: matched.score,
                        positions: Vec::new(),
                    })?,
            };
            Some(ContactHit {
                name,
                address: addresses.first().cloned().unwrap_or_default(),
                addresses,
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

fn contact_name(contact: &postio_model::Contact) -> String {
    contact.display_name().to_owned()
}

/// A `with:` clause over every address the person owns, spelled by
/// `crate::contacts::show_mail_query`.
/// The query, not a run search, so the user can keep building on it.
pub fn contact_query(hit: &ContactHit) -> String {
    crate::contacts::show_mail_query(&hit.addresses)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PlaceKind {
    Mailbox,
    Folder,
    Label,
    SavedSearch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    Mailbox(postio_model::ids::MailboxId),
    Label(postio_model::ids::LabelId),
    Search(String),
    /// A view over the Drafts folder, not a folder of its own.
    Outbox(postio_model::ids::AccountId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub kind: PlaceKind,
    pub name: String,
    pub count: Option<u32>,
    pub go: Option<postio_core::ActionId>,
    pub destination: Destination,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceHit<'a> {
    pub place: &'a Place,
    pub binding: Option<String>,
    pub positions: Vec<usize>,
    pub score: i32,
}

pub const COMMANDS_ONLY: char = '>';

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blend<'a> {
    pub commands: Vec<crate::palette::Entry>,
    pub places: Vec<PlaceHit<'a>>,
    /// The "Search mail for …" row; `None` when nothing was typed or `>` asked for commands only.
    pub search: Option<String>,
}

impl Blend<'_> {
    /// Whether the search row leads: a scattered-letter match on a command's title must not
    /// put it under the cursor, where Return would run it ("tide" must search).
    pub fn search_first(&self) -> bool {
        let Some(query) = &self.search else {
            return false;
        };
        !self
            .commands
            .iter()
            .any(|entry| is_named_by(query, entry.title))
            && !self
                .places
                .iter()
                .any(|hit| is_named_by(query, &hit.place.name))
    }
}

/// Every word of the query begins a word of the title. The palette's matcher also ranks
/// scattered letters, which is wrong for choosing what Return runs.
pub fn is_named_by(query: &str, title: &str) -> bool {
    let query = query.to_lowercase();
    let title = title.to_lowercase();
    let words: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut asked = query.split_whitespace().peekable();
    asked.peek().is_some() && asked.all(|ask| words.iter().any(|word| word.starts_with(ask)))
}

/// Whatever it runs acts on what the bar opened over, not what the list shows by then: see `Held`.
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

/// What the bar opened over, kept while it is open: typing in the bar must not change what
/// "Archive" archives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Held {
    pub scope: Option<postio_core::state::ViewScope>,
    pub selection: postio_core::Selection,
    pub cursor: Option<postio_model::ids::MessageId>,
}

impl Held {
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

    fn person(name: &str, addresses: &[&str]) -> postio_model::Contact {
        let owned = |id: i64, name: Option<&str>, address: &str| {
            postio_model::ContactAddress::new(
                postio_model::AddressId::new(id),
                postio_model::EmailAddress::new(name, address),
            )
        };
        let mut person = postio_model::Contact::new(owned(1, Some(name), addresses[0]));
        for (at, address) in addresses.iter().enumerate().skip(1) {
            person.addresses.push(owned(at as i64 + 1, None, address));
        }
        person
    }

    #[test]
    fn a_person_with_two_addresses_is_found_by_either_and_offered_once() {
        let ada = person(
            "Ada Lovelace",
            &["ada@work.example", "lovelace@home.example"],
        );
        for query in ["ada", "work.example", "home.example"] {
            let hits = contacts(std::slice::from_ref(&ada), query);
            assert_eq!(hits.len(), 1, "{query} finds ada once");
            assert_eq!(hits[0].name, "Ada Lovelace");
            assert_eq!(
                hits[0].addresses,
                ["ada@work.example", "lovelace@home.example"],
                "the hit carries every address, preferred first"
            );
            assert_eq!(
                contact_query(&hits[0]),
                "with:ada@work.example,lovelace@home.example",
                "picking them searches every address, not the first"
            );
        }
    }

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

    #[test]
    fn typing_arch_gives_the_three_groups_screen_09_draws() {
        let places = places();
        let found = blend("arch", &places, &defaults(), Context::List, an_account());

        let first = found.commands.first().expect("a command matches arch");
        assert_eq!(first.id, ActionId::Builtin(CommandId::Archive));
        assert_eq!(first.binding.as_deref(), Some("e"));
        assert_eq!(
            found.commands,
            crate::palette::entries(&defaults(), Context::List, an_account(), "arch"),
            "the commands are the palette's own, ranked as it ranks them"
        );

        assert_eq!(named(&found.places), ["Archive"]);
        assert_eq!(found.places[0].place.count, Some(18_204));

        assert_eq!(found.search.as_deref(), Some("arch"));
    }

    #[test]
    fn places_are_ranked_and_carry_the_key_that_goes_there() {
        let places = places();
        let found = blend("in", &places, &defaults(), Context::List, an_account());
        assert_eq!(named(&found.places), ["Inbox", "Waiting on reply"]);
        assert_eq!(found.places[0].binding.as_deref(), Some("g i"));
        assert_eq!(found.places[1].binding, None);
    }

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

    /// "tide" is a word to search for; a command whose letters contain it in order must not lead.
    #[test]
    fn a_word_that_names_no_command_leads_with_the_search() {
        let places = places();
        let found = blend("tide", &places, &defaults(), Context::List, an_account());
        assert!(found.search_first(), "{:?}", found.commands);
        let found = blend("harbour", &places, &defaults(), Context::List, an_account());
        assert!(found.search_first());
    }

    #[test]
    fn a_word_that_names_a_command_leaves_it_first() {
        let places = places();
        let found = blend("arch", &places, &defaults(), Context::List, an_account());
        assert!(!found.search_first());
        let found = blend("always", &places, &defaults(), Context::List, an_account());
        assert!(!found.search_first(), "a word of its title names it");
    }

    #[test]
    fn naming_is_by_the_start_of_words_not_scattered_letters() {
        assert!(is_named_by("arch", "Archive"));
        assert!(is_named_by("mark read", "Mark as read"));
        assert!(is_named_by("IMAGES", "Always show images from this sender"));
        assert!(!is_named_by("tide", "Always show images from this sender"));
        assert!(!is_named_by("", "Archive"));
        assert!(!is_named_by("chive", "Archive"));
    }

    struct Conversations;

    impl RowFacts for Conversations {
        fn row_kind(&self, message: MessageId) -> RowKind {
            RowKind::Thread(ThreadId::new(message.get()))
        }
    }

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
