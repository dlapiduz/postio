//! A write the sync path repeats compiles nothing the second time.
//!
//! Compiling a statement is most of what a small one costs this engine. A
//! first sync of three thousand messages, sampled with `eu-stack`, spent
//! more than half of its busy samples inside `Connection::prepare` -- for
//! `set_body`, for `update` and the address lookups under it -- compiling
//! SQL it had compiled for the previous message. The engine keeps a
//! per-connection cache of compiled programs; this is what holds the storage
//! layer to using it.
//!
//! Counted, not timed, for the reason
//! [`postio_storage::test_support::counting`] gives.

use chrono::{TimeZone, Utc};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

/// Files a message with an author and a recipient, updates it, and gives it
/// a body: the header and body writes a first sync makes for each message.
async fn write_one(repository: &MessageRepository<'_>, message: &mut Message) {
    repository
        .create(message)
        .await
        .expect("the message is filed");
    message.subject = Some("the interlock report, revised".to_owned());
    repository
        .update(message)
        .await
        .expect("the message is updated");
    let body = StoredBody {
        text: Some("The readings from every station.".to_owned()),
        ..StoredBody::default()
    };
    repository
        .set_body(message.id, &body, BodyState::Full)
        .await
        .expect("the body is stored");
}

#[tokio::test]
async fn a_repeated_message_write_compiles_nothing() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    let account = test_support::account(&connection).await;
    let mailbox = test_support::mailbox(&connection, &account, "INBOX").await;
    let repository = MessageRepository::new(&connection);

    let message_numbered = |n: i64| {
        let mut message = Message::new(account.id, mailbox.id, Utc.timestamp_opt(n, 0).unwrap());
        message.subject = Some(format!("interlock report {n}"));
        message.from = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        message.to = vec![EmailAddress::new(None::<String>, "postio@example.net")];
        message
    };

    // The first message compiles what it needs; that is the cache filling.
    write_one(&repository, &mut message_numbered(1)).await;

    let counts = counted_async(|| async {
        for n in 2..=6 {
            write_one(&repository, &mut message_numbered(n)).await;
        }
    })
    .await;
    eprintln!("  five more messages: {counts:?}");

    assert_eq!(
        counts.compiles, 0,
        "five messages written after the first compiled {} statements; every \
         statement in that path has been compiled once already, and compiling \
         is most of what a small statement costs",
        counts.compiles,
    );
}
