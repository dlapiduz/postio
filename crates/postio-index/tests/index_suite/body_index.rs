//! The body index is a column of `messages` — #407, the schema half of #379.
//!
//! `search_documents` is an ordinary table that existed only to feed an
//! external-content `messages_fts`, and it held a **full copy of every
//! message's body text**. That was free while nothing was indexed (#327) and
//! is the entire text corpus duplicated inside the database now that
//! everything is (ADR 0016). It also breaks the schema's own rule, which
//! `PRODUCT.md` §6 repeats: the database holds the blob key and the metadata
//! needed to list and search, not the bodies.
//!
//! #407 moved bodies to `message_bodies_fts`, a contentless FTS5 table keyed
//! by `message_id`. This engine has no virtual tables: its FTS is an *index
//! method* over a real column (`CREATE INDEX … USING fts`), so the body text
//! is `messages.body_search` and `messages_body_fts` is the index over it.
//! Same property, one fewer table — the body lives in exactly one place, and
//! there is nothing beside it to keep in step.
//!
//! Two consequences the tests below turn on. Re-indexing is an `UPDATE` rather
//! than a delete and an insert, so "the old text is still matchable" is now a
//! bug that would take a bad `WHERE` to write. And `body_search` is **the
//! empty string, not `NULL`**, for a message with no text: the column is also
//! the record that a message was indexed, and "tried, nothing there" spelled
//! as `NULL` is what #500's infinite loop was made of.

use postio_index::index::{ensure_schema, index_body, messages_missing_body_text};
use postio_model::{BodyState, Message};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn a_message(connection: &Connection, subject: &str) -> i64 {
    let (account, mailbox) = test_support::account_with_inbox(connection).await;
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::Full;
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    message.id.get()
}

/// The messages whose indexed body text matches `query`.
///
/// Folded on the way in, because the engine's tokenizer does not fold
/// diacritics and `index_body` folded the text it stored. A query that skips
/// the fold matches nothing an accented body contains, which is the whole
/// reason `postio_model::fold` exists.
async fn body_hits(connection: &Connection, query: &str) -> Vec<i64> {
    postio_storage::sql::all(
        connection,
        "SELECT message_id FROM message_search_bodies          WHERE fts_match(body_search, ?1) ORDER BY message_id",
        [postio_model::fold::fold(query)],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("query")
}

/// How many messages carry indexed body text.
///
/// A message that has been through `index_body` (or `set_body`) has a row in
/// `message_search_bodies` whether or not it had any words; one that has not
/// has none.
async fn indexed_bodies(connection: &Connection) -> i64 {
    postio_storage::sql::scalar(
        connection,
        "SELECT count(*) FROM message_search_bodies",
        (),
    )
    .await
    .expect("count")
}

#[tokio::test]
async fn a_body_is_searchable_in_a_table_of_its_own() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    index_body(&connection, id, Some("the difference engine is finished"))
        .await
        .expect("index");

    assert_eq!(body_hits(&connection, "difference").await, vec![id]);
    assert!(body_hits(&connection, "unrelated").await.is_empty());
}

#[tokio::test]
async fn re_indexing_replaces_the_body_rather_than_adding_a_second_row() {
    // A body is re-indexed whenever it is refetched. Under a contentless FTS5
    // table that meant a delete and an insert, and getting it wrong left the
    // old text matchable for ever -- search returning a message for words it
    // no longer contains. It is one `UPDATE` of one column now, so the failure
    // this guards is much harder to write; it stays because the assertion is
    // about the search result and not about how the write is spelled.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    index_body(&connection, id, Some("the first draft"))
        .await
        .expect("index");
    index_body(&connection, id, Some("the second draft"))
        .await
        .expect("re-index");

    assert_eq!(body_hits(&connection, "second").await, vec![id]);
    assert!(
        body_hits(&connection, "first").await.is_empty(),
        "the previous text is still matchable"
    );
    assert_eq!(indexed_bodies(&connection).await, 1);
}

#[tokio::test]
async fn clearing_a_body_removes_it_from_the_index() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;
    index_body(&connection, id, Some("something"))
        .await
        .expect("index");

    index_body(&connection, id, None).await.expect("clear");

    assert!(body_hits(&connection, "something").await.is_empty());
    // One row, matching nothing. This used to assert zero rows — "an empty
    // body is no row" — and that reading is what #500's infinite loop was
    // made of: with no row, the maintenance pass cannot tell "tried, empty"
    // from "never tried" and asks about the message on every pass for ever.
    // The row *is* the record that indexing happened.
    assert_eq!(indexed_bodies(&connection).await, 1);
}

#[tokio::test]
async fn deleting_a_message_takes_its_body_with_it() {
    // Under the contentless table this needed a trigger of its own: there was
    // no content row to cascade from, so without one the text of every deleted
    // message stayed in the index for ever, matchable and growing exactly the
    // way this issue exists to stop. The text is a column of the message now,
    // so it goes when the row goes -- which is the better answer, and worth an
    // assertion precisely because it is the kind of thing a later schema change
    // could quietly undo.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;
    index_body(&connection, id, Some("the difference engine"))
        .await
        .expect("index");

    MessageRepository::new(&connection)
        .delete(&[postio_model::MessageId::new(id)])
        .await
        .expect("delete");

    assert!(body_hits(&connection, "difference").await.is_empty());
    assert_eq!(indexed_bodies(&connection).await, 0);
}

#[tokio::test]
async fn a_body_indexed_before_this_table_existed_is_found_by_the_maintenance_pass() {
    // A message whose body is local and not in this index -- which is every
    // message in every store that indexed its bodies before this table
    // existed. The pass that catches one up is driven by
    // `messages_missing_body_text`, so it has to ask about *this* table;
    // asking the column that used to hold bodies would have answered
    // "nothing to do" for all of them and left the new index empty for ever.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Quarterly report").await;

    assert_eq!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates"),
        vec![id],
        "a body that is local and not indexed here is exactly the work"
    );

    index_body(&connection, id, Some("already indexed"))
        .await
        .expect("catch up");

    assert!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates")
            .is_empty(),
        "and once it is here, the pass leaves it alone"
    );
}

#[tokio::test]
async fn a_message_whose_text_is_local_but_whose_payloads_are_not_is_still_indexed() {
    // ADR 0017 split `full` in two: `partial` means the words are here and
    // the attachments are not, and it is the settled state of every
    // text-backfilled message carrying one. Asking only for `full` skips all
    // of them -- which on the reference account is 15% of the mailbox, and
    // the search corpus is exactly what the text axis exists to complete.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Statement attached").await;
    connection
        .execute(
            "UPDATE messages SET body_state = 'partial' WHERE id = ?1",
            [id],
        )
        .await
        .expect("the fixture writes");

    assert_eq!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates"),
        vec![id]
    );
}

#[tokio::test]
async fn a_message_whose_body_is_still_on_the_server_is_not_a_candidate() {
    // The other side of it. Indexing a message whose text has not arrived
    // would make search answer for a corpus it does not have.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Not fetched yet").await;
    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [id],
        )
        .await
        .expect("the fixture writes");

    assert!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates")
            .is_empty()
    );
}

#[tokio::test]
async fn a_message_with_nothing_to_index_leaves_the_candidate_set() {
    // The infinite loop of #500. An attachment-only message — a DMARC
    // report, a calendar invite, an image with no words — has a local body
    // and no indexable text. Indexing it must still *record that it was
    // tried*: if it writes nothing, `messages_missing_body_text` returns the
    // same message on every pass, and a store with more than one batch of
    // them turns the catch-up loop into a core-burning spin that never ends —
    // observed live, 35 minutes of CPU against a 912-body backlog.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let id = a_message(&connection, "Report attached").await;

    assert_eq!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates"),
        vec![id],
        "local body, never indexed: exactly the work"
    );

    index_body(&connection, id, None)
        .await
        .expect("index nothing");

    assert!(
        messages_missing_body_text(&connection, 10)
            .await
            .expect("candidates")
            .is_empty(),
        "tried and found empty is not the same state as never tried, \
         or the maintenance pass asks about this message for ever"
    );
}
