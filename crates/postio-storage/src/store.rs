//! Opening the local store and the connections onto it. A `turso` connection is a cold pager and
//! cache (#1602), so `Store::read` keeps a few warm readers, `connect_background` gives unwatched
//! work a small cache, and `connect` is for a writer. `WriteGate` orders writers.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use turso::Builder;

use crate::error::{Error, Result};
use crate::key::Subkey;
use crate::schema;

pub use turso::{Connection, Value};

/// How many passes may read and write the store at once; each concurrent sync pass contends for one
/// writer.
pub const MAX_CONCURRENT_PASSES: usize = 4;

/// AES-256-GCM: authenticated, with hardware support on every target CPU. The encryption and
/// engine are pre-1.0 and unaudited, hence the wrong-key tests in this crate.
pub const CIPHER: &str = "aes256gcm";

/// Whether a person is waiting on this write; interactive goes ahead of background.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePriority {
    /// Goes ahead of `Background`, waiting only for a background unit already in progress.
    Interactive,
    /// Yields to any waiting interactive caller before taking the lock, not after.
    Background,
}

/// Decides who gets SQLite's single write lock. `busy_timeout` is a retry, not a queue, and
/// back-to-back sync batches starved a keystroke's write for 1.8 s (#425). A background writer
/// never begins a write while an interactive one waits. Take the connection first, then the
/// permit, and hold one permit per thread: the gate is not re-entrant.
#[derive(Debug, Clone)]
pub struct WriteGate {
    inner: Arc<GateInner>,
}

#[derive(Debug)]
struct GateInner {
    /// A blocking `std::sync::Mutex`: nothing awaits while it is held.
    state: Mutex<GateState>,
    /// `Notify`, not a `Condvar`: a condvar blocks the worker thread, and waiters would deadlock
    /// the
    /// task that releases the permit.
    free: tokio::sync::Notify,
    interactive_done: tokio::sync::Notify,
}

#[derive(Debug, Default)]
struct GateState {
    held: bool,
    /// Counted before waiting, so a background writer sees them and stands aside.
    interactive_waiting: usize,
}

impl WriteGate {
    fn new() -> Self {
        Self {
            inner: Arc::new(GateInner {
                state: Mutex::new(GateState::default()),
                free: tokio::sync::Notify::new(),
                interactive_done: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Waits for the write lock; dropping the permit releases it. Async so a waiter yields its
    /// worker.
    pub async fn acquire(&self, priority: WritePriority) -> WritePermit {
        // Registered before the first look, so a background writer checking in between sees us.
        if priority == WritePriority::Interactive {
            self.lock().interactive_waiting += 1;
        }
        #[cfg(feature = "test-support")]
        crate::test_support::gate_log::requested(priority);
        loop {
            // Created and enabled before the state is read, so a permit released between the read
            // and the await still counts.
            let mut waiting = std::pin::pin!(self.inner.free.notified());
            waiting.as_mut().enable();

            {
                let mut state = self.lock();
                let free = match priority {
                    WritePriority::Interactive => !state.held,
                    WritePriority::Background => !state.held && state.interactive_waiting == 0,
                };
                if free {
                    state.held = true;
                    if priority == WritePriority::Interactive {
                        state.interactive_waiting -= 1;
                    }
                    drop(state);
                    #[cfg(feature = "test-support")]
                    crate::test_support::gate_log::granted(priority);
                    return WritePermit {
                        inner: Arc::clone(&self.inner),
                        priority,
                    };
                }
            }

            waiting.await;
        }
    }

    /// Woken when an interactive write finishes. Enable the `Notified` before checking the queue,
    /// or a write landing in between is missed.
    pub fn interactive_writes(&self) -> &tokio::sync::Notify {
        &self.inner.interactive_done
    }

    /// Whether an interactive writer is waiting; tests use it to establish queue order without a
    /// stopwatch.
    pub fn interactive_is_waiting(&self) -> bool {
        self.lock().interactive_waiting > 0
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Debug)]
pub struct WritePermit {
    inner: Arc<GateInner>,
    priority: WritePriority,
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.held = false;
        drop(state);
        // `notify_waiters`, not `notify_one`: waiters have different predicates, and waking one
        // arbitrary task can leave the lock idle with a queue on it.
        self.inner.free.notify_waiters();
        if self.priority == WritePriority::Interactive {
            self.inner.interactive_done.notify_waiters();
        }
    }
}

/// Per-connection settings, none of which default to what Postio wants: `foreign_keys` (off by
/// default, so cascades silently fail), `temp_store` (sorts must not spill plaintext, ADR 0014),
/// and `busy_timeout` (the engine defaults to 0, so a taken lock fails at once).
const PER_CONNECTION: &str = "\
PRAGMA foreign_keys = ON;
PRAGMA temp_store = 2;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -65536;
PRAGMA synchronous = 1;
";

/// `PER_CONNECTION` for a connection that writes through or reads once: its cache would fill with
/// clean pages nothing reads, and the engine keeps one cache per connection (#1602).
const PER_BACKGROUND_CONNECTION: &str = "\
PRAGMA foreign_keys = ON;
PRAGMA temp_store = 2;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -4096;
PRAGMA synchronous = 1;
";

/// A page read, a sidebar refresh and a search can overlap; a fourth caller waits rather than pay
/// a cold cache. Caps warm-read memory at 3 x 64 MiB.
const READERS: usize = 3;

const RECLAIM_FLOOR: u64 = 64 * 1024 * 1024;

const RECLAIM_FRACTION: f64 = 0.25;

/// A reclaim blocks every writer, so both must hold: a floor (skip a few MB) and a fraction (cost
/// scales with the whole file, gain only with the holes).
pub fn worth_reclaiming(free_pages: u64, total_pages: u64, page_size: u64) -> bool {
    if total_pages == 0 {
        return false;
    }
    let free_bytes = free_pages.saturating_mul(page_size);
    free_bytes >= RECLAIM_FLOOR && free_pages as f64 / total_pages as f64 >= RECLAIM_FRACTION
}

/// Connections this process has opened on any store. Process-wide because checkouts happen on
/// whichever thread does the work (#1602).
pub(crate) static CHECKOUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Message bodies this process has read and decoded; process-wide for the reason `CHECKOUTS` is
/// (#1589).
pub(crate) static BODY_LOADS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone)]
pub struct Store {
    database: turso::Database,
    path: Option<PathBuf>,
    gate: WriteGate,
    readers: Arc<Readers>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Opens the store at `path`, creating it; the parent directory gets the store's restricted
    /// permissions, so it is never world-readable.
    pub async fn open(path: impl AsRef<Path>, key: &Subkey) -> Result<Self> {
        Self::open_reporting(path, key, || {}).await
    }

    /// `open`, calling `migrating` before the store's shape changes, so a window can say a long
    /// migration is running.
    pub async fn open_reporting(
        path: impl AsRef<Path>,
        key: &Subkey,
        migrating: impl Fn(),
    ) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::perm::ensure_private_dir(parent)?;
        }

        let fresh = !path.exists();
        let database = Self::build(path, key).await.map_err(|error| {
            if held_elsewhere(&error) {
                Error::InUse
            } else if fresh {
                error
            } else {
                as_key_failure(error)
            }
        })?;
        let store = Self {
            database,
            path: Some(path.to_path_buf()),
            gate: WriteGate::new(),
            readers: Arc::new(Readers::new()),
        };

        if fresh {
            crate::perm::tighten_file(path)?;
            store.apply_schema().await?;
        } else {
            store.prove_the_key_fits().await?;
            store.bring_the_schema_to_head(&migrating).await?;
        }

        Ok(store)
    }

    /// Writes a store at `path` from `text` rather than `HEAD`, stamped as the build whose schema
    /// it
    /// was, for migration and refusal tests. `rows` is SQL run after it.
    #[cfg(feature = "test-support")]
    pub async fn create_at_schema(
        path: impl AsRef<Path>,
        key: &Subkey,
        text: &str,
        rows: &str,
    ) -> Result<()> {
        let path = path.as_ref();
        let database = Self::build(path, key).await?;
        let connection = database.connect()?;
        crate::sql::execute(&connection, "PRAGMA foreign_keys = OFF", ()).await?;
        connection.execute_batch(text).await?;
        crate::sql::execute(
            &connection,
            &format!("PRAGMA user_version = {}", schema::fingerprint(text)),
            (),
        )
        .await?;
        connection.execute_batch(rows).await?;
        Ok(())
    }

    async fn build(path: &Path, key: &Subkey) -> Result<turso::Database> {
        let hexkey = key.to_hex();
        let opts = turso::EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: hexkey.to_string(),
        };
        let built = Builder::new_local(&path.to_string_lossy())
            .experimental_encryption(true)
            .with_encryption(opts)
            // The schema uses all three; `index_method` is what `CREATE INDEX ... USING fts` needs.
            .experimental_triggers(true)
            .experimental_index_method(true)
            .experimental_generated_columns(true)
            // `VACUUM` is the only page reclaim this engine offers; the flag only lets it parse.
            .experimental_vacuum(true)
            .build()
            .await;
        drop(hexkey);
        built.map_err(Into::into)
    }

    /// Reads one page under the key so a wrong key surfaces here, not mid-query. The engine reports
    /// it as corruption; Postio writes one key per store, so `Error::WrongStoreKey` is likelier
    /// (#404).
    async fn prove_the_key_fits(&self) -> Result<()> {
        let connection = self.connect_bare()?;
        // Once per open, and on a bare connection: nothing to cache.
        #[allow(clippy::disallowed_methods)]
        match connection
            .query("SELECT count(*) FROM sqlite_schema", ())
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => Err(as_key_failure(error.into())),
        }
    }

    async fn apply_schema(&self) -> Result<()> {
        let connection = self.connect_bare()?;
        crate::sql::execute(&connection, "PRAGMA foreign_keys = OFF", ()).await?;
        connection.execute_batch(schema::HEAD).await?;
        crate::sql::execute(
            &connection,
            &format!("PRAGMA user_version = {}", schema::FINGERPRINT),
            (),
        )
        .await?;
        Ok(())
    }

    /// Migrates a store an earlier build wrote to this build's schema, or refuses it when no step
    /// leads from its stamp. `prove_the_key_fits` passes such a store, which would then fail per
    /// statement on missing columns. Steps and the new stamp run in one transaction.
    async fn bring_the_schema_to_head(&self, migrating: &impl Fn()) -> Result<()> {
        let connection = self.connect_bare()?;
        let found: i64 = crate::sql::scalar(&connection, "PRAGMA user_version", ()).await?;
        if found == schema::FINGERPRINT {
            return Ok(());
        }
        let Some(steps) = schema::migrations_from(found) else {
            return Err(Error::SchemaFromAnotherBuild {
                found,
                expected: schema::FINGERPRINT,
            });
        };
        migrating();
        tracing::info!(
            from = found,
            to = schema::FINGERPRINT,
            steps = steps.len(),
            "migrating the store's schema"
        );
        // A step may create a table that names one created after it, as
        // `HEAD` does; the store's own connections turn the keys back on.
        crate::sql::execute(&connection, "PRAGMA foreign_keys = OFF", ()).await?;
        // One transaction: a store cut off part-way keeps its old stamp and schema, not a
        // half-applied step that cannot run twice.
        let mut batch = String::from("BEGIN IMMEDIATE;\n");
        for step in steps {
            batch.push_str(step.statements);
            batch.push('\n');
        }
        batch.push_str(&format!(
            "PRAGMA user_version = {};\nCOMMIT;",
            schema::FINGERPRINT
        ));
        if let Err(error) = connection.execute_batch(&batch).await {
            let _ = connection.execute_batch("ROLLBACK;").await;
            return Err(error.into());
        }
        tracing::info!(to = schema::FINGERPRINT, "the store's schema is migrated");
        Ok(())
    }

    /// The schema stamp of the store at `path`, read without opening it as this build's. Writes
    /// nothing.
    pub async fn stamp_at(path: impl AsRef<Path>, key: &Subkey) -> Result<i64> {
        let database = Self::build(path.as_ref(), key).await.map_err(|error| {
            if held_elsewhere(&error) {
                Error::InUse
            } else {
                as_key_failure(error)
            }
        })?;
        let connection = database.connect()?;
        crate::sql::scalar(&connection, "PRAGMA user_version", ())
            .await
            .map_err(as_key_failure)
    }

    /// `Error::InUse` when another process has the store open, `Ok` otherwise (including no store,
    /// or a key that does not fit). Setting aside a store another Postio is writing would leave it
    /// writing into the copy.
    pub async fn refuse_if_in_use(path: impl AsRef<Path>, key: &Subkey) -> Result<()> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(());
        }
        match Self::build(path, key).await {
            Err(error) if held_elsewhere(&error) => Err(Error::InUse),
            _ => Ok(()),
        }
    }

    /// Copies accounts, identities and signatures from the store at `earlier` into this one under
    /// their ids; returns how many accounts. Reads without the schema check and only the columns
    /// both schemas have: a new column takes its default.
    pub async fn carry_accounts_from(
        &self,
        earlier: impl AsRef<Path>,
        key: &Subkey,
    ) -> Result<usize> {
        let earlier = Self::build(earlier.as_ref(), key)
            .await
            .map_err(as_key_failure)?;
        let from = earlier.connect()?;
        let to = self.connect_bare()?;
        crate::sql::execute(&to, "PRAGMA foreign_keys = OFF", ()).await?;
        let mut accounts = 0;
        for table in ["accounts", "signatures", "identities"] {
            let theirs = columns(&from, table).await?;
            let shared: Vec<String> = columns(&to, table)
                .await?
                .into_iter()
                .filter(|column| theirs.contains(column))
                .collect();
            if shared.is_empty() {
                continue;
            }
            let list = shared.join(", ");
            let marks = (1..=shared.len())
                .map(|at| format!("?{at}"))
                .collect::<Vec<_>>()
                .join(", ");
            // A handful of rows, a person's own: there is no page to bound.
            let rows = crate::sql::all_unbounded(
                &from,
                &format!("SELECT {list} FROM {table}"),
                (),
                |row| {
                    (0..shared.len())
                        .map(|at| row.get_value(at).map_err(Into::into))
                        .collect::<Result<Vec<Value>>>()
                },
            )
            .await?;
            for values in rows {
                crate::sql::execute(
                    &to,
                    &format!("INSERT INTO {table} ({list}) VALUES ({marks})"),
                    values,
                )
                .await?;
                if table == "accounts" {
                    accounts += 1;
                }
            }
        }
        Ok(accounts)
    }

    /// Bytes inside the file that a delete freed and the filesystem has not got back.
    pub async fn free_bytes(&self) -> Result<u64> {
        let connection = self.connect().await?;
        let free = crate::sql::scalar(&connection, "PRAGMA freelist_count", ()).await?;
        let page = crate::sql::scalar(&connection, "PRAGMA page_size", ()).await?;
        Ok((free.max(0) as u64).saturating_mul(page.max(0) as u64))
    }

    /// Both `RECLAIM_FLOOR` of holes and `RECLAIM_FRACTION` of the file must hold. Freed pages are
    /// reused, so a store plateaus and a reclaim pays only after a large one-off deletion
    /// (a `UIDVALIDITY` reset, #381).
    pub async fn is_worth_reclaiming(&self) -> Result<bool> {
        let connection = self.connect().await?;
        let free = crate::sql::scalar(&connection, "PRAGMA freelist_count", ()).await?;
        let pages = crate::sql::scalar(&connection, "PRAGMA page_count", ()).await?;
        let page = crate::sql::scalar(&connection, "PRAGMA page_size", ()).await?;
        Ok(worth_reclaiming(
            free.max(0) as u64,
            pages.max(0) as u64,
            page.max(0) as u64,
        ))
    }

    /// Rewrites the database without its holes; returns the bytes given back. A full `VACUUM` that
    /// blocks every other writer for its whole duration, so `is_worth_reclaiming` gates it and
    /// housekeeping calls it. The permit is `Background`: it cannot begin while an interactive
    /// writer is queued, and cannot yield once begun. An interrupted reclaim leaves the file
    /// intact.
    pub async fn reclaim_free_pages(&self) -> Result<u64> {
        let before = self.file_bytes();
        let connection = self.connect().await?;
        let _permit = self.gate.acquire(WritePriority::Background).await;
        crate::sql::execute(&connection, "VACUUM", ()).await?;
        drop(_permit);
        drop(connection);
        // The rewrite lands in the log; the file only shrinks once the log is folded back.
        self.truncate_log().await?;
        Ok(before.saturating_sub(self.file_bytes()))
    }

    fn file_bytes(&self) -> u64 {
        self.path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    /// A fresh connection with foreign keys on and the interactive page cache. Each checkout is a
    /// cold cache (#1602): reads that come in numbers take `read`, work that only writes through
    /// takes `connect_background`. Async because foreign keys are per connection and default off,
    /// which would silently skip every `ON DELETE CASCADE`.
    pub async fn connect(&self) -> Result<Checkout> {
        self.connect_with(PER_CONNECTION).await
    }

    pub async fn connect_background(&self) -> Result<Checkout> {
        self.connect_with(PER_BACKGROUND_CONNECTION).await
    }

    async fn connect_with(&self, pragmas: &str) -> Result<Checkout> {
        CHECKOUTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let connection = self.database.connect()?;
        connection.execute_batch(pragmas).await?;
        Ok(Checkout {
            connection,
            gate: self.gate.clone(),
        })
    }

    /// A turn on one of `READERS` long-lived reader connections, so list pages and searches reuse a
    /// warm cache (#1602). Readers only: a transaction left open on a shared connection is the next
    /// reader's problem.
    pub async fn read(&self) -> Result<Reader> {
        let turn = self
            .readers
            .turns
            .clone()
            .acquire_owned()
            .await
            .expect("the readers' semaphore is never closed");
        let idle = self
            .readers
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let checkout = match idle {
            Some(checkout) => checkout,
            None => self.connect().await?,
        };
        Ok(Reader {
            checkout: Some(checkout),
            home: Arc::clone(&self.readers),
            _turn: turn,
        })
    }

    /// Whether a read is in progress; a background pass asks between units to let it finish.
    pub fn reading(&self) -> bool {
        self.readers.turns.available_permits() < READERS
    }

    /// A connection and an interactive permit. The permit is taken before the writer; separate
    /// calls would allow the wrong order.
    pub async fn interactive_write(&self) -> Result<(Checkout, WritePermit)> {
        let permit = self.gate.acquire(WritePriority::Interactive).await;
        let connection = self.connect().await?;
        Ok((connection, permit))
    }

    pub fn write_gate(&self) -> &WriteGate {
        &self.gate
    }

    /// A connection with nothing configured: the schema is applied with foreign keys off, since it
    /// declares tables alphabetically and keys name tables not yet created.
    fn connect_bare(&self) -> Result<Connection> {
        self.database.connect().map_err(Into::into)
    }

    /// Truncates the write-ahead log; returns its size before. A call, not `journal_size_limit`,
    /// which this engine lacks: a 676 MB WAL slowed every launch (#1175). Housekeeping calls it,
    /// off
    /// the startup and interaction paths.
    pub async fn truncate_log(&self) -> Result<u64> {
        let before = self
            .path
            .as_ref()
            .map(|path| path.with_extension("db-wal"))
            .and_then(|wal| std::fs::metadata(wal).ok())
            .map(|meta| meta.len())
            .unwrap_or(0);

        let connection = self.connect().await?;
        // A query: it answers (busy, log, checkpointed) and the engine refuses a statement whose
        // rows
        // nobody reads.
        #[allow(clippy::disallowed_methods)]
        let mut rows = connection
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await?;
        let _ = rows.next().await?;
        drop(rows);
        Ok(before)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

struct Readers {
    turns: Arc<tokio::sync::Semaphore>,
    idle: Mutex<Vec<Checkout>>,
}

impl Readers {
    fn new() -> Self {
        Readers {
            turns: Arc::new(tokio::sync::Semaphore::new(READERS)),
            idle: Mutex::new(Vec::with_capacity(READERS)),
        }
    }
}

/// One turn on a reader; dropping it hands the connection back. `checkout` clones the same pager
/// and cache, so keep the `Reader` until the clone's work is done.
pub struct Reader {
    checkout: Option<Checkout>,
    home: Arc<Readers>,
    _turn: tokio::sync::OwnedSemaphorePermit,
}

impl Reader {
    pub fn checkout(&self) -> Checkout {
        self.checkout
            .as_ref()
            .expect("a reader holds its checkout until it is dropped")
            .clone()
    }
}

impl std::ops::Deref for Reader {
    type Target = Checkout;

    fn deref(&self) -> &Checkout {
        self.checkout
            .as_ref()
            .expect("a reader holds its checkout until it is dropped")
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(checkout) = self.checkout.take() {
            self.home
                .idle
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(checkout);
        }
    }
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader").finish_non_exhaustive()
    }
}

/// A connection carrying the gate that says who writes next, so a writer cannot forget the permit
/// and silently make a person wait.
#[derive(Debug, Clone)]
pub struct Checkout {
    connection: Connection,
    gate: WriteGate,
}

impl Checkout {
    pub fn write_gate(&self) -> &WriteGate {
        &self.gate
    }
}

impl std::ops::Deref for Checkout {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

/// `DerefMut` because `Connection::transaction` takes `&mut self`, which makes overlapping
/// transactions on one connection a compile error.
impl std::ops::DerefMut for Checkout {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

/// `table`'s columns on `connection`, in declared order; none when there is no such table.
async fn columns(connection: &Connection, table: &str) -> Result<Vec<String>> {
    use crate::sql::RowExt as _;
    // A table's columns are few, and a schema read once per start-over.
    crate::sql::all_unbounded(
        connection,
        &format!("PRAGMA table_info({table})"),
        (),
        |row| row.text(1),
    )
    .await
}

fn held_elsewhere(error: &Error) -> bool {
    let Error::Engine(engine) = error else {
        return false;
    };
    let said = engine.to_string();
    said.contains("locked by another process") || said.contains("already open")
}

fn as_key_failure(error: Error) -> Error {
    let said = error.to_string().to_lowercase();
    // The engine reports an unauthenticated page as corruption, and a file this build cannot read
    // (the old engine's store) as an unparseable header. Postio writes one key and one format, so a
    // store that will not open is the key or the format, never rot.
    let unreadable = [
        "decryption failed",
        "not a database",
        "invalid page size",
        "database header",
    ];
    if unreadable.iter().any(|hint| said.contains(hint)) {
        Error::WrongStoreKey
    } else {
        error
    }
}

#[cfg(test)]
mod reclaim_policy {
    use super::{RECLAIM_FLOOR, worth_reclaiming};

    const PAGE: u64 = 4096;

    const PER_MIB: u64 = 1024 * 1024 / PAGE;

    #[test]
    fn a_freshly_filled_store_is_left_alone() {
        assert!(!worth_reclaiming(4 * PER_MIB, 800 * PER_MIB, PAGE));
    }

    #[test]
    fn a_wiped_folder_is_worth_it() {
        assert!(worth_reclaiming(400 * PER_MIB, 900 * PER_MIB, PAGE));
    }

    #[test]
    fn a_small_store_that_is_mostly_holes_is_still_left_alone() {
        // Alarming proportion, trivial quantity: not worth blocking writers.
        assert!(!worth_reclaiming(45 * PER_MIB, 50 * PER_MIB, PAGE));
    }

    #[test]
    fn a_huge_store_with_a_thin_slice_free_is_left_alone() {
        assert!(!worth_reclaiming(200 * PER_MIB, 10 * 1024 * PER_MIB, PAGE));
    }

    #[test]
    fn the_floor_is_a_size_rather_than_a_page_count() {
        // Holes are measured in bytes, so larger pages reach the floor sooner.
        let pages = RECLAIM_FLOOR / 8192;
        assert!(worth_reclaiming(pages, pages * 2, 8192));
        assert!(!worth_reclaiming(pages, pages * 2, 4096));
    }

    #[test]
    fn an_empty_store_divides_by_nothing() {
        assert!(!worth_reclaiming(0, 0, PAGE));
    }
}
