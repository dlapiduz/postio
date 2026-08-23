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
//! [`memory`] for almost everything: it is faster and leaves nothing behind.
//! [`temp`] when the test is *about* the file — WAL behaviour, reopening,
//! anything that needs a real path — because an in-memory database has no
//! journal and no filesystem.
//!
//! ```
//! let database = postio_storage::test_support::memory();
//! let connection = database.connection().expect("checkout");
//! # let _ = connection;
//! ```

use std::path::Path;

use tempfile::TempDir;

use crate::db::Database;

/// A migrated in-memory database, shared by every connection its pool opens.
///
/// It lives as long as the returned handle and disappears with it.
///
/// # Panics
///
/// If the database cannot be opened or migrated.
pub fn memory() -> Database {
    Database::open_in_memory().expect("an in-memory database must always open")
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
