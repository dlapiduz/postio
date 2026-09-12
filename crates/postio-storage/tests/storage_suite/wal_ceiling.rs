//! The write-ahead log does not grow without bound (#1175).
//!
//! The live install reached a **676 MB** WAL against an 868 MB database, and
//! every launch paid for it: the WAL index is rebuilt before the first row
//! can be read, and `Phase::Store` sits in front of the first frame.
//!
//! # The mechanism changed, the requirement did not
//!
//! #1175 set `PRAGMA journal_size_limit`, a ceiling the engine enforced for
//! itself at every checkpoint. This engine has no such pragma -- checked, in
//! `the_engine_has_no_journal_size_limit` below, so the day it grows one this
//! file says so.
//!
//! What it has is `wal_checkpoint`, so the ceiling becomes a sweep:
//! [`Store::truncate_log`], called by the application's housekeeping worker.
//!
//! **That is weaker than what it replaces**, and the difference is worth
//! stating: a limit the engine enforces applies to every checkpoint of every
//! session, and a sweep applies when something calls it. A session that never
//! reaches housekeeping never truncates. The failure mode is the one #1175
//! was about, so it is written down here rather than assumed away.

use postio_storage::test_support;

/// Enough writes to make a WAL worth measuring.
const ROWS: usize = 2_000;

/// The value each row carries. Large enough that `ROWS` of them is megabytes
/// rather than kilobytes, so the assertion is not reading noise.
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
    assert_eq!(
        before, grew,
        "truncate_log should report the size it found"
    );

    let after = std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0);
    assert_eq!(
        after, 0,
        "the log is still {after} bytes after a TRUNCATE checkpoint; the \
         mechanism #1175 depends on is not working"
    );
}

#[tokio::test]
async fn the_store_still_reads_after_its_log_is_truncated() {
    // The obvious way to get a small WAL is to lose the writes in it. This is
    // what says the checkpoint moved them into the database rather than
    // dropping them.
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
    // #1175's own mechanism, asked for directly. When this starts answering,
    // the sweep above can go back to being a setting -- which is the stronger
    // of the two, because the engine applies it to every checkpoint rather
    // than only to the ones housekeeping reaches.
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
