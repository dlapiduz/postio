//! A draft's own rows are found through an index (#381).
//!
//! `recipients` and `attachments` each hold two disjoint populations: rows
//! that belong to a stored message, and rows that belong to a draft, with a
//! `CHECK` making it exactly one of the two. On any real store the second
//! population is a rounding error — measured on the reference store, 378,819
//! recipient rows of which **zero** had a `draft_id`.
//!
//! So these two indexes used to be partial, and this file used to assert that
//! they were: `... WHERE draft_id IS NOT NULL` turned `idx_recipients_draft`
//! from 6 MB — 3.9% of a 163 MB database — into an empty index, and the
//! planner still read through it because `draft_id = ?` proves the query
//! cannot want the rows the predicate left out.
//!
//! **Turso's planner does not read through a partial index at all**, so that
//! predicate now buys nothing and costs the read: the draft load scans
//! `recipients` instead. The predicates are gone and the indexes are whole.
//! `docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md` has
//! the measurement and the rule it leaves; the engine behaviour itself is
//! pinned in `tests/turso_capabilities.rs`, so if a later release starts
//! reading through them, something fails and says so.
//!
//! What this file asserts is therefore the half that survived and is the half
//! that matters to a person: a draft's recipients and attachments are reached
//! by a seek and not by a scan. The rows come back either way — that is what
//! makes the plan the thing to assert on.

use postio_storage::Connection;
use postio_storage::bind;


async fn migrated() -> (postio_storage::Store, postio_storage::Checkout) {
    let store = postio_storage::test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    (store, connection)
}

/// The `CREATE INDEX` statement the database is actually carrying.
async fn definition(connection: &Connection, index: &str) -> String {
    postio_storage::sql::one(&*connection, 
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",bind![index],
            |row| postio_storage::sql::RowExt::col::<String>(row, 0)).await
        .unwrap_or_else(|error| panic!("no index named {index}: {error}"))
}

/// How SQLite says it would answer `query`.
async fn plan(connection: &Connection, query: &str) -> String {
    postio_storage::test_support::plan(connection, query).await
}

#[tokio::test]
async fn the_draft_indexes_carry_no_predicate_the_planner_would_refuse() {
    let (_store, connection) = migrated().await;

    for index in ["idx_recipients_draft", "idx_attachments_draft"] {
        let definition = definition(&connection, index).await;
        assert!(
            !definition.to_ascii_uppercase().contains(" WHERE "),
            "{index} is partial, and this engine will not read through a \
             partial index -- so the draft load that this index exists for \
             scans the table, and the index is pure write cost. If the \
             predicate is back to save space, see \
             docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md:\n  {definition}"
        );
    }
}

#[tokio::test]
async fn a_drafts_own_rows_are_still_found_through_them() {
    let (_store, connection) = migrated().await;

    // The two reads `DraftRepository::fill` makes, verbatim. Asserting on the
    // plan rather than on the rows, because the rows come back either way --
    // by a full table scan, which is the regression this is guarding against.
    let recipients = plan(
        &connection,
        "SELECT r.kind, r.name, a.address FROM recipients r
           JOIN addresses a ON a.id = r.address_id
          WHERE r.draft_id = 1 ORDER BY r.kind, r.position, r.id",
    ).await;
    assert!(
        recipients.contains("idx_recipients_draft"),
        "a draft's recipients no longer reach their index:\n{recipients}"
    );

    let attachments = plan(
        &connection,
        "SELECT id, filename FROM attachments WHERE draft_id = 1 ORDER BY position, id",
    ).await;
    assert!(
        attachments.contains("idx_attachments_draft"),
        "a draft's attachments no longer reach their index:\n{attachments}"
    );
}
