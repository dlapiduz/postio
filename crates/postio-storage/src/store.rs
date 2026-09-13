//! Opening the local store, and the connections onto it.
//!
//! # One call
//!
//! [`Store::open`] creates the file and its parent directory, opens it under
//! the key, applies [`crate::schema::HEAD`] if the file is new, and hands back
//! a handle. Everything else in this crate takes a [`Connection`] borrowed
//! from it.
//!
//! # What replaced the pool
//!
//! There was a hand-written connection pool here, with a maximum, a checkout
//! path, an idle list and a `PooledConnection` guard that returned its
//! connection on drop. It is gone: the engine keeps its own pool behind
//! [`turso::Database::connect`], which is why that call is cheap, infallible
//! in practice, and not `async`. A `Connection` is `Clone`, `Send` and `Sync`,
//! and serialises its own operations internally — so the thing the old pool
//! was protecting is protected a layer down.
//!
//! What is *not* a layer down is [`WriteGate`], and that is a different
//! question: not "may two writes run at once" but "when a background sync and
//! a person's keystroke both want the writer, who goes first". See its
//! documentation.
//!
//! # Everything is async
//!
//! The engine is async to the bottom, so this crate is too. The old shape --
//! synchronous repositories reached from tokio through `spawn_blocking` --
//! is gone, and with it the thread pool that shape needed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use turso::Builder;

use crate::error::{Error, Result};
use crate::key::Subkey;
use crate::schema;

pub use turso::{Connection, Value};

/// How many passes may read and write the store at once.
///
/// There is no connection pool to exhaust any more -- the engine keeps its
/// own, and `connect` is cheap. This is the *concurrency* the old pool's size
/// was standing in for, and it is still a real limit: every concurrent sync
/// pass contends for one writer, and the UI thread reads through the same
/// store. The number is what it was.
pub const MAX_CONCURRENT_PASSES: usize = 4;

/// The cipher the store is written under.
///
/// AES-256-GCM: authenticated, and the one mode here with hardware support on
/// every CPU Postio targets. The engine also offers AEGIS, which is faster on
/// paper and much less deployed; this is the local mail store, not a
/// benchmark.
///
/// **Both the encryption and the engine are pre-1.0 and unaudited.** That is
/// why `store_is_unreadable_without_the_key` and `another_key_is_refused` are
/// acceptance criteria in `specs/004-turso-store/spec.md` and tests in this
/// crate, rather than properties taken on trust.
pub const CIPHER: &str = "aes256gcm";

/// Which kind of caller is asking — for SQLite's write lock ([`WriteGate`]),
/// or for a connection out of the [`Pool`] itself (#672).
///
/// One enum for both: they are the same distinction — "is a person waiting
/// on this, right now" — applied to two different contended resources, and a
/// caller declares it once rather than choosing a name per resource. See
/// [`WriteGate`] and [`Pool::get_interactive`] for why each has to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WritePriority {
    /// Work a person is waiting for: a flag, an archive, a draft autosave, a
    /// reading-pane body.
    ///
    /// Always goes ahead of [`WritePriority::Background`], and waits only for
    /// a background unit already in progress.
    Interactive,
    /// Bulk work nobody is watching: a sync pass writing a batch of headers,
    /// or reading one to sync it.
    ///
    /// Yields to any interactive caller that is waiting, *before* taking the
    /// lock or the connection rather than after — which is the whole point.
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

/// The store: a database handle and the path it came from.
///
/// Cheap to clone — the engine's handle is an `Arc` inside — so it is passed
/// by value rather than behind another layer of sharing.
#[derive(Clone)]
pub struct Store {
    database: turso::Database,
    path: Option<PathBuf>,
    gate: WriteGate,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open the store at `path`, creating it if it is not there.
    ///
    /// The parent directory is created too, with the same restricted
    /// permissions the store itself gets — a mail store is not world-readable
    /// even for the instant between `create` and `chmod`.
    pub async fn open(path: impl AsRef<Path>, key: &Subkey) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::perm::ensure_private_dir(parent)?;
        }

        let fresh = !path.exists();
        let database = Self::build(path, key)
            .await
            .map_err(|error| if fresh { error } else { as_key_failure(error) })?;
        let store = Self {
            database,
            path: Some(path.to_path_buf()),
            gate: WriteGate::new(),
        };

        if fresh {
            crate::perm::tighten_file(path)?;
            store.apply_schema().await?;
        } else {
            store.prove_the_key_fits().await?;
        }

        Ok(store)
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
            // The schema uses all three, and each is behind its own flag in
            // this release. `index_method` is what `CREATE INDEX ... USING
            // fts` is; without it the search index is a syntax error.
            .experimental_triggers(true)
            .experimental_index_method(true)
            .experimental_generated_columns(true)
            .build()
            .await;
        drop(hexkey);
        built.map_err(Into::into)
    }

    /// Read one page under the key, so a key that does not fit is discovered
    /// here rather than in the middle of the first query a person asked for.
    ///
    /// The engine reports a failed page authentication in the vocabulary of
    /// corruption — it cannot tell a wrong key from a damaged file, because
    /// they present identically. Postio can: it only ever writes one key per
    /// store, so the overwhelmingly likelier of the two is the key, and
    /// [`Error::WrongStoreKey`] says so in a sentence that is actionable
    /// (#404).
    async fn prove_the_key_fits(&self) -> Result<()> {
        let connection = self.connect_bare()?;
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
        connection.execute("PRAGMA foreign_keys = OFF", ()).await?;
        connection.execute_batch(schema::HEAD).await?;
        Ok(())
    }

    /// A connection onto the store, with foreign keys on.
    ///
    /// Cheap: the engine pools these itself, and the returned handle is
    /// `Clone + Send + Sync`. Make one per unit of work rather than holding
    /// one open across awaits that do not touch the database.
    ///
    /// # Why this is `async` when the engine's own `connect` is not
    ///
    /// Because of the pragmas. **Foreign keys are per connection and default
    /// to off**, so a connection that skipped this would see every `ON DELETE
    /// CASCADE` and `ON DELETE SET NULL` in the schema silently not happen --
    /// deleting an account would leave its mailboxes, and a cross-account move
    /// would go on naming an account that is gone. Found exactly that way: the
    /// saga test asserted the target went NULL and it did not.
    ///
    /// Paying an `async` on every checkout to make that impossible is the
    /// right trade. The alternative -- a `connect_raw` for callers who know
    /// better -- is an invitation to be wrong quietly.
    pub async fn connect(&self) -> Result<Checkout> {
        let connection = self.database.connect()?;
        connection.execute("PRAGMA foreign_keys = ON", ()).await?;
        // ADR 0014's threat model closes the temp spill explicitly: an
        // encrypted database whose sort scratch lands on disk in the clear
        // has encrypted the wrong thing. The engine defaults this to 0
        // (DEFAULT), not 2 (MEMORY) -- checked, not assumed, and
        // `temp_store_is_memory_so_sorts_never_spill_plaintext_to_disk` is
        // what keeps it checked.
        connection.execute("PRAGMA temp_store = 2", ()).await?;
        Ok(Checkout {
            connection,
            gate: self.gate.clone(),
        })
    }

    /// A connection and an interactive write permit, together.
    ///
    /// The pairing is the point: a write a person is waiting for has to take
    /// the permit *before* it takes the engine's writer, and the two calls
    /// being separate is what makes the wrong order possible. Every
    /// local-first verb in the application goes through here.
    pub async fn interactive_write(&self) -> Result<(Checkout, WritePermit)> {
        let permit = self.gate.acquire(WritePriority::Interactive);
        let connection = self.connect().await?;
        Ok((connection, permit))
    }

    /// Who gets the writer next, when two callers want it.
    ///
    /// Machine-wide for this store: one gate, cloned into every checkout.
    pub fn write_gate(&self) -> &WriteGate {
        &self.gate
    }

    /// A connection with nothing configured on it.
    ///
    /// Only for opening: [`apply_schema`](Self::apply_schema) needs foreign
    /// keys *off* while it runs, because the schema declares tables
    /// alphabetically and a key routinely names a table that does not exist
    /// yet.
    fn connect_bare(&self) -> Result<Connection> {
        self.database.connect().map_err(Into::into)
    }

    /// Truncate the write-ahead log, returning what it was before.
    ///
    /// # Why this is a call and not a setting
    ///
    /// #1175 bounded the WAL with `PRAGMA journal_size_limit`: a ceiling the
    /// engine enforced at every checkpoint, set once per connection. The live
    /// install had reached a **676 MB** WAL against an 868 MB database, and
    /// paid for it on every launch -- the WAL index is rebuilt before the
    /// first row can be read, and that sits in front of the first frame.
    ///
    /// This engine has no `journal_size_limit`. It does have
    /// `wal_checkpoint`, so the ceiling becomes a sweep: the housekeeping
    /// worker calls this, off the startup path and off the interaction path,
    /// and the log goes back to nothing.
    ///
    /// It is a weaker guarantee than a limit the engine enforces itself --
    /// a session that never reaches housekeeping never truncates -- and it is
    /// the mechanism available.
    pub async fn truncate_log(&self) -> Result<u64> {
        let before = self
            .path
            .as_ref()
            .map(|path| path.with_extension("db-wal"))
            .and_then(|wal| std::fs::metadata(wal).ok())
            .map(|meta| meta.len())
            .unwrap_or(0);

        let connection = self.connect().await?;
        // A query, not an `execute`: it answers with (busy, log, checkpointed)
        // and the engine refuses a statement whose rows nobody reads.
        let mut rows = connection
            .query("PRAGMA wal_checkpoint(TRUNCATE)", ())
            .await?;
        let _ = rows.next().await?;
        drop(rows);
        Ok(before)
    }

    /// Where the store lives, or `None` for one that is not on disk.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// A connection, and the gate that says who writes next.
///
/// # Why the gate travels with the connection
///
/// Because the alternative is remembering to fetch it. This is what
/// `PooledConnection` was, minus the pooling the engine now does itself:
/// [`Deref`] to the connection, so it is used exactly like one, with
/// [`write_gate`](Self::write_gate) beside it for the callers that are about
/// to write and have to say on whose behalf.
///
/// A background writer that forgets to take a permit does not fail -- it just
/// makes a person wait, somewhere else, for a reason that never appears in a
/// log. Carrying the gate is what keeps that from being a thing to remember.
#[derive(Debug, Clone)]
pub struct Checkout {
    connection: Connection,
    gate: WriteGate,
}

impl Checkout {
    /// Who gets the writer next. See [`WriteGate`].
    pub fn write_gate(&self) -> &WriteGate {
        &self.gate
    }

    /// The connection itself, for a caller that wants to hold one past this
    /// handle's lifetime.
    pub fn into_connection(self) -> Connection {
        self.connection
    }
}

impl std::ops::Deref for Checkout {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        &self.connection
    }
}

/// `DerefMut` because [`Connection::transaction`] takes `&mut self`.
///
/// It does not mutate anything a caller can observe -- the engine's own
/// transaction guard needs the exclusive borrow to make a second overlapping
/// transaction on one connection a compile error rather than a runtime one,
/// which is the same guarantee this crate wants.
impl std::ops::DerefMut for Checkout {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.connection
    }
}

/// Translate a failed page authentication into the sentence a person can act
/// on.
///
/// The engine decrypts page 1 to find out whether the file is a database at
/// all, so a wrong key and a damaged file arrive here as the same error:
/// "Decryption failed for page=1". They are not equally likely. Postio writes
/// exactly one key per store and takes it from the keyring, so the way this
/// happens in practice is a keyring entry that was replaced or belongs to
/// another installation -- and [`Error::WrongStoreKey`] says that, and says
/// the mail is intact, which the engine's own wording does not (#404).
///
/// Anything else passes through untouched: an unreadable file, a full disk
/// and a directory that is not writable are all still themselves.
fn as_key_failure(error: Error) -> Error {
    let said = error.to_string().to_lowercase();
    if said.contains("decryption failed") || said.contains("not a database") {
        Error::WrongStoreKey
    } else {
        error
    }
}
