//! Opening the local store, and the connections onto it.
//!
//! # One call
//!
//! [`Store::open`] creates the file and its parent directory, opens it under
//! the key, applies [`crate::schema::HEAD`] if the file is new, and hands back
//! a handle. Everything else in this crate takes a [`Connection`] borrowed
//! from it.
//!
//! # What replaced the pool, and what did not
//!
//! There was a hand-written connection pool here, with a maximum, a checkout
//! path, an idle list and a `Checkout` guard that returned its connection on
//! drop. It went with the engine swap on the belief that the engine keeps a
//! pool of its own behind [`turso::Database::connect`]. **It does not** (#1602):
//! that call builds a new pager with an empty page cache every time, and its
//! shared-cache field is read only for statistics. So a connection is a cold
//! cache over an encrypted file, capped by the `cache_size` pragma the store
//! sets on it and held until the checkout drops -- a first sync held five at
//! 64 MiB, and every list page opened two. What came back is smaller than
//! the old pool and shaped by that fact: [`Store::read`] keeps a few reader
//! connections warm and hands out turns on them; [`Store::connect_background`]
//! gives work nobody waits on a small cache; [`Store::connect`] is for a
//! writer, which wants a connection of its own and drops it when done. A
//! `Connection` is still `Clone`, `Send` and `Sync`, and serialises its own
//! operations internally, so a turn's clone is the same pager and cache.
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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

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

/// Which kind of caller is asking for the store's single writer
/// ([`WriteGate`]).
///
/// The distinction is "is a person waiting on this, right now" — an
/// interactive write goes ahead of a background one. It once named the same
/// distinction for a connection out of the pool too (#672); the pool went
/// with the engine swap, and the enum kept the half that outlived it. See
/// [`WriteGate`] for why it has to exist.
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
/// resolving a collision is the per-connection `busy_timeout`: the loser sleeps and
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
/// * **Take the connection first, then the permit.** Never the other
///   way round: a thread holding a permit and waiting on the pool could be
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
    /// The whole of the gate's state, under a *blocking* mutex.
    ///
    /// Deliberately `std::sync::Mutex` and not the async one: nothing awaits
    /// while it is held, every critical section is a handful of integer
    /// operations, and an async mutex here would cost a task wake-up per
    /// acquisition to protect nothing.
    state: Mutex<GateState>,
    /// Where waiters park. `tokio::sync::Notify`, not a `Condvar`, and that is
    /// the whole of the change this engine forced: a `Condvar` blocks the
    /// *thread*, and a thread in a tokio runtime is a worker. Three writers
    /// waiting on a condvar for a permit the sync pass will release when its
    /// own task next runs is a deadlock with no error message -- there is
    /// nobody left to run the task that releases it.
    free: tokio::sync::Notify,
    /// Told whenever an interactive write finishes. See
    /// [`WriteGate::interactive_writes`].
    interactive_done: tokio::sync::Notify,
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
                free: tokio::sync::Notify::new(),
                interactive_done: tokio::sync::Notify::new(),
            }),
        }
    }

    /// Waits for the right to hold the engine's write lock, and returns the
    /// permit that carries it. Releasing is dropping the permit.
    ///
    /// `async`, and it has to be: see [`GateInner::free`]. A waiter yields its
    /// worker rather than parking it, so the task that will release the permit
    /// can actually run.
    ///
    /// Read [`WriteGate`]'s two rules for callers before adding a call site.
    pub async fn acquire(&self, priority: WritePriority) -> WritePermit {
        // Registered before the first look, not after: a background writer
        // that checks the state in between must already see us, or it takes
        // the lock out from under the interactive writer this exists for.
        if priority == WritePriority::Interactive {
            self.lock().interactive_waiting += 1;
        }
        // The instrument behind the interaction-under-load gate: what this
        // gate did, in order, for a test to count rather than time.
        #[cfg(feature = "test-support")]
        crate::test_support::gate_log::requested(priority);
        loop {
            // The future is created and *enabled* before the state is read,
            // which is what closes the lost-wake-up window: a permit released
            // between the read and the await still counts.
            let waiting = self.inner.free.notified();
            tokio::pin!(waiting);
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

    /// What an interactive write finishing wakes.
    ///
    /// Every action a person takes that the server must hear about -- a flag,
    /// a move, a draft -- is written local-first through an interactive
    /// permit, and its queue row with it. So this is what the sync engine
    /// waits on for queued work, where it used to ask the store every half
    /// second, all day. Enable the `Notified` before checking the queue, or a
    /// write that lands in between is missed.
    pub fn interactive_writes(&self) -> &tokio::sync::Notify {
        &self.inner.interactive_done
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
        // `notify_waiters`, which wakes every one of them, not `notify_one`:
        // the waiters do not share a predicate — a background writer must also
        // see `interactive_waiting == 0` — so waking a single arbitrary one
        // can wake the only task that still has to go back to sleep, and leave
        // the lock idle with a queue on it.
        self.inner.free.notify_waiters();
        if self.priority == WritePriority::Interactive {
            self.inner.interactive_done.notify_waiters();
        }
    }
}

/// The settings every connection needs, because every one of them is per
/// connection and none of them defaults to what Postio wants.
///
/// Measured against a fresh store, which is the only honest way to write this
/// down -- an engine's documented default and its actual one have already
/// differed twice here:
///
/// ```text
/// foreign_keys         0  -> 1
/// temp_store           2     (already MEMORY; asserted anyway, see below)
/// busy_timeout         0  -> 5000
/// cache_size       -2000  -> -65536
/// synchronous          2  -> 1
/// journal_mode       wal     (already; setting it is a query, not an execute)
/// ```
///
/// **`foreign_keys`.** Per connection and off by default, so a connection that
/// skipped this would see every `ON DELETE CASCADE` and `ON DELETE SET NULL`
/// in the schema silently not happen -- deleting an account would leave its
/// mailboxes behind, and a cross-account move would go on naming an account
/// that is gone. Found exactly that way: the saga test asserted the target
/// went NULL and it did not.
///
/// **`temp_store`.** ADR 0014's threat model closes the temp spill explicitly:
/// an encrypted database whose sort scratch lands on disk in the clear has
/// encrypted the wrong thing. This engine already defaults it to MEMORY where
/// SQLite defaults it to DEFAULT, and it is set anyway --
/// `temp_store_is_memory_so_sorts_never_spill_plaintext_to_disk` asserts the
/// value rather than the statement, so a default that changes is caught.
///
/// **`busy_timeout`, and it was the one that mattered.** The engine defaults
/// it to **0**: a writer that finds the lock taken gets `Busy` immediately,
/// with no retry at all. SQLCipher's configuration set 5,000 ms and
/// `WriteGate`'s own documentation is written against that behaviour -- the
/// gate orders Postio's *own* writers, and the timeout is what covers
/// everything it does not. Without it
/// `a_resync_batch_does_not_lock_out_an_interactive_write` fails as
/// `database is locked`, which is exactly the symptom a person would see on
/// a keystroke during a sync.
///
/// **`cache_size`.** -2000 is two megabytes; -65536 is the 64 MiB the old
/// store used. A cap rather than a reservation -- the cache grows lazily, so
/// a small store never allocates it.
///
/// **`synchronous = NORMAL`.** FULL fsyncs on every commit, which under WAL
/// buys durability against power loss at a cost paid on every flag change.
/// NORMAL is the WAL-appropriate setting and what this store has always used.
const PER_CONNECTION: &str = "\
PRAGMA foreign_keys = ON;
PRAGMA temp_store = 2;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -65536;
PRAGMA synchronous = 1;
";

/// [`PER_CONNECTION`] for a connection that only ever writes through, or
/// reads once and is dropped: a sync lane, a body fetch, an indexer batch.
///
/// The one difference is the cache. A lane writes headers across the whole
/// file, so its cache fills to the cap with clean pages nothing reads
/// again, and the engine keeps one cache **per connection** (#1602): a
/// first sync held five such connections for its whole length, ~300 MB of
/// the heap peak that bought nothing. Four mebibytes covers the working set
/// of a unit's writes and their index pages; the interactive cache stays
/// where a person's reads are.
const PER_BACKGROUND_CONNECTION: &str = "\
PRAGMA foreign_keys = ON;
PRAGMA temp_store = 2;
PRAGMA busy_timeout = 5000;
PRAGMA cache_size = -4096;
PRAGMA synchronous = 1;
";

/// How many long-lived connections [`Store::read`] keeps for reads.
///
/// Three: a page read, a sidebar refresh and a search can overlap, and a
/// fourth caller waits its turn rather than paying a cold cache of its own.
/// Each holds up to the interactive `cache_size`, so this is also the cap on
/// what warm reads may hold: 3 x 64 MiB, filled only by what was read.
const READERS: usize = 3;

/// The least the holes must come to before a reclaim is worth blocking a
/// writer for. See [`worth_reclaiming`].
const RECLAIM_FLOOR: u64 = 64 * 1024 * 1024;

/// And the least share of the file they must be. See [`worth_reclaiming`].
const RECLAIM_FRACTION: f64 = 0.25;

/// Whether a database of `total_pages` with `free_pages` of holes is worth
/// rewriting.
///
/// A function of three numbers rather than a method on a store, so the policy
/// can be argued with at the magnitudes that matter — a ten-gigabyte archive,
/// a wiped folder — without seeding one. [`Store::is_worth_reclaiming`] reads
/// the three pragmas and asks this.
///
/// Both conditions have to hold, and they guard different mistakes. The
/// **floor** stops a rewrite that recovers a few megabytes: the reclaim blocks
/// every writer for its whole duration, and nothing should pay that for a
/// rounding error. The **fraction** stops a rewrite whose cost is the entire
/// file and whose gain is one per cent of it — the cost scales with
/// `total_pages` and the gain only with `free_pages`.
pub fn worth_reclaiming(free_pages: u64, total_pages: u64, page_size: u64) -> bool {
    if total_pages == 0 {
        return false;
    }
    let free_bytes = free_pages.saturating_mul(page_size);
    free_bytes >= RECLAIM_FLOOR && free_pages as f64 / total_pages as f64 >= RECLAIM_FRACTION
}

/// How many connections this process has opened on any store.
///
/// Process-wide, like `postio_runtime`'s folder-count counter and for the
/// same reason: a checkout is made on whichever thread does the work, and a
/// thread-local read from a test would answer zero. Each one is a fresh
/// engine pager with an empty page cache (#1602), so this is the number a
/// budget is written in when the question is "how many times did a page
/// cost its own cache". Read through `test_support::counting::checkouts`.
pub(crate) static CHECKOUTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The store: a database handle and the path it came from.
///
/// Cheap to clone — the engine's handle is an `Arc` inside — so it is passed
/// by value rather than behind another layer of sharing.
#[derive(Clone)]
pub struct Store {
    database: turso::Database,
    path: Option<PathBuf>,
    gate: WriteGate,
    /// The long-lived reader connections, taken in turns. See [`Store::read`].
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
            readers: Arc::new(Readers::new()),
        };

        if fresh {
            crate::perm::tighten_file(path)?;
            store.apply_schema().await?;
        } else {
            store.prove_the_key_fits().await?;
            store.prove_the_schema_matches().await?;
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
            // `VACUUM` is the only page reclaim this engine offers -- there
            // is no `auto_vacuum` toggle on this builder -- and the flag only
            // lets the statement parse. What decides whether one ever runs is
            // [`Store::is_worth_reclaiming`].
            .experimental_vacuum(true)
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
        // Stamped in the same breath as the schema it describes, so the two
        // cannot be written apart. See `prove_the_schema_matches`.
        crate::sql::execute(
            &connection,
            &format!("PRAGMA user_version = {}", schema::FINGERPRINT),
            (),
        )
        .await?;
        Ok(())
    }

    /// Refuses a store whose schema is not the one this build compiles
    /// against.
    ///
    /// [`prove_the_key_fits`](Self::prove_the_key_fits) answers "can this file
    /// be read at all", and a store written by an *earlier build of this
    /// engine* passes it: same cipher, same key, same file format. So it opens,
    /// and then fails one statement at a time on whatever column has been
    /// added since. Met on 2026-09-17 against a store two hours older than
    /// `messages.body_parsed_with`, which opened, synced, and warned
    /// `no such column` once per folder for as long as it ran.
    ///
    /// There are no migrations ([`schema::HEAD`] argues why), so this cannot
    /// repair anything and does not try. It refuses, names the remedy, and
    /// leaves the file alone — "rebuilt by resyncing" means the old one has to
    /// survive being refused.
    async fn prove_the_schema_matches(&self) -> Result<()> {
        let connection = self.connect_bare()?;
        let found: i64 = crate::sql::scalar(&connection, "PRAGMA user_version", ()).await?;
        if found == schema::FINGERPRINT {
            return Ok(());
        }
        Err(Error::SchemaFromAnotherBuild {
            found,
            expected: schema::FINGERPRINT,
        })
    }

    /// How many bytes the file is holding that nothing is using.
    ///
    /// `freelist_count * page_size`: pages inside the database that a delete
    /// freed and that the filesystem has not been given back.
    pub async fn free_bytes(&self) -> Result<u64> {
        let connection = self.connect().await?;
        let free = crate::sql::scalar(&connection, "PRAGMA freelist_count", ()).await?;
        let page = crate::sql::scalar(&connection, "PRAGMA page_size", ()).await?;
        Ok((free.max(0) as u64).saturating_mul(page.max(0) as u64))
    }

    /// Whether reclaiming those pages would be worth what it costs.
    ///
    /// Two conditions, and both have to hold.
    ///
    /// **At least [`RECLAIM_FLOOR`] of holes**, because the reclaim is a full
    /// rewrite and a rewrite that recovers a few megabytes is not worth
    /// blocking a writer for.
    ///
    /// **And at least [`RECLAIM_FRACTION`] of the file**, because the cost
    /// scales with the *whole* database and the gain only with the holes. A
    /// ten-gigabyte store with a hundred megabytes free would spend minutes to
    /// recover one per cent.
    ///
    /// # Why this is not simply "every start"
    ///
    /// Freed pages are **reused**. Measured: a 20,000-message store at
    /// 12.9 MiB, with 18,000 messages deleted and 18,000 put back, came to
    /// 12.9 MiB -- no growth at all. So a store does not creep upward; it
    /// plateaus at the largest it ever had to be, and a reclaim is worth
    /// something only after a large one-off deletion. A `UIDVALIDITY` reset
    /// wiping a folder is the case #381 was written for.
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

    /// Rewrite the database without its holes, and answer how many bytes that
    /// gave back.
    ///
    /// # What this costs, measured
    ///
    /// A full `VACUUM`, because this engine offers no incremental one. On this
    /// workstation it rewrites at roughly 25 MiB/s, and **it blocks every
    /// other writer for its whole duration** -- a keystroke's write issued
    /// during a 2.80 s reclaim waited 2,831 ms for it. On a store the size of
    /// ADR 0017's reference mailbox that is most of a minute of frozen writes,
    /// which is why [`is_worth_reclaiming`](Self::is_worth_reclaiming) gates
    /// it and why the caller is the housekeeping worker rather than anything
    /// a person is waiting on.
    ///
    /// Peak disk is about 1.1x the file, not the 2x a copy-and-swap would
    /// need.
    ///
    /// # Being interrupted is safe
    ///
    /// Measured by killing the process at six points across a 1.2 s reclaim of
    /// a 27 MB store: at 0.3, 0.6, 0.9, 1.0 and 1.1 seconds the file came back
    /// **byte for byte identical** to the original and every row read; at 1.2
    /// it had completed and reclaimed 88%. There is no half-written state to
    /// recover from, which is what makes this safe to start without a window
    /// asking permission first.
    ///
    /// # The permit
    ///
    /// Taken at [`WritePriority::Background`], so this cannot *begin* while an
    /// interactive writer is queued. It cannot yield once begun -- a rewrite
    /// is not interruptible -- and that is the whole reason for the gate.
    pub async fn reclaim_free_pages(&self) -> Result<u64> {
        let before = self.file_bytes();
        let connection = self.connect().await?;
        let _permit = self.gate.acquire(WritePriority::Background).await;
        crate::sql::execute(&connection, "VACUUM", ()).await?;
        drop(_permit);
        drop(connection);
        // The rewrite lands in the log; the file on disk only shrinks once
        // that is folded back in, and reporting the number before then would
        // report nothing.
        self.truncate_log().await?;
        Ok(before.saturating_sub(self.file_bytes()))
    }

    /// The database file's size, or 0 for a store with no path.
    fn file_bytes(&self) -> u64 {
        self.path
            .as_ref()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len())
            .unwrap_or(0)
    }

    /// A fresh connection onto the store, with foreign keys on and the
    /// interactive page cache.
    ///
    /// **Not pooled, and not cheap in the way this used to say.** The engine
    /// builds a new pager with an empty page cache for every connection
    /// (`turso_core::Database::connect`; its shared-cache field is read only
    /// for statistics), so each checkout starts cold over an encrypted file
    /// and holds up to `cache_size` of what it touched until it is dropped.
    /// A read that is one of many should take [`Store::read`], which keeps a
    /// few connections warm; a writer takes this, because a writer wants a
    /// connection of its own and drops it when the unit is done; anything
    /// that only writes through takes [`Store::connect_background`].
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
        self.connect_with(PER_CONNECTION).await
    }

    /// A fresh connection for work nobody is waiting on: a sync lane, a body
    /// fetch, an indexer batch. The same as [`Store::connect`] with a small
    /// page cache; see [`PER_BACKGROUND_CONNECTION`] for why.
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

    /// A turn on one of a few long-lived reader connections.
    ///
    /// For reads that come in numbers -- list pages, sidebar refreshes,
    /// searches -- where a connection of their own would mean a cold page
    /// cache each time (#1602: a list page opened two, and paid the pragmas
    /// and every page's decrypt twice per page). At most [`READERS`] callers
    /// hold a turn at once; the next waits, briefly, rather than opening a
    /// cache nothing else will ever hit. The connection goes back when the
    /// [`Reader`] is dropped, its cache intact for the next caller.
    ///
    /// Readers only. A writer takes [`Store::connect`] or
    /// [`Store::interactive_write`]: a transaction left open on a shared
    /// connection would be the next reader's problem.
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

    /// A connection and an interactive write permit, together.
    ///
    /// The pairing is the point: a write a person is waiting for has to take
    /// the permit *before* it takes the engine's writer, and the two calls
    /// being separate is what makes the wrong order possible. Every
    /// local-first verb in the application goes through here.
    pub async fn interactive_write(&self) -> Result<(Checkout, WritePermit)> {
        let permit = self.gate.acquire(WritePriority::Interactive).await;
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
        // and the engine refuses a statement whose rows nobody reads. Rare
        // enough that compiling it each time is nothing.
        #[allow(clippy::disallowed_methods)]
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

/// The reader connections a store keeps warm. See [`Store::read`].
struct Readers {
    /// Who may hold a reader right now: [`READERS`] turns.
    turns: Arc<tokio::sync::Semaphore>,
    /// Connections nobody is using, cache and all.
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

/// One turn on a long-lived reader connection. See [`Store::read`].
///
/// Dereferences to the [`Checkout`] for the length of the turn; dropping it
/// hands the connection back. [`Reader::checkout`] gives a clone of the same
/// connection for a caller whose closure takes one by value -- the engine's
/// handle is an `Arc`, so a clone is the same pager and the same cache --
/// and the turn still ends when the `Reader` is dropped, so keep it until
/// the work on the clone is done.
pub struct Reader {
    checkout: Option<Checkout>,
    home: Arc<Readers>,
    _turn: tokio::sync::OwnedSemaphorePermit,
}

impl Reader {
    /// The connection this turn holds, cloned. See the type's docs.
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

/// A connection, and the gate that says who writes next.
///
/// # Why the gate travels with the connection
///
/// Because the alternative is remembering to fetch it. This is what
/// `Checkout` was, minus the pooling the engine now does itself:
/// [`Deref`](std::ops::Deref) to the connection, so it is used exactly like one, with
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
    // Four spellings of the same thing, and the fourth is why this list grew.
    // The engine reports a page it cannot authenticate as corruption, because
    // from inside it that is indistinguishable -- and a *file this build has
    // never been able to read*, like a store the old engine wrote, surfaces
    // instead as a header it cannot parse: "invalid page size in database
    // header: 30639", which is ciphertext being read as a page size.
    //
    // Postio can tell what the engine cannot: it writes one key per store and
    // one format, so a store that will not open is the key or the format, and
    // never a disk that has rotted. Both have the same remedy and
    // `Error::WrongStoreKey` says it.
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

    /// 4 KiB, which is what this engine reports on a store it created.
    const PAGE: u64 = 4096;

    /// Pages in one mebibyte, to keep the cases below readable as sizes.
    const PER_MIB: u64 = 1024 * 1024 / PAGE;

    #[test]
    fn a_freshly_filled_store_is_left_alone() {
        // 800 MiB with 4 MiB of holes: the shape of ordinary use, where a
        // delete's pages are reused by the next message rather than
        // accumulating.
        assert!(!worth_reclaiming(4 * PER_MIB, 800 * PER_MIB, PAGE));
    }

    #[test]
    fn a_wiped_folder_is_worth_it() {
        // A `UIDVALIDITY` reset on a large folder: 400 MiB of a 900 MiB store
        // freed at once, which is the case #381 was written for.
        assert!(worth_reclaiming(400 * PER_MIB, 900 * PER_MIB, PAGE));
    }

    #[test]
    fn a_small_store_that_is_mostly_holes_is_still_left_alone() {
        // 90% free, and 45 MiB. The proportion is alarming and the quantity is
        // not: rewriting to recover this would cost every writer the length of
        // the rewrite to save less than a photograph.
        assert!(!worth_reclaiming(45 * PER_MIB, 50 * PER_MIB, PAGE));
    }

    #[test]
    fn a_huge_store_with_a_thin_slice_free_is_left_alone() {
        // 10 GiB with 200 MiB free: over the floor, and 2% of the file. The
        // rewrite's cost is the whole ten gigabytes.
        assert!(!worth_reclaiming(200 * PER_MIB, 10 * 1024 * PER_MIB, PAGE));
    }

    #[test]
    fn the_floor_is_a_size_rather_than_a_page_count() {
        // The same number of free pages, at two page sizes. A store with
        // larger pages reaches the floor sooner, which is the point of
        // measuring the holes in bytes: what is being recovered is disk.
        let pages = RECLAIM_FLOOR / 8192;
        assert!(worth_reclaiming(pages, pages * 2, 8192));
        assert!(!worth_reclaiming(pages, pages * 2, 4096));
    }

    #[test]
    fn an_empty_store_divides_by_nothing() {
        assert!(!worth_reclaiming(0, 0, PAGE));
    }
}
