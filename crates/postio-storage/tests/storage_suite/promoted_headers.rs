//! What a message's promoted headers say (spec 007, research R8):
//! `messages.unsubscribe_offered` and `messages.automation`, known at filing
//! on an incremental sync or from the body's headers later, and NULL until
//! then.

use chrono::{TimeDelta, TimeZone, Utc};
use postio_model::promoted::{PRECEDENCE_BULK, PromotedHeaders};
use postio_model::{Message, MessageId, RemoteId, Uid, UidValidity};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

const NEWSLETTER: PromotedHeaders = PromotedHeaders {
    unsubscribe_offered: true,
    automation: PRECEDENCE_BULK,
};

/// The message at `uid` in `mailbox`, as a header sync would bring it.
fn fetched(
    (account, mailbox): (postio_model::AccountId, postio_model::MailboxId),
    uid: u32,
    promoted: Option<PromotedHeaders>,
) -> Message {
    let mut message = Message::new(
        account,
        mailbox,
        Utc.with_ymd_and_hms(2026, 9, 26, 9, 0, 0).unwrap() + TimeDelta::seconds(i64::from(uid)),
    );
    message.server.uid = Some(Uid::new(uid));
    message.server.uid_validity = Some(UidValidity::new(7));
    message.server.remote_id = Some(RemoteId::new(format!("7:{uid}")));
    message.subject = Some(format!("Issue {uid}"));
    message.promoted = promoted;
    message
}

async fn stored(connection: &Connection, id: MessageId) -> Option<PromotedHeaders> {
    MessageRepository::new(connection)
        .get(id)
        .await
        .expect("a read")
        .expect("the message")
        .promoted
}

#[tokio::test]
async fn what_a_fetch_knew_is_stored_and_a_fetch_that_did_not_ask_keeps_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let here = (account.id, inbox);
    let messages = MessageRepository::new(&connection);

    let mut batch = vec![fetched(here, 1, Some(NEWSLETTER)), fetched(here, 2, None)];
    messages.upsert_batch(&mut batch).await.expect("filed");
    assert_eq!(stored(&connection, batch[0].id).await, Some(NEWSLETTER));
    assert_eq!(
        stored(&connection, batch[1].id).await,
        None,
        "a fetch that did not ask says nothing, rather than 'none of the three'"
    );

    // The next pass over the folder is a first-sync-shaped fetch that does
    // not ask: what was known stays known.
    let mut again = vec![fetched(here, 1, None), fetched(here, 2, Some(NEWSLETTER))];
    messages.upsert_batch(&mut again).await.expect("refiled");
    assert_eq!(stored(&connection, batch[0].id).await, Some(NEWSLETTER));
    assert_eq!(stored(&connection, batch[1].id).await, Some(NEWSLETTER));
}

/// The instrument counts reads (`counting`'s docs); the writes are the
/// insert and the update the batch already issued, with two more
/// parameters each. What this proves is that knowing the facts reads
/// nothing more, per message or per batch.
#[tokio::test]
async fn the_facts_ride_on_the_insert_and_cost_no_statement_of_their_own() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let here = (account.id, inbox);
    let messages = MessageRepository::new(&connection);
    // Warm, and the same shapes in both runs.
    let mut warm = vec![fetched(here, 100, Some(NEWSLETTER))];
    messages.upsert_batch(&mut warm).await.expect("warm");

    let mut known: Vec<Message> = (1..=5)
        .map(|uid| fetched(here, uid, Some(NEWSLETTER)))
        .collect();
    let with = counted_async(|| async {
        messages.upsert_batch(&mut known).await.expect("filed");
    })
    .await;
    let mut unknown: Vec<Message> = (11..=15).map(|uid| fetched(here, uid, None)).collect();
    let without = counted_async(|| async {
        messages.upsert_batch(&mut unknown).await.expect("filed");
    })
    .await;
    assert_eq!(
        stored(&connection, known[0].id).await,
        Some(NEWSLETTER),
        "and what was known was filed"
    );
    assert_eq!(
        with.statements, without.statements,
        "filing five messages whose promoted headers are known cost {} statements \
         against {} unknown",
        with.statements, without.statements
    );
    let mut again: Vec<Message> = (1..=5).map(|uid| fetched(here, uid, None)).collect();
    let updated = counted_async(|| async {
        messages.upsert_batch(&mut again).await.expect("refiled");
    })
    .await;
    let mut again_known: Vec<Message> = (11..=15)
        .map(|uid| fetched(here, uid, Some(NEWSLETTER)))
        .collect();
    let updated_known = counted_async(|| async {
        messages
            .upsert_batch(&mut again_known)
            .await
            .expect("refiled");
    })
    .await;
    assert_eq!(updated.statements, updated_known.statements);
}

#[tokio::test]
async fn the_body_s_headers_say_what_a_first_sync_did_not_ask() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut batch = vec![fetched((account.id, inbox), 1, None)];
    messages.upsert_batch(&mut batch).await.expect("filed");

    let counts = counted_async(|| async {
        assert!(
            messages
                .set_promoted(batch[0].id, NEWSLETTER)
                .await
                .expect("written")
        );
    })
    .await;
    // One write, and no read: the instrument counts reads, and a write
    // that read the row first would show here.
    assert_eq!((counts.statements, counts.rows), (0, 0), "{counts:?}");
    assert_eq!(stored(&connection, batch[0].id).await, Some(NEWSLETTER));
    assert!(
        !messages
            .set_promoted(MessageId::new(9_999), NEWSLETTER)
            .await
            .expect("no such message"),
        "a message that is gone is no error"
    );
}
