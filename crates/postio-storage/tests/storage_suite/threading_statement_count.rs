use chrono::{TimeZone, Utc};
use postio_model::{Message, RfcMessageId};
use postio_storage::repository::{MessageRepository, ThreadingRepository};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install};

const DEEP: usize = 40;

async fn statements_to_thread(references: usize) -> usize {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    install(&connection);
    let account = test_support::account(&connection).await;
    let mailbox = test_support::mailbox(&connection, &account, "INBOX").await;

    let mut message = Message::new(account.id, mailbox.id, Utc.timestamp_opt(0, 0).unwrap());
    message.rfc_message_id = Some(RfcMessageId::new("<subject@example.com>"));
    message.references = (0..references)
        .map(|n| RfcMessageId::new(format!("<ancestor-{n}@example.com>")))
        .collect();
    message.subject = Some("a conversation".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("the message is filed");

    let counts = counted_async(|| async {
        ThreadingRepository::new(&connection, account.id)
            .thread(&message)
            .await
            .expect("threading");
    })
    .await;
    counts.statements
}

#[tokio::test]
async fn threading_does_not_cost_a_statement_per_ancestor() {
    let shallow = statements_to_thread(0).await;
    let deep = statements_to_thread(DEEP).await;

    eprintln!("  threading statements: {shallow} with no references, {deep} with {DEEP}");

    assert!(
        deep <= shallow + 2,
        "threading a message cost {deep} statements against {shallow} for one \
         with no ancestors — about one per link in its `References` chain. \
         `LoadedIndex::load` asks the database once per ancestor, in series, \
         and `commit_batch` pays that for every message in every batch. On a \
         long thread that is the difference between a batch and a stall."
    );
}
