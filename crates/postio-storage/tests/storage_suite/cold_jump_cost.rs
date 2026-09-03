//! The list indexes can answer the list's filters (#638).
//!
//! Every list query filters on two columns beyond the one the scope names:
//! the message is not locally deleted, and it is not still snoozed. Only rows
//! that *pass* a `WHERE` count toward an `OFFSET`, so if the index cannot
//! answer those two, SQLite has to fetch and test every row it is about to
//! discard — fifty thousand table rows to return fifty. Migration 0005 put
//! the filter columns in the four list indexes, which keeps the skip inside
//! the index: the same encrypted jump went from 207ms to 3.5ms.
//!
//! # Why this is a schema assertion and not a count
//!
//! It is the wrong shape for one, and that took measuring to establish.
//! Neither statements nor rows can see a skipped row — an `OFFSET` discards
//! rows before they are returned, so the counts are one statement and fifty
//! rows either way. VDBE steps look like the answer and are not: measured
//! across the fix they are *identical*, 82,614 either way, because the
//! program SQLite runs is the same. What changed is whether each step's fetch
//! stays in the index or reaches the table b-tree, and that shows up only in
//! page reads, which `sqlite3_db_status` reports and rusqlite does not
//! expose.
//!
//! Wall-clock does see it — that is how it was found — but a 207ms-versus-3ms
//! assertion on a shared runner is exactly the kind of budget `#100` replaced,
//! and the bench in `postio-bench` keeps that measurement anyway.
//!
//! So assert the cause directly. The invariant is not "the jump is fast", it
//! is "the columns the list filters on are in the indexes the list uses", and
//! that is a fact about the schema: deterministic, free, and false the moment
//! somebody adds a filter without widening the indexes or narrows an index
//! back to what it was.

use postio_model::MailboxRole;
use postio_storage::repository::{ListQuery, ListScope, MessageRepository};
use postio_storage::seed::seed_small;
use postio_storage::test_support;

/// The indexes the list's scopes seek and order through.
const LIST_INDEXES: &[&str] = &[
    // ListScope::Mailbox
    "idx_messages_list",
    // ListScope::Account, ::Flagged, ::Snoozed
    "idx_messages_account_list",
    // ListScope::Unified — nothing pins the leading column, so recency is it
    "idx_messages_recency",
    // ListScope::Thread
    "idx_messages_thread",
];

/// The columns `where_clause` adds to every scope's own predicate.
///
/// `deleted_locally` is in all six scopes; `snoozed_until` is in all six too,
/// as `NOT_YET_DUE` in five and inverted as `STILL_SNOOZED` in `Snoozed`.
const FILTER_COLUMNS: &[&str] = &["deleted_locally", "snoozed_until"];

/// The columns `index` is keyed on, in order.
fn columns_of(connection: &rusqlite::Connection, index: &str) -> Vec<String> {
    connection
        .prepare(&format!("PRAGMA index_info({index})"))
        .expect("the index exists")
        .query_map([], |row| row.get::<_, Option<String>>(2))
        .expect("its columns")
        .filter_map(|column| column.expect("a column row"))
        .collect()
}

/// Which of [`FILTER_COLUMNS`] `index` cannot answer.
fn missing_from(connection: &rusqlite::Connection, index: &str) -> Vec<&'static str> {
    let columns = columns_of(connection, index);
    FILTER_COLUMNS
        .iter()
        .copied()
        .filter(|filter| !columns.iter().any(|column| column == filter))
        .collect()
}

#[test]
fn a_narrow_list_index_is_reported_as_missing_its_filters() {
    // The control. This test was written after migration 0005, so on its own
    // it says only that the schema is what it is today; what makes it an
    // assertion is that the same check fails on the shape 0005 replaced.
    // Built here rather than by undoing the migration, because re-breaking
    // working code to test the test is the one thing CLAUDE.md rules out.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    connection
        .execute_batch(
            "CREATE INDEX idx_probe_narrow_list
                 ON messages (mailbox_id, received_at DESC, id DESC);",
        )
        .expect("the pre-0005 shape");

    assert_eq!(
        missing_from(&connection, "idx_probe_narrow_list"),
        FILTER_COLUMNS,
        "the check does not notice an index that carries neither filter \
         column, so it cannot be what stands between this schema and #638"
    );
    assert!(
        missing_from(&connection, "idx_messages_list").is_empty(),
        "and it must not report the migrated index, or it would fail for \
         every schema alike and mean nothing"
    );
}

#[test]
fn every_list_index_carries_the_columns_every_list_query_filters_on() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");

    for index in LIST_INDEXES {
        let columns = columns_of(&connection, index);

        assert!(
            !columns.is_empty(),
            "{index} is not in the schema at all; the list scopes name it"
        );
        for filter in FILTER_COLUMNS {
            assert!(
                columns.iter().any(|column| column == filter),
                "{index} does not carry `{filter}`, which every list query \
                 filters on. SQLite counts only rows that pass the WHERE \
                 toward an OFFSET, so it will fetch each skipped row from the \
                 table to test it: a jump halfway down a mailbox becomes one \
                 table read per row skipped, and under SQLCipher a decrypt \
                 with it. That is #638 — it measured 207ms against 3.5ms. \
                 The index holds {columns:?}."
            );
        }
    }
}

#[test]
fn a_deep_page_returns_the_same_rows_the_narrow_index_would_have() {
    // The index changed shape, so the thing to prove beyond the schema is
    // that it still answers the same question. A wider key that reordered or
    // dropped rows would be a far worse bug than the one it fixes.
    let database = test_support::temp();
    let report = seed_small(&database, 11);
    let inbox = report.mailbox(MailboxRole::Inbox).expect("an inbox").id;
    let connection = database.connection().expect("a connection");
    let messages = MessageRepository::new(&connection);
    let query = ListQuery {
        scope: ListScope::Mailbox(inbox),
        limit: 5,
        after: None,
    };

    let through_the_index: Vec<_> = messages
        .page_at(&query, 3)
        .expect("a page three in")
        .iter()
        .map(|row| row.id)
        .collect();

    // The same window, taken without the index: `+0` on a column defeats its
    // use without changing what the query means, so this is the same rows by
    // a route the optimiser cannot take.
    let unindexed: Vec<_> = connection
        .prepare(
            "SELECT id FROM messages
              WHERE mailbox_id + 0 = ?1
                AND deleted_locally = 0
                AND (snoozed_until IS NULL
                     OR snoozed_until <= strftime('%s','now') * 1000)
              ORDER BY received_at DESC, id DESC
              LIMIT 5 OFFSET 3",
        )
        .expect("prepare")
        .query_map([inbox.get()], |row| row.get::<_, i64>(0))
        .expect("rows")
        .map(|id| postio_model::ids::MessageId::new(id.expect("an id")))
        .collect();

    assert_eq!(
        through_the_index, unindexed,
        "the widened index answers a deep page differently from a scan of the \
         same predicate, so it is not the same query any more"
    );
    assert!(
        !through_the_index.is_empty(),
        "the seed must leave enough messages for a page three in to mean \
         something"
    );
}
