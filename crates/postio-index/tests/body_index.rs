//! The body index gets a table of its own — #407, the schema half of #379.
//!
//! `search_documents` is an ordinary SQLite table that exists only to feed the
//! external-content `messages_fts`, and it held a **full copy of every
//! message's body text**. That was free while nothing was indexed (#327) and
//! is the entire text corpus duplicated inside the database now that
//! everything is (ADR 0016). It also breaks migration 0001's own rule, which
//! `PRODUCT.md` §6 repeats: SQLite holds the blob key and the metadata needed
//! to list and search, not the bodies.
//!
//! So bodies move to `message_bodies_fts` — `content=''`,
//! `contentless_delete=1`, rowid = `message_id`, no content table underneath
//! and nothing to keep in step with one.
//!
//! `search_documents.body` and `messages_fts.body` are gone as of #408, which
//! moved the executor onto this table. The tests below are what says the two
//! halves add up: the body is searchable here, and nowhere else.

use postio_index::index::{ensure_schema, index_body, messages_missing_body_text};
use postio_model::{BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

fn a_message(connection: &Connection, subject: &str) -> i64 {
    let (account, mailbox) = test_support::account_with_inbox(connection);
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::Full;
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create");
    message.id.get()
}

fn body_hits(connection: &Connection, query: &str) -> Vec<i64> {
    let mut statement = connection
        .prepare(
            "SELECT rowid FROM message_bodies_fts
              WHERE message_bodies_fts MATCH ?1 ORDER BY rowid",
        )
        .expect("prepare");
    statement
        .query_map([query], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows")
}

fn rows_in(connection: &Connection, table: &str) -> i64 {
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .expect("count")
}

#[test]
fn a_body_is_searchable_in_a_table_of_its_own() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Quarterly report");

    index_body(&connection, id, Some("the difference engine is finished")).expect("index");

    assert_eq!(body_hits(&connection, "difference"), vec![id]);
    assert!(body_hits(&connection, "unrelated").is_empty());
}

#[test]
fn re_indexing_replaces_the_body_rather_than_adding_a_second_row() {
    // A body is re-indexed whenever it is refetched, and a contentless table
    // has no `UPDATE`: a row is deleted and written again. Getting that wrong
    // leaves the old text matchable for ever, which reads as search returning
    // a message for words it no longer contains.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Quarterly report");

    index_body(&connection, id, Some("the first draft")).expect("index");
    index_body(&connection, id, Some("the second draft")).expect("re-index");

    assert_eq!(body_hits(&connection, "second"), vec![id]);
    assert!(
        body_hits(&connection, "first").is_empty(),
        "the previous text is still matchable"
    );
    assert_eq!(rows_in(&connection, "message_bodies_fts"), 1);
}

#[test]
fn clearing_a_body_removes_it_from_the_index() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Quarterly report");
    index_body(&connection, id, Some("something")).expect("index");

    index_body(&connection, id, None).expect("clear");

    assert!(body_hits(&connection, "something").is_empty());
    assert_eq!(
        rows_in(&connection, "message_bodies_fts"),
        0,
        "an empty body is no row, not a row of nothing — the whole point is \
         that this table holds only what there is to match"
    );
}

#[test]
fn deleting_a_message_takes_its_body_with_it() {
    // `search_documents` cascades from `messages`, and its delete trigger
    // takes `messages_fts` with it. A contentless table has no content row to
    // cascade, so without a trigger of its own the text of every deleted
    // message stays in the index for ever — matchable, and growing exactly
    // the way this issue exists to stop.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Quarterly report");
    index_body(&connection, id, Some("the difference engine")).expect("index");

    MessageRepository::new(&connection)
        .delete(&[postio_model::MessageId::new(id)])
        .expect("delete");

    assert!(body_hits(&connection, "difference").is_empty());
    assert_eq!(rows_in(&connection, "message_bodies_fts"), 0);
}

#[test]
fn a_body_indexed_before_this_table_existed_is_found_by_the_maintenance_pass() {
    // A message whose body is local and not in this index -- which is every
    // message in every store that indexed its bodies before this table
    // existed. The pass that catches one up is driven by
    // `messages_missing_body_text`, so it has to ask about *this* table;
    // asking the column that used to hold bodies would have answered
    // "nothing to do" for all of them and left the new index empty for ever.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Quarterly report");

    assert_eq!(
        messages_missing_body_text(&connection, 10).expect("candidates"),
        vec![id],
        "a body that is local and not indexed here is exactly the work"
    );

    index_body(&connection, id, Some("already indexed")).expect("catch up");

    assert!(
        messages_missing_body_text(&connection, 10)
            .expect("candidates")
            .is_empty(),
        "and once it is here, the pass leaves it alone"
    );
}

#[test]
fn a_message_whose_text_is_local_but_whose_payloads_are_not_is_still_indexed() {
    // ADR 0017 split `full` in two: `partial` means the words are here and
    // the attachments are not, and it is the settled state of every
    // text-backfilled message carrying one. Asking only for `full` skips all
    // of them -- which on the reference account is 15% of the mailbox, and
    // the search corpus is exactly what the text axis exists to complete.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Statement attached");
    connection
        .execute(
            "UPDATE messages SET body_state = 'partial' WHERE id = ?1",
            [id],
        )
        .expect("the fixture writes");

    assert_eq!(
        messages_missing_body_text(&connection, 10).expect("candidates"),
        vec![id]
    );
}

#[test]
fn a_message_whose_body_is_still_on_the_server_is_not_a_candidate() {
    // The other side of it. Indexing a message whose text has not arrived
    // would make search answer for a corpus it does not have.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let id = a_message(&connection, "Not fetched yet");
    connection
        .execute(
            "UPDATE messages SET body_state = 'headers_only' WHERE id = ?1",
            [id],
        )
        .expect("the fixture writes");

    assert!(
        messages_missing_body_text(&connection, 10)
            .expect("candidates")
            .is_empty()
    );
}
