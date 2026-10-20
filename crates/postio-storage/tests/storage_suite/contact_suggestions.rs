//! Who might be the same person (specs/005-contacts User Story 4, R11).
//!
//! Two live people whose names match, or an address that answered mail sent
//! to another, are offered as a pair with the evidence; a pair the user
//! dismissed is never offered again, whatever mail or joins come after; and
//! nothing here ever joins anyone.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{Account, ContactView, EmailAddress, MailboxId, Message, SuggestionReason};
use postio_storage::Connection;
use postio_storage::repository::ContactRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install, scans};

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

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

async fn from(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
    name: &str,
    email: &str,
    times: i64,
) {
    for day in 0..times {
        let mut message = Message::new(account.id, inbox, at(day));
        message.from = vec![EmailAddress::new(Some(name), email)];
        ContactRepository::new(connection)
            .record_message(&message, std::slice::from_ref(&account.address))
            .await
            .expect("record");
    }
}

/// Each suggestion as its two preferred addresses, sorted, and its reason.
async fn offered(connection: &Connection) -> Vec<(Vec<String>, SuggestionReason)> {
    ContactRepository::new(connection)
        .suggestions(50)
        .await
        .expect("suggestions")
        .into_iter()
        .map(|s| {
            let mut pair: Vec<String> = s
                .people
                .iter()
                .map(|p| {
                    p.preferred_address()
                        .expect("an address")
                        .address
                        .address
                        .clone()
                })
                .collect();
            pair.sort();
            (pair, s.reason)
        })
        .collect()
}

#[tokio::test]
async fn two_people_with_one_name_are_offered_with_their_evidence() {
    let (_db, connection, account, inbox) = setup().await;
    from(
        &connection,
        &account,
        inbox,
        "Ada Lovelace",
        "ada@work.example",
        3,
    )
    .await;
    from(
        &connection,
        &account,
        inbox,
        "ada  lovelace",
        "ada@home.example",
        1,
    )
    .await;
    from(
        &connection,
        &account,
        inbox,
        "Grace Hopper",
        "grace@example.org",
        1,
    )
    .await;

    let found = ContactRepository::new(&connection)
        .suggestions(50)
        .await
        .expect("suggestions");
    assert_eq!(found.len(), 1, "one pair: {found:?}");
    assert_eq!(found[0].reason, SuggestionReason::SameName);
    let mut seen: Vec<u32> = found[0]
        .people
        .iter()
        .map(|p| p.addresses[0].times_seen)
        .collect();
    seen.sort();
    assert_eq!(
        seen,
        [1, 3],
        "each side's own count, for the user to judge by"
    );
}

#[tokio::test]
async fn an_answer_from_another_address_is_offered() {
    let (_db, connection, account, inbox) = setup().await;
    from(&connection, &account, inbox, "Ada", "ada@work.example", 1).await;
    from(&connection, &account, inbox, "A. L.", "ada@home.example", 1).await;
    let contacts = ContactRepository::new(&connection);
    contacts
        .note_reply("ada@work.example", "ada@home.example")
        .await
        .expect("note");
    contacts
        .note_reply("ADA@work.example", "ada@home.example")
        .await
        .expect("note again");
    assert_eq!(
        offered(&connection).await,
        [(
            vec!["ada@home.example".to_owned(), "ada@work.example".to_owned()],
            SuggestionReason::Replied { replies: 2 }
        )]
    );
}

#[tokio::test]
async fn a_dismissed_pair_stays_dismissed_through_mail_and_joins() {
    let (_db, connection, account, inbox) = setup().await;
    from(
        &connection,
        &account,
        inbox,
        "Ada Lovelace",
        "ada@work.example",
        1,
    )
    .await;
    from(
        &connection,
        &account,
        inbox,
        "Ada Lovelace",
        "ada@home.example",
        1,
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let found = contacts.suggestions(50).await.expect("suggestions");
    let [a, b] = [found[0].people[0].id, found[0].people[1].id];
    let people_before = contacts.count(ContactView::Everyone).await.expect("count");

    contacts.dismiss(a, b).await.expect("dismiss");
    assert!(offered(&connection).await.is_empty());
    assert_eq!(
        contacts.count(ContactView::Everyone).await.expect("count"),
        people_before,
        "a suggestion joins nobody"
    );

    from(
        &connection,
        &account,
        inbox,
        "Ada Lovelace",
        "ada@work.example",
        2,
    )
    .await;
    assert!(
        offered(&connection).await.is_empty(),
        "more mail changes nothing"
    );

    // One side joins a third address; the dismissal still spans them.
    from(
        &connection,
        &account,
        inbox,
        "Countess",
        "countess@example.net",
        1,
    )
    .await;
    let third = contacts
        .by_address("countess@example.net")
        .await
        .expect("lookup")
        .expect("third");
    contacts
        .join(a, &[third.id], "Ada Lovelace", None)
        .await
        .expect("join");
    assert!(
        offered(&connection).await.is_empty(),
        "nor does a join on either side"
    );
}

#[tokio::test]
async fn the_suggestions_read_is_bounded_and_scans_nothing_it_does_not_need() {
    let (_db, connection, account, inbox) = setup().await;
    for i in 0..40 {
        from(
            &connection,
            &account,
            inbox,
            &format!("Person {i}"),
            &format!("p{i}@example.com"),
            1,
        )
        .await;
    }
    from(
        &connection,
        &account,
        inbox,
        "Person 1",
        "other1@example.com",
        1,
    )
    .await;
    install(&connection);
    let contacts = ContactRepository::new(&connection);
    let _ = contacts.suggestions(50).await.expect("warm");
    let counts = counted_async(|| async {
        let found = contacts.suggestions(50).await.expect("suggestions");
        assert_eq!(found.len(), 1);
    })
    .await;
    assert!(
        counts.statements <= 3,
        "the pairs, then their people and addresses: {} statements",
        counts.statements
    );
    for sql in test_support::contact_suggestion_statements() {
        // The reply candidates are the evidence and are read whole; every
        // other step has to seek.
        let scanned: Vec<String> = scans(&connection, &sql)
            .await
            .into_iter()
            .filter(|step| !step.contains("cand"))
            .collect();
        assert!(
            scanned.is_empty(),
            "suggestions walk every person or address: {scanned:?}"
        );
    }
}
