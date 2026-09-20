//! Threading one message costs the same statements however long its thread.
//!
//! `commit_batch` threads **one message at a time** —
//! `for message in &written { threading.thread(message).await? }` — so every
//! statement `thread` issues is paid per message, per batch, for the whole of
//! a first sync. `LoadedIndex::load` then walks `cue.links()` and issues one
//! `SELECT` per link, sequentially:
//!
//! ```text
//! for link in cue.links() {
//!     sql::first(connection, "SELECT thread_id FROM thread_links ...")
//! }
//! ```
//!
//! `cue.links()` is the `References`/`In-Reply-To` chain, which grows with
//! the conversation. So a message deep in a long thread costs a round trip
//! per ancestor — and Sent and Archive are exactly where threads are long,
//! while a fresh inbox is where they are short.
//!
//! A real account showed commits at 49–157 ms per message against fetches of
//! 4–610 ms per *batch*, with the slowest folder being Sent. Three other
//! explanations were measured and ruled out first: commit cost is flat in
//! store size (`examples/commit_cost.rs`), the body index no longer touches
//! header writes (`examples/fts_write_cost.rs`), and the link lookup does
//! seek rather than scan (`threading_lookup_cost.rs`).
//!
//! Counted, not timed, for the reason [`postio_storage::test_support::counting`]
//! gives: statements are the same number on any machine.

use chrono::{TimeZone, Utc};
use postio_model::{Message, RfcMessageId};
use postio_storage::repository::{MessageRepository, ThreadingRepository};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, install};

/// How long a chain to measure. Forty is an ordinary working thread, not a
/// pathological one.
const DEEP: usize = 40;

/// Files a message carrying `references` ancestors and reports what threading
/// it cost in statements.
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

    // Printed either way: when this fails the two numbers are the finding,
    // and when it passes they are the budget it now holds.
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
