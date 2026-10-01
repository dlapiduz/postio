//! A batch of server messages costs the same whatever the store holds.
//!
//! `upsert_batch` once read every local copy awaiting a server identity
//! (#942) into a map, once per batch, through a predicate no index covers: a
//! full scan of `messages`, decrypted page by page on an encrypted store. A
//! first sync pays that per batch, so each batch cost more than the last and
//! a live sync sat at 100% CPU for over an hour.
//!
//! The rows a batch reads are the instrument: with `N` local copies waiting
//! for a server identity the old read returned all `N`, and the fix reads
//! none of them for a message the server can place by uid.

use chrono::{TimeZone, Utc};
use postio_model::{MailboxId, Message, RemoteId, RfcMessageId, Uid, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support::{self, counting};

fn at(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// A message the server has named: a uid, and an identity.
fn from_server(mailbox: MailboxId, account: postio_model::AccountId, n: u32) -> Message {
    let mut message = Message::new(account, mailbox, at(i64::from(n)));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<server-{n}@example.com>")));
    message.server.uid = Some(Uid::new(n));
    message.server.uid_validity = Some(UidValidity::new(7));
    message.server.remote_id = Some(RemoteId::new(format!("7:{n}")));
    message
}

/// A local copy written before the server had named it.
fn local_copy(mailbox: MailboxId, account: postio_model::AccountId, n: u32) -> Message {
    let mut message = Message::new(account, mailbox, at(i64::from(n)));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<local-{n}@example.com>")));
    message
}

/// The rows one batch of fresh server messages reads, in a store already
/// holding `waiting` local copies.
async fn rows_read_by_a_batch(waiting: u32) -> usize {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    for n in 0..waiting {
        messages
            .create(&mut local_copy(inbox, account.id, n))
            .await
            .expect("a local copy");
    }
    counting::install(&connection);

    let mut batch: Vec<Message> = (1..=25)
        .map(|n| from_server(inbox, account.id, 10_000 + n))
        .collect();
    let counts = counting::counted_async(|| async {
        let report = messages.upsert_batch(&mut batch).await.expect("the batch");
        assert_eq!(report.inserted, 25);
    })
    .await;
    counts.rows
}

#[tokio::test]
async fn a_batch_of_uid_messages_does_not_read_the_stores_waiting_copies() {
    let small = rows_read_by_a_batch(20).await;
    let large = rows_read_by_a_batch(400).await;
    assert_eq!(
        small, large,
        "the rows a batch reads grew with the store ({small} against {large}): \
         something walks every local copy per batch"
    );
}

/// The statement `upsert_batch` runs for the identity fallback, written out so
/// the planner is asked about the SQL it sees.
const BY_IDENTITY: &str = "SELECT id FROM messages
      WHERE account_id = ?1 AND rfc_message_id = ?2 AND mailbox_id = ?3
        AND remote_id IS NULL AND uid IS NULL AND deleted_locally = 0";

#[tokio::test]
async fn the_identity_fallback_seeks_through_an_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    assert!(
        counting::scans(&connection, BY_IDENTITY).await.is_empty(),
        "a full scan of messages"
    );
    // Which index the planner prefers is its call, and with no statistics it
    // is not stable: either the message-id index or the mailbox's
    // (mailbox_id, remote_id) one seeks straight to the rows, which is the
    // property. What it must never do is read the table.
    let plan = test_support::plan(&connection, BY_IDENTITY).await;
    assert!(
        plan.starts_with("SEARCH messages USING INDEX"),
        "the fallback is not an index seek: {plan}"
    );
}

#[tokio::test]
async fn a_fetched_copy_adopts_the_local_row_whatever_the_case_of_its_message_id() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, sent) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut ours = local_copy(sent, account.id, 1);
    ours.rfc_message_id = Some(RfcMessageId::new("<Mixed.Case.Id@Example.COM>"));
    messages.create(&mut ours).await.expect("the local copy");

    let mut fetched = from_server(sent, account.id, 2);
    fetched.rfc_message_id = Some(RfcMessageId::new("<mixed.case.id@example.com>"));
    let report = messages
        .upsert_batch(&mut vec![fetched])
        .await
        .expect("the resync");

    assert_eq!(report.inserted, 0, "a second copy was inserted");
    assert_eq!(report.updated, 1);
}
