//! Opening the database: pragmas, pooling, and the [`Database`] handle.
//!
//! # Why there is a pool at all
//!
//! Postio's runtime is asynchronous and the UI never awaits the network, but it
//! does await SQLite — a message-list page, a search, a flag write. Those run on
//! blocking worker tasks, several at a time, and a `rusqlite::Connection` is
//! `!Sync`. Rather than serializing every query behind one mutex, [`Pool`] hands
//! each worker its own connection and takes it back afterwards. SQLite still
//! serializes *writes* internally; what the pool buys is that a reader never
//! waits for a writer.
//!
//! # Why these pragmas
//!
//! Defaults do not meet the `<100 ms` search and `<16 ms` interaction budgets in
//! CLAUDE.md, and one of them (`foreign_keys`) is off by default in SQLite
//! itself, which would quietly turn every `REFERENCES` clause in the schema into
//! a comment. [`PRAGMAS`] is applied to every connection the pool opens, not
//! just the first — pragmas are per-connection state, so a pool that configured
//! only one of them would be a source of intermittent, unreproducible bugs.
//!
//! ```no_run
//! # fn main() -> Result<(), postio_storage::Error> {
//! let database = postio_storage::Database::open("postio.db")?;
//! let connection = database.connection()?;
//! let unread: i64 = connection.query_row(
//!     "SELECT count(*) FROM messages WHERE seen = 0",
//!     [],
//!     |row| row.get(0),
//! )?;
//! # let _ = unread;
//! # Ok(())
//! # }
//! ```

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use rusqlite::{Connection, OpenFlags};

use crate::error::Result;
use crate::migrations;

/// How many connections a [`Pool`] opens before callers start waiting.
///
/// One writer and a few readers. SQLite serializes writes whatever this number
/// is, so a larger pool buys read concurrency and nothing else, while costing a
/// page cache per connection.
pub const DEFAULT_MAX_CONNECTIONS: usize = 4;

/// The pragmas applied to every connection, in order.
///
/// * `journal_mode = WAL` — readers do not block the writer and the writer does
///   not block readers. This is the pragma the whole local-first design rests
///   on: the UI reads while sync writes.
/// * `synchronous = NORMAL` — under WAL this still cannot corrupt the database,
///   only lose the very last transactions to a power cut. For a cache of mail
///   that lives on a server, that is the right trade against an `fsync` per
///   commit.
/// * `foreign_keys = ON` — off by default in SQLite. The schema's `REFERENCES`
///   clauses and `ON DELETE CASCADE` rules are load-bearing (deleting an account
///   must take its mailboxes and messages with it).
/// * `temp_store = MEMORY` — sorts and temporary indexes stay off the disk.
/// * `mmap_size` — 256 MiB, so page reads for the message list come out of the
///   page cache rather than through `read(2)`.
/// * `cache_size = -16000` — negative means KiB, not pages: 16 MiB regardless of
///   page size.
/// * `busy_timeout` — a writer waiting behind another writer retries for five
///   seconds instead of returning `SQLITE_BUSY` to the UI.
pub const PRAGMAS: &str = "\
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 268435456;
PRAGMA cache_size = -16000;
PRAGMA busy_timeout = 5000;
";

/// Applies [`PRAGMAS`] to a connection.
///
/// The pool does this for every connection it opens; call it directly only for
/// a connection opened outside the pool.
pub fn configure(connection: &Connection) -> Result<()> {
    // `execute_batch` rather than `pragma_update`: several of these return the
    // value they were set to, which `pragma_update` treats as an error.
    connection.execute_batch(PRAGMAS)?;
    Ok(())
}

/// The pragma values actually in force on a connection.
///
/// Reading them back rather than trusting [`configure`] is what lets a test
/// assert the settings, and what makes a bug report legible when a platform's
/// SQLite refuses one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppliedPragmas {
    /// `journal_mode`, lowercased: `wal` for a file, `memory` for an in-memory
    /// database, which has no journal to write.
    pub journal_mode: String,
    /// `synchronous`: `0` OFF, `1` NORMAL, `2` FULL.
    pub synchronous: i64,
    /// Whether foreign keys are enforced on this connection.
    pub foreign_keys: bool,
    /// `temp_store`: `0` default, `1` FILE, `2` MEMORY.
    pub temp_store: i64,
    /// `mmap_size`, in bytes.
    pub mmap_size: i64,
    /// `cache_size`. Negative means KiB of memory; positive means pages.
    pub cache_size: i64,
    /// `busy_timeout`, in milliseconds.
    pub busy_timeout: i64,
}

/// Reads back the pragmas in force on `connection`.
pub fn read_pragmas(connection: &Connection) -> Result<AppliedPragmas> {
    let journal_mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
    Ok(AppliedPragmas {
        journal_mode: journal_mode.to_ascii_lowercase(),
        synchronous: scalar(connection, "PRAGMA synchronous")?,
        foreign_keys: scalar(connection, "PRAGMA foreign_keys")? != 0,
        temp_store: scalar(connection, "PRAGMA temp_store")?,
        mmap_size: scalar(connection, "PRAGMA mmap_size")?,
        cache_size: scalar(connection, "PRAGMA cache_size")?,
        busy_timeout: scalar(connection, "PRAGMA busy_timeout")?,
    })
}

fn scalar(connection: &Connection, pragma: &str) -> Result<i64> {
    Ok(connection.query_row(pragma, [], |row| row.get(0))?)
}

/// Where a pool's connections point.
#[derive(Debug, Clone)]
enum Location {
    /// A database file on disk.
    File(PathBuf),
    /// A private in-memory database, named so that several connections can
    /// share it through SQLite's shared cache.
    Memory(String),
}

impl Location {
    /// A fresh in-memory location, unique to this process and this call.
    ///
    /// `cache=shared` is what makes it one database rather than one per
    /// connection; the counter is what keeps two harnesses in the same test
    /// binary from colliding.
    fn memory() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let serial = NEXT.fetch_add(1, Ordering::Relaxed);
        let process = std::process::id();
        Self::Memory(format!(
            "file:postio-{process}-{serial}?mode=memory&cache=shared"
        ))
    }

    fn open(&self) -> Result<Connection> {
        let connection = match self {
            Self::File(path) => Connection::open(path)?,
            Self::Memory(uri) => Connection::open_with_flags(
                uri,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_URI
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )?,
        };
        configure(&connection)?;
        Ok(connection)
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Memory(_) => None,
        }
    }
}

/// Which kind of writer is asking for SQLite's write lock.
///
/// See [`WriteGate`] for why the distinction has to exist at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePriority {
    /// A write a person is waiting for: a flag, an archive, a draft autosave.
    ///
    /// Always goes ahead of [`WritePriority::Background`], and waits only for
    /// a background unit already in progress.
    Interactive,
    /// Bulk work nobody is watching: a sync pass writing a batch of headers.
    ///
    /// Yields to any interactive writer that is waiting, *before* taking the
    /// lock rather than after — which is the whole point.
    Background,
}

/// Decides who gets SQLite's single write lock next.
///
/// # The problem this exists for (#425)
///
/// SQLite has one writer at a time, even under WAL, and its own way of
/// resolving a collision is [`PRAGMAS`]' `busy_timeout`: the loser sleeps and
/// retries, backing off up to a hundred milliseconds at a time. That is a
/// *timeout*, not a queue — there is no fairness in it and no ordering, and
/// the retrying writer simply races everyone else each time it wakes.
///
/// A first sync is the case where that falls apart. Two sync lanes take turns
/// writing batches back to back, with essentially no gap between one `COMMIT`
/// and the next `BEGIN IMMEDIATE`, so a keystroke's write wakes up, finds the
/// lock taken *again*, and sleeps longer. Measured on the reproduction in
/// `postio-session/tests/interactive_write.rs`: an archive keystroke took
/// **1.8 seconds** to write one row while a backfill ran, with the connection
/// pool almost idle (`Pool::get` returned in two microseconds) — so it was
/// never pool exhaustion, and never the network. Shortening the background
/// transactions does not fix it either: cut to an eighth of their size, the
/// same keystroke still took half a second, because the number of races it
/// had to lose went *up* as each one got shorter.
///
/// So the fix cannot be a bigger pool or a shorter transaction. It has to be
/// an actual queue with a priority in it, which is this.
///
/// # What it guarantees
///
/// A background writer never *begins* a write while an interactive writer is
/// waiting. So an interactive write waits at most for the one background unit
/// already in progress, however long the backfill as a whole runs — which is
/// what turns "wait for the download to finish" into "wait for one batch".
/// Bounding that unit is the other half of the fix, and lives with the sync
/// batch itself.
///
/// # Two rules for callers
///
/// * **Take the pooled connection first, then the permit.** Never the other
///   way round: a thread holding a permit and waiting on [`Pool::get`] can be
///   waiting for a connection held by a thread that is waiting for the permit.
///   Every caller in this workspace acquires in that order.
/// * **One permit at a time per thread.** The gate is not re-entrant, so a
///   permit taken while holding another deadlocks against itself. A permit is
///   meant to wrap one write unit, not to be threaded through a call graph.
///
/// Interactive writers are human-paced, so background work cannot be starved
/// by them in any real workload; the gate deliberately does not try to be
/// fair in that direction.
#[derive(Debug, Clone)]
pub struct WriteGate {
    inner: Arc<GateInner>,
}

#[derive(Debug)]
struct GateInner {
    state: Mutex<GateState>,
    free: Condvar,
}

#[derive(Debug, Default)]
struct GateState {
    /// Whether a permit is outstanding.
    held: bool,
    /// Interactive writers blocked in [`WriteGate::acquire`] right now.
    ///
    /// Counted *before* waiting, which is what lets a background writer see
    /// them and stand aside rather than taking the lock out from under them.
    interactive_waiting: usize,
}

impl WriteGate {
    fn new() -> Self {
        Self {
            inner: Arc::new(GateInner {
                state: Mutex::new(GateState::default()),
                free: Condvar::new(),
            }),
        }
    }

    /// Waits for the right to hold SQLite's write lock, and returns the permit
    /// that carries it. Releasing is dropping the permit.
    ///
    /// Read [`WriteGate`]'s two rules for callers before adding a call site.
    pub fn acquire(&self, priority: WritePriority) -> WritePermit {
        let mut state = self.lock();
        match priority {
            WritePriority::Interactive => {
                state.interactive_waiting += 1;
                while state.held {
                    state = self
                        .inner
                        .free
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                state.interactive_waiting -= 1;
            }
            WritePriority::Background => {
                while state.held || state.interactive_waiting > 0 {
                    state = self
                        .inner
                        .free
                        .wait(state)
                        .unwrap_or_else(PoisonError::into_inner);
                }
            }
        }
        state.held = true;
        WritePermit {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Whether an interactive writer is waiting for the lock right now.
    ///
    /// This is what makes the gate's ordering *observable*, and so testable
    /// without a stopwatch: `postio-storage/tests/write_gate.rs` uses it to
    /// establish that a writer has actually queued before asserting who is
    /// served next. A background writer with a long unit to do could also
    /// consult it to stop between chunks rather than only at its next
    /// acquisition; none does today, because re-acquiring per write unit
    /// already bounds the wait.
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

/// The right to hold SQLite's write lock, released when this is dropped.
///
/// Handed out by [`WriteGate::acquire`].
#[derive(Debug)]
pub struct WritePermit {
    inner: Arc<GateInner>,
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
        // `notify_all`, not `notify_one`: the waiters do not share a predicate
        // — a background writer must also see `interactive_waiting == 0` — so
        // waking a single arbitrary one can wake the only thread that still
        // has to go back to sleep, and leave the lock idle with a queue on it.
        self.inner.free.notify_all();
    }
}

/// A pool of configured connections to one database.
///
/// Cloning a `Pool` is cheap and gives another handle to the *same* pool, which
/// is how a worker thread gets one.
#[derive(Debug, Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

struct Inner {
    location: Location,
    max_connections: usize,
    state: Mutex<State>,
    returned: Condvar,
    /// Who may take SQLite's write lock next. One per database, because that
    /// is the scope of the lock it is arbitrating.
    write_gate: WriteGate,
    /// Something that must live exactly as long as the pool — the temporary
    /// directory a scratch database sits in (`test_support::memory`). `None`
    /// for every real database. Declared last so the connections in `state`
    /// close before whatever this owns is torn down.
    _guard: Option<Box<dyn std::any::Any + Send + Sync>>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("location", &self.location)
            .field("max_connections", &self.max_connections)
            .field("guarded", &self._guard.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct State {
    /// Connections open and not currently checked out.
    idle: Vec<Connection>,
    /// How many connections exist at all, checked out or not.
    live: usize,
    /// For an in-memory database only: the connection that keeps it alive.
    ///
    /// SQLite discards an in-memory database when its last connection closes,
    /// so a pool that let itself go idle would lose the data. This one is opened
    /// with the pool and never handed out.
    /// Never read: holding it open is the whole job.
    _keeper: Option<Connection>,
}

impl Pool {
    fn new(
        location: Location,
        max_connections: usize,
        guard: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<Self> {
        assert!(max_connections > 0, "a pool needs at least one connection");

        let keeper = match location {
            Location::File(_) => None,
            Location::Memory(_) => Some(location.open()?),
        };

        Ok(Self {
            inner: Arc::new(Inner {
                location,
                max_connections,
                state: Mutex::new(State {
                    idle: Vec::new(),
                    live: 0,
                    _keeper: keeper,
                }),
                returned: Condvar::new(),
                write_gate: WriteGate::new(),
                _guard: guard,
            }),
        })
    }

    /// Checks a connection out, waiting if every one is already in use.
    ///
    /// The connection goes back to the pool when the guard is dropped.
    ///
    /// # Errors
    ///
    /// Only if a *new* connection has to be opened and SQLite refuses.
    pub fn get(&self) -> Result<PooledConnection> {
        let mut state = self.lock();
        loop {
            if let Some(connection) = state.idle.pop() {
                return Ok(self.guard(connection));
            }
            if state.live < self.inner.max_connections {
                // Count it before releasing the lock, so two callers racing here
                // cannot both decide there is room for one more.
                state.live += 1;
                drop(state);
                return match self.inner.location.open() {
                    Ok(connection) => Ok(self.guard(connection)),
                    Err(error) => {
                        self.lock().live -= 1;
                        // Someone may be waiting on the slot this failure freed.
                        self.inner.returned.notify_one();
                        Err(error)
                    }
                };
            }
            state = self
                .inner
                .returned
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The largest number of connections this pool will open.
    pub fn max_connections(&self) -> usize {
        self.inner.max_connections
    }

    /// Who may take this database's write lock next — see [`WriteGate`].
    pub fn write_gate(&self) -> &WriteGate {
        &self.inner.write_gate
    }

    /// How many connections exist right now, checked out or idle.
    ///
    /// Connections are opened lazily, so this starts at zero and never exceeds
    /// [`Pool::max_connections`].
    pub fn live_connections(&self) -> usize {
        self.lock().live
    }

    /// How many connections are open and available right now.
    pub fn idle_connections(&self) -> usize {
        self.lock().idle.len()
    }

    /// The database file, or `None` for an in-memory database.
    pub fn path(&self) -> Option<&Path> {
        self.inner.location.path()
    }

    fn guard(&self, connection: Connection) -> PooledConnection {
        PooledConnection {
            inner: Arc::clone(&self.inner),
            connection: Some(connection),
        }
    }

    /// The pool lock, ignoring poisoning.
    ///
    /// A panic while holding the lock leaves the pool's own bookkeeping
    /// consistent — the guard's `Drop` runs during the unwind — so refusing to
    /// hand out connections afterwards would turn one failed test or task into
    /// a dead application.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// A connection borrowed from a [`Pool`], returned when it is dropped.
///
/// Derefs to [`rusqlite::Connection`], so it is used exactly like one.
#[derive(Debug)]
pub struct PooledConnection {
    inner: Arc<Inner>,
    /// `Some` until `Drop` takes the connection back.
    connection: Option<Connection>,
}

impl PooledConnection {
    /// Who may take this database's write lock next — see [`WriteGate`].
    ///
    /// Reachable from the connection rather than only from [`Database`] so
    /// that code handed a connection to write through can arbitrate for the
    /// lock without also being handed the pool. A borrowed connection and the
    /// gate that governs writes on it are then impossible to separate, which
    /// is what keeps a background writer from quietly skipping the queue.
    pub fn write_gate(&self) -> &WriteGate {
        &self.inner.write_gate
    }
}

impl Deref for PooledConnection {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.connection
            .as_ref()
            .expect("a checked-out connection is only taken back on drop")
    }
}

impl DerefMut for PooledConnection {
    fn deref_mut(&mut self) -> &mut Connection {
        self.connection
            .as_mut()
            .expect("a checked-out connection is only taken back on drop")
    }
}

impl Drop for PooledConnection {
    fn drop(&mut self) {
        let Some(connection) = self.connection.take() else {
            return;
        };

        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);

        // A connection left mid-transaction would hand the next caller an open
        // write lock and a half-finished statement. Roll back rather than
        // returning it: a transaction that was not committed was not wanted.
        if connection.is_autocommit() {
            state.idle.push(connection);
        } else {
            let rolled_back = connection.execute_batch("ROLLBACK").is_ok();
            if rolled_back && connection.is_autocommit() {
                state.idle.push(connection);
            } else {
                // Unsalvageable: drop it and let the pool open a fresh one.
                state.live -= 1;
                drop(connection);
            }
        }

        drop(state);
        self.inner.returned.notify_one();
    }
}

/// An open, migrated Postio database.
///
/// This is what the rest of the application holds: it owns the [`Pool`], and
/// opening one is the single call that creates the file, configures it and
/// brings the schema to head.
#[derive(Debug, Clone)]
pub struct Database {
    pool: Pool,
}

impl Database {
    /// Opens (or creates) the database at `path`, creating parent directories,
    /// and migrates it to the latest schema version.
    ///
    /// The parent directory is created (or repaired) `0700` and the database
    /// file `0600`: this is the user's mail, and `$XDG_DATA_HOME` is commonly
    /// world-traversable, so without this everything here inherits whatever
    /// the process umask says. See [`crate::perm`].
    ///
    /// # Errors
    ///
    /// [`Error::Io`](crate::error::Error::Io) if the parent directory cannot be created, or any error
    /// [`migrate`](crate::migrate) can return.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with(path, DEFAULT_MAX_CONNECTIONS)
    }

    /// [`Database::open`], with an explicit pool size.
    pub fn open_with(path: impl AsRef<Path>, max_connections: usize) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            crate::perm::ensure_private_dir(parent)?;
        }
        let database = Self::from_location(Location::File(path.to_path_buf()), max_connections)?;
        // After from_location: that call is what actually creates the file
        // (SQLite opens it lazily, on the pool's first checkout), so there is
        // nothing to tighten before it exists.
        crate::perm::tighten_file(path)?;
        Ok(database)
    }

    /// Opens a private in-memory database, migrated to head.
    ///
    /// Every connection the pool opens sees the *same* database — via
    /// SQLite's `cache=shared` — and it lives exactly as long as this handle
    /// does.
    ///
    /// # Concurrency caveat
    ///
    /// Shared cache brings **table-level locking**: a read transaction on
    /// one pooled connection makes a concurrent write on another fail
    /// immediately with `SQLITE_LOCKED`, and `busy_timeout` does not apply
    /// to that lock (it covers the file lock only). This is fine for a
    /// single-connection caller and wrong for anything that overlaps a
    /// reader and a writer — which is why `test_support::memory` is
    /// file-backed rather than built on this (#204).
    pub fn open_in_memory() -> Result<Self> {
        Self::open_in_memory_with(DEFAULT_MAX_CONNECTIONS)
    }

    /// [`Database::open_in_memory`], with an explicit pool size.
    pub fn open_in_memory_with(max_connections: usize) -> Result<Self> {
        Self::from_location(Location::memory(), max_connections)
    }

    /// [`Database::open`] for a scratch database whose backing storage is
    /// owned by `guard` — the temporary directory `test_support::memory`
    /// creates. The pool keeps the guard alive for as long as any clone of
    /// this handle exists, so a lazily opened connection never finds the
    /// path already deleted.
    ///
    /// Gated the same as its only caller (`test_support`, #489): a build
    /// without the `test-support` feature has no way to reach this and
    /// `dead_code` is right to say so.
    #[cfg(feature = "test-support")]
    pub(crate) fn open_file_with_guard(
        path: &Path,
        guard: Box<dyn std::any::Any + Send + Sync>,
    ) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            crate::perm::ensure_private_dir(parent)?;
        }
        let database = Self::from_location_with_guard(
            Location::File(path.to_path_buf()),
            DEFAULT_MAX_CONNECTIONS,
            Some(guard),
        )?;
        crate::perm::tighten_file(path)?;
        Ok(database)
    }

    fn from_location(location: Location, max_connections: usize) -> Result<Self> {
        Self::from_location_with_guard(location, max_connections, None)
    }

    fn from_location_with_guard(
        location: Location,
        max_connections: usize,
        guard: Option<Box<dyn std::any::Any + Send + Sync>>,
    ) -> Result<Self> {
        let pool = Pool::new(location, max_connections, guard)?;
        let mut connection = pool.get()?;
        migrations::migrate(&mut connection)?;
        drop(connection);
        Ok(Self { pool })
    }

    /// Checks out a connection, waiting if the pool is fully in use.
    pub fn connection(&self) -> Result<PooledConnection> {
        self.pool.get()
    }

    /// The connection pool, for handing to a worker.
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Who may take this database's write lock next — see [`WriteGate`].
    pub fn write_gate(&self) -> &WriteGate {
        self.pool.write_gate()
    }

    /// A connection, plus the right to write through it ahead of bulk
    /// background work. What every user-initiated write should use.
    ///
    /// The connection is taken first and the permit second, which is the
    /// ordering [`WriteGate`] requires of every caller.
    pub fn interactive_write(&self) -> Result<(PooledConnection, WritePermit)> {
        let connection = self.connection()?;
        let permit = self.write_gate().acquire(WritePriority::Interactive);
        Ok((connection, permit))
    }

    /// The database file, or `None` when it is in memory.
    pub fn path(&self) -> Option<&Path> {
        self.pool.path()
    }

    /// The schema version the database is at.
    pub fn schema_version(&self) -> Result<u32> {
        let connection = self.connection()?;
        migrations::schema_version(&connection)
    }

    /// Lets SQLite refresh the statistics its query planner uses.
    ///
    /// Cheap, and worth calling when the application is going idle or shutting
    /// down: `ANALYZE` data that reflects a mailbox of ten messages will plan
    /// badly for one of ten thousand.
    pub fn optimize(&self) -> Result<()> {
        self.connection()?.execute_batch("PRAGMA optimize")?;
        Ok(())
    }
}
