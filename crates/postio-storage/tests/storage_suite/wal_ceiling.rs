//! The write-ahead log has a ceiling (#1175).
//!
//! The live install reached a **676 MB** WAL against an 868 MB database, and
//! every launch paid for it: SQLCipher decrypts pages while SQLite rebuilds
//! the WAL index, before the first row can be read, and `Phase::Store` sits
//! in front of the first frame.
//!
//! The issue's reading was that a long-lived reader pins the WAL open, and
//! that is not what this asserts, because it is not needed and there is no
//! such reader in the crate — `grep` finds no held `Transaction` and no open
//! blob handle. The simpler mechanism is sufficient: **`journal_size_limit`
//! is unset, so the WAL file never shrinks.** A checkpoint resets the log and
//! SQLite starts writing from the top of the same file again, but it does not
//! give the space back. Any single burst that pushes the WAL high — a sync
//! pass writing a mailbox in one transaction, which auto-checkpoints only at
//! commit — leaves the file at that high-water mark for the rest of the
//! install's life.
//!
//! So the assertion is not "the WAL stays small while writing", which would
//! be asking SQLite not to work the way it works -- a checkpoint cannot pass
//! the oldest open reader, and while one is open the log has to keep every
//! frame. It is: **after the writing stops, the file goes back down.**
//!
//! Measured on this seed: 20,000 messages with no reader open leave a 6.8 MB
//! log, and the same writes with one reader holding a snapshot leave 44 MB.
//! Neither shrinks by a byte afterwards, which is the part that turns one bad
//! afternoon into a permanently slow launch.

use postio_storage::{Database, seed::seed_large, test_support};

/// What the WAL is allowed to keep once the writing has stopped.
///
/// Deliberately a plain number here rather than a read of the pragma this
/// enforces: a test that asks the code what its own limit is cannot fail when
/// the limit is wrong. This is the budget from outside — a mail store may not
/// leave tens of megabytes of log behind after an idle moment.
const CEILING: u64 = 32 * 1024 * 1024;

fn wal_bytes(path: &std::path::Path) -> u64 {
    let wal = path.with_file_name(format!(
        "{}-wal",
        path.file_name().expect("a file name").to_string_lossy()
    ));
    std::fs::metadata(&wal).map(|meta| meta.len()).unwrap_or(0)
}

#[test]
fn the_wal_gives_its_space_back_once_the_writing_stops() {
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("store.db");
    let database = Database::open(&path, &test_support::key()).expect("a store");

    // A reader with an open snapshot across the whole of the writing, which
    // is what lets the log grow at all: a checkpoint cannot pass the oldest
    // reader, so every frame written while this is open is a frame the log
    // has to keep. Measured on this seed, it is the difference between a
    // 6.8 MB log and a 44 MB one -- and the live install reached 676 MB.
    //
    // This is here to *build* a large log, not as a claim about which reader
    // did it on the live store. Whatever holds the snapshot, the assertion
    // below is the same: once it lets go, the space comes back.
    let reader = database.connection().expect("a reader");
    reader.execute_batch("BEGIN;").expect("begin");
    let _: i64 = reader
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("take the read lock");

    // A first sync's worth of mail, which is what grows it.
    seed_large(&database, 11, 20_000);
    let peak = wal_bytes(&path);
    reader.execute_batch("COMMIT;").expect("end the read");
    drop(reader);

    // The writing has stopped, and something commits afterwards -- which is
    // every ordinary moment in a running Postio between one sync pass and the
    // next. That is when the log should hand its space back.
    let connection = database.connection().expect("a connection");
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS wal_ceiling_probe(x); DROP TABLE wal_ceiling_probe;",
        )
        .expect("a commit after the writing stopped");
    drop(connection);

    let settled = wal_bytes(&path);
    assert!(
        settled <= CEILING,
        "the write-ahead log peaked at {peak} bytes and settled at {settled}, \
         over the {CEILING}-byte ceiling. Without `journal_size_limit` a \
         checkpoint resets the log without giving the file back, so the \
         high-water mark of one sync is what every later launch reads."
    );
}

#[test]
fn a_log_an_unclean_exit_left_behind_is_reclaimed_on_the_next_open() {
    // `journal_size_limit` bounds what a *completed checkpoint* retains, and
    // a checkpoint cannot pass an open reader. So a store that never gets a
    // quiet moment keeps whatever it grew to — and the live one had 676 MB to
    // read before its first frame.
    //
    // On a clean close SQLite checkpoints and deletes the log outright, which
    // measured here as a `-wal` of zero bytes. So the live install is not
    // closing cleanly, and `db.rs` says why in another comment: the exit path
    // calls `exit()` deliberately, after #794 and #699 made an eager teardown
    // its own crash. Nothing runs, nothing is checkpointed, and the log is
    // still there next time.
    //
    // `mem::forget` is that exit: the pool is never dropped, so SQLite never
    // gets its close. What is left on disk is what the live store looks like.
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("store.db");
    let database = Database::open(&path, &test_support::key()).expect("a store");

    let reader = database.connection().expect("a reader");
    reader.execute_batch("BEGIN;").expect("begin");
    let _: i64 = reader
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("take the read lock");
    seed_large(&database, 11, 20_000);
    drop(reader);
    std::mem::forget(database);

    let left_behind = wal_bytes(&path);
    assert!(
        left_behind > CEILING,
        "this case is about a log too big to leave alone, and the unclean \
         exit left only {left_behind} bytes -- it is not testing what it says"
    );

    let reopened = Database::open(&path, &test_support::key()).expect("reopen");
    let after = wal_bytes(&path);
    drop(reopened);

    assert!(
        after <= CEILING,
        "opening a store found {left_behind} bytes of log left by the last \
         run and still had {after} after. Every launch pays to read it, which \
         is #1175's five seconds"
    );
}
