//! The Contacts screen's rules, without a toolkit (specs/005-contacts).
//!
//! What a row says, what "show mail" searches, and which command means
//! something on which kind of row -- decided here, where a test proves them
//! in milliseconds, so the widget only draws the answers.

use chrono::{DateTime, Local, Utc};
use postio_core::CommandId;
use postio_model::{ContactListRow, ContactSource};

/// The query "show mail" writes for a person: every address they own, in one
/// `with:` field (research R6). Addresses, not the person, so a search pinned
/// from it keeps meaning the addresses it named.
pub fn show_mail_query(addresses: &[String]) -> String {
    let value = addresses.join(",");
    // Quoted only when the value could not survive being typed back in: the
    // parser splits tokens on whitespace, and a quote inside would end it.
    if value.contains(char::is_whitespace) {
        format!("with:\"{}\"", value.replace('"', ""))
    } else {
        format!("with:{value}")
    }
}

/// Whether a row carries the mark for a person the user made or imported
/// (FR-005): the list distinguishes the address book from the mail.
pub fn is_made(source: ContactSource) -> bool {
    !matches!(source, ContactSource::Mail)
}

/// The line under a person's name: how many addresses, and when they were
/// last in touch -- `2 addresses · Thu`, `1 address`.
pub fn secondary_line(row: &ContactListRow, now: DateTime<Local>) -> String {
    let count = match row.address_count {
        1 => "1 address".to_owned(),
        n => format!("{n} addresses"),
    };
    match last_in_touch(row.last_seen_at, now) {
        Some(when) => format!("{count} · {when}"),
        None => count,
    }
}

/// The kind of row the keyboard is on. The Contacts screen is one context,
/// and a command acts on the focused row's kind (contracts/commands.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// A person in the list.
    Person,
    /// One of a person's addresses, in the detail.
    Address,
    /// A group.
    Group,
    /// A suggestion that two people may be one.
    Suggestion,
}

/// Why a command did nothing on the focused row: the one-line hint the
/// screen shows instead of failing silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint(pub &'static str);

/// Whether `command` means something on a row of `kind`, and the hint to show
/// when it does not.
pub fn applies(command: CommandId, kind: RowKind) -> Result<(), Hint> {
    use CommandId as C;
    use RowKind as K;
    match (command, kind) {
        // A suggestion is two people; showing mail or writing to it would
        // have to guess which one.
        (C::ContactShowMail | C::ContactCompose, K::Suggestion) => {
            Err(Hint("Choose one of the two people first"))
        }
        // Everything else either names a person, an address or a group --
        // each of which it means something on -- or is the screen's own.
        _ => Ok(()),
    }
}

/// When a person was last in touch, for a row: [`crate::row::timestamp`]'s
/// words, or nothing for someone no mail has involved.
fn last_in_touch(at: Option<DateTime<Utc>>, now: DateTime<Local>) -> Option<String> {
    at.map(|at| crate::row::timestamp(at, now))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use postio_model::{ContactId, ContactState};

    fn row(addresses: u32, last: Option<DateTime<Utc>>, source: ContactSource) -> ContactListRow {
        ContactListRow {
            id: ContactId::new(1),
            name: "Ada Lovelace".into(),
            preferred: Some("ada@example.com".into()),
            address_count: addresses,
            last_seen_at: last,
            source,
            state: ContactState::Live,
            sort_key: "ada lovelace".into(),
        }
    }

    fn now() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 9, 24, 12, 0, 0).unwrap()
    }

    #[test]
    fn show_mail_names_every_address_in_one_with() {
        assert_eq!(
            show_mail_query(&["ada@work.example".into(), "ada@home.example".into()]),
            "with:ada@work.example,ada@home.example"
        );
        assert_eq!(
            show_mail_query(&["grace@example.org".into()]),
            "with:grace@example.org"
        );
    }

    #[test]
    fn show_mail_query_is_read_back_as_the_filter_it_means() {
        let query = show_mail_query(&["ada@work.example".into(), "ada@home.example".into()]);
        let parsed = postio_search::parse(&query, now().date_naive());
        let filters: Vec<_> = parsed.filters().map(|c| c.filter.clone()).collect();
        assert_eq!(
            filters,
            vec![postio_search::query::Filter::With(vec![
                "ada@work.example".into(),
                "ada@home.example".into()
            ])]
        );
    }

    #[test]
    fn an_address_that_could_not_be_typed_back_is_quoted() {
        let query = show_mail_query(&["odd one@example.com".into()]);
        let parsed = postio_search::parse(&query, now().date_naive());
        let filters: Vec<_> = parsed.filters().map(|c| c.filter.clone()).collect();
        assert_eq!(
            filters,
            vec![postio_search::query::Filter::With(vec![
                "odd one@example.com".into()
            ])]
        );
    }

    #[test]
    fn the_address_book_is_marked_and_the_mail_is_not() {
        assert!(is_made(ContactSource::User));
        assert!(is_made(ContactSource::Import));
        assert!(!is_made(ContactSource::Mail));
    }

    #[test]
    fn the_secondary_line_counts_addresses_and_says_when() {
        let today = Utc.with_ymd_and_hms(2026, 9, 24, 9, 14, 0).unwrap();
        assert!(
            secondary_line(&row(2, Some(today), ContactSource::Mail), now())
                .starts_with("2 addresses · "),
        );
        assert_eq!(
            secondary_line(&row(1, None, ContactSource::User), now()),
            "1 address",
            "no mail, no date -- never a date nobody saw"
        );
    }

    #[test]
    fn a_command_acts_on_the_row_kind_it_names_and_hints_elsewhere() {
        assert!(applies(CommandId::ContactShowMail, RowKind::Person).is_ok());
        assert!(applies(CommandId::ContactCompose, RowKind::Person).is_ok());
        assert!(
            applies(CommandId::ContactsFilter, RowKind::Group).is_ok(),
            "the filter is the screen's"
        );
        let hint = applies(CommandId::ContactCompose, RowKind::Suggestion)
            .expect_err("a suggestion is two people; which one would it write to?");
        assert!(!hint.0.is_empty(), "a refusal says what it wants");
    }

    #[test]
    fn last_in_touch_is_nothing_for_someone_no_mail_involved() {
        assert_eq!(last_in_touch(None, now()), None);
    }
}

/// The Contacts list's paging: which pages are resident, which are on their
/// way, and which generation of the list a delivered page belongs to.
///
/// The message list's [`crate::list::ListWindow`] does this for messages, and
/// carries message-only meaning with it -- threads, the aim a verb takes --
/// so the contacts list has this smaller window of its own rather than a
/// generic one bent to fit (specs/005-contacts research R5, as revised).
/// Like that one, it owns the rows, so what the list holds is bounded by
/// [`crate::list::CACHE_PAGES`] pages however long the list is.
#[derive(Debug, Default)]
pub struct ContactsWindow {
    generation: u64,
    total: u32,
    pages: std::collections::HashMap<u32, Vec<ContactListRow>>,
    /// Least-recently-used first.
    order: std::collections::VecDeque<u32>,
    pending: std::collections::HashSet<u32>,
}

/// What one position answers with.
#[derive(Debug, PartialEq, Eq)]
pub enum Slot<'a> {
    /// The row is here.
    Row(&'a ContactListRow),
    /// Not yet; `request` is the page to ask for, or `None` when it has been
    /// asked for already.
    Loading {
        /// The page to request, the first time it is missed.
        request: Option<u32>,
    },
}

/// What a delivery changed.
#[derive(Debug, PartialEq, Eq)]
pub enum Delivered {
    /// For an older generation of the list; dropped.
    Stale,
    /// These positions now have rows, and the list is `total` long.
    Filled {
        /// The positions the page covers.
        positions: std::ops::Range<u32>,
        /// Whether the length moved, which a view must be told separately.
        total_changed: bool,
        /// Pages dropped to keep the bound; objects a view holds for their
        /// positions stand for nothing now.
        evicted: Vec<u32>,
    },
}

impl ContactsWindow {
    /// Starts the list over at `total` rows: a new view, a new filter, a
    /// change to the people. Returns the new generation, which a request
    /// carries so its answer can be told from one for the list as it was.
    pub fn reset(&mut self, total: u32) -> u64 {
        self.generation += 1;
        self.total = total;
        self.pages.clear();
        self.order.clear();
        self.pending.clear();
        self.generation
    }

    /// How long the list is.
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The generation requests are being made for.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// What position `index` holds.
    pub fn row_at(&mut self, index: u32) -> Slot<'_> {
        let page = index / crate::list::PAGE_SIZE;
        if self.pages.contains_key(&page) {
            self.touch(page);
            let offset = (index % crate::list::PAGE_SIZE) as usize;
            return match self.pages.get(&page).and_then(|rows| rows.get(offset)) {
                Some(row) => Slot::Row(row),
                // A short last page: the position is past what the store
                // holds now, and the next delivery will say so.
                None => Slot::Loading { request: None },
            };
        }
        if self.pending.insert(page) {
            Slot::Loading {
                request: Some(page),
            }
        } else {
            Slot::Loading { request: None }
        }
    }

    /// A page arrived.
    pub fn deliver(
        &mut self,
        generation: u64,
        page: u32,
        rows: Vec<ContactListRow>,
        total: u32,
    ) -> Delivered {
        if generation != self.generation {
            return Delivered::Stale;
        }
        self.pending.remove(&page);
        let start = page * crate::list::PAGE_SIZE;
        let positions = start..start + rows.len() as u32;
        self.pages.insert(page, rows);
        self.touch(page);
        let mut evicted = Vec::new();
        while self.order.len() > crate::list::CACHE_PAGES {
            if let Some(dropped) = self.order.pop_front() {
                self.pages.remove(&dropped);
                evicted.push(dropped);
            }
        }
        let total_changed = total != self.total;
        self.total = total;
        Delivered::Filled {
            positions,
            total_changed,
            evicted,
        }
    }

    /// Gives up on a page whose read failed, so the next miss asks again.
    pub fn abandon(&mut self, generation: u64, page: u32) {
        if generation == self.generation {
            self.pending.remove(&page);
        }
    }

    /// How many rows are held, for the bound this exists to keep.
    pub fn resident_rows(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    /// Where a person sits, if their page is resident.
    pub fn position_of(&self, id: postio_model::ContactId) -> Option<u32> {
        self.pages.iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.id == id)
                .map(|offset| page * crate::list::PAGE_SIZE + offset as u32)
        })
    }

    /// Marks `page` most recently used.
    fn touch(&mut self, page: u32) {
        self.order.retain(|held| *held != page);
        self.order.push_back(page);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    use crate::list::{CACHE_PAGES, PAGE_SIZE};
    use postio_model::{ContactId, ContactState};

    fn page(page: u32, len: u32) -> Vec<ContactListRow> {
        (0..len)
            .map(|i| {
                let n = page * PAGE_SIZE + i;
                ContactListRow {
                    id: ContactId::new(i64::from(n) + 1),
                    name: format!("Person {n}"),
                    preferred: None,
                    address_count: 1,
                    last_seen_at: None,
                    source: ContactSource::Mail,
                    state: ContactState::Live,
                    sort_key: format!("person {n:06}"),
                }
            })
            .collect()
    }

    #[test]
    fn a_missed_position_asks_for_its_page_once() {
        let mut window = ContactsWindow::default();
        window.reset(500);
        assert_eq!(window.total(), 500);
        assert_eq!(window.row_at(120), Slot::Loading { request: Some(2) });
        assert_eq!(
            window.row_at(130),
            Slot::Loading { request: None },
            "the page is on its way; asking again would double the read"
        );
    }

    #[test]
    fn a_delivered_page_answers_its_positions() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(120);
        let _ = window.row_at(60);
        assert_eq!(
            window.deliver(generation, 1, page(1, 50), 120),
            Delivered::Filled {
                positions: 50..100,
                total_changed: false,
                evicted: vec![]
            }
        );
        match window.row_at(60) {
            Slot::Row(row) => assert_eq!(row.name, "Person 60"),
            other => panic!("expected a row, got {other:?}"),
        }
        assert_eq!(window.position_of(ContactId::new(61)), Some(60));
    }

    #[test]
    fn an_answer_for_an_older_list_is_dropped() {
        let mut window = ContactsWindow::default();
        let old = window.reset(120);
        let _ = window.row_at(0);
        let new = window.reset(3);
        assert_ne!(old, new);
        assert_eq!(window.deliver(old, 0, page(0, 50), 120), Delivered::Stale);
        assert_eq!(window.row_at(0), Slot::Loading { request: Some(0) });
    }

    #[test]
    fn a_delivery_that_moves_the_length_says_so() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(120);
        let _ = window.row_at(0);
        assert_eq!(
            window.deliver(generation, 0, page(0, 50), 121),
            Delivered::Filled {
                positions: 0..50,
                total_changed: true,
                evicted: vec![]
            }
        );
        assert_eq!(window.total(), 121);
    }

    #[test]
    fn an_abandoned_page_is_asked_for_again() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(120);
        let _ = window.row_at(0);
        window.abandon(generation, 0);
        assert_eq!(
            window.row_at(0),
            Slot::Loading { request: Some(0) },
            "a failed read must not leave skeletons nothing can clear"
        );
    }

    #[test]
    fn a_delivery_names_the_pages_it_evicted() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(20_000);
        let mut evicted = Vec::new();
        for p in 0..=(CACHE_PAGES as u32) {
            let _ = window.row_at(p * PAGE_SIZE);
            if let Delivered::Filled {
                evicted: dropped, ..
            } = window.deliver(generation, p, page(p, PAGE_SIZE), 20_000)
            {
                evicted.extend(dropped);
            }
        }
        assert_eq!(evicted, [0], "the least recently used page goes first");
    }

    #[test]
    fn scrolling_the_whole_list_holds_a_bounded_number_of_rows() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(20_000);
        for p in 0..(20_000 / PAGE_SIZE) {
            let _ = window.row_at(p * PAGE_SIZE);
            window.deliver(generation, p, page(p, PAGE_SIZE), 20_000);
        }
        assert!(
            window.resident_rows() <= CACHE_PAGES * PAGE_SIZE as usize,
            "{} rows held for 20,000 people",
            window.resident_rows()
        );
        assert_eq!(
            window.row_at(0),
            Slot::Loading { request: Some(0) },
            "the first page was evicted, and is asked for again"
        );
    }
}
