//! Contacts, accumulated from headers, and the autocomplete they rank.
//!
//! The bead's acceptance criteria are "prefix search returns
//! most-recent/most-frequent first" and "contacts deduplicate on normalized
//! address".

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{AccountId, Contact, ContactId, EmailAddress, Message};
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn address(name: Option<&str>, address: &str) -> EmailAddress {
    EmailAddress::new(name, address)
}

// ---------------------------------------------------------------------------
// Acceptance: deduplication on the normalized address
// ---------------------------------------------------------------------------

#[test]
fn seeing_the_same_address_twice_is_one_contact_seen_twice() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    let first = contacts
        .record(
            Some(account.id),
            &address(Some("Ada Norwood"), "ada@example.com"),
            at(0),
        )
        .expect("record");
    let second = contacts
        .record(
            Some(account.id),
            &address(Some("Ada"), "ADA@Example.COM"),
            at(1),
        )
        .expect("record again");

    assert_eq!(
        first, second,
        "addresses compare case-insensitively, so this is one correspondent"
    );

    let stored = contacts.get(first).expect("get").expect("the contact");
    assert_eq!(stored.times_seen, 2);
    assert_eq!(stored.last_seen_at, Some(at(1)));
    assert_eq!(
        stored.address.address, "ADA@Example.COM",
        "the most recently seen spelling is what we show"
    );
    assert_eq!(stored.address.name.as_deref(), Some("Ada"));
    assert_eq!(contacts.list(Some(account.id)).expect("list").len(), 1);
}

#[test]
fn last_seen_never_moves_backwards() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    let id = contacts
        .record(Some(account.id), &address(None, "ada@example.com"), at(10))
        .expect("record");
    contacts
        .record(Some(account.id), &address(None, "ada@example.com"), at(2))
        .expect("an older message arriving late");

    let stored = contacts.get(id).expect("get").expect("the contact");
    assert_eq!(stored.last_seen_at, Some(at(10)));
    assert_eq!(stored.times_seen, 2, "but it still counts as a sighting");
}

#[test]
fn a_contact_in_one_account_is_not_a_contact_in_another() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let first = test_support::account(&connection);
    let mut second_account = postio_model::Account::new(
        "Second",
        EmailAddress::new(None::<String>, "second@example.com"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut second_account)
        .expect("create");
    let contacts = ContactRepository::new(&connection);

    let one = contacts
        .record(Some(first.id), &address(None, "ada@example.com"), at(0))
        .expect("record");
    let other = contacts
        .record(
            Some(second_account.id),
            &address(None, "ada@example.com"),
            at(0),
        )
        .expect("record");

    assert_ne!(one, other, "accounts keep their own address books");
    assert_eq!(contacts.list(Some(first.id)).expect("list").len(), 1);
    assert_eq!(
        contacts.list(Some(second_account.id)).expect("list").len(),
        1
    );
}

#[test]
fn a_shared_contact_belongs_to_no_account() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let contacts = ContactRepository::new(&connection);

    let id = contacts
        .record(None, &address(None, "ada@example.com"), at(0))
        .expect("record");
    let again = contacts
        .record(None, &address(None, "ada@example.com"), at(1))
        .expect("record");

    assert_eq!(id, again);
    let stored = contacts.get(id).expect("get").expect("the contact");
    assert_eq!(stored.account_id, None);
}

#[test]
fn every_address_on_a_message_becomes_a_contact_once() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let contacts = ContactRepository::new(&connection);

    let mut message = Message::new(account.id, inbox, at(0));
    message.from = vec![address(Some("Ada"), "ada@example.com")];
    message.to = vec![
        address(Some("Quinn"), "quinn@example.net"),
        // The same person twice in one message must not count twice.
        address(None, "QUINN@example.net"),
    ];
    message.cc = vec![address(None, "list@example.org")];

    let recorded = contacts.record_message(&message).expect("record");

    assert_eq!(recorded, 3, "three distinct correspondents");
    let stored = contacts.list(Some(account.id)).expect("list");
    assert_eq!(stored.len(), 3);
    let quinn = stored
        .iter()
        .find(|contact| contact.address.normalized() == "quinn@example.net")
        .expect("quinn");
    assert_eq!(
        quinn.times_seen, 1,
        "appearing twice in one message is one sighting"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: prefix search, most frequent and most recent first
// ---------------------------------------------------------------------------

/// Records `address` `times` times, most recently `days_ago` days ago.
fn seen(
    contacts: &ContactRepository<'_>,
    account: AccountId,
    name: &str,
    email: &str,
    times: u32,
    day: i64,
) -> ContactId {
    let mut id = ContactId::UNASSIGNED;
    for index in 0..times {
        id = contacts
            .record(
                Some(account),
                &address(Some(name), email),
                at(day - i64::from(times - index - 1)),
            )
            .expect("record");
    }
    id
}

/// Frequency is the tie-break, not the lead.
///
/// Every contact here is last seen on the same day -- `seen(.., times, 0)`
/// walks backwards from `at(0)`, so they all land on it -- which is what
/// makes this a test about `times_seen` at all. It was named for ranking
/// "most written to first" when frequency led the ordering; #424 put recency
/// first, and the assertion below survived unchanged because a tie on
/// recency is exactly the case where frequency still decides. The name now
/// says that, so nobody reads it as the rule.
#[test]
fn frequency_decides_between_addresses_used_equally_recently() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    seen(
        &contacts,
        account.id,
        "Ada Norwood",
        "ada@example.com",
        1,
        0,
    );
    seen(
        &contacts,
        account.id,
        "Adam Byrne",
        "adam@example.com",
        9,
        0,
    );
    seen(
        &contacts,
        account.id,
        "Adele Fisk",
        "adele@example.com",
        4,
        0,
    );
    seen(
        &contacts,
        account.id,
        "Quinn Abara",
        "quinn@example.net",
        20,
        0,
    );

    let matches = contacts.search(Some(account.id), "ad", 10).expect("search");

    let addresses: Vec<&str> = matches
        .iter()
        .map(|contact| contact.address.address.as_str())
        .collect();
    assert_eq!(
        addresses,
        ["adam@example.com", "adele@example.com", "ada@example.com"],
        "all three were last seen on the same day, so the most written to \
         wins; quinn does not match the prefix at all"
    );
}

#[test]
fn a_tie_on_frequency_is_broken_by_recency() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    seen(
        &contacts,
        account.id,
        "Ada One",
        "ada.one@example.com",
        3,
        0,
    );
    seen(
        &contacts,
        account.id,
        "Ada Two",
        "ada.two@example.com",
        3,
        30,
    );

    let matches = contacts
        .search(Some(account.id), "ada", 10)
        .expect("search");

    assert_eq!(
        matches[0].address.address, "ada.two@example.com",
        "when two are equally familiar, the one written to yesterday wins"
    );
}

/// #424: recency outranks frequency, so a robot cannot bury a person.
///
/// ADR 0007 Q6 names this exact pathology -- "`times_seen = 400` for a
/// mailing list robot is not evidence that the user wants to write to it" --
/// and answers it with *bands*, ranking `user`/`import` contacts above `mail`
/// sightings and keeping `(times_seen DESC, last_seen_at DESC)` inside the
/// mail band. There are no bands: `contacts` has no `source` column and every
/// row here is a mail sighting, so the band that was supposed to rescue the
/// person does not exist and frequency decides everything. Until it does,
/// within-band order is the only lever, and the reported behaviour is what
/// the address a person actually uses should get.
#[test]
fn the_address_used_most_recently_comes_before_the_one_used_most_often() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    // Seen constantly, and last seen a month before the person below.
    seen(
        &contacts,
        account.id,
        "Announce Robot",
        "announce@example.net",
        400,
        0,
    );
    // Written to exactly once, today.
    seen(
        &contacts,
        account.id,
        "Anna Beck",
        "anna@example.org",
        1,
        30,
    );

    let matches = contacts.search(Some(account.id), "an", 10).expect("search");
    let addresses: Vec<&str> = matches
        .iter()
        .map(|contact| contact.address.address.as_str())
        .collect();
    assert_eq!(
        addresses,
        ["anna@example.org", "announce@example.net"],
        "the address used most recently comes first; 400 sightings of a robot \
         are not evidence that anybody wants to write to it"
    );
}

#[test]
fn autocomplete_matches_the_display_name_as_well_as_the_address() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    seen(
        &contacts,
        account.id,
        "Quinn Abara",
        "q.abara@example.net",
        2,
        0,
    );

    assert_eq!(
        contacts
            .search(Some(account.id), "quinn", 10)
            .expect("search")
            .len(),
        1,
        "people type the name they remember, not the address"
    );
    assert_eq!(
        contacts
            .search(Some(account.id), "abara", 10)
            .expect("search")
            .len(),
        1,
        "including the surname"
    );
    assert!(
        contacts
            .search(Some(account.id), "zz", 10)
            .expect("search")
            .is_empty()
    );
}

#[test]
fn a_user_set_name_overrides_what_the_headers_carried() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    let id = seen(
        &contacts,
        account.id,
        "ADA NORWOOD",
        "ada@example.com",
        1,
        0,
    );
    contacts.set_name(id, Some("Ada")).expect("rename");

    let stored = contacts.get(id).expect("get").expect("the contact");
    assert_eq!(stored.name.as_deref(), Some("Ada"));
    assert_eq!(stored.display_name(), "Ada");
    assert_eq!(
        stored.address.name.as_deref(),
        Some("ADA NORWOOD"),
        "what the headers said is kept; the user's name simply wins"
    );

    contacts
        .record(
            Some(account.id),
            &address(Some("A. Norwood"), "ada@example.com"),
            at(5),
        )
        .expect("seen again");
    assert_eq!(
        contacts
            .get(id)
            .expect("get")
            .expect("the contact")
            .name
            .as_deref(),
        Some("Ada"),
        "and a later sighting does not overwrite it"
    );
}

#[test]
fn a_contact_can_be_looked_up_and_deleted() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    let id = seen(&contacts, account.id, "Ada", "ada@example.com", 1, 0);

    assert_eq!(
        contacts
            .by_address(Some(account.id), "ADA@example.com")
            .expect("lookup")
            .map(|contact: Contact| contact.id),
        Some(id)
    );
    assert!(contacts.delete(id).expect("delete"));
    assert!(!contacts.delete(id).expect("delete again"));
    assert!(contacts.get(id).expect("get").is_none());
}

#[test]
fn searching_an_empty_prefix_returns_the_most_familiar_correspondents() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let contacts = ContactRepository::new(&connection);

    seen(&contacts, account.id, "Ada", "ada@example.com", 2, 0);
    seen(&contacts, account.id, "Quinn", "quinn@example.net", 5, 0);

    let matches = contacts.search(Some(account.id), "", 1).expect("search");

    assert_eq!(matches.len(), 1, "the limit is respected");
    assert_eq!(matches[0].address.address, "quinn@example.net");
}
