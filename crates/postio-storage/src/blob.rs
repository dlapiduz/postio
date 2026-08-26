//! The content-addressed blob store.
//!
//! SQLite holds metadata; the bytes live here. A message's raw RFC 5322 source,
//! its decoded bodies and its attachment payloads are written to a file named
//! after the BLAKE3 digest of its content, and the database keeps only the
//! digest. That is what keeps the database small enough to meet the `<100 ms`
//! search and `<16 ms` interaction budgets in CLAUDE.md — the message list pages
//! through rows of a few hundred bytes, not through inboxes of PDFs.
//!
//! # Why content addressing
//!
//! The same attachment forwarded around a team, the same newsletter delivered to
//! two accounts, the same message copied into Archive: all of them write the
//! same bytes, and all of them cost one file. Deduplication is not a feature
//! layered on top, it is what naming a file after its content means.
//!
//! # The guarantees
//!
//! * **A blob is never partially visible.** Writes go to a temporary file and
//!   are renamed into place only once every byte is on disk, so a crash or a
//!   dropped connection leaves a stray temp file (cleaned by
//!   [`BlobStore::purge_temporary`]) and never a truncated blob under a digest
//!   that promises otherwise.
//! * **Reads stream.** [`BlobStore::reader`] hands back a file, not a buffer;
//!   a 30 MiB attachment is never held in memory just to be handed to a viewer.
//! * **Blobs are immutable.** Nothing ever rewrites one; the only mutation is
//!   removal.
//!
//! # Garbage rather than reference counts
//!
//! Nothing tracks how many rows point at a blob. A reference count would have to
//! be updated in the same transaction as every message write, delete, expunge
//! and move — an invariant that has to hold forever, across every future
//! repository, or blobs leak silently. Instead [`BlobStore::collect_garbage`]
//! asks the database which digests are still referenced and deletes the rest.
//! It is a sweep rather than an accounting, and a sweep cannot drift out of
//! sync with the data.
//!
//! The one hazard is the window where bytes are written but the row that
//! references them is not committed yet, so collection ignores blobs younger
//! than [`GarbageCollection::min_age`].
//!
//! ```no_run
//! # fn main() -> Result<(), postio_storage::Error> {
//! use postio_storage::blob::{BlobStore, GarbageCollection};
//!
//! let store = BlobStore::open("~/.local/share/postio/blobs")?;
//! let id = store.put(b"raw message bytes")?;
//! assert_eq!(store.get(&id)?, b"raw message bytes");
//!
//! let database = postio_storage::Database::open("postio.db")?;
//! let connection = database.connection()?;
//! let report = store.collect_garbage(&connection, GarbageCollection::default())?;
//! eprintln!("reclaimed {} bytes", report.bytes_reclaimed);
//! # Ok(())
//! # }
//! ```

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use postio_model::BlobId;
use rusqlite::Connection;

use crate::error::{Error, Result};

/// How many hex characters of the digest name each shard directory.
///
/// Two levels of two characters: 256 directories holding 256 directories, so a
/// mailbox of a million attachments averages 16 files per directory. Flat would
/// put a million entries in one directory, which several filesystems handle
/// badly and every `ls` handles worse.
const SHARD_WIDTH: usize = 2;

/// The number of shard levels.
const SHARD_LEVELS: usize = 2;

/// Length of a BLAKE3 digest in hex characters.
const DIGEST_CHARS: usize = 64;

/// Name of the directory holding in-progress writes.
///
/// Not a valid shard name (`tmp` is not hex), so it can never collide with one.
const TEMPORARY: &str = "tmp";

/// How much is read from a source at a time while streaming a blob in.
const CHUNK: usize = 64 * 1024;

/// A directory of content-addressed blobs.
///
/// Cheap to clone; it is a path and nothing else. Blobs are immutable, so
/// concurrent readers and writers need no coordination beyond the atomic rename
/// that publishes each one.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    temporary: PathBuf,
}

impl BlobStore {
    /// Opens (or creates) a blob store rooted at `root`.
    ///
    /// `root` is created (or repaired) `0700`, same as `temporary` below it —
    /// this holds raw messages and attachments, so it gets the same
    /// treatment `Database::open` gives the SQLite file beside it. See
    /// [`crate::perm`].
    ///
    /// # Errors
    ///
    /// [`crate::error::Error::Io`] if the directory tree cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        // Named explicitly rather than left to `create_dir_all(&temporary)`
        // creating it as an ancestor: an ancestor is only tightened when
        // this call is the one that creates it, and a store from before this
        // existed already has `root` sitting there at the umask.
        crate::perm::ensure_private_dir(&root)?;
        let temporary = root.join(TEMPORARY);
        create_dir_all(&temporary)?;
        Ok(Self { root, temporary })
    }

    /// The directory the blobs live under.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The directory in-progress writes go to.
    ///
    /// Nothing in here is a blob, whatever it holds; see
    /// [`BlobStore::purge_temporary`].
    pub fn temporary_directory(&self) -> &Path {
        &self.temporary
    }

    /// Stores `content` and returns its digest.
    ///
    /// Writing content that is already stored is a no-op that returns the same
    /// id: this is the deduplication the whole design rests on.
    pub fn put(&self, content: &[u8]) -> Result<BlobId> {
        self.put_reader(content)
    }

    /// Stores everything `source` yields, without buffering it whole.
    ///
    /// The digest is computed as the bytes stream past, so a 30 MiB attachment
    /// costs one 64 KiB buffer rather than 30 MiB of memory.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the source fails or the bytes cannot be written. In
    /// either case nothing is published: the partial file is deleted and no
    /// digest is returned.
    pub fn put_reader(&self, mut source: impl Read) -> Result<BlobId> {
        let mut writer = self.writer()?;
        let mut buffer = vec![0u8; CHUNK];

        loop {
            let read = source.read(&mut buffer).map_err(|source| Error::Io {
                path: writer.path().to_path_buf(),
                source,
            })?;
            if read == 0 {
                break;
            }
            writer.write(&buffer[..read])?;
        }

        writer.finish()
    }

    /// Begins a blob whose bytes will be pushed in, a chunk at a time.
    ///
    /// The push counterpart of [`BlobStore::put_reader`], for the caller that
    /// cannot hand over a [`Read`] because the bytes are arriving from
    /// somewhere else — a socket delivering a `FETCH` response, above all.
    /// Without it such a caller has to buffer the whole thing first, which for
    /// a message is the forty megabytes ADR 0017's second axis is about.
    ///
    /// Nothing is visible until [`BlobWriter::finish`]. A writer dropped
    /// before then — a cancelled fetch, a dead connection, a panic — leaves a
    /// file in the temp directory for [`BlobStore::purge_temporary`] and no
    /// blob at all, so the store's "never partially visible" guarantee holds
    /// for this form exactly as it does for the others.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the temporary file cannot be created.
    pub fn writer(&self) -> Result<BlobWriter> {
        Ok(BlobWriter {
            root: self.root.clone(),
            temporary: TemporaryBlob::create(&self.temporary)?,
            hasher: blake3::Hasher::new(),
        })
    }

    /// Reads a whole blob into memory.
    ///
    /// For a body or a header block, which is what the reading pane wants. Use
    /// [`BlobStore::reader`] for anything that might be an attachment.
    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>> {
        let path = self.path_of(id)?;
        std::fs::read(&path).map_err(|source| self.read_error(id, path, source))
    }

    /// Opens a blob for streaming.
    pub fn reader(&self, id: &BlobId) -> Result<File> {
        let path = self.path_of(id)?;
        File::open(&path).map_err(|source| self.read_error(id, path, source))
    }

    /// Whether the blob is stored.
    pub fn contains(&self, id: &BlobId) -> bool {
        self.path_of(id).is_ok_and(|path| path.is_file())
    }

    /// The size of a stored blob, in bytes.
    pub fn len_of(&self, id: &BlobId) -> Result<u64> {
        let path = self.path_of(id)?;
        std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .map_err(|source| self.read_error(id, path, source))
    }

    /// Deletes a blob, returning whether it was there.
    ///
    /// Prefer [`BlobStore::collect_garbage`]: a caller that deletes by hand has
    /// to know that nothing else references the content, and content addressing
    /// means something else very well might.
    pub fn remove(&self, id: &BlobId) -> Result<bool> {
        let path = self.path_of(id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    /// Where a blob is, or would be, stored.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidBlobId`] unless the id is a digest of exactly the right
    /// shape. Ids reach this crate from the database and from other processes'
    /// data, so this is also what stops `../../` in an id from resolving to a
    /// path outside the store.
    pub fn path_of(&self, id: &BlobId) -> Result<PathBuf> {
        path_of(&self.root, id)
    }

    /// Deletes every leftover temporary file, returning how many there were.
    ///
    /// Worth calling on start: a power cut during a fetch leaves a `.part` file
    /// that nothing will ever finish.
    pub fn purge_temporary(&self) -> Result<usize> {
        let entries = read_dir(&self.temporary)?;
        let mut purged = 0;
        for entry in entries {
            let path = entry.path();
            if path.is_file() && std::fs::remove_file(&path).is_ok() {
                purged += 1;
            }
        }
        Ok(purged)
    }

    /// Deletes every stored blob the database no longer references.
    ///
    /// The reference set is read from the columns that hold a blob key:
    /// `messages.raw_blob_id`, `body_text_blob_id`, `body_html_blob_id`,
    /// `headers_blob_id` and `attachments.blob_id`. A blob younger than
    /// `options.min_age` is left alone even if unreferenced — see the
    /// [module docs](self).
    ///
    /// Files under the root that are not named like a digest are ignored
    /// rather than deleted: this directory belongs to the user, and a sweep
    /// that removes things it does not understand is a sweep nobody should run.
    pub fn collect_garbage(
        &self,
        connection: &Connection,
        options: GarbageCollection,
    ) -> Result<GarbageReport> {
        let referenced = referenced_blobs(connection)?;
        let mut report = GarbageReport::default();

        for (id, path) in self.stored_blobs()? {
            report.scanned += 1;
            if referenced.contains(id.as_str()) {
                continue;
            }
            if !is_older_than(&path, options.min_age) {
                report.skipped_too_young += 1;
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    report.removed += 1;
                    report.bytes_reclaimed += size;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path, source }),
            }
        }

        Ok(report)
    }

    /// Every blob in the store, as `(id, path)`.
    fn stored_blobs(&self) -> Result<Vec<(BlobId, PathBuf)>> {
        let mut blobs = Vec::new();
        for first in read_dir(&self.root)? {
            let first_path = first.path();
            if first_path == self.temporary || !first_path.is_dir() {
                continue;
            }
            let Some(first_name) = shard_name(&first_path) else {
                continue;
            };
            for second in read_dir(&first_path)? {
                let second_path = second.path();
                if !second_path.is_dir() {
                    continue;
                }
                let Some(second_name) = shard_name(&second_path) else {
                    continue;
                };
                for file in read_dir(&second_path)? {
                    let path = file.path();
                    if !path.is_file() {
                        continue;
                    }
                    let Some(tail) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    let digest = format!("{first_name}{second_name}{tail}");
                    if digest.len() != DIGEST_CHARS
                        || !digest.chars().all(|c| c.is_ascii_hexdigit())
                    {
                        continue;
                    }
                    blobs.push((BlobId::new(digest), path));
                }
            }
        }
        Ok(blobs)
    }

    fn read_error(&self, id: &BlobId, path: PathBuf, source: std::io::Error) -> Error {
        if source.kind() == std::io::ErrorKind::NotFound {
            Error::BlobNotFound {
                id: id.as_str().to_owned(),
            }
        } else {
            Error::Io { path, source }
        }
    }
}

/// How [`BlobStore::collect_garbage`] should behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarbageCollection {
    /// How old an unreferenced blob must be before it is deleted.
    ///
    /// The bytes of a message are written before the row that references them
    /// is committed, so a sweep with no grace period races every fetch in
    /// flight. An hour is far longer than any single write and short enough
    /// that space comes back the same session.
    pub min_age: Duration,
}

impl Default for GarbageCollection {
    fn default() -> Self {
        Self {
            min_age: Duration::from_secs(60 * 60),
        }
    }
}

impl GarbageCollection {
    /// Collects every unreferenced blob regardless of age.
    ///
    /// Safe only when nothing else is writing — a test, or a maintenance pass
    /// with sync stopped.
    pub fn immediate() -> Self {
        Self {
            min_age: Duration::ZERO,
        }
    }

    /// Collects unreferenced blobs older than `min_age`.
    pub fn after(min_age: Duration) -> Self {
        Self { min_age }
    }
}

/// What one garbage collection pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GarbageReport {
    /// How many blobs were considered.
    pub scanned: usize,
    /// How many were deleted.
    pub removed: usize,
    /// How much disk space came back.
    pub bytes_reclaimed: u64,
    /// How many unreferenced blobs were spared for being too young.
    pub skipped_too_young: usize,
}

/// Every blob key the database currently points at.
fn referenced_blobs(connection: &Connection) -> Result<HashSet<String>> {
    const SQL: &str = "\
SELECT raw_blob_id       FROM messages    WHERE raw_blob_id       IS NOT NULL
UNION
SELECT body_text_blob_id FROM messages    WHERE body_text_blob_id IS NOT NULL
UNION
SELECT body_html_blob_id FROM messages    WHERE body_html_blob_id IS NOT NULL
UNION
SELECT headers_blob_id   FROM messages    WHERE headers_blob_id   IS NOT NULL
UNION
SELECT blob_id           FROM attachments WHERE blob_id           IS NOT NULL";

    let mut statement = connection.prepare(SQL)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut referenced = HashSet::new();
    for row in rows {
        referenced.insert(row?);
    }
    Ok(referenced)
}

/// A blob being written: a temporary file that deletes itself unless published.
/// Where a blob is, or would be, stored under `root`.
///
/// A free function rather than a method because [`BlobWriter`] needs it too and
/// holds a root rather than a store. See [`BlobStore::path_of`] for the id
/// validation, which is also what stops `../../` in an id — ids arrive from the
/// database and from other processes' data — resolving outside the store.
fn path_of(root: &Path, id: &BlobId) -> Result<PathBuf> {
    let digest = id.as_str();
    if digest.len() != DIGEST_CHARS || !digest.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::InvalidBlobId {
            id: digest.to_owned(),
        });
    }
    let mut path = root.to_path_buf();
    for level in 0..SHARD_LEVELS {
        let start = level * SHARD_WIDTH;
        path.push(&digest[start..start + SHARD_WIDTH]);
    }
    path.push(&digest[SHARD_LEVELS * SHARD_WIDTH..]);
    Ok(path)
}

/// A blob being written, a chunk at a time.
///
/// Obtained from [`BlobStore::writer`]. The digest is computed as the bytes go
/// past, so the writer holds one caller-sized chunk and never the blob: the
/// file on disk is the buffer.
///
/// Publishing is [`BlobWriter::finish`] and nothing else. Dropping without it
/// discards the write — which is what makes a cancelled fetch safe, and is why
/// there is no `abort`: the absence of `finish` already means abandon, the
/// same way [`crate::backend`-style sinks](BlobStore::writer) treat a missing
/// completion call.
#[derive(Debug)]
pub struct BlobWriter {
    root: PathBuf,
    temporary: TemporaryBlob,
    hasher: blake3::Hasher,
}

impl BlobWriter {
    /// Appends `bytes` to the blob.
    ///
    /// Chunk boundaries carry no meaning: the digest is over the concatenation,
    /// so the same content split differently is the same blob.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the bytes cannot be written. The writer should be
    /// dropped after an error rather than reused; nothing has been published.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.hasher.update(bytes);
        self.temporary.write(bytes)
    }

    /// The temporary file's path, for error reporting.
    pub fn path(&self) -> &Path {
        &self.temporary.path
    }

    /// Publishes the blob and returns its digest.
    ///
    /// Consumes the writer, so "finished" is not a state anything has to check.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the bytes cannot be flushed or the file cannot be
    /// renamed into place. Nothing is published in either case.
    pub fn finish(mut self) -> Result<BlobId> {
        let id = BlobId::new(self.hasher.finalize().to_hex().to_string());
        let destination = path_of(&self.root, &id)?;

        // Already stored. The bytes are identical by construction, so the
        // cheapest correct thing is to drop what we just wrote.
        if destination.exists() {
            return Ok(id);
        }

        if let Some(parent) = destination.parent() {
            create_dir_all(parent)?;
        }
        self.temporary.publish(&destination)?;
        Ok(id)
    }
}

#[derive(Debug)]
struct TemporaryBlob {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryBlob {
    fn create(directory: &Path) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = directory.join(format!(
            "{}-{}.part",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // `private_file_options` rather than `File::create`: the temporary
        // file is renamed into place as the published blob (`publish`,
        // below), which does not change its mode, so whatever this creates
        // it with is what the blob ends up at.
        let file = crate::perm::private_file_options()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
        Ok(Self {
            path,
            file: Some(file),
        })
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.file
            .as_mut()
            .expect("the file is taken only when publishing")
            .write_all(bytes)
            .map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })
    }

    /// Flushes to disk and renames into place. After this the blob exists and
    /// the temporary file does not.
    fn publish(&mut self, destination: &Path) -> Result<()> {
        let file = self
            .file
            .take()
            .expect("publish is called once, before drop");
        // Durable before it is visible: a rename that reaches the disk ahead of
        // the bytes it names would leave a blob whose content is zeroes.
        file.sync_all().map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        drop(file);

        std::fs::rename(&self.path, destination).map_err(|source| Error::Io {
            path: destination.to_path_buf(),
            source,
        })
    }
}

impl Drop for TemporaryBlob {
    fn drop(&mut self) {
        // Published blobs have already been renamed away, so this only ever
        // removes the debris of a write that failed.
        if self.file.is_some() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether the file has not been modified for at least `age`.
///
/// A file whose timestamp is in the future — clock skew, a restored backup — is
/// treated as too young: refusing to delete is always the recoverable mistake.
fn is_older_than(path: &Path, age: Duration) -> bool {
    if age.is_zero() {
        return true;
    }
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|elapsed| elapsed >= age)
}

fn shard_name(path: &Path) -> Option<&str> {
    path.file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.len() == SHARD_WIDTH && name.chars().all(|c| c.is_ascii_hexdigit()))
}

fn create_dir_all(path: &Path) -> Result<()> {
    crate::perm::ensure_private_dir(path)
}

fn read_dir(path: &Path) -> Result<Vec<std::fs::DirEntry>> {
    let entries = std::fs::read_dir(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(entries.flatten().collect())
}
