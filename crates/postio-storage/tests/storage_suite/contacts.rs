//! People, built from the mail, and the completion they rank.
//!
//! specs/005-contacts: a contact is a person who owns addresses. These are
//! the behaviours the one-row-per-address table had — one sighting per
//! message, case-insensitive identity, recency before frequency, the address
//! book above the mail — asked again of people, plus what the person model
//! adds: written-to, the user's own addresses left out, and one person for
//! two addresses.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, ContactId, ContactSource, ContactState, EmailAddress, MailboxId, Message,
};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

fn address(name: Option<&str>, address: &str) -> EmailAddress {
    EmailAddress::new(name, address)
}

/// A store with one account and its inbox.
async fn setup() -> (
    postio_storage::Store,
    postio_storage::Checkout,
    Account,
    MailboxId,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

/// Records a message from `from` on `day`, the way sync does.
async fn received(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    from: EmailAddress,
    day: i64,
) {
    let mut message = Message::new(account.id, inbox, at(day));
    message.from = vec![from];
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record");
}

/// Records `times` messages from `email`, the last of them on `day`.
async fn seen(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    name: &str,
    email: &str,
    times: u32,
    day: i64,
) {
    for index in 0..times {
        received(
            connection,
            account,
            inbox,
            address(Some(name), email),
            day - i64::from(times - index - 1),
        )
        .await;
    }
}

/// The preferred address of each person completion offers, in order.
async fn completes(connection: &Connection, prefix: &str) -> Vec<String> {
    ContactRepository::new(connection)
        .complete(prefix, 10)
        .await
        .expect("complete")
        .iter()
        .map(|person| person.addresses[0].address.address.clone())
        .collect()
}

async fn person(connection: &Connection, email: &str) -> postio_model::Contact {
    ContactRepository::new(connection)
        .by_address(email)
        .await
        .expect("by_address")
        .unwrap_or_else(|| panic!("nobody owns {email}"))
}

async fn set_state(connection: &Connection, id: ContactId, state: &str) {
    postio_storage::sql::execute(
        connection,
        "UPDATE contacts SET state = ?2 WHERE id = ?1",
        postio_storage::sql::bind![id.get(), state],
    )
    .await
    .expect("set state");
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

#[tokio::test]
async fn seeing_the_same_address_twice_is_one_person_seen_twice() {
    let (_db, connection, account, inbox) = setup().await;
    received(
        &connection,
        &account,
        inbox,
        address(Some("Ada Norwood"), "ada@example.com"),
        0,
    )
    .await;
    received(
        &connection,
        &account,
        inbox,
        address(Some("Ada"), "ADA@Example.COM"),
        1,
    )
    .await;

    let stored = person(&connection, "ada@example.com").await;
    assert_eq!(
        stored.times_seen, 2,
        "addresses compare case-insensitively (FR-011)"
    );
    assert_eq!(stored.last_seen_at, Some(at(1)));
    assert_eq!(
        stored.addresses.len(),
        1,
        "one address, not two spellings of it"
    );
    assert_eq!(stored.addresses[0].times_seen, 2);
    assert_eq!(
        stored.display_name(),
        "Ada",
        "the name the most recent message carried is what we show"
    );
    assert_eq!(
        ContactRepository::new(&connection)
            .people(100)
            .await
            .expect("people")
            .len(),
        1
    );
}

#[tokio::test]
async fn last_seen_never_moves_backwards_and_an_older_name_does_not_win() {
    let (_db, connection, account, inbox) = setup().await;
    received(
        &connection,
        &account,
        inbox,
        address(Some("Ada Now"), "ada@example.com"),
        10,
    )
    .await;
    received(
        &connection,
        &account,
        inbox,
        address(Some("Ada Then"), "ada@example.com"),
        2,
    )
    .await;

    let stored = person(&connection, "ada@example.com").await;
    assert_eq!(stored.times_seen, 2, "a late message is still a sighting");
    assert_eq!(
        stored.last_seen_at,
        Some(at(10)),
        "but it does not make them more recent"
    );
    assert_eq!(stored.display_name(), "Ada Now");
}

#[tokio::test]
async fn two_accounts_keep_separate_evidence_about_one_person() {
    // People are shared across accounts; what each mailbox saw stays its own
    // (specs/005-contacts, the address-book decision's Q5).
    let (_db, connection, work, inbox) = setup().await;
    let mut home = Account::new("Home", address(None, "me@home.example"));
    home.incoming.host = "imap.example.com".into();
    home.outgoing.host = "smtp.example.com".into();
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut home)
        .await
        .expect("second account");
    let home_inbox = test_support::mailbox(&connection, &home, "INBOX").await.id;

    received(
        &connection,
        &work,
        inbox,
        address(None, "ada@example.com"),
        0,
    )
    .await;
    received(
        &connection,
        &home,
        home_inbox,
        address(None, "ada@example.com"),
        1,
    )
    .await;

    let people = ContactRepository::new(&connection)
        .people(100)
        .await
        .expect("people");
    assert_eq!(people.len(), 1, "one person, whichever account saw them");
    assert_eq!(people[0].times_seen, 2);
    let rows =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM contact_sightings", ())
            .await
            .expect("count");
    assert_eq!(rows, 2, "one row of evidence per account");
}

#[tokio::test]
async fn every_address_on_a_message_becomes_a_person_once() {
    let (_db, connection, account, inbox) = setup().await;
    let mut message = Message::new(account.id, inbox, at(0));
    message.from = vec![address(Some("Ada"), "ada@example.com")];
    message.to = vec![
        address(Some("Quinn"), "quinn@example.net"),
        // The same person twice in one message must not count twice.
        address(None, "QUINN@example.net"),
    ];
    message.cc = vec![address(None, "list@example.org")];

    let recorded = ContactRepository::new(&connection)
        .record_message(&message, &[])
        .await
        .expect("record");

    assert_eq!(recorded, 3, "three distinct correspondents");
    assert_eq!(
        ContactRepository::new(&connection)
            .people(100)
            .await
            .expect("people")
            .len(),
        3
    );
    assert_eq!(
        person(&connection, "quinn@example.net").await.times_seen,
        1,
        "appearing twice in one message is one sighting"
    );
}

#[tokio::test]
async fn the_users_own_addresses_are_never_contacts() {
    let (_db, connection, account, inbox) = setup().await;
    let mut message = Message::new(account.id, inbox, at(0));
    message.from = vec![address(Some("Ada"), "ada@example.com")];
    message.to = vec![address(Some("Me"), "TEST@example.com")];

    ContactRepository::new(&connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record");

    assert!(
        ContactRepository::new(&connection)
            .by_address("test@example.com")
            .await
            .expect("lookup")
            .is_none(),
        "the account's own address, in any case, is not a person"
    );
}

#[tokio::test]
async fn mail_the_user_sent_marks_every_recipient_written_to() {
    let (_db, connection, account, inbox) = setup().await;
    let mut sent = Message::new(account.id, inbox, at(0));
    sent.from = vec![account.address.clone()];
    sent.to = vec![address(Some("Quinn"), "quinn@example.net")];
    sent.cc = vec![address(None, "ada@example.com")];
    sent.bcc = vec![address(None, "grace@example.org")];
    ContactRepository::new(&connection)
        .record_message(&sent, std::slice::from_ref(&account.address))
        .await
        .expect("record sent");

    // A newsletter the user only received.
    received(
        &connection,
        &account,
        inbox,
        address(Some("News"), "news@example.com"),
        1,
    )
    .await;

    for email in ["quinn@example.net", "ada@example.com", "grace@example.org"] {
        assert_eq!(
            person(&connection, email).await.written,
            1,
            "{email} was written to"
        );
    }
    assert_eq!(
        person(&connection, "news@example.com").await.written,
        0,
        "received is not written to"
    );
}

#[tokio::test]
async fn a_deleted_persons_address_keeps_counting_and_the_person_stays_deleted() {
    // specs/005-contacts R4: deletion keeps the person, so the next message
    // from them counts toward someone hidden instead of bringing them back.
    let (_db, connection, account, inbox) = setup().await;
    received(
        &connection,
        &account,
        inbox,
        address(None, "robot@example.com"),
        0,
    )
    .await;
    let robot = person(&connection, "robot@example.com").await;
    set_state(&connection, robot.id, "deleted").await;

    received(
        &connection,
        &account,
        inbox,
        address(None, "robot@example.com"),
        1,
    )
    .await;

    let after = person(&connection, "robot@example.com").await;
    assert_eq!(after.id, robot.id, "no second person appears");
    assert_eq!(after.state, ContactState::Deleted);
    assert_eq!(after.times_seen, 2);
    assert!(completes(&connection, "robot").await.is_empty());
}

// ---------------------------------------------------------------------------
// Completion: banded, then recency, then frequency
// ---------------------------------------------------------------------------

/// Frequency is the tie-break, not the lead: every person here was last seen
/// on the same day, which is what makes this a test about frequency at all.
#[tokio::test]
async fn frequency_decides_between_people_seen_equally_recently() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Ada Norwood",
        "ada@example.com",
        1,
        0,
    )
    .await;
    seen(
        &connection,
        &account,
        inbox,
        "Adam Byrne",
        "adam@example.com",
        9,
        0,
    )
    .await;
    seen(
        &connection,
        &account,
        inbox,
        "Adele Fisk",
        "adele@example.com",
        4,
        0,
    )
    .await;
    seen(
        &connection,
        &account,
        inbox,
        "Quinn Abara",
        "quinn@example.net",
        20,
        0,
    )
    .await;

    assert_eq!(
        completes(&connection, "ad").await,
        ["adam@example.com", "adele@example.com", "ada@example.com"],
        "quinn does not match the prefix at all"
    );
}

#[tokio::test]
async fn a_tie_on_frequency_is_broken_by_recency() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Ada One",
        "ada.one@example.com",
        3,
        0,
    )
    .await;
    seen(
        &connection,
        &account,
        inbox,
        "Ada Two",
        "ada.two@example.com",
        3,
        30,
    )
    .await;

    assert_eq!(
        completes(&connection, "ada").await[0],
        "ada.two@example.com"
    );
}

/// #424: recency outranks frequency, so a robot cannot bury a person.
#[tokio::test]
async fn the_person_seen_most_recently_comes_before_the_one_seen_most_often() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Announce Robot",
        "announce@example.net",
        400,
        0,
    )
    .await;
    seen(
        &connection,
        &account,
        inbox,
        "Anna Beck",
        "anna@example.org",
        1,
        30,
    )
    .await;

    assert_eq!(
        completes(&connection, "an").await,
        ["anna@example.org", "announce@example.net"],
    );
}

#[tokio::test]
async fn completion_matches_either_name_and_any_part_of_an_address() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Quinn Abara",
        "q.abara@example.net",
        2,
        0,
    )
    .await;

    for prefix in ["quinn", "abara", "Q.ABARA", "q.abara@ex", "example"] {
        assert_eq!(completes(&connection, prefix).await.len(), 1, "{prefix}");
    }
    assert!(completes(&connection, "zz").await.is_empty());
}

#[tokio::test]
async fn an_empty_prefix_offers_whoever_was_seen_last() {
    let (_db, connection, account, inbox) = setup().await;
    seen(&connection, &account, inbox, "Old", "old@example.com", 5, 0).await;
    seen(
        &connection,
        &account,
        inbox,
        "Quinn",
        "quinn@example.net",
        1,
        9,
    )
    .await;

    let first = ContactRepository::new(&connection)
        .complete("", 1)
        .await
        .expect("complete");
    assert_eq!(first.len(), 1, "the limit is respected");
    assert_eq!(first[0].addresses[0].address.address, "quinn@example.net");
}

#[tokio::test]
async fn a_person_the_user_made_outranks_a_frequent_mail_sighting() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Ann Robot",
        "ann.robot@example.net",
        50,
        5,
    )
    .await;
    ContactRepository::new(&connection)
        .create(Some("Ann Lee"), &[address(None, "ann@example.org")])
        .await
        .expect("create");

    assert_eq!(
        completes(&connection, "ann").await,
        ["ann@example.org", "ann.robot@example.net"],
        "the address book is a band above the mail, never mixed into it"
    );
}

#[tokio::test]
async fn a_person_with_two_addresses_is_offered_once_preferred_first() {
    let (_db, connection, _account, _inbox) = setup().await;
    let id = ContactRepository::new(&connection)
        .create(
            Some("Ada Lovelace"),
            &[
                address(None, "ada@work.example"),
                address(None, "ada@home.example"),
            ],
        )
        .await
        .expect("create");

    let offered = ContactRepository::new(&connection)
        .complete("ada", 10)
        .await
        .expect("complete");
    assert_eq!(offered.len(), 1, "one person, not one row per address");
    assert_eq!(offered[0].id, id);
    let addresses: Vec<&str> = offered[0]
        .addresses
        .iter()
        .map(|a| a.address.address.as_str())
        .collect();
    assert_eq!(
        addresses,
        ["ada@work.example", "ada@home.example"],
        "the first address given is preferred, and offered first"
    );
}

#[tokio::test]
async fn a_deleted_person_is_offered_nowhere() {
    let (_db, connection, account, inbox) = setup().await;
    seen(
        &connection,
        &account,
        inbox,
        "Quinn",
        "quinn@example.net",
        3,
        0,
    )
    .await;
    let quinn = person(&connection, "quinn@example.net").await;
    set_state(&connection, quinn.id, "deleted").await;

    assert!(completes(&connection, "quinn").await.is_empty());
    assert!(completes(&connection, "").await.is_empty());
    assert!(
        ContactRepository::new(&connection)
            .people(100)
            .await
            .expect("people")
            .is_empty(),
        "nor in what the finder holds"
    );
}

// ---------------------------------------------------------------------------
// Creating a person
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_person_can_be_made_for_an_address_no_mail_has_carried() {
    let (_db, connection, _account, _inbox) = setup().await;
    let id = ContactRepository::new(&connection)
        .create(Some("Grace"), &[address(None, "grace@example.org")])
        .await
        .expect("create");

    let grace = ContactRepository::new(&connection)
        .get(id)
        .await
        .expect("get")
        .expect("grace");
    assert_eq!(grace.source, ContactSource::User);
    assert_eq!(grace.times_seen, 0, "making a person is not a sighting");
    assert_eq!(completes(&connection, "grace").await, ["grace@example.org"]);
}

#[tokio::test]
async fn a_name_the_user_gave_is_never_overwritten_by_the_mail() {
    let (_db, connection, account, inbox) = setup().await;
    ContactRepository::new(&connection)
        .create(Some("Ada Lovelace"), &[address(None, "ada@example.com")])
        .await
        .expect("create");
    received(
        &connection,
        &account,
        inbox,
        address(Some("ADA (via List)"), "ada@example.com"),
        1,
    )
    .await;

    let ada = person(&connection, "ada@example.com").await;
    assert_eq!(ada.display_name(), "Ada Lovelace");
    assert_eq!(
        ada.seen_name.as_deref(),
        Some("ADA (via List)"),
        "the mail's name is kept, not shown"
    );
}

#[tokio::test]
async fn an_address_a_live_person_owns_is_refused_and_names_the_owner() {
    let (_db, connection, account, inbox) = setup().await;
    received(
        &connection,
        &account,
        inbox,
        address(None, "ada@example.com"),
        0,
    )
    .await;
    let owner = person(&connection, "ada@example.com").await;

    let refused = ContactRepository::new(&connection)
        .create(Some("Someone"), &[address(None, "Ada@Example.com")])
        .await;
    match refused {
        Err(postio_storage::Error::AddressOwned { owner: named, .. }) => {
            assert_eq!(named, owner.id.get())
        }
        other => panic!("expected AddressOwned, got {other:?}"),
    }
}

#[tokio::test]
async fn an_address_a_deleted_person_owns_moves_and_keeps_its_history() {
    let (_db, connection, account, inbox) = setup().await;
    seen(&connection, &account, inbox, "Ada", "ada@example.com", 3, 0).await;
    let old = person(&connection, "ada@example.com").await;
    set_state(&connection, old.id, "deleted").await;

    let id = ContactRepository::new(&connection)
        .create(Some("Ada Lovelace"), &[address(None, "ada@example.com")])
        .await
        .expect("create");

    let ada = person(&connection, "ada@example.com").await;
    assert_eq!(ada.id, id);
    assert_eq!(ada.state, ContactState::Live);
    assert_eq!(
        ada.times_seen, 3,
        "the sightings travel with the address (FR-024)"
    );
    let emptied = ContactRepository::new(&connection)
        .get(old.id)
        .await
        .expect("get")
        .expect("kept, for an undo to give the address back to");
    assert_eq!(
        emptied.state,
        ContactState::Merged,
        "a deleted person left with no address is folded away, offered nowhere"
    );
    assert!(emptied.addresses.is_empty());
}
