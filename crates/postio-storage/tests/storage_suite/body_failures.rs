//! A body the server will not give up is remembered on the store (#1918).
//!
//! The backfill used to forget it at every restart and the reader could not
//! tell it from a body still on its way. The mark is keyed to the message's
//! `MODSEQ`, so a message that changed is a message worth trying again.

use chrono::{TimeZone, Utc};
use postio_model::{BodyState, Message, MessageId, ModSeq, RemoteId, Uid, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn one_message_owing_its_body() -> (postio_storage::Store, postio_model::MailboxId, MessageId)
{
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let at = Utc.timestamp_opt(1_770_000_000, 0).single().unwrap();
    let mut message = Message::new(account.id, inbox, at);
    message.size = 2_048;
    message.server.uid = Some(Uid::new(7));
    message.server.uid_validity = Some(UidValidity::new(99));
    message.server.remote_id = Some(RemoteId::new("99:7"));
    message.server.mod_seq = Some(ModSeq::new(10));
    message.sync.body_state = BodyState::HeadersOnly;
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create");
    (database, inbox, message.id)
}

#[tokio::test]
async fn a_message_whose_body_failed_is_not_offered_again() {
    let (database, inbox, id) = one_message_owing_its_body().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);

    assert!(!messages.body_failed(id).await.expect("read"));
    messages.mark_body_failed(id).await.expect("mark");

    assert!(messages.body_failed(id).await.expect("read"));
    assert!(
        messages
            .needing_backfill(inbox, 10)
            .await
            .expect("query")
            .is_empty(),
        "a body the server refused is not asked for again by every seed"
    );
    assert_eq!(
        messages.get(id).await.unwrap().unwrap().sync.body_state,
        BodyState::HeadersOnly,
        "the headers stay, and the body is still owed to anyone who opens it"
    );
}

#[tokio::test]
async fn a_message_that_changed_is_worth_another_try() {
    let (database, inbox, id) = one_message_owing_its_body().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    messages.mark_body_failed(id).await.expect("mark");

    let mut row = messages.get(id).await.unwrap().unwrap();
    row.server.mod_seq = Some(ModSeq::new(11));
    messages
        .update(&mut row)
        .await
        .expect("the server changed it");

    assert!(!messages.body_failed(id).await.expect("read"));
    assert_eq!(
        messages.needing_backfill(inbox, 10).await.unwrap().len(),
        1,
        "a new MODSEQ clears the mark"
    );
}

#[tokio::test]
async fn opening_it_and_getting_the_body_clears_the_mark() {
    let (database, inbox, id) = one_message_owing_its_body().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    messages.mark_body_failed(id).await.expect("mark");

    messages.clear_body_failed(id).await.expect("clear");

    assert!(!messages.body_failed(id).await.expect("read"));
    assert_eq!(messages.needing_backfill(inbox, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_the_message_takes_the_mark_with_it() {
    let (database, _inbox, id) = one_message_owing_its_body().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    messages.mark_body_failed(id).await.expect("mark");
    messages.delete(&[id]).await.expect("delete");
    assert!(!messages.body_failed(id).await.expect("read"));
}
