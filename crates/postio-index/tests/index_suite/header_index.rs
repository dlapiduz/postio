//! `message_headers`: the table `header:` matches against (ADR 0025 Q2).
//!
//! The rows are derived data. What these tests hold to account is the part
//! that makes them safe to derive — that re-indexing replaces rather than
//! accumulates, that a deleted message takes its rows with it, that the two
//! caps in ADR 0025 Q3 actually bind, and that a message the pass has
//! finished with stops being offered to it. That last one is not a tidiness
//! point: a candidate query that does not shrink is what ran a core flat out
//! for as long as the application was open in #500.

use postio_index::index::{
    HEADER_ROWS_PER_MESSAGE, ensure_schema, index_headers, messages_missing_header_rows,
};
use postio_model::headers::{Headers, VALUE_LIMIT};
use postio_model::{BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

/// A message with a stored header block, which is what the catch-up pass
/// looks for. The block's *content* does not matter here — every one of
/// these tests indexes explicitly — only that `body_headers` is not NULL.
fn a_message_with_a_stored_block(connection: &Connection) -> Message {
    let (account, mailbox) = test_support::account_with_inbox(connection);
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    let messages = MessageRepository::new(connection);
    messages.create(&mut message).expect("create");
    messages
        .set_body(
            message.id,
            &postio_storage::repository::StoredBody {
                text: Some("the body".to_string()),
                html: None,
                headers: Some("X-Mailer: mutt 1.5.24".to_string()),
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .expect("store a block");
    message
}

fn rows(connection: &Connection, message_id: i64) -> Vec<(String, String, i64)> {
    let mut statement = connection
        .prepare(
            "SELECT name, value, ordinal FROM message_headers
              WHERE message_id = ?1 ORDER BY ordinal",
        )
        .expect("prepare");
    statement
        .query_map([message_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query")
        .collect::<rusqlite::Result<_>>()
        .expect("rows")
}

#[test]
fn a_block_becomes_one_row_per_occurrence_in_wire_order() {
    // `Received` chains are the reason `Headers` keeps duplicates at all, and
    // ADR 0025 Q6 says any occurrence matching is a match — which needs every
    // occurrence to be a row of its own.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    let headers: Headers = [
        ("Received", "from a.example.com"),
        ("X-Mailer", "Mutt 1.5.24"),
        ("Received", "from b.example.com"),
    ]
    .into_iter()
    .collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    assert_eq!(
        rows(&connection, message.id.get()),
        vec![
            ("received".to_string(), "from a.example.com".to_string(), 0),
            ("x-mailer".to_string(), "Mutt 1.5.24".to_string(), 1),
            ("received".to_string(), "from b.example.com".to_string(), 2),
        ],
        "names lowercased, values as normalized, ordinals in wire order"
    );
}

#[test]
fn a_value_past_the_cap_is_stored_exactly_as_the_matcher_would_hold_it() {
    // The correctness half of ADR 0025 Q3: the index holds a prefix and an
    // in-memory matcher holds the whole value, so they disagree about every
    // long header unless both pass through `normalize_value`. Storing the
    // raw value here would be a row `header:` could match and the matcher
    // could not, or the other way round.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    let long = "a".repeat(VALUE_LIMIT * 2);
    let headers: Headers = [("DKIM-Signature", long.as_str())].into_iter().collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    let stored = rows(&connection, message.id.get());
    assert_eq!(stored.len(), 1);
    assert_eq!(
        stored[0].1,
        postio_model::headers::normalize_value(&long),
        "the stored value has to be the normalized one, byte for byte"
    );
    assert_eq!(stored[0].1.len(), VALUE_LIMIT);
}

#[test]
fn a_message_is_capped_at_the_rows_per_message_limit() {
    // The other cap in ADR 0025 Q3. A twenty-hop mailing-list message with a
    // signature at each hop must not decide the size of the index.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    let headers: Headers = (0..HEADER_ROWS_PER_MESSAGE * 2)
        .map(|nth| {
            (
                "Received".to_string(),
                format!("from hop-{nth}.example.com"),
            )
        })
        .collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    let stored = rows(&connection, message.id.get());
    assert_eq!(stored.len(), HEADER_ROWS_PER_MESSAGE);
    assert_eq!(
        stored[0].1, "from hop-0.example.com",
        "the cap keeps the *first* fields in wire order, not an arbitrary set"
    );
}

#[test]
fn re_indexing_a_message_replaces_its_rows_rather_than_adding_to_them() {
    // The pass is resumable and a version bump refills the whole table, so
    // indexing the same message twice is the ordinary case, not an error one.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    let first: Headers = [("X-Mailer", "mutt")].into_iter().collect();
    index_headers(&connection, message.id.get(), &first).expect("index");
    let second: Headers = [("X-Mailer", "notmuch")].into_iter().collect();
    index_headers(&connection, message.id.get(), &second).expect("re-index");

    assert_eq!(
        rows(&connection, message.id.get()),
        vec![("x-mailer".to_string(), "notmuch".to_string(), 0)],
        "the stale value must not survive beside the new one"
    );
}

#[test]
fn deleting_a_message_takes_its_header_rows_with_it() {
    // `ON DELETE CASCADE` rather than a hand-written trigger, which is what
    // `message_bodies_fts` needed only because a contentless FTS table has no
    // content row to cascade from.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);
    let headers: Headers = [("X-Mailer", "mutt")].into_iter().collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    MessageRepository::new(&connection)
        .delete(&[message.id])
        .expect("delete");

    assert!(
        rows(&connection, message.id.get()).is_empty(),
        "a deleted message's headers stay matchable for ever otherwise"
    );
}

#[test]
fn the_catch_up_query_offers_a_stored_block_that_has_no_rows_yet() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    assert_eq!(
        messages_missing_header_rows(&connection, 10).expect("candidates"),
        vec![message.id.get()],
        "a message with a block and no rows is exactly what the pass is for"
    );

    let headers: Headers = [("X-Mailer", "mutt")].into_iter().collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    assert!(
        messages_missing_header_rows(&connection, 10)
            .expect("candidates")
            .is_empty(),
        "indexing a message has to remove it from the answer"
    );
}

#[test]
fn a_message_with_no_stored_block_is_not_a_candidate() {
    // The pass is local-only: a message whose block has never been stored is
    // `repair_header_blocks`'s or the backfill lane's, and offering it here
    // would put work on a queue that can do nothing about it.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create");

    assert!(
        messages_missing_header_rows(&connection, 10)
            .expect("candidates")
            .is_empty()
    );
}

#[test]
fn a_block_that_yields_no_fields_still_stops_being_a_candidate() {
    // The #500 shape, in this table's costume. A block that parses to nothing
    // — malformed mail, or a block that was cut to nothing — writes no
    // ordinary rows, so `NOT EXISTS` would keep offering it for ever and the
    // pass would re-read the same batch every lap.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);

    index_headers(&connection, message.id.get(), &Headers::new()).expect("index nothing");

    assert!(
        messages_missing_header_rows(&connection, 10)
            .expect("candidates")
            .is_empty(),
        "\"tried, there was nothing there\" has to be recorded as done"
    );
}

#[test]
fn bumping_the_headers_half_refills_it_and_leaves_the_bodies_alone() {
    // The mechanism ADR 0025 Q3 leans on: the caps stay revisable because a
    // version bump drops this table and the catch-up pass refills it from
    // `body_headers`, with no network. Refilling the *body* index means
    // decompressing every body on disk, so it must not be dropped for this.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);
    postio_index::index::index_body(&connection, message.id.get(), Some("the difference engine"))
        .expect("index a body");
    let headers: Headers = [("X-Mailer", "mutt")].into_iter().collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    connection
        .execute(
            "UPDATE search_schema SET version = version - 1 WHERE half = 'headers'",
            [],
        )
        .expect("the version regresses");
    ensure_schema(&connection).expect("the rebuild applies");

    assert!(
        rows(&connection, message.id.get()).is_empty(),
        "the headers half is dropped on a version mismatch"
    );
    assert_eq!(
        messages_missing_header_rows(&connection, 10).expect("candidates"),
        vec![message.id.get()],
        "and the message is offered to the pass again, which is the refill"
    );

    let bodies: i64 = connection
        .query_row(
            "SELECT count(*) FROM message_bodies_fts WHERE rowid = ?1",
            [message.id.get()],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(bodies, 1, "the body index survived the headers rebuild");
}

#[test]
fn a_metadata_upgrade_never_drops_the_header_rows() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let message = a_message_with_a_stored_block(&connection);
    let headers: Headers = [("X-Mailer", "mutt")].into_iter().collect();
    index_headers(&connection, message.id.get(), &headers).expect("index");

    connection
        .execute(
            "UPDATE search_schema SET version = version - 1 WHERE half = 'metadata'",
            [],
        )
        .expect("the version regresses");
    ensure_schema(&connection).expect("the rebuild applies");

    assert_eq!(
        rows(&connection, message.id.get()).len(),
        1,
        "a metadata rebuild must not cost the header index its rows"
    );
}
