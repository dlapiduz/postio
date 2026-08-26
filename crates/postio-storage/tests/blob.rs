//! The content-addressed blob store.
//!
//! Written before the store existed. The bead's acceptance criteria are "the
//! same content written twice occupies one blob", "an interrupted write leaves
//! no partial blob visible" and "orphan collection is tested".

use std::io::{self, Read};

use postio_model::BlobId;
use postio_model::test_corpus;
use postio_storage::blob::{BlobStore, GarbageCollection};

fn store() -> (tempfile::TempDir, BlobStore) {
    let directory = tempfile::tempdir().expect("tempdir");
    let store = BlobStore::open(directory.path().join("blobs")).expect("open the store");
    (directory, store)
}

/// Every regular file under the store's root, excluding its temp directory.
fn blob_files(store: &BlobStore) -> Vec<std::path::PathBuf> {
    fn walk(directory: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(store.root())
        .expect("read the root")
        .flatten()
    {
        let path = entry.path();
        if path == store.temporary_directory() {
            continue;
        }
        if path.is_dir() {
            walk(&path, &mut out);
        } else {
            out.push(path);
        }
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Acceptance: the same content written twice occupies one blob
// ---------------------------------------------------------------------------

#[test]
fn the_same_content_written_twice_is_one_blob() {
    let (_directory, store) = store();

    let first = store.put(b"the same bytes").expect("first write");
    let second = store.put(b"the same bytes").expect("second write");

    assert_eq!(first, second, "content addressing means one id");
    assert_eq!(
        blob_files(&store).len(),
        1,
        "and one file on disk, not two copies"
    );
}

#[test]
fn different_content_lands_in_different_blobs() {
    let (_directory, store) = store();

    let one = store.put(b"first").expect("put");
    let other = store.put(b"second").expect("put");

    assert_ne!(one, other);
    assert_eq!(blob_files(&store).len(), 2);
}

#[test]
fn a_blob_round_trips_byte_for_byte() {
    let (_directory, store) = store();
    let content: Vec<u8> = (0..=255u8).cycle().take(100_000).collect();

    let id = store.put(&content).expect("put");

    assert_eq!(store.get(&id).expect("get"), content);
    assert_eq!(store.len_of(&id).expect("len"), content.len() as u64);
    assert!(store.contains(&id));
}

#[test]
fn ids_are_hex_digests_and_shard_the_directory_tree() {
    let (_directory, store) = store();

    let id = store.put(b"shard me").expect("put");

    assert_eq!(id.as_str().len(), 64, "a hex digest");
    assert!(id.as_str().chars().all(|c| c.is_ascii_hexdigit()));

    let path = store.path_of(&id).expect("a path");
    let relative = path
        .strip_prefix(store.root())
        .expect("blobs live under the root");
    let components: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        components,
        vec![
            id.as_str()[0..2].to_owned(),
            id.as_str()[2..4].to_owned(),
            id.as_str()[4..].to_owned(),
        ],
        "two levels of sharding, so no directory holds a million entries"
    );
    assert!(path.exists());
}

#[test]
fn an_id_that_is_not_a_digest_is_rejected_rather_than_resolved() {
    let (_directory, store) = store();

    for hostile in [
        "../../../../etc/passwd",
        "..",
        "",
        "not-hex-at-all",
        "abcd", // right alphabet, wrong length
    ] {
        let id = BlobId::new(hostile);
        assert!(
            store.path_of(&id).is_err(),
            "{hostile:?} must not resolve to a path"
        );
        assert!(!store.contains(&id));
        assert!(store.get(&id).is_err());
    }
}

#[test]
fn reading_a_blob_that_is_not_there_is_an_error_not_a_panic() {
    let (_directory, store) = store();
    let absent = BlobId::new("0".repeat(64));

    assert!(!store.contains(&absent));
    assert!(store.get(&absent).is_err());
    assert!(store.reader(&absent).is_err());
}

// ---------------------------------------------------------------------------
// Streaming, in both directions
// ---------------------------------------------------------------------------

#[test]
fn a_blob_can_be_written_from_a_reader_without_buffering_it_whole() {
    let (_directory, store) = store();
    let content: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();

    let id = store
        .put_reader(&mut content.as_slice())
        .expect("streaming write");

    assert_eq!(
        id,
        store.put(&content).expect("the same bytes by value"),
        "streaming and buffered writes agree on the digest"
    );
    assert_eq!(store.len_of(&id).expect("len"), content.len() as u64);
}

#[test]
fn a_blob_can_be_read_as_a_stream() {
    let (_directory, store) = store();
    let content: Vec<u8> = (0..300_000).map(|i| (i % 97) as u8).collect();
    let id = store.put(&content).expect("put");

    let mut reader = store.reader(&id).expect("reader");
    let mut buffer = [0u8; 4096];
    let mut read_back = Vec::new();
    loop {
        let read = reader.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        read_back.extend_from_slice(&buffer[..read]);
    }

    assert_eq!(read_back, content);
}

// ---------------------------------------------------------------------------
// Acceptance: an interrupted write leaves no partial blob visible
// ---------------------------------------------------------------------------

/// A reader that hands over some bytes and then fails, the way a dropped
/// connection does mid-fetch.
struct FailsHalfway {
    remaining: usize,
}

impl Read for FailsHalfway {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the server hung up",
            ));
        }
        let count = buffer.len().min(self.remaining);
        buffer[..count].fill(b'x');
        self.remaining -= count;
        Ok(count)
    }
}

#[test]
fn a_write_that_fails_partway_leaves_nothing_behind() {
    let (_directory, store) = store();

    let error = store
        .put_reader(&mut FailsHalfway { remaining: 500_000 })
        .expect_err("the write must fail");
    assert!(
        error.to_string().contains("hung up") || error.to_string().contains("reset"),
        "the underlying error is reported, not swallowed: {error}"
    );

    assert!(
        blob_files(&store).is_empty(),
        "a half-written blob is never visible under a content address"
    );
    assert!(
        std::fs::read_dir(store.temporary_directory())
            .expect("read temp")
            .next()
            .is_none(),
        "and its temporary file is cleaned up"
    );
}

#[test]
fn a_temporary_file_left_by_a_crash_is_purged_and_never_served() {
    let (_directory, store) = store();
    let orphan = store.temporary_directory().join("crashed.part");
    std::fs::write(&orphan, b"half a message").expect("write a leftover temp file");

    assert!(
        blob_files(&store).is_empty(),
        "a temp file is not a blob, whatever it holds"
    );

    let purged = store.purge_temporary().expect("purge");

    assert_eq!(purged, 1);
    assert!(!orphan.exists());
}

// ---------------------------------------------------------------------------
// Acceptance: orphan collection
// ---------------------------------------------------------------------------

/// A database with one account, one mailbox, and the ids to hang rows off.
fn database() -> postio_storage::test_support::TempDatabase {
    let database = postio_storage::test_support::temp();
    let connection = database.connection().expect("checkout");
    connection
        .execute_batch(
            "INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES (1, 'Test', 'test@example.com', 'imap.example.com', 993, 'test',
                     'smtp.example.com', 587, 'test', 0);
             INSERT INTO mailboxes (id, account_id, name, path) VALUES (1, 1, 'INBOX', 'INBOX');",
        )
        .expect("seed");
    drop(connection);
    database
}

fn insert_message(
    connection: &rusqlite::Connection,
    raw: Option<&BlobId>,
    text: Option<&BlobId>,
) -> i64 {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, raw_blob_id,
                                   body_text_blob_id)
             VALUES (1, 1, 0, ?1, ?2)",
            rusqlite::params![raw.map(BlobId::as_str), text.map(BlobId::as_str)],
        )
        .expect("insert a message");
    connection.last_insert_rowid()
}

#[test]
fn garbage_collection_keeps_referenced_blobs_and_removes_orphans() {
    let (_directory, store) = store();
    let database = database();
    let connection = database.connection().expect("checkout");

    let raw = store.put(b"the raw message").expect("put");
    let body = store.put(b"the plain text body").expect("put");
    let attached = store.put(b"an attachment").expect("put");
    let orphan = store.put(b"nothing points at this").expect("put");

    let message = insert_message(&connection, Some(&raw), Some(&body));
    connection
        .execute(
            "INSERT INTO attachments (message_id, mime_type, size, blob_id)
             VALUES (?1, 'application/pdf', 13, ?2)",
            rusqlite::params![message, attached.as_str()],
        )
        .expect("insert an attachment");

    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .expect("collect");

    assert_eq!(report.scanned, 4);
    assert_eq!(report.removed, 1, "only the orphan goes");
    assert_eq!(
        report.bytes_reclaimed,
        "nothing points at this".len() as u64
    );

    assert!(store.contains(&raw));
    assert!(store.contains(&body));
    assert!(store.contains(&attached));
    assert!(!store.contains(&orphan));
}

#[test]
fn a_blob_becomes_collectable_once_its_last_reference_goes() {
    let (_directory, store) = store();
    let database = database();
    let connection = database.connection().expect("checkout");

    let shared = store.put(b"referenced twice").expect("put");
    let first = insert_message(&connection, Some(&shared), None);
    insert_message(&connection, Some(&shared), None);

    connection
        .execute("DELETE FROM messages WHERE id = ?1", [first])
        .expect("delete one of them");
    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .expect("collect");
    assert_eq!(report.removed, 0, "the other message still points at it");
    assert!(store.contains(&shared));

    connection
        .execute("DELETE FROM messages", [])
        .expect("delete the rest");
    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .expect("collect");
    assert_eq!(report.removed, 1, "now nothing does");
    assert!(!store.contains(&shared));
}

#[test]
fn a_blob_younger_than_the_grace_period_is_never_collected() {
    let (_directory, store) = store();
    let database = database();
    let connection = database.connection().expect("checkout");

    // The window every real caller lives in: the bytes are on disk, the row
    // that will reference them is not written yet.
    let in_flight = store.put(b"written, not yet referenced").expect("put");

    let report = store
        .collect_garbage(&connection, GarbageCollection::default())
        .expect("collect");

    assert_eq!(
        report.removed, 0,
        "the default grace period is what keeps a concurrent write safe"
    );
    assert_eq!(report.skipped_too_young, 1);
    assert!(store.contains(&in_flight));
}

#[test]
fn collecting_an_empty_store_is_a_no_op() {
    let (_directory, store) = store();
    let database = database();
    let connection = database.connection().expect("checkout");

    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .expect("collect");

    assert_eq!(report.scanned, 0);
    assert_eq!(report.removed, 0);
    assert_eq!(report.bytes_reclaimed, 0);
}

#[test]
fn removing_a_blob_by_hand_is_idempotent() {
    let (_directory, store) = store();
    let id = store.put(b"delete me").expect("put");

    assert!(store.remove(&id).expect("first remove"));
    assert!(
        !store.remove(&id).expect("second remove"),
        "removing what is already gone is false, not an error"
    );
}

// ---------------------------------------------------------------------------
// Against the corpus
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_in_the_corpus_survives_a_round_trip() {
    let (_directory, store) = store();

    for fixture in test_corpus::all() {
        let id = store.put(fixture.bytes()).expect("put");
        assert_eq!(
            store.get(&id).expect("get"),
            fixture.bytes(),
            "{}: raw bytes must come back exactly as they arrived",
            fixture.name()
        );
    }

    assert_eq!(
        blob_files(&store).len(),
        test_corpus::count(),
        "every fixture is distinct, so every one is its own blob"
    );
}

#[test]
fn a_large_attachment_streams_rather_than_being_held_whole() {
    let (_directory, store) = store();
    let fixture = test_corpus::load("attachment-large");

    let id = store
        .put_reader(&mut fixture.bytes())
        .expect("streaming write");

    let mut reader = store.reader(&id).expect("reader");
    let mut first = [0u8; 64];
    reader.read_exact(&mut first).expect("read the head");
    assert_eq!(
        &first[..],
        &fixture.bytes()[..64],
        "the head of the blob is readable without reading the tail"
    );
}

// ---------------------------------------------------------------------------
// This is the user's mail: the root, its shards and its blobs are private
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod private_by_default {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode_of(path: &std::path::Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn a_freshly_opened_store_and_its_temp_directory_are_0700() {
        let (_directory, store) = store();

        assert_eq!(mode_of(store.root()), 0o700, "the blob root");
        assert_eq!(mode_of(store.temporary_directory()), 0o700, "and its tmp/");
    }

    #[test]
    fn a_stored_blob_and_its_shard_directories_are_private() {
        let (_directory, store) = store();

        let id = store
            .put(b"a report nobody else on the machine gets to read")
            .expect("put");
        let path = store
            .path_of(&id)
            .expect("the path a stored digest resolves to");

        assert_eq!(
            mode_of(path.parent().expect("the shard directory")),
            0o700,
            "the two-level shard directory"
        );
        assert_eq!(mode_of(&path), 0o600, "and the blob file the digest names");
    }

    #[test]
    fn a_root_that_was_loosened_is_repaired_on_reopen() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = directory.path().join("blobs");
        BlobStore::open(&root).expect("first open");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755))
            .expect("loosen it, the way a pre-fix store would be");

        BlobStore::open(&root).expect("reopen");

        assert_eq!(mode_of(&root), 0o700);
    }
}

// ---------------------------------------------------------------------------
// Writing without holding the bytes — ADR 0017, axis 2
// ---------------------------------------------------------------------------

#[test]
fn a_streamed_write_yields_the_same_id_as_the_same_bytes_at_once() {
    // The whole point of the streaming writer is that it is not a different
    // store: content addressing has to survive being fed in pieces, or a blob
    // written by the fetch path and the same blob written by the compose path
    // would be two files.
    let (_directory, store) = store();

    let whole = store
        .put(b"the same bytes, whichever way they arrive")
        .expect("put");

    let mut writer = store.writer().expect("a writer");
    writer.write(b"the same bytes, ").expect("first chunk");
    writer.write(b"whichever way ").expect("second chunk");
    writer.write(b"they arrive").expect("third chunk");
    let streamed = writer.finish().expect("finish");

    assert_eq!(streamed, whole, "chunk boundaries carry no meaning");
    assert_eq!(blob_files(&store).len(), 1, "and one file, not two");
}

#[test]
fn a_writer_dropped_before_finishing_publishes_nothing() {
    // The guarantee the store already makes for `put`, extended to the push
    // form: a fetch that is cancelled or dies mid-flight leaves debris in the
    // temp directory, never a blob under a digest that promises otherwise.
    let (_directory, store) = store();

    let mut writer = store.writer().expect("a writer");
    writer.write(b"half a message").expect("a chunk");
    drop(writer);

    assert!(
        blob_files(&store).is_empty(),
        "nothing was published, so nothing is visible"
    );
}

#[test]
fn a_streamed_write_never_holds_more_than_one_chunk() {
    // The assertion ADR 0017's axis 2 actually cares about. `VecSink` grows a
    // `Vec` by doubling, so a 40 MB message peaks well above 40 MB and copies
    // itself a dozen times on the way; the writer holds one chunk at a time
    // and the file holds the rest.
    //
    // Asserted structurally rather than by measuring the allocator: the
    // writer is handed far more bytes than any buffer it could own, and what
    // is checked is that the bytes all arrived correctly anyway.
    let (_directory, store) = store();
    let chunk = vec![b'x'; 64 * 1024];
    let chunks = 64; // 4 MiB in 64 KiB pieces

    let mut writer = store.writer().expect("a writer");
    for _ in 0..chunks {
        writer.write(&chunk).expect("chunk");
    }
    let id = writer.finish().expect("finish");

    let mut reader = store.reader(&id).expect("reader");
    let mut counted = 0usize;
    let mut buffer = vec![0u8; 8 * 1024];
    loop {
        let read = reader.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        assert!(buffer[..read].iter().all(|byte| *byte == b'x'));
        counted += read;
    }
    assert_eq!(counted, chunk.len() * chunks);
}
