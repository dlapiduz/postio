//! Content-addressed store for raw messages and attachments: files named by BLAKE3 digest, the
//! database keeps the digest. Bodies are not here (ADR 0020). No reference counts:
//! `collect_garbage` sweeps, skipping blobs younger than `min_age` because bytes land before their
//! row commits.

mod format;
pub(crate) use format::LEVEL as format_level;
mod seal;

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::sql::{self, RowExt as _};
use crate::store::Connection;
use postio_model::BlobId;

use crate::error::{Error, Result};
use crate::key::BlobKeys;

/// Two levels of two hex characters; a flat directory is slow on several filesystems.
const SHARD_WIDTH: usize = 2;

const SHARD_LEVELS: usize = 2;

const DIGEST_CHARS: usize = 64;

/// Not hex, so it never collides with a shard.
const TEMPORARY: &str = "tmp";

const CHUNK: usize = seal::CHUNK;

/// Cheap to clone. Blobs are immutable; the atomic rename that publishes each one is the only
/// coordination.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
    temporary: PathBuf,
    keys: BlobKeys,
}

impl BlobStore {
    /// `root` is created or repaired `0700` (see [`crate::perm`]); it holds raw mail.
    pub fn open(root: impl Into<PathBuf>, keys: &BlobKeys) -> Result<Self> {
        let root = root.into();
        // Explicit: an ancestor is only tightened by the call that creates it, and older stores
        // have `root` at the umask.
        crate::perm::ensure_private_dir(&root)?;
        let temporary = root.join(TEMPORARY);
        create_dir_all(&temporary)?;
        Ok(Self {
            root,
            temporary,
            keys: keys.clone(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn temporary_directory(&self) -> &Path {
        &self.temporary
    }

    /// Writing content already stored is a no-op returning the same id.
    pub fn put(&self, content: &[u8]) -> Result<BlobId> {
        self.put_reader(content)
    }

    /// Streams: the digest is computed as bytes pass, with one 64 KiB buffer.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] on a source or write failure; nothing is published and the partial file is
    /// deleted.
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

    /// Push form of [`BlobStore::put_reader`] for bytes arriving from elsewhere (a `FETCH`
    /// response), so the caller need not buffer a whole message (ADR 0017).
    ///
    /// Nothing is visible until [`BlobWriter::finish`]; a dropped writer leaves a temp file for
    /// [`BlobStore::purge_temporary`] and no blob.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the temporary file cannot be created.
    pub fn writer(&self) -> Result<BlobWriter> {
        let temporary = TemporaryBlob::create(&self.temporary)?;
        Ok(BlobWriter {
            root: self.root.clone(),
            path: temporary.path.clone(),
            // Keyed, so the same bytes get a different name per installation (ADR 0014 Q2); dedup
            // is unaffected.
            hasher: blake3::Hasher::new_keyed(self.keys.id().expose()),
            keys: self.keys.clone(),
            state: State::Deciding(temporary),
            probe: Vec::new(),
        })
    }

    /// Reads a whole blob into memory; use [`BlobStore::reader`] for anything that might be an
    /// attachment.
    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>> {
        let mut content = Vec::new();
        self.reader(id)?
            .read_to_end(&mut content)
            .map_err(|source| unreadable(id, source))?;
        Ok(content)
    }

    /// Decompresses as it streams, so an attachment never exists whole in memory.
    pub fn reader(&self, id: &BlobId) -> Result<Box<dyn Read + Send>> {
        let path = self.path_of(id)?;
        let mut file =
            File::open(&path).map_err(|source| self.read_error(id, path.clone(), source))?;

        let mut start = [0u8; format::HEADER_LEN];
        let read = read_up_to(&mut file, &mut start)
            .map_err(|source| self.read_error(id, path.clone(), source))?;

        let container = match format::framing_of(&start[..read])? {
            format::Framing::Legacy => {
                // Rewind: those bytes are content, not a header.
                let file = File::open(&path)
                    .map_err(|source| self.read_error(id, path.clone(), source))?;
                return Ok(Box::new(file));
            }
            format::Framing::Container(container) => container,
        };

        // A version 1 payload started earlier: hand those bytes back before the file's own.
        let payload_at = container.payload_at();
        let carried = start[payload_at..read].to_vec();
        let payload = std::io::Cursor::new(carried).chain(file);

        let payload: Box<dyn Read + Send> = match container.nonce {
            Some(nonce) => Box::new(seal::Opener::new(payload, self.keys.content(), nonce)),
            None => Box::new(payload),
        };

        match container.codec {
            format::Codec::None => Ok(payload),
            format::Codec::Zstd => {
                let decoder = zstd::stream::read::Decoder::new(payload).map_err(|source| {
                    Error::UnreadableBlob {
                        reason: format!("its compressed payload could not be opened: {source}"),
                    }
                })?;
                Ok(Box::new(decoder))
            }
        }
    }

    pub fn contains(&self, id: &BlobId) -> bool {
        self.path_of(id).is_ok_and(|path| path.is_file())
    }

    /// Stored size (compressed, with header), not the content length.
    pub fn len_of(&self, id: &BlobId) -> Result<u64> {
        let path = self.path_of(id)?;
        std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .map_err(|source| self.read_error(id, path, source))
    }

    /// Prefer [`BlobStore::collect_garbage`]: something else may reference the same content.
    pub fn remove(&self, id: &BlobId) -> Result<bool> {
        let path = self.path_of(id)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(Error::Io { path, source }),
        }
    }

    /// # Errors
    ///
    /// [`Error::InvalidBlobId`] unless the id is a well-formed digest, which also stops `../../` in
    /// a database-supplied id escaping the store.
    pub fn path_of(&self, id: &BlobId) -> Result<PathBuf> {
        path_of(&self.root, id)
    }

    /// Call on start: a power cut during a fetch leaves a `.part` file nothing finishes. Files this
    /// process is still writing are kept, so the purge may run beside a sync that is fetching.
    pub fn purge_temporary(&self) -> Result<usize> {
        let entries = read_dir(&self.temporary)?;
        let mut purged = 0;
        for entry in entries {
            let path = entry.path();
            if path.is_file()
                && !TemporaryBlob::is_ours(&path)
                && std::fs::remove_file(&path).is_ok()
            {
                purged += 1;
            }
        }
        Ok(purged)
    }

    /// Deletes refetchable blobs, oldest mail first, until the store fits in `budget` bytes.
    ///
    /// Order: raw message source, then attachment payloads. Never message text: bodies are
    /// `messages` columns (ADR 0020), and losing text would shrink search silently.
    /// Oldest first by `received_at`, not LRU: blobs are immutable and atime is unusable.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if a blob cannot be deleted, or a database error. A partial pass keeps what it
    /// freed.
    pub async fn evict_to_fit(
        &self,
        connection: &Connection,
        budget: u64,
    ) -> Result<EvictionReport> {
        let mut report = EvictionReport::default();
        let mut occupied: u64 = self
            .stored_blobs()?
            .iter()
            .map(|(_, path)| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0))
            .sum();
        report.bytes_remaining = occupied;
        if occupied <= budget {
            return Ok(report);
        }

        for candidate in evictable(connection).await? {
            if occupied <= budget {
                break;
            }
            let Ok(path) = self.path_of(&candidate.blob) else {
                continue;
            };
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                // Already gone (concurrent collection or a manual tidy): carry on and clear the
                // row.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(Error::Io { path, source }),
            }
            candidate.forget(connection).await?;
            occupied = occupied.saturating_sub(size);
            report.removed += 1;
            report.bytes_reclaimed += size;
        }

        report.bytes_remaining = occupied;
        Ok(report)
    }

    /// Deletes every stored blob no row references, skipping those younger than `options.min_age`.
    ///
    /// Files not named like a digest are left alone: this directory belongs to the user.
    /// Unlike [`evict_to_fit`](Self::evict_to_fit), this is always safe.
    pub async fn collect_garbage(
        &self,
        connection: &Connection,
        options: GarbageCollection,
    ) -> Result<GarbageReport> {
        let referenced = referenced_blobs(connection).await?;
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

    pub(crate) fn stored_blobs(&self) -> Result<Vec<(BlobId, PathBuf)>> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GarbageCollection {
    /// Grace period: bytes are written before the row that references them commits, so a sweep
    /// without one races in-flight fetches.
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
    /// Safe only when nothing else is writing (a test, or sync stopped).
    pub fn immediate() -> Self {
        Self {
            min_age: Duration::ZERO,
        }
    }

    pub fn after(min_age: Duration) -> Self {
        Self { min_age }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GarbageReport {
    pub scanned: usize,
    pub removed: usize,
    pub bytes_reclaimed: u64,
    pub skipped_too_young: usize,
}

#[derive(Debug)]
struct Evictable {
    blob: BlobId,
    reference: Reference,
}

#[derive(Debug, Clone, Copy)]
enum Reference {
    RawSource,
    Payload,
}

impl Evictable {
    async fn forget(&self, connection: &Connection) -> Result<()> {
        match self.reference {
            Reference::RawSource => {
                // The file is shared by every reference to this blob key.
                // Clear ownership and the reader projection together.
                sql::execute(
                    connection,
                    "UPDATE message_contents SET raw_blob_id = NULL WHERE raw_blob_id = ?1",
                    [self.blob.as_str()],
                )
                .await?;
                sql::execute(
                    connection,
                    "UPDATE messages SET raw_blob_id = NULL WHERE raw_blob_id = ?1",
                    [self.blob.as_str()],
                )
                .await?;
            }
            Reference::Payload => {
                sql::execute(
                    connection,
                    "UPDATE message_contents SET body_state = 'partial'
                      WHERE namespace IS NULL AND body_state IN ('partial','full')
                        AND id IN (SELECT m.content_id FROM messages m
                                   JOIN attachments a ON a.message_id = m.id WHERE a.blob_id = ?1)",
                    [self.blob.as_str()],
                )
                .await?;
                sql::execute(
                    connection,
                    "UPDATE messages SET body_state = 'partial'
                      WHERE body_state = 'full' AND id IN
                        (SELECT message_id FROM attachments WHERE blob_id = ?1)",
                    [self.blob.as_str()],
                )
                .await?;
                sql::execute(
                    connection,
                    "UPDATE message_content_parts SET blob_id = NULL WHERE blob_id = ?1",
                    [self.blob.as_str()],
                )
                .await?;
                sql::execute(
                    connection,
                    "UPDATE attachments SET blob_id = NULL WHERE blob_id = ?1",
                    [self.blob.as_str()],
                )
                .await?;
            }
        }
        Ok(())
    }
}

async fn evictable(connection: &Connection) -> Result<Vec<Evictable>> {
    let raw = |signed: bool| {
        let sql = format!(
            "SELECT id, raw_blob_id FROM messages
              WHERE raw_blob_id IS NOT NULL
                AND coalesce(lower(content_type), '') {} ({SIGNED_TYPES})
              ORDER BY received_at ASC, id ASC",
            if signed { "IN" } else { "NOT IN" }
        );
        async move {
            sql::all(connection, &sql, (), |row| {
                Ok(Evictable {
                    reference: Reference::RawSource,
                    blob: BlobId::new(row.col::<String>(1)?),
                })
            })
            .await
        }
    };
    let mut out = raw(false).await?;
    out.extend(
        sql::all(
            connection,
            "SELECT a.id, a.blob_id, m.id FROM attachments a
               JOIN messages m ON m.id = a.message_id
              WHERE a.blob_id IS NOT NULL
              ORDER BY m.received_at ASC, a.id ASC",
            (),
            |row| {
                Ok(Evictable {
                    reference: Reference::Payload,
                    blob: BlobId::new(row.col::<String>(1)?),
                })
            },
        )
        .await?,
    );
    out.extend(raw(true).await?);
    Ok(out)
}

const SIGNED_TYPES: &str = "'multipart/signed', 'multipart/encrypted', 'application/pkcs7-mime', \
                            'application/x-pkcs7-mime', 'application/pkcs7-signature'";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EvictionReport {
    pub removed: u64,
    pub bytes_reclaimed: u64,
    /// Above the budget means nothing left was allowed to evict (only text remains); the backfill
    /// should stop fetching.
    pub bytes_remaining: u64,
}

/// A reference added to the schema and not to this list is mail the sweep deletes while a row still
/// points at it.
pub(crate) const BLOB_REFERENCES: [(&str, &str); 5] = [
    ("messages", "raw_blob_id"),
    ("message_contents", "raw_blob_id"),
    ("message_content_parts", "blob_id"),
    ("attachments", "blob_id"),
    ("cross_account_moves", "raw_blob_id"),
];

async fn referenced_blobs(connection: &Connection) -> Result<HashSet<String>> {
    let clauses: Vec<String> = BLOB_REFERENCES
        .iter()
        .map(|(table, column)| format!("SELECT {column} FROM {table} WHERE {column} IS NOT NULL"))
        .collect();
    let sql = clauses.join("\nUNION\n");

    Ok(sql::all(connection, &sql, (), |row| row.col::<String>(0))
        .await?
        .into_iter()
        .collect())
}

/// Free function because [`BlobWriter`] needs it with only a root. See [`BlobStore::path_of`] for
/// id validation.
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

/// Obtained from [`BlobStore::writer`]. Publishing is [`BlobWriter::finish`] only: dropping
/// discards the write, so a cancelled fetch is safe and there is no `abort`.
#[derive(Debug)]
pub struct BlobWriter {
    root: PathBuf,
    /// Kept beside the state because the encoder owns the file once compression starts.
    path: PathBuf,
    hasher: blake3::Hasher,
    keys: BlobKeys,
    state: State,
    probe: Vec<u8>,
}

/// The header names the codec, which cannot be chosen until enough bytes show whether the payload
/// compresses; hence it begins undecided.
enum State {
    Deciding(TemporaryBlob),
    Plain(seal::Sealer<TemporaryBlob>),
    /// Through zstd then the seal: id, compress, encrypt (ADR 0017).
    Zstd(Box<zstd::stream::write::Encoder<'static, seal::Sealer<TemporaryBlob>>>),
    /// Momentarily empty while swapping states; never observable outside one method.
    Taken,
}

impl std::fmt::Debug for State {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Deciding(_) => formatter.write_str("Deciding"),
            Self::Plain(_) => formatter.write_str("Plain"),
            Self::Zstd(_) => formatter.write_str("Zstd"),
            Self::Taken => formatter.write_str("Taken"),
        }
    }
}

const PROBE_BYTES: usize = CHUNK;

impl BlobWriter {
    /// Chunk boundaries do not matter: the digest is over the concatenation.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] if the bytes cannot be written; drop the writer after an error.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        // Digest over the plaintext, before compression and encryption, so dedup survives both (ADR
        // 0014).
        self.hasher.update(bytes);

        if let State::Deciding(_) = self.state {
            self.probe.extend_from_slice(bytes);
            if self.probe.len() < PROBE_BYTES {
                return Ok(());
            }
            return self.decide_and_flush();
        }
        self.write_payload(bytes)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// # Errors
    ///
    /// [`Error::Io`] if the bytes cannot be flushed or renamed into place; nothing is published.
    pub fn finish(mut self) -> Result<BlobId> {
        if let State::Deciding(_) = self.state {
            self.decide_and_flush()?;
        }

        let sealer = match std::mem::replace(&mut self.state, State::Taken) {
            State::Plain(sealer) => sealer,
            State::Zstd(encoder) => encoder.finish().map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?,
            State::Deciding(_) | State::Taken => {
                unreachable!("deciding was resolved above, and Taken never escapes a method")
            }
        };
        let mut temporary = sealer.finish().map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;

        let id = BlobId::new(self.hasher.finalize().to_hex().to_string());
        let destination = path_of(&self.root, &id)?;

        // Already stored, and identical by construction: drop what was just written.
        if destination.exists() {
            return Ok(id);
        }

        if let Some(parent) = destination.parent() {
            create_dir_all(parent)?;
        }
        temporary.publish(&destination)?;
        Ok(id)
    }

    fn decide_and_flush(&mut self) -> Result<()> {
        let State::Deciding(mut temporary) = std::mem::replace(&mut self.state, State::Taken)
        else {
            unreachable!("only called while deciding");
        };

        let codec = if format::looks_compressible(&self.probe) {
            format::Codec::Zstd
        } else {
            format::Codec::None
        };
        // The header goes in before the sealer takes the file: it is the one non-ciphertext part,
        // since it names the nonce.
        let nonce = seal::fresh_nonce_prefix();
        temporary.write(&format::header(codec, format::NO_DICTIONARY, &nonce))?;
        let sealer = seal::Sealer::with_prefix(temporary, self.keys.content(), nonce);

        self.state =
            match codec {
                format::Codec::None => State::Plain(sealer),
                format::Codec::Zstd => {
                    let encoder = zstd::stream::write::Encoder::new(sealer, format::LEVEL)
                        .map_err(|source| Error::Io {
                            path: self.path.clone(),
                            source,
                        })?;
                    State::Zstd(Box::new(encoder))
                }
            };

        let sample = std::mem::take(&mut self.probe);
        self.write_payload(&sample)
    }

    fn write_payload(&mut self, bytes: &[u8]) -> Result<()> {
        match &mut self.state {
            State::Plain(sealer) => sealer.write_all(bytes).map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            }),
            State::Zstd(encoder) => encoder.write_all(bytes).map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            }),
            State::Deciding(_) | State::Taken => {
                unreachable!("the codec is chosen before any payload is written")
            }
        }
    }
}

#[derive(Debug)]
struct TemporaryBlob {
    path: PathBuf,
    file: Option<File>,
}

impl TemporaryBlob {
    // Not the pid alone: inside a Flatpak sandbox every start can have the same pid, and debris
    // named like this process's own would never be purged.
    fn prefix() -> &'static str {
        static PREFIX: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        PREFIX.get_or_init(|| {
            let started = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{}-{started}-", std::process::id())
        })
    }

    fn is_ours(path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(Self::prefix()))
    }

    fn create(directory: &Path) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = directory.join(format!(
            "{}{}.part",
            Self::prefix(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // `publish` renames this file and keeps its mode, so the published blob gets the mode it is
        // created with here.
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

/// Every path here is an authentication failure or a corrupt payload; the io error's words say
/// which.
fn unreadable(id: &BlobId, source: io::Error) -> Error {
    Error::UnreadableBlob {
        reason: format!("blob {}: {source}", id.as_str()),
    }
}

/// `Read::read` may stop short; a header split across two reads would look like a legacy blob.
fn read_up_to(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(filled)
}

impl Write for TemporaryBlob {
    /// `write_all`, not a partial write: a retried short write would interleave badly with the
    /// digest.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file
            .as_mut()
            .expect("the file is taken only when publishing")
            .write_all(bytes)
            .map(|()| bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
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

/// A timestamp in the future (clock skew, restored backup) counts as too young: refusing to delete
/// is recoverable.
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
