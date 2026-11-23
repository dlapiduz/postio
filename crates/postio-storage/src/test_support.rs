use std::path::Path;
use std::time::Duration;

use postio_model::{Account, EmailAddress, Mailbox, MailboxId};

use tempfile::TempDir;

use crate::key::{BlobKeys, Purpose, StoreKey, Subkey};
use crate::repository::{AccountRepository, MailboxRepository};
use crate::store::Connection;
use crate::store::Store;

/// Fixed so a test can close and reopen a store; every helper goes through the encrypted path as a
/// real store does (ADR 0014).
pub fn key() -> Subkey {
    master().derive(Purpose::Database)
}

/// The blob half of `key`, from the same fixed master (ADR 0014); two stores in one test dedup
/// against each other.
pub fn blob_keys() -> BlobKeys {
    BlobKeys::derive(&master())
}

fn master() -> StoreKey {
    StoreKey::from_bytes([0x5a; crate::key::KEY_BYTES])
}

pub mod counting;
pub mod gate_log;
/// A migrated scratch database. File-backed, on `/dev/shm` where it exists: the engine refuses to
/// key an in-memory database, and `cache=shared` brought table locks no busy timeout could wait out
/// (#204). Panics if it cannot be created or migrated.
pub async fn memory() -> Store {
    let root = scratch_root();
    sweep_orphaned_scratch_dirs(&root);
    let directory = new_scratch_dir(&root).expect("a scratch directory must always open");
    let path = directory.path().join("postio.db");
    let store = Store::open(&path, &key())
        .await
        .expect("a scratch database must always open");
    // Leaked deliberately: `Store` has no guard slot, and the sweep keeps the leak from
    // accumulating.
    std::mem::forget(directory);
    store
}

/// `/dev/shm` where it exists, so scratch costs RAM rather than disk, else the system temp
/// directory. Both are swept.
fn scratch_root() -> std::path::PathBuf {
    let shm = Path::new("/dev/shm");
    if shm.is_dir() {
        shm.to_path_buf()
    } else {
        std::env::temp_dir()
    }
}

fn new_scratch_dir(root: &Path) -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix(SWEEP_PREFIX)
        .tempdir_in(root)
}

/// Prefix of every scratch directory; the sweep also reclaims any directory holding
/// `SCRATCH_DATABASE`.
const SWEEP_PREFIX: &str = "postio-test-";

/// Marks older scratch directories made under `tempfile`'s default name, which a prefix-only sweep
/// never reaches. Matching on this file keeps the sweep off other `/dev/shm` entries.
const SCRATCH_DATABASE: &str = "postio.db";

/// Below this age a directory might belong to a test binary still starting; the sweep never touches
/// it.
const SWEEP_GRACE: Duration = Duration::from_secs(60);

/// Past this age a directory cannot belong to a live test binary; the sweep reclaims it
/// unconditionally.
const SWEEP_MIN_AGE: Duration = Duration::from_secs(30 * 60);

/// Cap on scratch directories between the grace period and `SWEEP_MIN_AGE`, so one busy day of
/// leaks cannot build up (#442).
const SWEEP_MAX_COUNT: usize = 200;

/// Once per process: `memory()` runs in every test and a directory listing is not free.
fn sweep_orphaned_scratch_dirs(dir: &Path) {
    static SWEEP: std::sync::Once = std::sync::Once::new();
    let dir = dir.to_path_buf();
    SWEEP.call_once(|| sweep_now(&dir));
}

/// Removes orphaned scratch directories under `dir`. Never panics: an entry it cannot list or
/// remove is skipped, so cleaning up after other runs cannot fail this one.
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

    candidates.sort_by_key(|(_, age)| std::cmp::Reverse(*age));
    let over_cap = candidates.len().saturating_sub(SWEEP_MAX_COUNT);

    for (index, (path, age)) in candidates.iter().enumerate() {
        if *age >= SWEEP_MIN_AGE || index < over_cap {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

/// A migrated database in a temporary directory that deletes itself. Panics if it cannot be
/// created.
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

#[derive(Debug)]
pub struct TempStore {
    store: Store,
    /// Dropped last, after the database's connections are closed.
    _directory: TempDir,
}

impl TempStore {
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

/// A throwaway account. Panics if the insert fails.
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

/// A mailbox at `path` in `account`. Panics if the insert fails.
pub async fn mailbox(connection: &Connection, account: &Account, path: &str) -> Mailbox {
    let mut mailbox = Mailbox::new(account.id, path, Some('/'));
    MailboxRepository::new(connection)
        .create(&mut mailbox)
        .await
        .expect("create a test mailbox");
    mailbox
}

/// Marks the store as begun without contacts: every message it holds predates anything recording
/// who it was from, as in a migrated store.
pub async fn mark_contacts_history(connection: &Connection) {
    connection
        .execute_batch(
            "INSERT OR REPLACE INTO settings (key, account_id, value, updated_at)
             VALUES ('contacts.history_through', NULL,
                     (SELECT CAST(coalesce(max(id), 0) AS TEXT) FROM messages), 0);",
        )
        .await
        .expect("the history mark");
}

/// An account with an INBOX. Panics if either insert fails.
pub async fn account_with_inbox(connection: &Connection) -> (Account, MailboxId) {
    let account = account(connection).await;
    let inbox = mailbox(connection, &account, "INBOX").await;
    (account, inbox.id)
}

/// The `EXPLAIN QUERY PLAN` lines for `sql`, newline-joined, for asserting a read uses an index.
/// Binds `1` to each `?N` placeholder, since the planner needs bindings but not their values.
/// Panics if the statement will not prepare.
pub async fn plan(connection: &Connection, sql: &str) -> String {
    let mut statement = crate::sql::statement(connection, &format!("EXPLAIN QUERY PLAN {sql}"))
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

/// The highest index, not the count: `?1` may appear twice and be one parameter.
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

/// Whether a plan sorts. Both spellings (`USE TEMP B-TREE FOR ORDER BY`, `USE SORTER FOR ORDER BY`)
/// live here: asserting only the SQLite one cannot fail on this engine.
pub fn sorts(plan: &str) -> bool {
    let plan = plan.to_ascii_uppercase();
    plan.contains("TEMP B-TREE") || plan.contains("USE SORTER")
}

/// The error the engine returns when another connection holds the write lock past `busy_timeout`
/// (#1594).
pub fn busy() -> crate::Error {
    crate::Error::Engine(turso::Error::Busy("database is locked".to_owned()))
}

/// Every statement the Contacts list issues, named, so a budget asks the planner about the SQL that
/// runs, not a copy.
pub fn contact_list_statements() -> Vec<(&'static str, String)> {
    crate::repository::contact_list_statements()
}

pub fn contact_suggestion_statements() -> Vec<String> {
    crate::repository::contact_suggestion_statements()
}

#[cfg(test)]
mod sweep_tests {
    use std::time::{Duration, SystemTime};

    use super::*;

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
        // Enough old directories to pass the count cap alone, so only the grace period protects the
        // young one.
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
    fn every_platform_sweeps_where_its_scratch_directories_go() {
        let root = scratch_root();
        let expected = if Path::new("/dev/shm").is_dir() {
            std::path::PathBuf::from("/dev/shm")
        } else {
            std::env::temp_dir()
        };
        assert_eq!(root, expected, "memory() must sweep the root it writes to");

        let probe = tempfile::tempdir().expect("tempdir");
        let leaked = new_scratch_dir(probe.path()).expect("scratch dir").keep();
        let stamp = SystemTime::now()
            .checked_sub(SWEEP_MIN_AGE + Duration::from_secs(1))
            .expect("age fits before now");
        std::fs::File::open(&leaked)
            .expect("open dir")
            .set_modified(stamp)
            .expect("backdate mtime");

        sweep_now(probe.path());

        assert!(
            !leaked.exists(),
            "a leaked scratch directory, wherever it was made, must be one the sweep reclaims"
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
        // All past grace, none past `SWEEP_MIN_AGE`: only the count cap reclaims. `paths[0]` is the
        // youngest.
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
        // Directories predating `SWEEP_PREFIX` carry `tempfile`'s default name; a prefix-only sweep
        // could never reach them.
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
        // `/dev/shm` is shared: a sweep that took an unrelated old directory holding a database
        // would be worse.
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
