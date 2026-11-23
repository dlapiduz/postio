//! Asserts a draft's recipients and attachments are reached by a seek, not a scan. This engine's
//! planner does not read through a partial index, so the indexes are whole
//! (docs/notes/2026-09-12-a-partial-index-the-planner-will-not-read.md).

use postio_storage::Connection;
use postio_storage::bind;

async fn migrated() -> (postio_storage::Store, postio_storage::Checkout) {
    let store = postio_storage::test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    (store, connection)
}

async fn definition(connection: &Connection, index: &str) -> String {
    postio_storage::sql::one(
        connection,
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
        bind![index],
        |row| postio_storage::sql::RowExt::col::<String>(row, 0),
    )
    .await
    .unwrap_or_else(|error| panic!("no index named {index}: {error}"))
}

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

    // Asserts the plan, not the rows: they come back either way, by a full scan.
    let recipients = plan(
        &connection,
        "SELECT r.kind, r.name, a.address FROM recipients r
           JOIN addresses a ON a.id = r.address_id
          WHERE r.draft_id = 1 ORDER BY r.kind, r.position, r.id",
    )
    .await;
    assert!(
        recipients.contains("idx_recipients_draft"),
        "a draft's recipients no longer reach their index:\n{recipients}"
    );

    let attachments = plan(
        &connection,
        "SELECT id, filename FROM attachments WHERE draft_id = 1 ORDER BY position, id",
    )
    .await;
    assert!(
        attachments.contains("idx_attachments_draft"),
        "a draft's attachments no longer reach their index:\n{attachments}"
    );
}
