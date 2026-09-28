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
