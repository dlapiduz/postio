//! The evidence a contact's kind is decided from, and the decision stored
//! (specs/011-focus-contacts S2).

use chrono::{DateTime, TimeZone, Utc};

use postio_model::promoted::{AUTO_GENERATED, PRECEDENCE_BULK, PromotedHeaders};
use postio_model::{Account, ContactId, ContactKind, EmailAddress, KindSource, MailboxId, Message};
use postio_storage::repository::{ContactRepository, MessageRepository};
use postio_storage::test_support::counting::{counted_async, unbounded};
use postio_storage::{Checkout, test_support};

fn at(days: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap() + chrono::Duration::days(days)
}

async fn setup() -> (postio_storage::Store, Checkout, Account, MailboxId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

/// Stores and records a message from `from`, as sync does.
async fn received(
    connection: &Checkout,
    account: &Account,
    inbox: MailboxId,
    day: i64,
    from: &str,
    promoted: Option<PromotedHeaders>,
    list_id: Option<&str>,
) {
    let mut message = Message::new(account.id, inbox, at(day));
    message.from = vec![EmailAddress::new(Some("Ledger Weekly"), from)];
    message.to = vec![account.address.clone()];
    message.promoted = promoted;
    message.list_id = list_id.map(str::to_owned);
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("store");
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record");
}

async fn person(connection: &Checkout, email: &str) -> ContactId {
    ContactRepository::new(connection)
        .by_address(email)
        .await
        .expect("read")
        .expect("a person")
        .id
}

#[tokio::test]
async fn a_senders_signals_count_what_their_mail_carried() {
    let (_database, connection, account, inbox) = setup().await;
    let bulk = PromotedHeaders {
        unsubscribe_offered: true,
        automation: PRECEDENCE_BULK,
    };
    for day in 0..3 {
        received(
            &connection,
            &account,
            inbox,
            day,
            "news@ledger.example",
            Some(bulk),
            Some("<weekly.ledger.example>"),
        )
        .await;
    }
    received(
        &connection,
        &account,
        inbox,
        4,
        "news@ledger.example",
        Some(PromotedHeaders {
            unsubscribe_offered: false,
            automation: AUTO_GENERATED,
        }),
        None,
    )
    .await;
    received(
        &connection,
        &account,
        inbox,
        5,
        "news@ledger.example",
        None,
        None,
    )
    .await;
    let contacts = ContactRepository::new(&connection);

    let signals = contacts
        .kind_signals(person(&connection, "news@ledger.example").await)
        .await
        .unwrap()
        .expect("a live person");

    assert_eq!(signals.address, "news@ledger.example");
    assert_eq!(signals.name.as_deref(), Some("Ledger Weekly"));
    assert_eq!(signals.messages, 5);
    assert_eq!(signals.with_list_id, 3);
    assert_eq!(signals.with_unsubscribe, 3);
    assert_eq!(signals.with_bulk, 3);
    assert_eq!(signals.with_auto_generated, 1);
    assert_eq!(signals.written_to, 0);
}

#[tokio::test]
async fn only_the_newest_fifty_messages_are_counted_and_it_costs_the_same_for_one() {
    let (_database, connection, account, inbox) = setup().await;
    for day in 0..60 {
        received(
            &connection,
            &account,
            inbox,
            day,
            "busy@robot.example",
            None,
            None,
        )
        .await;
    }
    received(
        &connection,
        &account,
        inbox,
        70,
        "quiet@robot.example",
        None,
        None,
    )
    .await;
    let contacts = ContactRepository::new(&connection);
    let (busy, quiet) = (
        person(&connection, "busy@robot.example").await,
        person(&connection, "quiet@robot.example").await,
    );

    let few = counted_async(|| async {
        contacts.kind_signals(quiet).await.unwrap();
    })
    .await;
    let mut many = None;
    let counted = counted_async(|| async {
        many = contacts.kind_signals(busy).await.unwrap();
    })
    .await;

    assert_eq!(
        many.unwrap().messages,
        50,
        "a window, not the whole history"
    );
    assert_eq!(few.statements, counted.statements);
    assert!(counted.rows <= 60, "read {} rows", counted.rows);
    assert_eq!(
        unbounded(
            &connection,
            ContactRepository::explain_kind_window(),
            &["messages", "recipients"]
        )
        .await,
        Vec::<String>::new(),
        "the window seeks the sender's newest rows, it does not walk their history"
    );
}

#[tokio::test]
async fn people_at_the_same_domain_are_counted_without_the_sender_or_the_robots() {
    let (_database, connection, account, inbox) = setup().await;
    for who in [
        "info@acme.example",
        "jo@acme.example",
        "kim@acme.example",
        "bot@acme.example",
        "lee@other.example",
    ] {
        received(&connection, &account, inbox, 1, who, None, None).await;
    }
    let contacts = ContactRepository::new(&connection);
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET kind = 'automated'
          WHERE id = (SELECT contact_id FROM addresses WHERE address_normalized = 'bot@acme.example')",
        (),
    )
    .await
    .unwrap();

    let signals = contacts
        .kind_signals(person(&connection, "info@acme.example").await)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        signals.people_at_domain, 2,
        "jo and kim; not info, not the bot, not lee"
    );
}

#[tokio::test]
async fn a_detected_kind_is_stored_once_per_detector_and_a_users_kind_is_never_overwritten() {
    let (_database, connection, account, inbox) = setup().await;
    for who in ["a@x.example", "b@x.example", "c@x.example"] {
        received(&connection, &account, inbox, 1, who, None, None).await;
    }
    let contacts = ContactRepository::new(&connection);
    let (a, b, c) = (
        person(&connection, "a@x.example").await,
        person(&connection, "b@x.example").await,
        person(&connection, "c@x.example").await,
    );
    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET kind = 'company', kind_source = 'user' WHERE id = ?1",
        postio_storage::sql::bind![b.get()],
    )
    .await
    .unwrap();

    assert_eq!(
        contacts.pending_kinds(1, None, 10).await.unwrap().len(),
        2,
        "b chose for themselves"
    );
    contacts
        .set_detected_kind(a, ContactKind::Newsletter, 1)
        .await
        .unwrap();
    contacts
        .set_detected_kind(b, ContactKind::Automated, 1)
        .await
        .unwrap();

    let a = contacts.get(a).await.unwrap().unwrap();
    assert_eq!(
        (a.kind, a.kind_source),
        (ContactKind::Newsletter, KindSource::Detected)
    );
    let b = contacts.get(b).await.unwrap().unwrap();
    assert_eq!(
        (b.kind, b.kind_source),
        (ContactKind::Company, KindSource::User),
        "the user's word stands"
    );
    assert_eq!(
        contacts.pending_kinds(1, None, 10).await.unwrap(),
        vec![c],
        "only c is still to decide"
    );
    assert_eq!(
        contacts.pending_kinds(2, None, 10).await.unwrap().len(),
        2,
        "a better detector decides again"
    );
}
