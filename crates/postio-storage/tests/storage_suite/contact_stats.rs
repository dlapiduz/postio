//! What a card reads about a person: tiles, a twelve-month strip, the newest
//! messages, and the Contacts tabs' counts (specs/011-focus-contacts S5).
//!
//! All of it is kept as mail is recorded and read in a fixed number of
//! statements over a person's addresses, never over their mail, which the
//! budgets below hold at two sizes of store.

use chrono::{DateTime, TimeZone, Utc};

use postio_model::{
    Account, ContactId, ContactKind, EmailAddress, MailboxId, Message, MessageId, RECENT, month_key,
};
use postio_storage::repository::{ContactRepository, MessageRepository};
use postio_storage::test_support::counting::{counted_async, unbounded};
use postio_storage::{Checkout, test_support};

fn at(year: i32, month: u32, day: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, 12, 0, 0).unwrap()
}

async fn setup() -> (postio_storage::Store, Checkout, Account, MailboxId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

/// A message `from` `to` on `when`, stored and recorded as sync does.
async fn mail(
    connection: &Checkout,
    account: &Account,
    inbox: MailboxId,
    when: DateTime<Utc>,
    from: &str,
    to: &str,
    read: bool,
) -> MessageId {
    let mut message = Message::new(account.id, inbox, when);
    message.from = vec![EmailAddress::new(None::<String>, from)];
    message.to = vec![EmailAddress::new(None::<String>, to)];
    if read {
        message.flags.insert(postio_model::Flag::Seen);
    }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("store");
    ContactRepository::new(connection)
        .record_message(&message, std::slice::from_ref(&account.address))
        .await
        .expect("record");
    message.id
}

async fn person(connection: &Checkout, email: &str) -> ContactId {
    ContactRepository::new(connection)
        .by_address(email)
        .await
        .unwrap()
        .expect("a person")
        .id
}

#[tokio::test]
async fn tiles_months_and_the_ring_follow_the_mail_as_it_is_recorded() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let a = mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 5),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    let b = mail(
        &connection,
        &account,
        inbox,
        at(2026, 1, 20),
        "ada@example.org",
        &me,
        false,
    )
    .await;
    let c = mail(
        &connection,
        &account,
        inbox,
        at(2026, 3, 2),
        &me,
        "ada@example.org",
        true,
    )
    .await;
    let ada = person(&connection, "ada@example.org").await;

    let stats = ContactRepository::new(&connection)
        .stats(ada, month_key(at(2026, 1, 1)))
        .await
        .unwrap();

    assert_eq!(stats.messages, 3);
    assert_eq!(stats.from_you, 1, "the one the user sent");
    assert_eq!(stats.opened, 1, "of the two from her, one was read");
    assert_eq!(stats.last, Some(at(2026, 3, 2)));
    assert_eq!(
        stats.months,
        [
            (month_key(at(2026, 1, 1)), 2),
            (month_key(at(2026, 3, 1)), 1)
        ]
    );
    let ring: Vec<(MessageId, bool)> = stats
        .recent
        .iter()
        .map(|r| (r.message, r.outgoing))
        .collect();
    assert_eq!(
        ring,
        [(c, true), (b, false), (a, false)],
        "newest first, out for the one sent"
    );
}

#[tokio::test]
async fn reading_and_unreading_a_message_moves_the_opened_count() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let m = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "ada@example.org",
        &me,
        false,
    )
    .await;
    let ada = person(&connection, "ada@example.org").await;
    let contacts = ContactRepository::new(&connection);
    let opened = || async { contacts.stats(ada, 0).await.unwrap().opened };
    assert_eq!(opened().await, 0);

    postio_storage::sql::execute(
        &connection,
        "UPDATE messages SET seen = 1 WHERE id = ?1",
        [m.get()],
    )
    .await
    .unwrap();
    assert_eq!(
        opened().await,
        1,
        "a trigger on the flip, not a count over her mail"
    );
    postio_storage::sql::execute(
        &connection,
        "UPDATE messages SET seen = 0 WHERE id = ?1",
        [m.get()],
    )
    .await
    .unwrap();
    assert_eq!(opened().await, 0);
}

#[tokio::test]
async fn the_ring_keeps_the_newest_fifty_and_a_person_sums_their_addresses() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    for day in 1..=28 {
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 4, day),
            "ada@work.example",
            &me,
            true,
        )
        .await;
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 5, day),
            "ada@home.example",
            &me,
            true,
        )
        .await;
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 6, day),
            "ada@home.example",
            &me,
            true,
        )
        .await;
    }
    let contacts = ContactRepository::new(&connection);
    let work = person(&connection, "ada@work.example").await;
    let home = person(&connection, "ada@home.example").await;
    contacts.join(work, &[home], "Ada", None).await.unwrap();

    let stats = contacts.stats(work, 0).await.unwrap();

    assert_eq!(stats.messages, 84, "every address counts, joined or not");
    assert_eq!(stats.recent.len(), RECENT, "bounded, however much mail");
    assert!(
        stats.recent.iter().all(|r| r.at >= at(2026, 5, 1)),
        "the newest fifty across both rings are June's 28 and May's newest 22"
    );
    assert_eq!(stats.months.len(), 3);
    let ring_rows: i64 = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM address_recent r JOIN addresses a ON a.id = r.address_id
          WHERE a.address_normalized = 'ada@home.example'",
        (),
    )
    .await
    .unwrap();
    assert_eq!(ring_rows, 50, "each address keeps fifty and no more");
}

#[tokio::test]
async fn the_tabs_counts_are_kept_by_triggers_as_people_come_change_and_go() {
    let (_database, connection, _account, _inbox) = setup().await;
    let contacts = ContactRepository::new(&connection);
    let count = || async {
        contacts
            .kind_counts()
            .await
            .unwrap()
            .into_iter()
            .map(|(kind, total, saved)| (kind.as_str().to_owned(), total, saved))
            .collect::<Vec<_>>()
    };
    assert!(count().await.is_empty());

    let ada = contacts
        .create(
            Some("Ada"),
            &[EmailAddress::new(None::<String>, "ada@example.org")],
        )
        .await
        .unwrap();
    contacts
        .person_for_email(&EmailAddress::new(None::<String>, "bot@example.org"))
        .await
        .unwrap();
    assert_eq!(count().await, [("person".to_owned(), 2, 0)]);

    postio_storage::sql::execute(
        &connection,
        "UPDATE contacts SET saved_at = 5 WHERE id = ?1",
        [ada.get()],
    )
    .await
    .unwrap();
    let bot = person(&connection, "bot@example.org").await;
    contacts
        .set_detected_kind(bot, ContactKind::Automated, 1)
        .await
        .unwrap();
    assert_eq!(
        count().await,
        [("automated".to_owned(), 1, 0), ("person".to_owned(), 1, 1)]
    );

    contacts.delete(ada).await.unwrap();
    assert_eq!(
        count().await,
        [("automated".to_owned(), 1, 0)],
        "a deleted person is nobody's tab"
    );
    contacts
        .restore(ada, postio_model::ContactState::Live)
        .await
        .unwrap();
    assert_eq!(
        count().await,
        [("automated".to_owned(), 1, 0), ("person".to_owned(), 1, 1)]
    );
}

#[tokio::test]
async fn old_mail_is_folded_into_the_months_the_ring_and_the_opened_count() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    // Stored before anything recorded it, as in a migrated store.
    for (day, read) in [(3u32, true), (9, false), (21, true)] {
        let mut message = Message::new(account.id, inbox, at(2026, 2, day));
        message.from = vec![EmailAddress::new(None::<String>, "ada@example.org")];
        message.to = vec![EmailAddress::new(None::<String>, me.clone())];
        if read {
            message.flags.insert(postio_model::Flag::Seen);
        }
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
    }
    test_support::mark_contacts_history(&connection).await;
    let contacts = ContactRepository::new(&connection);
    let ada = contacts
        .person_for_email(&EmailAddress::new(None::<String>, "ada@example.org"))
        .await
        .unwrap();

    let stats = contacts.stats(ada, 0).await.unwrap();

    assert_eq!(stats.messages, 3);
    assert_eq!(stats.opened, 2);
    assert_eq!(stats.months, [(month_key(at(2026, 2, 1)), 3)]);
    assert_eq!(stats.recent.len(), 3);
}

#[tokio::test]
async fn a_card_costs_the_statements_and_rows_of_the_addresses_not_of_the_mail() {
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "quiet@example.org",
        &me,
        true,
    )
    .await;
    for day in 0..120u32 {
        mail(
            &connection,
            &account,
            inbox,
            at(2026, 1, 1) + chrono::Duration::days(i64::from(day)),
            "busy@example.org",
            &me,
            day % 2 == 0,
        )
        .await;
    }
    let contacts = ContactRepository::new(&connection);
    let (quiet, busy) = (
        person(&connection, "quiet@example.org").await,
        person(&connection, "busy@example.org").await,
    );
    let _ = contacts.stats(busy, 0).await.unwrap();

    let few = counted_async(|| async {
        contacts
            .stats(quiet, month_key(at(2025, 6, 1)))
            .await
            .unwrap();
    })
    .await;
    let many = counted_async(|| async {
        contacts
            .stats(busy, month_key(at(2025, 6, 1)))
            .await
            .unwrap();
    })
    .await;

    assert_eq!(
        few.statements, many.statements,
        "120 messages cost what 1 does"
    );
    assert!(
        many.statements <= 4,
        "a card is at most four reads: {}",
        many.statements
    );
    assert!(
        many.rows <= 1 + 1 + 12 + RECENT + 2,
        "rows follow the addresses and the strip and the ring, not the mail: {}",
        many.rows
    );
    for sql in ContactRepository::explain_stats(1) {
        assert_eq!(
            unbounded(
                &connection,
                &sql,
                &[
                    "contact_sightings",
                    "address_months",
                    "address_recent",
                    "addresses"
                ]
            )
            .await,
            Vec::<String>::new(),
            "no scan and no walk of a whole scope: {sql}"
        );
    }
}

#[tokio::test]
async fn what_is_open_between_you_is_read_from_the_ring_whoever_sent_it() {
    use postio_model::OpenSide;
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let asked = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    let dismissed = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 2),
        "ada@example.org",
        &me,
        true,
    )
    .await;
    let sent = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 3),
        &me,
        "ada@example.org",
        true,
    )
    .await;
    let answered = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 4),
        &me,
        "ada@example.org",
        true,
    )
    .await;
    for (id, text, dismissed_at) in [
        (asked, "Approve the Q3 numbers by Friday", None),
        (dismissed, "Old question", Some(5)),
    ] {
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO markers (message_id, kind, source, excerpt, dismissed_at)
             VALUES (?1, 'question', 'detector', ?2, ?3)",
            postio_storage::sql::bind![id.get(), text, dismissed_at],
        )
        .await
        .unwrap();
    }
    postio_storage::sql::execute(
        &connection,
        "UPDATE messages SET subject = 'Q4 headcount numbers' WHERE id = ?1",
        [sent.get()],
    )
    .await
    .unwrap();
    for (id, settled) in [(sent, None), (answered, Some(9))] {
        postio_storage::sql::execute(
            &connection,
            "INSERT INTO reminders (thread_id, anchor_message_id, set_at, due_at, settled_at)
             VALUES (1, ?1, 1, 2, ?2)",
            postio_storage::sql::bind![id.get(), settled],
        )
        .await
        .unwrap();
    }
    let contacts = ContactRepository::new(&connection);
    let ada = person(&connection, "ada@example.org").await;

    let items = contacts.open_items(ada, 2).await.unwrap();

    assert_eq!(
        items
            .iter()
            .map(|i| (i.side, i.text.as_str(), i.message))
            .collect::<Vec<_>>(),
        [
            (
                OpenSide::WaitingOnYou,
                "Approve the Q3 numbers by Friday",
                asked
            ),
            (OpenSide::WaitingOnThem, "Q4 headcount numbers", sent),
        ],
        "one each, a dismissed marker and a settled reminder are not open"
    );
    assert_eq!(contacts.open_items(ada, 1).await.unwrap().len(), 1);
    for sql in ContactRepository::explain_open_items(1) {
        assert_eq!(
            unbounded(
                &connection,
                &sql,
                &["markers", "reminders", "address_recent", "messages"]
            )
            .await,
            Vec::<String>::new(),
            "bounded by the ring: {sql}"
        );
    }
}

#[tokio::test]
async fn where_the_default_rules_put_a_sender_is_what_they_did_with_their_newest_mail() {
    use postio_model::ContactArrival;
    let (_database, connection, account, inbox) = setup().await;
    let me = account.address.address.clone();
    let contacts = ContactRepository::new(&connection);
    let old = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 1),
        "robot@example.org",
        &me,
        true,
    )
    .await;
    let robot = person(&connection, "robot@example.org").await;
    assert_eq!(
        contacts.default_arrival(robot).await.unwrap(),
        ContactArrival::Inbox
    );

    let new = mail(
        &connection,
        &account,
        inbox,
        at(2026, 2, 2),
        "robot@example.org",
        &me,
        true,
    )
    .await;
    postio_storage::sql::execute(
        &connection,
        "INSERT INTO filter_decisions (message_id, reason, layer, decided_at)
         VALUES (?1, 'notification', 'header', 1)",
        [new.get()],
    )
    .await
    .unwrap();
    assert_eq!(
        contacts.default_arrival(robot).await.unwrap(),
        ContactArrival::Filtered,
        "the newest mail was filed away"
    );

    postio_storage::sql::execute(
        &connection,
        "UPDATE filter_decisions SET restored_at = 2 WHERE message_id = ?1",
        [new.get()],
    )
    .await
    .unwrap();
    assert_eq!(
        contacts.default_arrival(robot).await.unwrap(),
        ContactArrival::Inbox
    );

    postio_storage::sql::execute(
        &connection,
        "INSERT INTO digest_holds (message_id, rule, held_at) VALUES (?1, 'weekly', 1)",
        [new.get()],
    )
    .await
    .unwrap();
    assert_eq!(
        contacts.default_arrival(robot).await.unwrap(),
        ContactArrival::Digest
    );
    let _ = old;
    for sql in ContactRepository::explain_default_arrival(1) {
        assert_eq!(
            unbounded(
                &connection,
                &sql,
                &["address_recent", "filter_decisions", "digest_holds"]
            )
            .await,
            Vec::<String>::new(),
            "{sql}"
        );
    }
}
