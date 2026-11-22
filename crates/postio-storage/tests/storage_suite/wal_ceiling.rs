//! This engine has no `journal_size_limit` (#1175), so the WAL ceiling is a sweep,
//! `Store::truncate_log`
//! from housekeeping: weaker, since a session that never reaches housekeeping never truncates.

use postio_storage::test_support;

const ROWS: usize = 2_000;

const PADDING: usize = 400;

#[tokio::test]
async fn truncating_the_log_returns_the_file_to_nothing() {
    let store = test_support::temp().await;
    let connection = store.connect().await.expect("a connection");

    for n in 0..ROWS {
        connection
            .execute(
                "INSERT INTO settings (key, account_id, value, updated_at)
                 VALUES (?1, NULL, ?2, 0)",
                postio_storage::sql::bind![format!("key-{n}"), "x".repeat(PADDING)],
            )
            .await
            .expect("a write");
    }

    let wal = store
        .path()
        .expect("a file-backed store")
        .with_extension("db-wal");
    let grew = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert!(
        grew > 0,
        "the setup did not produce a write-ahead log to truncate"
    );

    let before = store.truncate_log().await.expect("truncate");
    assert_eq!(before, grew, "truncate_log should report the size it found");

    let after = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert_eq!(
        after, 0,
        "the log is still {after} bytes after a TRUNCATE checkpoint; the \
         mechanism #1175 depends on is not working"
    );
}

#[tokio::test]
async fn the_store_still_reads_after_its_log_is_truncated() {
    // Asserts the checkpoint moved the writes into the database rather than dropping them.
    let store = test_support::temp().await;
    let connection = store.connect().await.expect("a connection");
    connection
        .execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES ('survivor', NULL, 'still here', 0)",
            (),
        )
        .await
        .expect("a write");

    store.truncate_log().await.expect("truncate");

    let value: Option<String> = postio_storage::sql::first(
        &connection,
        "SELECT value FROM settings WHERE key = 'survivor'",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("a read");
    assert_eq!(
        value.as_deref(),
        Some("still here"),
        "a truncating checkpoint must move the log into the database, not \
         discard it"
    );
}

#[tokio::test]
async fn the_engine_has_no_journal_size_limit() {
    // When the engine grows `journal_size_limit`, the sweep can go back to being a setting.
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");

    let answered = connection
        .query("PRAGMA journal_size_limit", ())
        .await
        .ok()
        .map(|mut rows| async move { rows.next().await });

    let has_limit = match answered {
        Some(row) => matches!(row.await, Ok(Some(_))),
        None => false,
    };
    assert!(
        !has_limit,
        "the engine now answers `journal_size_limit`. `Store::truncate_log` \
         and the housekeeping call to it can become a per-connection setting \
         again -- see this file's own documentation."
    );
}
