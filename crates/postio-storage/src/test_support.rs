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
use std::time::Duration;

use postio_model::{Account, EmailAddress, Mailbox, MailboxId};
use rusqlite::Connection;
use tempfile::TempDir;

use crate::db::Database;
use crate::key::{BlobKeys, Purpose, StoreKey, Subkey};
use crate::repository::{AccountRepository, MailboxRepository};

/// The key every scratch database is encrypted under.
///
/// Fixed, and that is the point: ADR 0014 Q3 says the suite must exercise the
/// *encrypted* path, because a plaintext configuration that no longer ships is
/// not worth testing. Every helper here goes through SQLCipher exactly as a
/// real store does — so a repository that would break under page encryption
/// breaks in the ordinary test run rather than on somebody's mail.
///
/// Fixed rather than random so a test can close a store and reopen it. It is
/// not a secret and must never be used by anything that ships; nothing outside
/// the `test-support` feature can reach it.
pub fn key() -> Subkey {
    master().derive(Purpose::Database)
}

/// The keys every scratch blob store is opened under.
///
/// The blob half of [`key`], from the same fixed master, and there for the
/// same reason: ADR 0014 Q2 gave blobs a per-file AEAD and a keyed id, and a
/// suite that opened plaintext blob stores would be testing a configuration
/// that no longer ships. Every `BlobStore::open` in the workspace's tests goes
/// through this, so the encrypted path is the ordinary path.
///
/// Fixed, so a test can close a store and reopen it — and so two stores in one
/// test dedup against each other, which several of them rely on. A test that
/// is *about* two installations not sharing ids derives its own keys from
/// different masters instead.
pub fn blob_keys() -> BlobKeys {
    BlobKeys::derive(&master())
}

/// The master key both of the above derive from.
///
/// Not a secret, and nothing outside the `test-support` feature can reach it.
fn master() -> StoreKey {
    StoreKey::from_bytes([0x5a; crate::key::KEY_BYTES])
}

pub mod counting;
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
        sweep_orphaned_scratch_dirs(shm);
        tempfile::Builder::new()
            .prefix(SWEEP_PREFIX)
            .tempdir_in(shm)
    } else {
        tempfile::tempdir()
    }
    .expect("a scratch directory must always open");
    let path = directory.path().join("postio.db");
    Database::open_file_with_guard(&path, &key(), Box::new(directory))
        .expect("a scratch database must always open")
}

/// Every directory [`memory`] ever creates carries this prefix, and the
/// sweep below never touches a `/dev/shm` entry without it — nothing else
/// this crate puts there is its business to delete.
const SWEEP_PREFIX: &str = "postio-test-";

/// Below this age, a directory might still belong to a test binary that has
/// not finished starting up. The sweep never touches it, however many
/// directories there are — this is the one guard that must always hold.
const SWEEP_GRACE: Duration = Duration::from_secs(60);

/// A directory this old cannot belong to a test binary that is still
/// running: nothing in this suite comes anywhere near this long, even
/// loaded down (`docs/engineering-notes.md`). The sweep reclaims it
/// unconditionally. 30 minutes because that is what the manual sweep during
/// the #442 incident used, and it worked.
const SWEEP_MIN_AGE: Duration = Duration::from_secs(30 * 60);

/// However many scratch directories are allowed to accumulate before the
/// sweep starts reclaiming ones that are past [`SWEEP_GRACE`] but not yet
/// past [`SWEEP_MIN_AGE`]. An age-only sweep still lets one very busy day of
/// leaks build up between sweeps; this bounds it outright. #442 found 1346
/// live at once.
const SWEEP_MAX_COUNT: usize = 200;

/// Runs [`sweep_now`] against `dir` once per process.
///
/// Once, not once per call: `memory()` is called by every test in a binary,
/// and a directory listing is not free enough to repeat per test. The first
/// call in a process pays for the whole run.
fn sweep_orphaned_scratch_dirs(dir: &Path) {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    let dir = dir.to_path_buf();
    SWEEP.call_once(|| sweep_now(&dir));
}

/// Removes orphaned scratch directories under `dir` — see [`SWEEP_PREFIX`],
/// [`SWEEP_GRACE`], [`SWEEP_MIN_AGE`] and [`SWEEP_MAX_COUNT`] for exactly
/// which ones and why.
///
/// Never panics: a directory this cannot list, or an entry it cannot remove
/// (a race with another process, a permissions oddity), is skipped rather
/// than treated as a failure. This runs inside every test binary that calls
/// [`memory`], and cleaning up after other test runs must never be why this
/// one fails.
fn sweep_now(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let now = std::time::SystemTime::now();
    let mut candidates: Vec<(std::path::PathBuf, Duration)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(SWEEP_PREFIX) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age < SWEEP_GRACE {
            continue;
        }
        candidates.push((entry.path(), age));
    }

    // Oldest first, so a count-cap reclamation below takes the
    // longest-orphaned directories rather than an arbitrary subset.
    candidates.sort_by_key(|(_, age)| std::cmp::Reverse(*age));
    let over_cap = candidates.len().saturating_sub(SWEEP_MAX_COUNT);

    for (index, (path, age)) in candidates.iter().enumerate() {
        if *age >= SWEEP_MIN_AGE || index < over_cap {
            let _ = std::fs::remove_dir_all(path);
        }
    }
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
    let database = Database::open(directory.path().join("postio.db"), &key())
        .expect("a temporary database must always open");
    TempDatabase {
        database,
        _directory: directory,
    }
}

/// A store written the way one was before `auto_vacuum` was chosen (#381).
///
/// Keyed and migrated with SQLite's own `auto_vacuum = NONE`, which is what
/// every store created before that decision is carrying. The conversion is a
/// one-time rewrite, so the only way to test that it happens — and that it
/// happens *once* — is against a store that genuinely needs it.
///
/// Here rather than hand-rolled in each suite because the two callers are in
/// different crates and only this one may link `rusqlite`: `postio-app`'s
/// integration tests drive the composition root, and reaching for a raw
/// connection there would put SQL in the crate whose whole boundary rule is
/// that the view layer above it has none.
///
/// # Panics
///
/// If the database cannot be created, keyed or migrated.
pub fn unconverted_store(path: &Path) -> Database {
    {
        let mut connection = rusqlite::Connection::open(path).expect("a connection");
        connection
            .execute_batch("PRAGMA cipher_memory_security = OFF;")
            .expect("memory security off, before the key");
        let hex = key().to_hex();
        connection
            .execute_batch(&format!("PRAGMA key = \"x'{}'\";", *hex))
            .expect("the store key");
        drop(hex);
        // Every pragma the pool applies except the one under test, so what
        // this differs from a real store in is exactly one line.
        connection
            .execute_batch(&crate::db::PRAGMAS.replace("PRAGMA auto_vacuum = INCREMENTAL;\n", ""))
            .expect("the pragmas as they were");
        crate::migrate(&mut connection).expect("migrate");
    }
    Database::open(path, &key()).expect("the store reopens")
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

#[cfg(test)]
mod sweep_tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    /// Creates a directory named `{SWEEP_PREFIX}{name}` under `root` and
    /// backdates its mtime by `age`, so the sweep sees it exactly as it would
    /// see a real orphaned scratch directory of that age.
    fn scratch_dir(root: &Path, name: &str, age: Duration) -> std::path::PathBuf {
        let path = root.join(format!("{SWEEP_PREFIX}{name}"));
        std::fs::create_dir(&path).expect("create scratch dir");
        let stamp = SystemTime::now()
            .checked_sub(age)
            .expect("age fits before now");
        let file = std::fs::File::open(&path).expect("open dir for its metadata");
        file.set_modified(stamp).expect("backdate mtime");
        path
    }

    #[test]
    fn a_directory_inside_the_grace_period_is_never_touched() {
        let root = tempfile::tempdir().expect("tempdir");
        // Enough old directories to blow through the count cap on their own,
        // so only the grace period can be protecting the young one below.
        for i in 0..SWEEP_MAX_COUNT + 5 {
            scratch_dir(root.path(), &format!("old-{i}"), SWEEP_MIN_AGE * 2);
        }
        let young = scratch_dir(root.path(), "young", Duration::from_secs(1));

        sweep_now(root.path());

        assert!(
            young.is_dir(),
            "a directory inside the grace period must survive regardless of the count cap"
        );
    }

    #[test]
    fn a_directory_past_the_minimum_age_is_reclaimed() {
        let root = tempfile::tempdir().expect("tempdir");
        let stale = scratch_dir(root.path(), "stale", SWEEP_MIN_AGE + Duration::from_secs(1));

        sweep_now(root.path());

        assert!(
            !stale.exists(),
            "a directory past the minimum age must be reclaimed"
        );
    }

    #[test]
    fn a_directory_without_the_scratch_prefix_is_left_alone() {
        let root = tempfile::tempdir().expect("tempdir");
        let other = root.path().join("not-ours");
        std::fs::create_dir(&other).expect("create dir");
        let stamp = SystemTime::now()
            .checked_sub(SWEEP_MIN_AGE + Duration::from_secs(1))
            .expect("age fits before now");
        std::fs::File::open(&other)
            .expect("open dir")
            .set_modified(stamp)
            .expect("backdate mtime");

        sweep_now(root.path());

        assert!(
            other.is_dir(),
            "the sweep must only ever touch directories carrying its own prefix"
        );
    }

    #[test]
    fn the_count_cap_reclaims_the_oldest_first_once_past_the_grace_period() {
        let root = tempfile::tempdir().expect("tempdir");
        // All past the grace period, none past `SWEEP_MIN_AGE` -- only the
        // count cap should trigger any reclamation in this test. Age grows
        // with `i`, so `paths[0]` is the youngest of the set and
        // `paths.last()` is the oldest.
        let mut paths = Vec::new();
        for i in 0..SWEEP_MAX_COUNT + 3 {
            let age = SWEEP_GRACE + Duration::from_secs(60 + i as u64);
            paths.push(scratch_dir(root.path(), &format!("n-{i:04}"), age));
        }

        sweep_now(root.path());

        let remaining = std::fs::read_dir(root.path()).expect("read root").count();
        assert!(
            remaining <= SWEEP_MAX_COUNT,
            "the count cap must bound accumulation even when nothing is past \
             the minimum age, got {remaining} remaining"
        );
        assert!(
            !paths.last().unwrap().exists(),
            "the oldest directory over the cap should be reclaimed first"
        );
        assert!(
            paths[0].is_dir(),
            "the youngest directories should survive while the total is over \
             the cap but shrinking toward it"
        );
    }

    #[test]
    fn sweeping_a_directory_with_no_leaks_does_not_panic() {
        let root = tempfile::tempdir().expect("tempdir");
        sweep_now(root.path());
    }
}
