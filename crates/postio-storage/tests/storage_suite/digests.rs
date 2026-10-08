//! Focus's digests, as the store keeps them (spec 007, data-model.md
//! "`digest_deliveries` and `digest_holds`"): mail a rule holds until its
//! time comes, then delivered together as one row.

use chrono::{DateTime, TimeZone, Utc};
use postio_model::{DeliveryId, Message, MessageId};
use postio_storage::Connection;
use postio_storage::repository::{DigestRepository, MessageRepository};
use postio_storage::test_support;

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
}

/// `count` messages in one inbox.
async fn messages(connection: &Connection, count: u32) -> Vec<MessageId> {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let mut ids = Vec::new();
    for hour in 0..count {
        ids.push(
            MessageRepository::new(connection)
                .create(&mut Message::new(account.id, inbox, at(23, hour)))
                .await
                .expect("a message"),
        );
    }
    ids
}

/// Where `message`'s hold stands: absent, waiting (no delivery yet), or in
/// a delivery.
async fn hold_of(connection: &Connection, message: MessageId) -> Option<Option<DeliveryId>> {
    postio_storage::sql::first(
        connection,
        "SELECT delivery_id FROM digest_holds WHERE message_id = ?1",
        [message.get()],
        |row| Ok(postio_storage::sql::RowExt::col::<Option<i64>>(row, 0)?.map(DeliveryId::new)),
    )
    .await
    .expect("a read")
}

#[tokio::test]
async fn a_rule_s_held_mail_is_delivered_together_and_nothing_is_delivered_empty() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 3).await;
    let digests = DigestRepository::new(&connection);

    assert!(
        digests
            .hold(ids[0], "Newsletters", at(23, 9))
            .await
            .expect("held")
    );
    assert!(
        digests
            .hold(ids[1], "Newsletters", at(24, 9))
            .await
            .expect("held")
    );
    assert!(
        digests
            .hold(ids[2], "Receipts", at(24, 10))
            .await
            .expect("held")
    );
    assert!(
        !digests
            .hold(ids[0], "Receipts", at(25, 9))
            .await
            .expect("held once"),
        "a message is held by the first rule that matched it, once"
    );
    assert_eq!(hold_of(&connection, ids[0]).await, Some(None), "waiting");

    // Sunday 09:00: the weekly rule comes due and takes everything it holds.
    let delivery = digests
        .deliver("Newsletters", at(27, 9), at(27, 9))
        .await
        .expect("a delivery")
        .expect("it held something");
    assert_eq!(hold_of(&connection, ids[0]).await, Some(Some(delivery)));
    assert_eq!(hold_of(&connection, ids[1]).await, Some(Some(delivery)));
    assert_eq!(
        hold_of(&connection, ids[2]).await,
        Some(None),
        "another rule's mail waits for its own"
    );

    // Due again with nothing new held: no delivery, and no row.
    assert_eq!(
        digests
            .deliver("Newsletters", at(28, 9), at(28, 9))
            .await
            .expect("a read"),
        None
    );
    let deliveries: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM digest_deliveries", ())
            .await
            .expect("a count");
    assert_eq!(deliveries, 1, "an empty digest is not created");
}

#[tokio::test]
async fn a_release_or_an_archived_delivery_lets_the_mail_go() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 3).await;
    let digests = DigestRepository::new(&connection);
    for id in &ids {
        digests
            .hold(*id, "Newsletters", at(23, 9))
            .await
            .expect("held");
    }

    // Stopping a sender releases one message; removing the rule, the rest.
    assert!(digests.release(ids[0]).await.expect("released"));
    assert_eq!(hold_of(&connection, ids[0]).await, None);
    assert!(!digests.release(ids[0]).await.expect("already gone"));
    assert_eq!(
        digests.release_rule("Newsletters").await.expect("released"),
        2
    );
    assert_eq!(hold_of(&connection, ids[1]).await, None);

    // `⇧A` archives a delivery; its holds stay, marked by the delivery.
    digests
        .hold(ids[2], "Receipts", at(24, 9))
        .await
        .expect("held");
    let delivery = digests
        .deliver("Receipts", at(27, 9), at(27, 10))
        .await
        .expect("a delivery")
        .expect("it held something");
    assert!(
        digests
            .archive_delivery(delivery, at(27, 11))
            .await
            .expect("archived")
    );
    let archived: Option<i64> = postio_storage::sql::one(
        &connection,
        "SELECT archived_at FROM digest_deliveries WHERE id = ?1",
        [delivery.get()],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("a read");
    assert!(archived.is_some());
}

#[tokio::test]
async fn letting_go_of_what_waits_leaves_delivered_mail_in_its_digest() {
    // FR-122: a message whose body shows a question or a to-do is let go of
    // while it waits for its digest. Once delivered it is in a digest row
    // the person can already see, and stays there.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 2).await;
    let digests = DigestRepository::new(&connection);
    digests
        .hold(ids[0], "Newsletters", at(23, 9))
        .await
        .expect("held");
    let delivery = digests
        .deliver("Newsletters", at(27, 9), at(27, 9))
        .await
        .expect("delivered")
        .expect("a delivery");
    digests
        .hold(ids[1], "Newsletters", at(28, 9))
        .await
        .expect("held");

    assert!(
        digests.release_waiting(ids[1]).await.expect("released"),
        "waiting: let go"
    );
    assert!(
        !digests.release_waiting(ids[0]).await.expect("a write"),
        "delivered: kept"
    );
    assert_eq!(hold_of(&connection, ids[1]).await, None);
    assert_eq!(hold_of(&connection, ids[0]).await, Some(Some(delivery)));
}

#[tokio::test]
async fn a_rule_waits_since_its_oldest_undelivered_hold_in_one_seek() {
    // The due timer's question (T135): since when has a rule held mail
    // nobody has been given yet? Its next delivery is the first due time
    // after that; with nothing waiting there is none (US10 scenario 6).
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 4).await;
    let digests = DigestRepository::new(&connection);
    assert_eq!(
        digests.waiting_since("Newsletters").await.expect("a read"),
        None,
        "nothing held, nothing waits"
    );

    digests
        .hold(ids[0], "Newsletters", at(20, 9))
        .await
        .expect("held");
    digests
        .deliver("Newsletters", at(21, 9), at(21, 9))
        .await
        .expect("delivered");
    digests
        .hold(ids[2], "Newsletters", at(24, 8))
        .await
        .expect("held");
    digests
        .hold(ids[1], "Newsletters", at(23, 7))
        .await
        .expect("held");
    digests
        .hold(ids[3], "Other", at(19, 1))
        .await
        .expect("another rule's");

    let mut since = None;
    let counts = postio_storage::test_support::counting::counted_async(|| async {
        since = digests.waiting_since("Newsletters").await.expect("a read");
    })
    .await;
    assert_eq!(since, Some(at(23, 7)), "the oldest of what waits");
    assert_eq!(counts.statements, 1, "{counts:?}");
    let plan = test_support::plan(&connection, DigestRepository::explain_waiting_since()).await;
    assert!(
        !plan
            .lines()
            .any(|step| step.trim_start().starts_with("SCAN")),
        "{plan}"
    );
}

#[tokio::test]
async fn a_delivery_keeps_its_summary_and_its_row_reads_it_in_the_same_statement() {
    // Spec 007 T154, data-model.md: `summary` and `summary_written_at` on the
    // delivery. The digest row's line comes from it, so the open deliveries
    // read it with no statement more; what still waits for a summary is one
    // seek on the open deliveries.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 2).await;
    let digests = DigestRepository::new(&connection);
    for (id, rule) in ids.iter().zip(["Newsletters", "School"]) {
        digests.hold(*id, rule, at(23, 9)).await.expect("held");
    }
    let first = digests
        .deliver("Newsletters", at(24, 9), at(24, 9))
        .await
        .expect("delivered")
        .expect("a delivery");
    let second = digests
        .deliver("School", at(24, 10), at(24, 10))
        .await
        .expect("delivered")
        .expect("a delivery");

    let mut waiting = Vec::new();
    let counts = postio_storage::test_support::counting::counted_async(|| async {
        waiting = digests.unsummarised(8).await.expect("a read");
    })
    .await;
    assert_eq!(waiting, [second, first], "newest first");
    assert_eq!(counts.statements, 1, "{counts:?}");

    // One `UPDATE`; the counting seam counts reads, not writes.
    assert!(
        digests
            .set_summary(first, r#"{"statements":[]}"#, at(24, 11))
            .await
            .expect("written")
    );
    assert_eq!(
        digests.unsummarised(8).await.expect("a read"),
        [second],
        "a written summary, even an empty one, is not asked for again"
    );
    assert_eq!(
        digests.summary(first).await.expect("a read").as_deref(),
        Some(r#"{"statements":[]}"#)
    );
    assert_eq!(digests.summary(second).await.expect("a read"), None);

    let mut open = Vec::new();
    let counts = postio_storage::test_support::counting::counted_async(|| async {
        open = digests.open_deliveries().await.expect("a read");
    })
    .await;
    assert_eq!(counts.statements, 1, "{counts:?}");
    let summaries: Vec<_> = open
        .iter()
        .map(|delivery| (delivery.id, delivery.summary.clone()))
        .collect();
    assert_eq!(
        summaries,
        [
            (second, None),
            (first, Some(r#"{"statements":[]}"#.to_owned()))
        ]
    );
}

#[tokio::test]
async fn each_rule_counts_what_it_holds_now_in_one_statement() {
    // The `g d` list (T139): each rule with how many messages it holds now,
    // waiting for its next delivery -- not what it already delivered. One
    // statement for every rule, each count a seek on idx_digest_holds_rule.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 5).await;
    let digests = DigestRepository::new(&connection);
    for message in &ids[..2] {
        digests
            .hold(*message, "Newsletters", at(1, 9))
            .await
            .expect("held");
    }
    digests
        .deliver("Newsletters", at(2, 9), at(2, 9))
        .await
        .expect("delivered");
    digests
        .hold(ids[2], "Newsletters", at(3, 9))
        .await
        .expect("held");
    for message in &ids[3..] {
        digests
            .hold(*message, "Receipts", at(3, 9))
            .await
            .expect("held");
    }

    let rules = ["Newsletters", "Receipts", "Nobody"];
    let _ = digests.waiting(&rules).await.expect("warm");
    let mut waiting = Vec::new();
    let counts = counted_async(|| async {
        waiting = digests.waiting(&rules).await.expect("the counts");
    })
    .await;
    assert_eq!(
        waiting,
        [1, 2, 0],
        "what each holds now, in the rules' order"
    );
    assert_eq!(counts.statements, 1, "{counts:?}");
    let walks: Vec<String> = scans(&connection, &DigestRepository::explain_waiting(3))
        .await
        .into_iter()
        .filter(|step| step != "SCAN CONSTANT ROW")
        .collect();
    assert!(walks.is_empty(), "{walks:?}");
}

#[tokio::test]
async fn which_messages_a_digest_holds_is_one_statement_by_key() {
    // US10 scenario 7 (T140): a search result says where held mail waits.
    // Of the messages asked about: which a rule holds, waiting or
    // delivered; not one whose digest was archived; not one never held.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages(&connection, 4).await;
    let digests = DigestRepository::new(&connection);
    digests
        .hold(ids[0], "Newsletters", at(1, 9))
        .await
        .expect("held");
    let delivery = digests
        .deliver("Newsletters", at(2, 9), at(2, 9))
        .await
        .expect("delivered")
        .expect("a delivery");
    digests
        .hold(ids[1], "Newsletters", at(3, 9))
        .await
        .expect("held");
    digests
        .hold(ids[2], "Receipts", at(3, 9))
        .await
        .expect("held");
    digests
        .deliver("Receipts", at(4, 9), at(4, 9))
        .await
        .expect("delivered")
        .expect("a delivery");
    let receipts = DeliveryId::new(delivery.get() + 1);
    digests
        .archive_delivery(receipts, at(5, 9))
        .await
        .expect("archived");

    let _ = digests.held(&ids).await.expect("warm");
    let mut held = Vec::new();
    let counts = counted_async(|| async {
        held = digests.held(&ids).await.expect("a read");
    })
    .await;
    assert_eq!(
        held,
        [
            (ids[0], "Newsletters".to_owned(), true),
            (ids[1], "Newsletters".to_owned(), false),
        ],
        "delivered, waiting; the archived one and the never-held one are not"
    );
    assert_eq!(counts.statements, 1, "{counts:?}");
    assert!(
        scans(&connection, &DigestRepository::explain_held(4))
            .await
            .is_empty(),
        "each message by its key"
    );
}

/// `senders.len()` messages in one inbox, each from the sender at its index.
async fn messages_from(connection: &Connection, senders: &[(&str, &str)]) -> Vec<MessageId> {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let mut ids = Vec::new();
    for (hour, (name, address)) in senders.iter().enumerate() {
        let mut message = Message::new(account.id, inbox, at(23, hour as u32));
        message.from = vec![postio_model::EmailAddress::new(Some(*name), *address)];
        ids.push(
            MessageRepository::new(connection)
                .create(&mut message)
                .await
                .expect("a message"),
        );
    }
    ids
}

#[tokio::test]
async fn stopping_a_sender_releases_only_what_the_rule_still_waits_on() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages_from(
        &connection,
        &[
            ("Ada", "ada@example.com"),
            ("Ada", "ada@example.com"),
            ("Grace", "grace@example.com"),
            ("Ada", "ada@example.com"),
        ],
    )
    .await;
    let digests = DigestRepository::new(&connection);
    for id in &ids[..3] {
        digests
            .hold(*id, "Newsletters", at(23, 9))
            .await
            .expect("held");
    }
    digests
        .hold(ids[3], "Receipts", at(23, 9))
        .await
        .expect("held");
    // Ada's first message is already in a delivery; it stays there.
    let delivered = digests
        .deliver("Newsletters", at(24, 9), at(24, 9))
        .await
        .expect("delivered")
        .expect("a delivery");
    assert!(
        !digests
            .hold(ids[1], "Newsletters", at(25, 9))
            .await
            .expect("a write"),
        "a delivered message is not held again"
    );
    assert_eq!(
        digests.delivery_messages(delivered).await.expect("a read"),
        ids[..3].to_vec()
    );

    let waiting = messages_from(
        &connection,
        &[("Ada", "ada@example.com"), ("Grace", "grace@example.com")],
    )
    .await;
    for id in &waiting {
        digests
            .hold(*id, "Newsletters", at(25, 9))
            .await
            .expect("held");
    }
    let released = digests
        .release_sender("Newsletters", "ada@example.com")
        .await
        .expect("released");
    assert_eq!(
        released,
        vec![waiting[0]],
        "delivered mail stays in its digest"
    );
    assert_eq!(hold_of(&connection, waiting[0]).await, None);
    assert_eq!(
        hold_of(&connection, waiting[1]).await,
        Some(None),
        "another sender's mail waits on"
    );
    assert_eq!(
        hold_of(&connection, ids[3]).await,
        Some(None),
        "another rule's mail waits on"
    );
    assert!(
        digests
            .release_sender("Newsletters", "ada@example.com")
            .await
            .expect("released")
            .is_empty(),
        "nothing left to release"
    );
}

#[tokio::test]
async fn a_delivery_names_its_senders_most_first_and_can_be_archived_and_reopened() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let ids = messages_from(
        &connection,
        &[
            ("Grace", "grace@example.com"),
            ("Ada", "ada@example.com"),
            ("Ada", "ada@example.com"),
        ],
    )
    .await;
    let digests = DigestRepository::new(&connection);
    for id in &ids {
        digests
            .hold(*id, "Newsletters", at(23, 9))
            .await
            .expect("held");
    }
    let delivery = digests
        .deliver("Newsletters", at(24, 9), at(24, 9))
        .await
        .expect("delivered")
        .expect("a delivery");

    assert!(digests.senders_of(&[]).await.expect("a read").is_empty());
    let senders = digests.senders_of(&[delivery]).await.expect("a read");
    assert_eq!(senders.len(), 2);
    assert_eq!(
        (senders[0].0, senders[0].1.normalized(), senders[0].2),
        (delivery, "ada@example.com".to_owned(), 2)
    );
    assert_eq!(senders[0].1.name.as_deref(), Some("Ada"));
    assert_eq!(
        (senders[1].1.normalized(), senders[1].2),
        ("grace@example.com".to_owned(), 1)
    );

    assert!(
        !digests.reopen_delivery(delivery).await.expect("a write"),
        "not archived yet"
    );
    assert!(
        digests
            .archive_delivery(delivery, at(24, 10))
            .await
            .expect("archived")
    );
    assert!(
        digests.reopen_delivery(delivery).await.expect("reopened"),
        "undo brings the row back"
    );
    assert!(!digests.reopen_delivery(delivery).await.expect("a write"));
}
