//! One-call throwaway databases for tests.
//!
//! Every test that touches storage needs a migrated database and needs it to
//! cost nothing, so that "write the failing test first" (CLAUDE.md) stays
//! practical. These helpers panic rather than returning a `Result`: a harness
//! that cannot open an in-memory database is a broken build, not a test
//! failure, and unwrapping in every test would drown the assertions.
//!
//! # Availability
//!
//! Behind the off-by-default `test-support` cargo feature, so an ordinary build
//! of `postio-storage` carries none of it — the same arrangement
//! `postio-model` uses for its `.eml` corpus. Downstream crates opt in from
//! their dev-dependencies:
//!
//! ```toml
//! [dev-dependencies]
//! postio-storage = { workspace = true, features = ["test-support"] }
//! ```
//!
//! # Which one to use
//!
//! [`memory`] for almost everything: it is fast (tmpfs where available) and
//! leaves nothing behind. [`temp`] when the test is *about* the file's
//! location — reopening from a path the test controls, permissions,
//! anything that needs [`TempDatabase::directory`].
//!
//! ```
//! let database = postio_storage::test_support::memory();
//! let connection = database.connection().expect("checkout");
//! # let _ = connection;
//! ```

use std::path::Path;

use postio_model::{Account, EmailAddress, Mailbox, MailboxId};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::db::Database;
use crate::repository::{AccountRepository, MailboxRepository};

/// A migrated scratch database, shared by every connection its pool opens.
///
/// It lives as long as the returned handle (clones included) and disappears
/// with it.
///
/// Despite the name it is **file-backed**, in a temporary directory the
/// handle owns — on `/dev/shm` where that exists, so it still costs RAM
/// rather than disk. It used to be `Database::open_in_memory`, whose
/// `cache=shared` brings table-level locks that `busy_timeout` cannot wait
/// out: under load, a fixture write could fail with "database table is
/// locked" in a test that is not about locking at all (#204). A file gets
/// WAL and the ordinary busy handler, where a reader never fails a writer.
///
/// # Panics
///
/// If the directory or the database cannot be created or migrated.
pub fn memory() -> Database {
    let shm = Path::new("/dev/shm");
    let directory = if shm.is_dir() {
        tempfile::tempdir_in(shm)
    } else {
        tempfile::tempdir()
    }
    .expect("a scratch directory must always open");
    let path = directory.path().join("postio.db");
    Database::open_file_with_guard(&path, Box::new(directory))
        .expect("a scratch database must always open")
}

/// A migrated database in a temporary directory that deletes itself.
///
/// The [`TempDir`] is kept alive by the returned handle, so the caller does not
/// have to hold anything extra; when the [`Database`] is dropped the directory
/// and its WAL files go with it.
///
/// # Panics
///
/// If the temporary directory or the database cannot be created.
pub fn temp() -> TempDatabase {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let database = Database::open(directory.path().join("postio.db"))
        .expect("a temporary database must always open");
    TempDatabase {
        database,
        _directory: directory,
    }
}

/// A file-backed [`Database`] plus the temporary directory holding it.
///
/// Derefs to [`Database`], so it is used exactly like one; the directory is
/// removed when this value is dropped.
#[derive(Debug)]
pub struct TempDatabase {
    database: Database,
    /// Dropped last, after the database's connections are closed.
    _directory: TempDir,
}

impl TempDatabase {
    /// The directory the database file lives in.
    pub fn directory(&self) -> &Path {
        self._directory.path()
    }
}

impl std::ops::Deref for TempDatabase {
    type Target = Database;

    fn deref(&self) -> &Database {
        &self.database
    }
}

/// Creates a throwaway account, so a test that is about something else does not
/// have to spell one out.
///
/// # Panics
///
/// If the insert fails.
pub fn account(connection: &Connection) -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Test User"), "test@example.com"),
    );
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("create a test account");
    account
}

/// Creates a mailbox at `path` in `account`.
///
/// # Panics
///
/// If the insert fails.
pub fn mailbox(connection: &Connection, account: &Account, path: &str) -> Mailbox {
    let mut mailbox = Mailbox::new(account.id, path, Some('/'));
    MailboxRepository::new(connection)
        .create(&mut mailbox)
        .expect("create a test mailbox");
    mailbox
}

/// Creates an account with an INBOX, the shape almost every test wants.
///
/// # Panics
///
/// If either insert fails.
pub fn account_with_inbox(connection: &Connection) -> (Account, MailboxId) {
    let account = account(connection);
    let inbox = mailbox(connection, &account, "INBOX");
    (account, inbox.id)
}
