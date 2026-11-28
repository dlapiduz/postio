//! The Contacts screen's rules without a toolkit: row wording, "show mail" queries, command
//! applicability.

use chrono::{DateTime, Local, Utc};
use postio_core::CommandId;
use postio_model::{Contact, ContactId, ContactListRow, ContactSource};

/// One `with:` field over every address they own, so a pinned search keeps meaning the addresses it
/// named.
pub fn show_mail_query(addresses: &[String]) -> String {
    let value = addresses.join(",");
    // Quoted only when the value could not survive being typed back: the parser splits on
    // whitespace.
    if value.contains(char::is_whitespace) {
        format!("with:\"{}\"", value.replace('"', ""))
    } else {
        format!("with:{value}")
    }
}

pub fn group_mail_query(name: &str) -> String {
    if name.contains(char::is_whitespace) {
        format!("group:\"{}\"", name.replace('"', ""))
    } else {
        format!("group:{name}")
    }
}

pub fn is_made(source: ContactSource) -> bool {
    !matches!(source, ContactSource::Mail)
}

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

/// The Contacts screen is one context; a command acts on the focused row's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Person,
    Address,
    Group,
    Suggestion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hint(pub &'static str);

pub fn applies(command: CommandId, kind: RowKind) -> Result<(), Hint> {
    use CommandId as C;
    use RowKind as K;
    match (command, kind) {
        // A suggestion is two people; mail or writing would have to guess which.
        (C::ContactShowMail | C::ContactCompose, K::Suggestion) => {
            Err(Hint("Choose one of the two people first"))
        }
        _ => Ok(()),
    }
}

fn last_in_touch(at: Option<DateTime<Utc>>, now: DateTime<Local>) -> Option<String> {
    at.map(|at| crate::row::timestamp(at, now))
}

pub fn suggestion_line(suggestion: &postio_model::JoinSuggestion) -> String {
    let side = |person: &Contact| {
        let addresses: Vec<String> = person
            .addresses
            .iter()
            .map(|a| format!("{} · {}", a.address.address, a.times_seen))
            .collect();
        format!("{} ({})", person.display_name(), addresses.join(", "))
    };
    let why = match suggestion.reason {
        postio_model::SuggestionReason::SameName => "same name".to_owned(),
        postio_model::SuggestionReason::Replied { replies: 1 } => {
            "answered from the other address once".to_owned()
        }
        postio_model::SuggestionReason::Replied { replies } => {
            format!("answered from the other address {replies} times")
        }
    };
    format!(
        "{} and {} — {why}",
        side(&suggestion.people[0]),
        side(&suggestion.people[1])
    )
}

pub fn import_summary_line(summary: &postio_model::card::ImportSummary) -> String {
    let people = |n: usize| if n == 1 { "person" } else { "people" };
    let mut done: Vec<String> = Vec::new();
    match (summary.people_created, summary.people_updated) {
        (0, 0) => {}
        (0, updated) => done.push(format!("{updated} {} you already had", people(updated))),
        (created, 0) => done.push(format!("{created} new {}", people(created))),
        (created, updated) => {
            done.push(format!("{created} new {}", people(created)));
            done.push(format!("updated {updated}"));
        }
    }
    match summary.groups_created {
        0 => {}
        1 => done.push("made 1 group".to_owned()),
        n => done.push(format!("made {n} groups")),
    }
    let mut notes: Vec<String> = Vec::new();
    if summary.name_conflicts > 0 {
        notes.push(format!("kept your name for {}", summary.name_conflicts));
    }
    let offered = summary.look_alikes + summary.offered_merges;
    if offered > 0 {
        notes.push(format!(
            "{offered} may be {} you already have, to review",
            if offered == 1 { "someone" } else { "people" }
        ));
    }
    for skip in &summary.skipped {
        notes.push(format!("skipped card {} ({})", skip.index + 1, skip.reason));
    }
    if done.is_empty() && notes.is_empty() {
        return "Nothing to import".to_owned();
    }
    let mut line = if done.is_empty() {
        "Imported nothing".to_owned()
    } else {
        format!("Imported {}", done.join(", "))
    };
    if !notes.is_empty() {
        line.push_str("; ");
        line.push_str(&notes.join("; "));
    }
    line
}

/// The names a join offers (chosen first, then mail's, newest first, each once), the first
/// preselected, and who survives the join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinChoices {
    pub names: Vec<String>,
    /// Organisations carried, each once; more than one is a conflict the user settles, so nothing
    /// is dropped unchosen.
    pub organizations: Vec<String>,
    pub into: ContactId,
}

impl JoinChoices {
    pub fn organization_conflict(&self) -> bool {
        self.organizations.len() > 1
    }
}

/// `people` must not be empty.
pub fn join_name_choices(people: &[Contact]) -> JoinChoices {
    let mut newest: Vec<&Contact> = people.iter().collect();
    newest.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at).then(a.id.cmp(&b.id)));
    let filled =
        |value: Option<&String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let mut offered: Vec<(String, ContactId)> = Vec::new();
    let chosen = newest
        .iter()
        .filter_map(|p| filled(p.name.as_ref()).map(|n| (n, p.id)));
    let seen = newest
        .iter()
        .filter_map(|p| filled(p.seen_name.as_ref()).map(|n| (n, p.id)));
    let addresses = newest.iter().filter_map(|p| {
        p.preferred_address()
            .map(|a| (a.address.address.clone(), p.id))
    });
    let key = |name: &str| {
        name.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    for (name, id) in chosen.chain(seen) {
        if !offered.iter().any(|(n, _)| key(n) == key(&name)) {
            offered.push((name, id));
        }
    }
    if offered.is_empty() {
        offered.extend(addresses);
    }
    let into = offered
        .first()
        .map(|(_, id)| *id)
        .or_else(|| newest.first().map(|p| p.id))
        .expect("a join has people in it");
    let mut organizations: Vec<String> = Vec::new();
    for person in people {
        if let Some(org) = filled(person.organization.as_ref())
            && !organizations.iter().any(|o| key(o) == key(&org))
        {
            organizations.push(org);
        }
    }
    JoinChoices {
        names: offered.into_iter().map(|(name, _)| name).collect(),
        organizations,
        into,
    }
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
            kind: postio_model::ContactKind::Person,
            messages: 0,
            saved: false,
            arrival: None,
            notify: false,
            html_original: false,
            digest_cadence: None,
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

/// The Contacts list's paging window; owns the rows, bounded by `crate::list::CACHE_PAGES`.
#[derive(Debug, Default)]
pub struct ContactsWindow {
    generation: u64,
    total: u32,
    pages: std::collections::HashMap<u32, Vec<ContactListRow>>,
    stale: std::collections::HashMap<u32, Vec<ContactListRow>>,
    order: std::collections::VecDeque<u32>,
    pending: std::collections::HashSet<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Slot<'a> {
    Row(&'a ContactListRow),
    /// Not yet; `request` is the page to ask for, or `None` if it was already asked.
    Loading {
        request: Option<u32>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Delivered {
    Stale,
    Filled {
        positions: std::ops::Range<u32>,
        total_changed: bool,
        /// Pages dropped to keep the bound; views holding their positions must forget them.
        evicted: Vec<u32>,
    },
}

impl ContactsWindow {
    /// Starts the list over at `total` rows; returns the new generation, which requests carry so
    /// stale answers can be told apart.
    pub fn reset(&mut self, total: u32) -> u64 {
        let generation = self.renew(total);
        self.stale.clear();
        generation
    }

    /// As `reset`, but `drawn` gives each page's old rows until the page is read again.
    pub fn renew(&mut self, total: u32) -> u64 {
        self.generation += 1;
        self.total = total;
        self.stale.extend(self.pages.drain());
        self.order.clear();
        self.pending.clear();
        self.generation
    }

    pub fn renewing(&self) -> Vec<u32> {
        let mut pages: Vec<u32> = self.stale.keys().copied().collect();
        pages.sort_unstable();
        pages
    }

    pub fn drawn(&self, index: u32) -> Option<&ContactListRow> {
        if index >= self.total {
            return None;
        }
        self.peek(index).or_else(|| {
            self.stale
                .get(&(index / crate::list::PAGE_SIZE))
                .and_then(|rows| rows.get((index % crate::list::PAGE_SIZE) as usize))
        })
    }

    pub fn total(&self) -> u32 {
        self.total
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn row_at(&mut self, index: u32) -> Slot<'_> {
        let page = index / crate::list::PAGE_SIZE;
        if self.pages.contains_key(&page) {
            self.touch(page);
            let offset = (index % crate::list::PAGE_SIZE) as usize;
            return match self.pages.get(&page).and_then(|rows| rows.get(offset)) {
                Some(row) => Slot::Row(row),
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
        self.stale.remove(&page);
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

    pub fn abandon(&mut self, generation: u64, page: u32) {
        if generation == self.generation {
            self.pending.remove(&page);
        }
    }

    pub fn resident_rows(&self) -> usize {
        self.pages.values().map(Vec::len).sum()
    }

    /// Does not touch the LRU order or request anything.
    pub fn peek(&self, index: u32) -> Option<&ContactListRow> {
        self.pages
            .get(&(index / crate::list::PAGE_SIZE))
            .and_then(|rows| rows.get((index % crate::list::PAGE_SIZE) as usize))
    }

    pub fn position_of(&self, id: postio_model::ContactId) -> Option<u32> {
        self.pages.iter().find_map(|(page, rows)| {
            rows.iter()
                .position(|row| row.id == id)
                .map(|offset| page * crate::list::PAGE_SIZE + offset as u32)
        })
    }

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
                    kind: postio_model::ContactKind::Person,
                    messages: 0,
                    saved: false,
                    arrival: None,
                    notify: false,
                    html_original: false,
                    digest_cadence: None,
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

    #[test]
    fn a_renewed_window_draws_its_old_rows_until_each_page_lands() {
        let mut window = ContactsWindow::default();
        let generation = window.reset(120);
        let _ = window.row_at(0);
        let _ = window.deliver(generation, 0, page(0, PAGE_SIZE), 120);
        let renewed = window.renew(119);
        assert_eq!(window.peek(0), None, "the page is read again");
        assert_eq!(
            window.drawn(0).map(|row| row.name.as_str()),
            Some("Person 0"),
            "and drawn as it was meanwhile"
        );
        assert_eq!(window.row_at(0), Slot::Loading { request: Some(0) });
        let mut fresh = page(0, PAGE_SIZE);
        fresh[0].name = "Merged".to_owned();
        let _ = window.deliver(renewed, 0, fresh, 119);
        assert_eq!(window.drawn(0).map(|row| row.name.as_str()), Some("Merged"));
        window.reset(3);
        assert_eq!(window.drawn(0), None, "a reset draws nothing old");
    }
}

#[cfg(test)]
mod join_tests {
    use super::*;
    use chrono::TimeZone;
    use postio_model::{AddressId, Contact, ContactAddress, ContactId, EmailAddress};

    fn person(
        id: i64,
        name: Option<&str>,
        seen: Option<&str>,
        organization: Option<&str>,
        day: u32,
    ) -> Contact {
        let mut person = Contact::new(ContactAddress {
            id: AddressId::new(id),
            address: EmailAddress::new(None::<String>, format!("p{id}@example.com")),
            times_seen: 1,
            last_seen_at: None,
            written: 0,
        });
        person.id = ContactId::new(id);
        person.name = name.map(str::to_owned);
        person.seen_name = seen.map(str::to_owned);
        person.organization = organization.map(str::to_owned);
        person.last_seen_at = Some(Utc.with_ymd_and_hms(2026, 3, day, 12, 0, 0).unwrap());
        person
    }

    #[test]
    fn names_the_user_chose_come_first_and_the_most_recent_is_preselected() {
        let choices = join_name_choices(&[
            person(1, None, Some("A. L."), None, 9),
            person(2, Some("Ada Lovelace"), Some("ada"), None, 1),
            person(3, None, Some("ada"), None, 5),
        ]);
        assert_eq!(
            choices.names,
            ["Ada Lovelace", "A. L.", "ada"],
            "the user's own name first, then what the mail said, newest first, \
             each once"
        );
        assert_eq!(
            choices.into,
            ContactId::new(2),
            "the person the user named survives"
        );
    }

    #[test]
    fn with_no_name_chosen_the_newest_seen_name_leads() {
        let choices = join_name_choices(&[
            person(1, None, Some("Old Name"), None, 1),
            person(2, None, Some("New Name"), None, 9),
        ]);
        assert_eq!(choices.names, ["New Name", "Old Name"]);
        assert_eq!(choices.into, ContactId::new(2));
    }

    #[test]
    fn names_differing_only_in_case_or_spacing_are_one_choice() {
        let choices = join_name_choices(&[
            person(1, None, Some("Ada  Lovelace"), None, 2),
            person(2, None, Some("ada lovelace"), None, 1),
        ]);
        assert_eq!(choices.names, ["Ada  Lovelace"]);
    }

    #[test]
    fn a_nameless_pair_is_still_offered_a_name() {
        let choices = join_name_choices(&[
            person(1, None, None, None, 1),
            person(2, None, None, None, 2),
        ]);
        assert_eq!(choices.names, ["p2@example.com", "p1@example.com"]);
    }

    #[test]
    fn a_group_with_a_space_is_quoted_in_its_query() {
        assert_eq!(group_mail_query("Family"), "group:Family");
        assert_eq!(group_mail_query("Book club"), "group:\"Book club\"");
    }

    #[test]
    fn an_import_summary_says_what_happened_and_nothing_that_did_not() {
        use postio_model::card::{ImportSummary, Skip};
        let summary = ImportSummary {
            people_created: 2,
            people_updated: 1,
            offered_merges: 1,
            look_alikes: 1,
            groups_created: 1,
            name_conflicts: 1,
            skipped: vec![Skip {
                index: 3,
                reason: "it has no email address".into(),
            }],
            ..ImportSummary::default()
        };
        assert_eq!(
            import_summary_line(&summary),
            "Imported 2 new people, updated 1, made 1 group; \
             kept your name for 1; 2 may be people you already have, to review; \
             skipped card 4 (it has no email address)"
        );
        assert_eq!(
            import_summary_line(&ImportSummary {
                people_updated: 1,
                ..Default::default()
            }),
            "Imported 1 person you already had"
        );
        assert_eq!(
            import_summary_line(&ImportSummary::default()),
            "Nothing to import"
        );
    }

    #[test]
    fn a_suggestion_reads_as_its_evidence() {
        let line = suggestion_line(&postio_model::JoinSuggestion {
            people: [
                person(1, None, Some("Ada"), None, 1),
                person(2, None, Some("Ada"), None, 2),
            ],
            reason: postio_model::SuggestionReason::Replied { replies: 2 },
        });
        assert_eq!(
            line,
            "Ada (p1@example.com · 1) and Ada (p2@example.com · 1) — \
             answered from the other address 2 times"
        );
    }

    #[test]
    fn organisations_are_asked_about_only_when_they_disagree() {
        let agree = join_name_choices(&[
            person(1, Some("Ada"), None, Some("Engines Ltd"), 1),
            person(2, None, Some("ada"), None, 2),
        ]);
        assert!(!agree.organization_conflict());
        assert_eq!(agree.organizations, ["Engines Ltd"]);
        let disagree = join_name_choices(&[
            person(1, Some("Ada"), None, Some("Engines Ltd"), 1),
            person(2, None, Some("ada"), Some("Looms & Co"), 2),
        ]);
        assert!(disagree.organization_conflict());
        assert_eq!(disagree.organizations, ["Engines Ltd", "Looms & Co"]);
    }
}
