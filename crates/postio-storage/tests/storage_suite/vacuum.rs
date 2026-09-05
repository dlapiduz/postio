//! Pages a deleted message frees go back to the filesystem (#381).
//!
//! SQLite's default `auto_vacuum = NONE` never shrinks a database file. A
//! page freed by a delete goes on the freelist and is reused by the next
//! insert, which is the right trade for a store whose size is roughly
//! constant — and the wrong one for this store, which is about to hold a
//! complete mailbox replica (ADR 0016) that a `UIDVALIDITY` reset can wipe
//! and re-sync in one server-side event nobody asked for. Under
//! `INCREMENTAL`, the freed pages are still on the freelist, but
//! `PRAGMA incremental_vacuum` can hand them back, which is what makes
//! deleting mail actually recover disk.
//!
//! # Why the mode has to be chosen at creation
//!
//! `auto_vacuum` is a header field, and SQLite only lets it change on a
//! database that has no tables yet — or through a full `VACUUM`, which
//! rewrites every page. So a new store gets it from [`PRAGMAS`], before the
//! schema exists, and a store that predates the setting has to be converted
//! once. Both paths are asserted here, because the second one is the one
//! that has to work on the only store that currently exists.

use postio_model::MailboxRole;
use postio_storage::seed::seed_large;
use postio_storage::{Database, test_support};

/// `auto_vacuum`: 0 NONE, 1 FULL, 2 INCREMENTAL.
const INCREMENTAL: i64 = 2;

fn auto_vacuum(connection: &rusqlite::Connection) -> i64 {
    connection
        .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
        .expect("auto_vacuum")
}

#[test]
fn a_new_store_is_created_ready_to_give_pages_back() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    assert_eq!(
        auto_vacuum(&connection),
        INCREMENTAL,
        "a store created today can never return a freed page to the \
         filesystem, and changing that later costs a full rewrite"
    );
}

#[test]
fn a_store_that_predates_the_setting_is_converted_once() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("old.db");

    // A store as it would have been created before this existed.
    let database = test_support::unconverted_store(&path);
    assert_eq!(
        auto_vacuum(&database.connection().expect("a connection")),
        0,
        "the fixture was supposed to be an unconverted store, so the \
         conversion below could not fail"
    );

    let converted = database.adopt_incremental_vacuum().expect("convert");
    assert!(converted, "an unconverted store reports that it converted");

    let connection = database.connection().expect("a connection");
    assert_eq!(auto_vacuum(&connection), INCREMENTAL);
    drop(connection);

    assert!(
        !database
            .adopt_incremental_vacuum()
            .expect("the second call is cheap"),
        "a converted store must not rewrite itself on every start"
    );
}

#[test]
fn deleting_mail_hands_the_pages_back() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("store.db");
    let key = test_support::key();
    let database = Database::open(&path, &key).expect("a store");

    let seeded = seed_large(&database, 11, 4_000);
    let inbox = seeded
        .mailbox(MailboxRole::Inbox)
        .expect("the seed has an inbox")
        .id;
    let connection = database.connection().expect("a connection");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
    let before = std::fs::metadata(&path).expect("a file").len();

    connection
        .execute("DELETE FROM messages WHERE mailbox_id = ?1", [inbox.get()])
        .expect("delete a mailbox's worth of mail");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
    drop(connection);

    let freed = database.reclaim_free_pages().expect("reclaim");
    let connection = database.connection().expect("a connection");
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint");
    let after = std::fs::metadata(&path).expect("a file").len();

    assert!(
        freed > 0,
        "a mailbox's worth of mail was deleted and not one page came back"
    );
    assert!(
        after < before,
        "the pages are on the freelist and the file is the same size: \
         {before} bytes before, {after} after. Deleting mail freed nothing \
         a user could see."
    );
}
