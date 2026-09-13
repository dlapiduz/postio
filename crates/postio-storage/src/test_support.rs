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
//! anything that needs [`TempStore::directory`].
//!
//! ```
//! # async fn example() {
//! let store = postio_storage::test_support::memory().await;
//! let connection = store.connect().await.expect("connect");
//! # let _ = connection;
//! # }
//! ```
//!
//! # Every one of them is file-backed
//!
//! Including [`memory`], which is a name rather than a description. The engine
//! refuses to key an in-memory database at all (research.md Q6), and `memory`
//! was already a file on `/dev/shm` before that mattered: `cache=shared`
//! brought table-level locks that no busy timeout could wait out, so a fixture
//! write could fail with "database table is locked" in a test that was not
//! about locking (#204).

use std::path::Path;
use std::time::Duration;

use postio_model::{Account, EmailAddress, Mailbox, MailboxId};

use tempfile::TempDir;

use crate::key::{BlobKeys, Purpose, StoreKey, Subkey};
use crate::repository::{AccountRepository, MailboxRepository};
use crate::store::Connection;
use crate::store::Store;

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
/// Despite the name it is **file-backed**, in a temporary directory on
/// `/dev/shm` where that exists, so it still costs RAM rather than disk. See
/// the module docs for the two separate reasons it is not in memory.
///
/// # Panics
///
/// If the directory or the database cannot be created or migrated.
pub async fn memory() -> Store {
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
    let store = Store::open(&path, &key())
        .await
        .expect("a scratch database must always open");
    // The directory has to outlive every connection onto the file. There is no
    // guard slot on `Store` to hand it to -- the engine's handle owns nothing
    // of ours -- so it is leaked deliberately, and the sweep above is what
    // makes that affordable rather than a leak that accumulates across runs.
    std::mem::forget(directory);
    store
}

/// Every directory [`memory`] creates today carries this prefix. The sweep
/// below reclaims those, and also any directory holding a
/// [`SCRATCH_DATABASE`] — which is how it reaches the ones made before this
/// prefix existed. Nothing without one of those two marks is its business to
/// delete.
const SWEEP_PREFIX: &str = "postio-test-";

/// The file every scratch directory this module makes contains, and the other
/// half of "is this one of ours".
///
/// The prefix alone is not enough, and the gap is not hypothetical: scratch
/// directories predating [`SWEEP_PREFIX`] carry `tempfile`'s default name
/// instead, so an age-and-prefix sweep can never reach them however long it
/// runs. A box that has been building this workspace for a week accumulates
/// gigabytes of them, in `/dev/shm`, which is memory — the machine starts
/// swapping and nothing on it explains why.
///
/// Matching on the database file rather than loosening the prefix is what
/// keeps the sweep from touching a `/dev/shm` entry that is not this crate's
/// business: nothing else puts a `postio.db` there.
const SCRATCH_DATABASE: &str = "postio.db";

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
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        // Ours by name, or -- for the ones made before the name existed --
        // ours by what is inside.
        if !name.starts_with(SWEEP_PREFIX) && !entry.path().join(SCRATCH_DATABASE).is_file() {
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
/// have to hold anything extra; when the [`TempStore`] is dropped the directory
/// and its WAL files go with it.
///
/// # Panics
///
/// If the temporary directory or the database cannot be created.
pub async fn temp() -> TempStore {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let store = Store::open(directory.path().join("postio.db"), &key())
        .await
        .expect("a temporary database must always open");
    TempStore {
        store,
        _directory: directory,
    }
}

/// A file-backed [`Store`] plus the temporary directory holding it.
///
/// Derefs to [`Store`], so it is used exactly like one; the directory is
/// removed when this value is dropped.
#[derive(Debug)]
pub struct TempStore {
    store: Store,
    /// Dropped last, after the database's connections are closed.
    _directory: TempDir,
}

impl TempStore {
    /// The directory the database file lives in.
    pub fn directory(&self) -> &Path {
        self._directory.path()
    }
}

impl std::ops::Deref for TempStore {
    type Target = Store;

    fn deref(&self) -> &Store {
        &self.store
    }
}

/// Creates a throwaway account, so a test that is about something else does not
/// have to spell one out.
///
/// # Panics
///
/// If the insert fails.
pub async fn account(connection: &Connection) -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(Some("Test User"), "test@example.com"),
    );
    account.incoming.host = "imap.example.com".to_owned();
    account.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create a test account");
    account
}

/// Creates a mailbox at `path` in `account`.
///
/// # Panics
///
/// If the insert fails.
pub async fn mailbox(connection: &Connection, account: &Account, path: &str) -> Mailbox {
    let mut mailbox = Mailbox::new(account.id, path, Some('/'));
    MailboxRepository::new(connection)
        .create(&mut mailbox)
        .await
        .expect("create a test mailbox");
    mailbox
}

/// Creates an account with an INBOX, the shape almost every test wants.
///
/// # Panics
///
/// If either insert fails.
pub async fn account_with_inbox(connection: &Connection) -> (Account, MailboxId) {
    let account = account(connection).await;
    let inbox = mailbox(connection, &account, "INBOX").await;
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
            "the sweep must only ever touch directories that are this \
             crate's: its own prefix, or a scratch database inside"
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

    /// A directory named the way `tempfile` names one, holding `file`.
    ///
    /// What a scratch directory made before [`SWEEP_PREFIX`] existed looks
    /// like on disk.
    fn unprefixed_dir(root: &Path, name: &str, age: Duration, file: &str) -> std::path::PathBuf {
        let path = root.join(name);
        std::fs::create_dir(&path).expect("create dir");
        std::fs::write(path.join(file), b"x").expect("write the marker file");
        let stamp = SystemTime::now()
            .checked_sub(age)
            .expect("age fits before now");
        std::fs::File::open(&path)
            .expect("open dir")
            .set_modified(stamp)
            .expect("backdate mtime");
        path
    }

    #[test]
    fn a_scratch_directory_made_before_the_prefix_existed_is_still_reclaimed() {
        // The leak this closes. Directories predating `SWEEP_PREFIX` carry
        // `tempfile`'s default name, so a prefix-only sweep could never reach
        // them however long it ran -- they are not merely missed, they are
        // unreachable for ever. On `/dev/shm`, which is memory, a week of
        // them is gigabytes and the machine starts swapping with nothing on
        // it saying why.
        let root = tempfile::tempdir().expect("tempdir");
        let old_style = unprefixed_dir(
            root.path(),
            ".tmpAbCdEf",
            SWEEP_MIN_AGE + Duration::from_secs(1),
            SCRATCH_DATABASE,
        );

        sweep_now(root.path());

        assert!(
            !old_style.exists(),
            "a pre-prefix scratch directory was left behind, which is the \
             whole of the leak"
        );
    }

    #[test]
    fn an_old_directory_holding_somebody_elses_database_is_left_alone() {
        // The half that matters more than the leak: `/dev/shm` is shared, and
        // a sweep that took an unrelated directory because it was merely old
        // and had a database in it would be a far worse bug.
        let root = tempfile::tempdir().expect("tempdir");
        let theirs = unprefixed_dir(
            root.path(),
            ".tmpSomeoneElse",
            SWEEP_MIN_AGE + Duration::from_secs(1),
            "their-data.db",
        );

        sweep_now(root.path());

        assert!(
            theirs.is_dir(),
            "an old directory holding a database that is not this crate's \
             was deleted"
        );
    }

    #[test]
    fn sweeping_a_directory_with_no_leaks_does_not_panic() {
        let root = tempfile::tempdir().expect("tempdir");
        sweep_now(root.path());
    }
}

/// The query plan for `sql`, as the lines `EXPLAIN QUERY PLAN` prints, joined
/// by newlines.
///
/// The tests that use this are asserting that a read resolves through an index
/// rather than a scan or a sort, and every one of them was writing the same
/// six lines to get the text. The awkward part is the placeholders: a
/// repository's SQL is parameterised, and the planner will not explain a
/// statement it cannot bind, but it does not care what the values *are*. So
/// this counts the `?N` placeholders and binds a `1` to each.
///
/// # Panics
///
/// If the statement will not prepare or the plan will not read, which for a
/// query the caller just built means the SQL is wrong.
pub async fn plan(connection: &Connection, sql: &str) -> String {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .await
        .unwrap_or_else(|error| panic!("prepare {sql}: {error}"));
    let arguments = vec![1i64; placeholders(sql)];
    crate::sql::mapped(&mut statement, arguments, |row| {
        crate::sql::RowExt::col::<String>(row, 3)
    })
    .await
    .unwrap_or_else(|error| panic!("explain {sql}: {error}"))
    .join("\n")
}

/// How many distinct `?N` placeholders `sql` carries.
///
/// The highest index rather than the count, because `?1` may appear twice and
/// still be one parameter — which is exactly what a query filtering two
/// columns on the same account id looks like.
fn placeholders(sql: &str) -> usize {
    let mut highest = 0;
    let mut rest = sql;
    while let Some(at) = rest.find('?') {
        rest = &rest[at + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        highest = highest.max(digits.parse().unwrap_or(0));
        rest = &rest[digits.len()..];
    }
    highest
}

/// Whether a query plan says the engine had to sort.
///
/// The spelling is the engine's and it changed: SQLite writes
/// `USE TEMP B-TREE FOR ORDER BY`, this one writes `USE SORTER FOR ORDER BY`.
/// Four suites were asserting `!plan.contains("TEMP B-TREE")`, which on this
/// engine is an assertion that cannot fail — a sort went unnoticed in the
/// thread list for exactly that reason. Both spellings live here so the next
/// one is a single edit.
pub fn sorts(plan: &str) -> bool {
    let plan = plan.to_ascii_uppercase();
    plan.contains("TEMP B-TREE") || plan.contains("USE SORTER")
}
