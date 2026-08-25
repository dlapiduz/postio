//! Connection setup, pragmas, pooling, and the in-memory test harness.
//!
//! Written before any of it existed. The bead's acceptance criteria are
//! "a helper gives a migrated DB in a single call", "pragmas asserted by a
//! test" and "a concurrent read during a write does not block", and each one
//! has a test here.

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use postio_storage::db::{self, DEFAULT_MAX_CONNECTIONS, Database};
use postio_storage::migrations;

// ---------------------------------------------------------------------------
// Acceptance: the helper gives a migrated database in a single call
// ---------------------------------------------------------------------------

#[test]
fn the_in_memory_harness_is_migrated_in_one_call() {
    let database = postio_storage::test_support::memory();

    assert_eq!(
        database.schema_version().expect("version"),
        migrations::latest_version(),
        "the harness hands back a database already at head"
    );

    let connection = database.connection().expect("checkout");
    let messages: i64 = connection
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .expect("the schema is there to query");
    assert_eq!(messages, 0, "and it is empty");
}

#[test]
fn the_tempdir_harness_is_a_real_file_on_disk() {
    let database = postio_storage::test_support::temp();

    assert_eq!(
        database.schema_version().expect("version"),
        migrations::latest_version()
    );
    assert!(
        database
            .path()
            .expect("a file-backed harness has a path")
            .exists(),
        "the database file was created"
    );
}

#[test]
fn two_harnesses_are_independent() {
    let one = postio_storage::test_support::memory();
    let other = postio_storage::test_support::memory();

    one.connection()
        .expect("checkout")
        .execute(
            "INSERT INTO accounts (display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES ('One', 'one@example.com', 'imap.example.com', 993, 'one',
                     'smtp.example.com', 587, 'one', 0)",
            [],
        )
        .expect("insert");

    let count: i64 = other
        .connection()
        .expect("checkout")
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        count, 0,
        "one in-memory harness must not see the other's rows"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: pragmas are asserted by a test
// ---------------------------------------------------------------------------

#[test]
fn a_file_backed_connection_carries_postios_pragmas() {
    let database = postio_storage::test_support::temp();
    let connection = database.connection().expect("checkout");
    let pragmas = db::read_pragmas(&connection).expect("read pragmas");

    assert_eq!(
        pragmas.journal_mode, "wal",
        "WAL is what lets a reader run during a write"
    );
    assert_eq!(pragmas.synchronous, 1, "synchronous = NORMAL");
    assert!(pragmas.foreign_keys, "foreign keys are enforced");
    assert_eq!(pragmas.temp_store, 2, "temp_store = MEMORY");
    assert!(pragmas.mmap_size > 0, "mmap is on");
    assert!(
        pragmas.cache_size < 0,
        "cache_size is negative, i.e. expressed in KiB rather than pages"
    );
    assert!(
        pragmas.busy_timeout >= 1_000,
        "a busy timeout the pool can actually wait out"
    );
}

#[test]
fn every_pooled_connection_is_configured_not_just_the_first() {
    let database = postio_storage::test_support::temp();

    // Hold the first checkout so the second has to be a freshly opened one.
    let first = database.connection().expect("first");
    let second = database.connection().expect("second");

    for connection in [&first, &second] {
        let pragmas = db::read_pragmas(connection).expect("read pragmas");
        assert!(pragmas.foreign_keys);
        assert_eq!(pragmas.synchronous, 1);
    }
}

#[test]
fn foreign_keys_are_actually_enforced() {
    let database = postio_storage::test_support::memory();
    let connection = database.connection().expect("checkout");

    let error = connection
        .execute(
            "INSERT INTO mailboxes (account_id, name, path) VALUES (404, 'Inbox', 'INBOX')",
            [],
        )
        .expect_err("a mailbox under a nonexistent account must be rejected");

    assert!(
        error.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign key violation, got: {error}"
    );
}

// ---------------------------------------------------------------------------
// Acceptance: a concurrent read during a write does not block (WAL verified)
// ---------------------------------------------------------------------------

#[test]
fn a_read_proceeds_while_a_write_transaction_is_open() {
    let database = postio_storage::test_support::temp();

    let mut writer = database.connection().expect("writer");
    let transaction = writer
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .expect("begin immediate");
    transaction
        .execute(
            "INSERT INTO accounts (display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES ('Uncommitted', 'ghost@example.com', 'imap.example.com', 993, 'ghost',
                     'smtp.example.com', 587, 'ghost', 0)",
            [],
        )
        .expect("write inside the open transaction");

    // A second connection, on another thread, while that write is still open.
    let pool = database.pool().clone();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        let connection = pool.get().expect("reader checkout");
        let count: i64 = connection
            .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
            .expect("read during a write");
        sender.send(count).expect("report");
    });

    let count = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("the reader must not block on the open write transaction");
    assert_eq!(
        count, 0,
        "the reader sees the pre-write snapshot, not a lock"
    );

    transaction.commit().expect("commit");
    reader.join().expect("reader thread");

    let count: i64 = database
        .connection()
        .expect("checkout")
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count after commit");
    assert_eq!(count, 1, "and sees the row once it is committed");
}

// ---------------------------------------------------------------------------
// The pool itself
// ---------------------------------------------------------------------------

#[test]
fn a_returned_connection_is_reused_rather_than_reopened() {
    let database = postio_storage::test_support::memory();
    let pool = database.pool();

    assert_eq!(
        pool.live_connections(),
        1,
        "opening migrated on one connection"
    );

    {
        let _first = pool.get().expect("checkout");
        assert_eq!(pool.live_connections(), 1);
    }
    let _again = pool.get().expect("checkout");
    assert_eq!(
        pool.live_connections(),
        1,
        "the returned connection was handed straight back out"
    );
}

#[test]
fn the_pool_opens_up_to_its_limit_and_no_further() {
    let database = Database::open_in_memory_with(2).expect("open");
    let pool = database.pool();

    let _one = pool.get().expect("first");
    let _two = pool.get().expect("second");
    assert_eq!(pool.live_connections(), 2);
    assert_eq!(pool.max_connections(), 2);

    // A third checkout has to wait for one of these to come back.
    let borrowed = pool.clone();
    let (sender, receiver) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let connection = borrowed.get().expect("third");
        sender.send(connection.is_autocommit()).expect("report");
    });

    assert!(
        receiver.recv_timeout(Duration::from_millis(200)).is_err(),
        "with every connection checked out, the third caller waits"
    );

    drop(_two);
    assert!(
        receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("the waiter is woken when a connection comes back")
    );
    waiter.join().expect("waiter thread");
    assert_eq!(
        pool.live_connections(),
        2,
        "and the pool never grew past its limit"
    );
}

#[test]
fn pooled_connections_share_one_in_memory_database() {
    let database = postio_storage::test_support::memory();

    database
        .connection()
        .expect("checkout")
        .execute(
            "INSERT INTO accounts (display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES ('Shared', 'shared@example.com', 'imap.example.com', 993, 'shared',
                     'smtp.example.com', 587, 'shared', 0)",
            [],
        )
        .expect("insert");

    // Force a second, distinct connection and read the same row through it.
    let held = database.connection().expect("hold the first");
    let other = database.connection().expect("second");
    assert_eq!(database.pool().live_connections(), 2);

    let count: i64 = other
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count");
    assert_eq!(
        count, 1,
        "an in-memory harness is one database, not one per connection"
    );
    drop(held);
}

#[test]
fn the_default_pool_size_is_sane() {
    assert!(
        (2..=16).contains(&DEFAULT_MAX_CONNECTIONS),
        "enough for a writer plus readers, few enough not to thrash"
    );
    let database = postio_storage::test_support::memory();
    assert_eq!(database.pool().max_connections(), DEFAULT_MAX_CONNECTIONS);
}

#[test]
fn many_threads_can_read_and_write_through_the_pool() {
    let database = postio_storage::test_support::temp();

    let handles: Vec<_> = (0..8)
        .map(|worker| {
            let pool = database.pool().clone();
            thread::spawn(move || {
                let connection = pool.get().expect("checkout");
                connection
                    .execute(
                        "INSERT INTO accounts (display_name, address, incoming_host,
                                               incoming_port, incoming_username, outgoing_host,
                                               outgoing_port, outgoing_username, created_at)
                         VALUES (?1, ?2, 'imap.example.com', 993, ?2,
                                 'smtp.example.com', 587, ?2, 0)",
                        rusqlite::params![
                            format!("Worker {worker}"),
                            format!("worker{worker}@example.com")
                        ],
                    )
                    .expect("concurrent insert");
            })
        })
        .collect();
    for handle in handles {
        handle.join().expect("worker thread");
    }

    let count: i64 = database
        .connection()
        .expect("checkout")
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 8, "every worker's write landed");
    assert!(database.pool().live_connections() <= DEFAULT_MAX_CONNECTIONS);
}

#[test]
fn opening_a_database_creates_the_parent_directory() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("nested/deeper/postio.db");

    let database = Database::open(&path).expect("open creates what it needs");

    assert!(path.exists());
    assert_eq!(
        database.schema_version().expect("version"),
        migrations::latest_version()
    );
}

#[test]
fn reopening_a_database_keeps_its_rows_and_does_not_remigrate() {
    let directory = tempfile::tempdir().expect("tempdir");
    let path = directory.path().join("postio.db");

    {
        let database = Database::open(&path).expect("open");
        database
            .connection()
            .expect("checkout")
            .execute(
                "INSERT INTO accounts (display_name, address, incoming_host, incoming_port,
                                       incoming_username, outgoing_host, outgoing_port,
                                       outgoing_username, created_at)
                 VALUES ('Kept', 'kept@example.com', 'imap.example.com', 993, 'kept',
                         'smtp.example.com', 587, 'kept', 0)",
                [],
            )
            .expect("insert");
    }

    let database = Database::open(&path).expect("reopen");
    let count: i64 = database
        .connection()
        .expect("checkout")
        .query_row("SELECT count(*) FROM accounts", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// This is the user's mail: the directory and the file are private
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod private_by_default {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn a_freshly_created_store_is_0700_directory_0600_file() {
        let directory = tempfile::tempdir().expect("tempdir");
        // A parent that does not exist yet either, the way
        // `$XDG_DATA_HOME/postio` does on a first run: nothing has ever
        // created it, only `Database::open` will.
        let path = directory.path().join("postio").join("postio.db");

        let database = Database::open(&path).expect("open");
        drop(database);

        assert_eq!(
            mode_of(path.parent().expect("a parent")),
            0o700,
            "the data directory must not be at the process umask"
        );
        assert_eq!(mode_of(&path), 0o600, "nor the database file itself");
    }

    #[test]
    fn an_existing_store_that_was_loose_is_repaired_on_reopen() {
        let directory = tempfile::tempdir().expect("tempdir");
        let store = directory.path().join("postio");
        let path = store.join("postio.db");

        // A store from before this existed: created, then loosened, the way
        // a pre-fix Postio -- or a stray `chmod -R` -- would leave it.
        let _ = Database::open(&path).expect("first open");
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o755))
            .expect("loosen the directory");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("loosen the file");

        Database::open(&path).expect("reopen");

        assert_eq!(
            mode_of(&store),
            0o700,
            "the directory is repaired, not just guarded on creation"
        );
        assert_eq!(mode_of(&path), 0o600, "and so is the file");
    }
}

#[test]
fn a_reader_mid_transaction_does_not_fail_a_concurrent_writer() {
    // #204. The scratch databases tests run on used to be `:memory:` with
    // `cache=shared`, where a read transaction on one pooled connection makes
    // a write on another fail *immediately* with SQLITE_LOCKED — a
    // table-level lock that `busy_timeout` does not cover, so the failure
    // rate tracked machine load rather than anything in the test. The shape
    // below is the minimal reproduction: a reader holding a snapshot (a list
    // page mid-iteration, in real code) while a fixture writes.
    let database = postio_storage::test_support::memory();
    let reader = database.connection().expect("a reader");
    let writer = database.connection().expect("a writer");

    reader
        .execute_batch("BEGIN; SELECT count(*) FROM accounts;")
        .expect("a read transaction opens");

    // Under shared cache this panics inside the helper with "database table
    // is locked" — the exact symptom from the field, on a line that is only
    // a fixture. On the file-backed scratch database it must simply work.
    let account = postio_storage::test_support::account(&writer);

    reader.execute_batch("COMMIT").expect("the reader finishes");
    assert!(account.id.get() > 0, "the write landed");
}
