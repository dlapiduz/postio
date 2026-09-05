//! The draft indexes index drafts, and nothing else (#381).
//!
//! `recipients` and `attachments` each hold two disjoint populations: rows
//! that belong to a stored message, and rows that belong to a draft, with a
//! `CHECK` making it exactly one of the two. On any real store the second
//! population is a rounding error — measured on the reference store, 378,819
//! recipient rows of which **zero** had a `draft_id` — so an index on
//! `draft_id` that is not partial is an entry per message recipient, all of
//! them `NULL`, sorted and stored for nobody. `idx_recipients_draft` alone
//! measured 6 MB, 3.9% of a 163 MB database.
//!
//! Two things have to be true together, which is why they are asserted
//! together: the indexes are partial, *and* a draft's own recipients and
//! attachments are still found through them. A partial index whose `WHERE`
//! does not match the query's is an index the planner silently declines to
//! use, and the query still returns the right rows — by scanning the table.
//! So the shape assertion alone cannot fail usefully, and the rows-come-back
//! assertion alone cannot either.

use rusqlite::Connection;

use postio_storage::migrate;

fn migrated() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory sqlite");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys");
    migrate(&mut connection).expect("migrate");
    connection
}

/// The `CREATE INDEX` statement the database is actually carrying.
fn definition(connection: &Connection, index: &str) -> String {
    connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
            [index],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|error| panic!("no index named {index}: {error}"))
}

/// How SQLite says it would answer `query`.
fn plan(connection: &Connection, query: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
        .expect("a query plan");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("plan rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("plan rows");
    rows.join("\n")
}

#[test]
fn the_draft_indexes_cover_only_the_rows_that_have_a_draft() {
    let connection = migrated();

    for index in ["idx_recipients_draft", "idx_attachments_draft"] {
        let definition = definition(&connection, index);
        assert!(
            definition.contains("draft_id IS NOT NULL"),
            "{index} indexes every row in the table, and on a real store \
             almost every one of them has a NULL draft_id -- an entry per \
             message recipient, stored and sorted for nobody:\n  {definition}"
        );
    }
}

#[test]
fn a_drafts_own_rows_are_still_found_through_them() {
    let connection = migrated();

    // The two reads `DraftRepository::fill` makes, verbatim: a partial index
    // is only used when the planner can prove the query cannot want the rows
    // it left out, and `draft_id = ?` is that proof. Asserting on the plan
    // rather than on the rows, because the rows come back either way -- by a
    // full table scan, which is the regression this is guarding against.
    let recipients = plan(
        &connection,
        "SELECT r.kind, r.name, a.address FROM recipients r
           JOIN addresses a ON a.id = r.address_id
          WHERE r.draft_id = 1 ORDER BY r.kind, r.position, r.id",
    );
    assert!(
        recipients.contains("idx_recipients_draft"),
        "a draft's recipients no longer reach their index:\n{recipients}"
    );

    let attachments = plan(
        &connection,
        "SELECT id, filename FROM attachments WHERE draft_id = 1 ORDER BY position, id",
    );
    assert!(
        attachments.contains("idx_attachments_draft"),
        "a draft's attachments no longer reach their index:\n{attachments}"
    );
}
