//! Turning a plaintext store into an encrypted one, without losing mail.
//!
//! ADR 0014 Q4. New stores encrypt from first open; this is for the handful
//! that existed before they did. It runs once, at open, and after it has run
//! there is nothing plaintext left to find.
//!
//! # The ordering is the whole design
//!
//! Every step here is arranged so that a process killed at any point between
//! two of them loses nothing:
//!
//! 1. **Drain first.** The queue and the drafts are the only things in a store
//!    that are not a copy of something on a server. Everything else can be
//!    refetched, so everything else is safe to rebuild; the queue is not, and
//!    a migration that ran with operations still pending could drop a move
//!    the server never heard about. This refuses rather than deciding for
//!    somebody ([`crate::Error::QueueNotDrained`]).
//! 2. **Build beside, never in place.** The encrypted database and the
//!    re-encrypted blobs are assembled in a staging directory. Nothing the
//!    user has is touched, so a death here costs only the work.
//! 3. **Verify before replacing.** The staged store is opened, checked and
//!    read back — every blob the database references, through the AEAD, so
//!    the check is the tag rather than a file's existence. A staged store
//!    that does not verify is deleted and the plaintext one is left alone.
//! 4. **Swap last.** Only then do the originals move aside and the staged
//!    store move in, and only after *that* are the originals deleted.
//!
//! # Resuming
//!
//! The swap is several renames, and there is no filesystem call that does
//! several renames at once. What makes it safe is that the swap is
//! *idempotent*: [`resume`] is the same sequence of moves, each guarded by
//! whether it has already happened, so running it from any interruption point
//! converges on the same finished store. The presence of the aside directory
//! is what says a swap was in progress, and it is created only once the staged
//! store has verified.
//!
//! `stopping_after` (behind the `test-support` feature) is how the kill test
//! gets at those points: returning early from a stage leaves exactly the
//! on-disk state a killed process leaves, because nothing here publishes
//! anything from a destructor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use postio_model::BlobId;
use rusqlite::Connection;

use crate::blob::BlobStore;
use crate::db::Database;
use crate::error::{Error, Result};
use crate::key::{BlobKeys, Purpose, StoreKey};

/// The directory the encrypted store is assembled in.
///
/// Beside the store rather than in `/tmp`: the swap has to be a rename, and a
/// rename does not cross filesystems.
const STAGING: &str = ".postio-encrypting";

/// The directory the plaintext store is moved to before it is deleted.
///
/// Its existence is what tells the next open that a swap was interrupted.
const ASIDE: &str = ".postio-plaintext";

/// What an encrypted SQLite file does not begin with.
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

/// The files that move as one when the store is swapped.
///
/// The write-ahead log and its shared-memory index are named because the
/// database's newest pages can still be in them: moving `postio.db` and
/// leaving `postio.db-wal` behind would be moving a database back in time.
const DATABASE_PARTS: [&str; 3] = ["", "-wal", "-shm"];

/// What the blob directory is called, beside the database.
///
/// The same answer `postio_session::open_store_at` reaches by
/// `with_file_name`; here rather than passed in, because a migration that
/// re-encrypted a *different* directory from the one the store will open is a
/// bug with no symptom until the mail is gone.
const BLOBS: &str = "blobs";

/// What a migration did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// There is no store at this path yet. A first run.
    NoStore,
    /// The store is already encrypted; nothing to do. The ordinary answer on
    /// every open after the first.
    AlreadyEncrypted,
    /// A plaintext store was replaced with an encrypted one.
    Encrypted(Report),
    /// A swap that had been interrupted was carried to the end.
    Resumed,
}

/// What was re-encrypted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// How many blobs were re-encrypted under new, keyed ids.
    pub blobs: usize,
    /// How many bytes of plaintext blob content passed through.
    pub bytes: u64,
}

/// A point a migration can be stopped at, to prove that stopping there is
/// survivable.
///
/// Ordered as the migration runs. Nothing in a shipping build takes one:
/// [`stopping_after`], behind the `test-support` feature, is the only thing
/// that does. It is spelled out here anyway because these are the points the
/// ordering at the top of this module is *about*, and a reader looking for
/// "what can this be interrupted between" should find the list rather than
/// count `rename` calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// The staging directory holds a complete encrypted store, unverified.
    Staged,
    /// The staged store has been read back and is good. Nothing has moved.
    Verified,
    /// The plaintext database has moved aside; its blobs have not.
    DatabaseAside,
    /// Both halves of the plaintext store have moved aside. The store path
    /// holds nothing at all — the narrowest window there is.
    OriginalsAside,
    /// The encrypted database is in place; its blobs are still in staging.
    DatabaseInPlace,
    /// Both halves are in place. Only deleting the plaintext copy is left.
    Swapped,
}

/// Encrypts the store at `database`, if it is not encrypted already.
///
/// `database` is the database file's path; the blob store is the `blobs`
/// directory beside it, which is where [`crate::BlobStore`] is opened from.
///
/// Safe to call on every open, and meant to be: the answer for a store that
/// is already encrypted is [`Outcome::AlreadyEncrypted`] and no work.
///
/// # Errors
///
/// [`Error::QueueNotDrained`] if operations are still waiting for the server —
/// the caller is expected to drain and try again. [`Error::MigrationDidNotVerify`]
/// if the encrypted store it built did not read back; in that case nothing was
/// replaced and the plaintext store is exactly as it was.
pub fn encrypt_store(database: &Path, master: &StoreKey) -> Result<Outcome> {
    run(database, master, None)
}

/// [`encrypt_store`], stopped after `stage`.
///
/// Behind the `test-support` feature, so it is not in a shipping build. It
/// exists because "a migration that dies half-way loses nothing" is a claim
/// about points *between* filesystem calls, and the only way to assert it is
/// to stand at each one.
///
/// `#[doc(hidden)]` because it is the codebase's mark for "a test reaches
/// this and nothing else should" — which is also what
/// `check-uncalled-pub-fn.py` reads. It would otherwise be flagged: that
/// check blanks string literals before it looks for `#[cfg(test)]`, so
/// `#[cfg(feature = "test-support")]` does not read as test scaffolding to
/// it, and the only caller is an integration test, which it deliberately
/// does not count as one.
#[doc(hidden)]
#[cfg(feature = "test-support")]
pub fn stopping_after(database: &Path, master: &StoreKey, stage: Stage) -> Result<Outcome> {
    run(database, master, Some(stage))
}

/// Whether the migration should stop now, having just finished `reached`.
fn stop_at(stop: Option<Stage>, reached: Stage) -> bool {
    stop == Some(reached)
}

fn run(database: &Path, master: &StoreKey, stop: Option<Stage>) -> Result<Outcome> {
    let layout = Layout::around(database);

    // An interrupted swap comes first, before anything looks at what the store
    // holds: half of what it holds is still in the staging directory.
    if layout.aside.is_dir() {
        resume(&layout)?;
        return Ok(Outcome::Resumed);
    }
    // Staging with no aside directory is a run that died before it published
    // anything. Its contents are derivable and the plaintext store is intact,
    // so the cheapest correct thing is to start over.
    if layout.staging.exists() {
        std::fs::remove_dir_all(&layout.staging).map_err(|source| Error::Io {
            path: layout.staging.clone(),
            source,
        })?;
    }

    match plaintext_state(&layout.database)? {
        Plaintext::NoFile => return Ok(Outcome::NoStore),
        Plaintext::Encrypted => return Ok(Outcome::AlreadyEncrypted),
        Plaintext::Yes => {}
    }

    let report = stage_encrypted_store(&layout, master)?;
    if stop_at(stop, Stage::Staged) {
        return Ok(Outcome::Encrypted(report));
    }

    verify(&layout, master)?;
    if stop_at(stop, Stage::Verified) {
        return Ok(Outcome::Encrypted(report));
    }

    swap(&layout, stop)?;
    Ok(Outcome::Encrypted(report))
}

/// Where every path this migration touches lives.
struct Layout {
    /// The directory the store lives in.
    directory: PathBuf,
    /// The store's database file.
    database: PathBuf,
    /// The blob directory beside it.
    blobs: PathBuf,
    /// Where the encrypted store is assembled.
    staging: PathBuf,
    /// Where the plaintext store waits to be deleted.
    aside: PathBuf,
}

impl Layout {
    fn around(database: &Path) -> Self {
        let directory = database.parent().unwrap_or(Path::new("."));
        Self {
            directory: directory.to_path_buf(),
            database: database.to_path_buf(),
            blobs: database.with_file_name(BLOBS),
            staging: directory.join(STAGING),
            aside: directory.join(ASIDE),
        }
    }

    /// The database file's name, which the staged and set-aside copies keep.
    fn database_name(&self) -> &std::ffi::OsStr {
        self.database
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("postio.db"))
    }

    /// The three entries that make up a database, at some root.
    fn database_parts(&self, root: &Path) -> Vec<PathBuf> {
        let name = self.database_name().to_string_lossy().into_owned();
        DATABASE_PARTS
            .iter()
            .map(|suffix| root.join(format!("{name}{suffix}")))
            .collect()
    }
}

/// Whether the file at `path` is a plaintext SQLite database.
enum Plaintext {
    /// No database yet.
    NoFile,
    /// It begins with SQLite's own header, so nothing encrypted it.
    Yes,
    /// It does not, which for a file this build wrote means SQLCipher.
    Encrypted,
}

fn plaintext_state(path: &Path) -> Result<Plaintext> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Plaintext::NoFile),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut header = [0u8; 16];
    let read = {
        use std::io::Read;
        file.read(&mut header).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
    };
    if read == header.len() && header.as_slice() == SQLITE_MAGIC {
        Ok(Plaintext::Yes)
    } else {
        Ok(Plaintext::Encrypted)
    }
}

/// Builds the whole encrypted store in the staging directory.
fn stage_encrypted_store(layout: &Layout, master: &StoreKey) -> Result<Report> {
    crate::perm::ensure_private_dir(&layout.staging)?;

    let plaintext = Connection::open(&layout.database)?;
    refuse_if_queue_is_undrained(&plaintext)?;

    let staged_database = layout.staging.join(layout.database_name());
    export(&plaintext, &staged_database, master)?;
    drop(plaintext);

    let report = reencrypt_blobs(layout, master, &staged_database)?;
    Ok(report)
}

/// Refuses a store whose queue still holds work the server has not seen.
///
/// A store old enough not to have the table at all has nothing to lose, which
/// is the one case this waves through.
fn refuse_if_queue_is_undrained(plaintext: &Connection) -> Result<()> {
    let exists: bool = plaintext.query_row(
        "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'operation_queue'",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(());
    }
    let pending: i64 = plaintext.query_row(
        "SELECT count(*) FROM operation_queue WHERE state IN ('pending', 'in_flight')",
        [],
        |row| row.get(0),
    )?;
    if pending > 0 {
        return Err(Error::QueueNotDrained {
            pending: pending as usize,
        });
    }
    Ok(())
}

/// Copies the plaintext database into an encrypted one, page for page.
///
/// `sqlcipher_export` rather than a schema-aware copy: it is SQLCipher's own
/// answer to this exact problem, it carries the FTS5 shadow tables and the
/// triggers with it, and it cannot fall behind a schema change the way a
/// hand-written table list would.
fn export(plaintext: &Connection, destination: &Path, master: &StoreKey) -> Result<()> {
    let key = master.derive(Purpose::Database);
    // The path is interpolated because SQLite takes no parameter in `ATTACH`'s
    // filename slot when it is followed by `KEY`; single quotes are doubled the
    // way SQLite's own string literals escape them.
    let quoted = destination.to_string_lossy().replace('\'', "''");
    let hex = key.to_hex();
    plaintext.execute_batch(&format!(
        "ATTACH DATABASE '{quoted}' AS encrypted KEY \"x'{}'\";",
        *hex
    ))?;
    drop(hex);
    let exported = plaintext.query_row("SELECT sqlcipher_export('encrypted')", [], |_| Ok(()));
    // Detach whatever happened, or the connection keeps a handle on a file the
    // caller is about to delete.
    let detached = plaintext.execute_batch("DETACH DATABASE encrypted;");
    exported?;
    detached?;
    Ok(())
}

/// Re-encrypts every blob into staging and repoints the staged database's rows.
///
/// The ids change — they are keyed now — so this is not a file copy: every
/// blob is read out of the old store, written into the new one, and the row
/// that named it is updated to the name it has now.
fn reencrypt_blobs(layout: &Layout, master: &StoreKey, staged_database: &Path) -> Result<Report> {
    let keys = BlobKeys::derive(master);
    let mut report = Report::default();
    if !layout.blobs.is_dir() {
        return Ok(report);
    }

    // The old store is opened under the *new* keys, which is harmless and
    // deliberate: reading dispatches on each file's own header, and every file
    // in a plaintext store is a legacy or version 1 blob that carries no
    // ciphertext. The keys are never used on the way in — only on the way out,
    // into the staged store below.
    let old = BlobStore::open(&layout.blobs, &keys)?;
    let new = BlobStore::open(layout.staging.join(BLOBS), &keys)?;

    // One entry per blob. A store large enough for this to matter is a store
    // that postdates the migration: this runs once, on a pre-release store,
    // and the alternative is a temporary table in a database that is being
    // rebuilt underneath it.
    let mut renamed: HashMap<String, String> = HashMap::new();
    for (id, _) in old.stored_blobs()? {
        let reader = old.reader(&id)?;
        let mut counting = Counting {
            inner: reader,
            seen: 0,
        };
        let fresh = new.put_reader(&mut counting)?;
        report.blobs += 1;
        report.bytes += counting.seen;
        renamed.insert(id.as_str().to_owned(), fresh.as_str().to_owned());
    }

    let staged = Connection::open(staged_database)?;
    crate::db::configure(&staged, &master.derive(Purpose::Database))?;
    repoint(&staged, &renamed)?;
    // The rows are the point of the exercise, so they reach the file before
    // anything decides the staged store is good.
    staged.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
    drop(staged);

    Ok(report)
}

/// Every column that holds a blob key, and the table it is on.
///
/// The same three [`crate::blob`]'s garbage collection reads. Named once,
/// because a fourth added to one list and not the other is mail that survives
/// the migration on disk and vanishes from the database.
pub(crate) const BLOB_REFERENCES: [(&str, &str); 3] = [
    ("messages", "raw_blob_id"),
    ("attachments", "blob_id"),
    ("cross_account_moves", "raw_blob_id"),
];

/// Rewrites every blob reference to the id the blob has now.
fn repoint(staged: &Connection, renamed: &HashMap<String, String>) -> Result<()> {
    for (table, column) in BLOB_REFERENCES {
        let sql = format!("UPDATE {table} SET {column} = ?2 WHERE {column} = ?1");
        let mut statement = staged.prepare(&sql)?;
        for (old, new) in renamed {
            statement.execute(rusqlite::params![old, new])?;
        }
    }
    Ok(())
}

/// Counts the bytes that pass through, for the report.
struct Counting<R> {
    inner: R,
    seen: u64,
}

impl<R: std::io::Read> std::io::Read for Counting<R> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(out)?;
        self.seen += read as u64;
        Ok(read)
    }
}

/// Reads the staged store back before anything is replaced.
///
/// Not `integrity_check` alone: a database can be structurally perfect and
/// still point at blobs that will not open, which is the failure this
/// migration could actually introduce. So every referenced blob is streamed
/// through its own AEAD, and the tag is the verdict.
fn verify(layout: &Layout, master: &StoreKey) -> Result<()> {
    let staged_database = layout.staging.join(layout.database_name());
    let key = master.derive(Purpose::Database);
    let database =
        Database::open(&staged_database, &key).map_err(|error| Error::MigrationDidNotVerify {
            reason: format!("the encrypted database would not open: {error}"),
        })?;
    let connection = database
        .connection()
        .map_err(|error| Error::MigrationDidNotVerify {
            reason: format!("the encrypted database would not hand out a connection: {error}"),
        })?;

    let verdict: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| Error::MigrationDidNotVerify {
            reason: format!("the integrity check would not run: {error}"),
        })?;
    if verdict != "ok" {
        return Err(Error::MigrationDidNotVerify {
            reason: format!("the encrypted database reports `{verdict}`"),
        });
    }

    let blobs = BlobStore::open(layout.staging.join(BLOBS), &BlobKeys::derive(master))?;
    for id in referenced(&connection)? {
        let mut reader = blobs
            .reader(&id)
            .map_err(|error| Error::MigrationDidNotVerify {
                reason: format!("blob {} did not open: {error}", id.as_str()),
            })?;
        std::io::copy(&mut reader, &mut std::io::sink()).map_err(|error| {
            Error::MigrationDidNotVerify {
                reason: format!("blob {} did not read back: {error}", id.as_str()),
            }
        })?;
    }

    drop(connection);
    drop(database);
    Ok(())
}

/// Every blob key the staged database points at.
fn referenced(connection: &Connection) -> Result<Vec<BlobId>> {
    let mut out = Vec::new();
    for (table, column) in BLOB_REFERENCES {
        let sql = format!("SELECT {column} FROM {table} WHERE {column} IS NOT NULL");
        let mut statement = connection.prepare(&sql)?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            out.push(BlobId::new(row?));
        }
    }
    Ok(out)
}

/// Moves the plaintext store aside, moves the staged one in, and deletes the
/// plaintext copy.
///
/// Each move is guarded by whether it has already happened, so this is also
/// [`resume`]'s body: running it twice, or from the middle, converges.
fn swap(layout: &Layout, stop: Option<Stage>) -> Result<()> {
    crate::perm::ensure_private_dir(&layout.aside)?;

    // Out first, both halves, before either replacement goes in. The window
    // where the store path holds *neither* is the only one a resume can read
    // unambiguously: an entry that is there is either untouched plaintext (its
    // aside copy is missing) or the finished encrypted one (its aside copy is
    // there).
    for part in layout.database_parts(&layout.directory) {
        move_aside(&part, &layout.aside.join(part.file_name().expect("a name")))?;
    }
    if stop_at(stop, Stage::DatabaseAside) {
        return Ok(());
    }
    move_aside(&layout.blobs, &layout.aside.join(BLOBS))?;
    if stop_at(stop, Stage::OriginalsAside) {
        return Ok(());
    }

    for part in layout.database_parts(&layout.staging) {
        move_in(
            &part,
            &layout
                .database
                .with_file_name(part.file_name().expect("a name")),
        )?;
    }
    if stop_at(stop, Stage::DatabaseInPlace) {
        return Ok(());
    }
    move_in(&layout.staging.join(BLOBS), &layout.blobs)?;
    if stop_at(stop, Stage::Swapped) {
        return Ok(());
    }

    remove_tree(&layout.staging)?;
    // Last, and only now: until this line the plaintext store is still on
    // disk, which is what makes every death above recoverable.
    remove_tree(&layout.aside)?;
    Ok(())
}

/// Carries an interrupted swap to the end.
///
/// Called when the aside directory exists, which only happens once the staged
/// store has verified — so finishing forward is always the right answer, and
/// there is no case where the plaintext store should be put back.
fn resume(layout: &Layout) -> Result<()> {
    swap(layout, None)
}

/// Moves `from` to `to` unless the destination already holds it.
///
/// The guard is what makes the swap idempotent: a destination that is already
/// there means this move happened before the interruption, and `from` — if it
/// exists at all — is the replacement rather than the original.
fn move_aside(from: &Path, to: &Path) -> Result<()> {
    if to.exists() || !from.exists() {
        return Ok(());
    }
    rename(from, to)
}

/// Moves a staged entry into place, if it is still in staging.
fn move_in(from: &Path, to: &Path) -> Result<()> {
    if !from.exists() {
        return Ok(());
    }
    rename(from, to)
}

fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|source| Error::Io {
        path: to.to_path_buf(),
        source,
    })
}

fn remove_tree(path: &Path) -> Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(Error::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}
