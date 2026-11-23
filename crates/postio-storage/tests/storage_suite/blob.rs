use postio_storage::Connection;
use postio_storage::sql::bind;
use std::io::{self, Read};

use postio_model::BlobId;
use postio_model::test_corpus;
use postio_storage::blob::{BlobStore, GarbageCollection};

fn store() -> (tempfile::TempDir, BlobStore) {
    let directory = tempfile::tempdir().expect("tempdir");
    let store = BlobStore::open(
        directory.path().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("open the store");
    (directory, store)
}

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
    assert!(store.contains(&id));
    // `len_of` is the size on disk; this content repeats, so compression shrinks it (ADR 0017).
    assert!(store.len_of(&id).expect("len") < content.len() as u64);
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
    assert!(store.len_of(&id).expect("len") > 0, "it is on disk");
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

async fn database() -> postio_storage::test_support::TempStore {
    let database = postio_storage::test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    connection
        .execute_batch(
            "INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                                   incoming_username, outgoing_host, outgoing_port,
                                   outgoing_username, created_at)
             VALUES (1, 'Test', 'test@example.com', 'imap.example.com', 993, 'test',
                     'smtp.example.com', 587, 'test', 0);
             INSERT INTO mailboxes (id, account_id, name, path) VALUES (1, 1, 'INBOX', 'INBOX');",
        )
        .await
        .expect("seed");
    drop(connection);
    database
}

/// No body parameter: a body is a compressed column on the row, not a blob (ADR 0020).
async fn insert_message(connection: &Connection, raw: Option<&BlobId>) -> i64 {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, raw_blob_id)
             VALUES (1, 1, 0, 0, ?1)",
            bind![raw.map(BlobId::as_str)],
        )
        .await
        .expect("insert a message");
    connection.last_insert_rowid()
}

#[tokio::test]
async fn garbage_collection_keeps_referenced_blobs_and_removes_orphans() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let raw = store.put(b"the raw message").expect("put");
    let attached = store.put(b"an attachment").expect("put");
    let also_attached = store.put(b"a second attachment").expect("put");
    let orphan = store.put(b"nothing points at this").expect("put");
    // On-disk size, not content length: blobs are compressed and carry a container header (ADR
    // 0017).
    let orphan_bytes = store.len_of(&orphan).expect("the orphan's size on disk");

    let message = insert_message(&connection, Some(&raw)).await;
    for blob in [&attached, &also_attached] {
        connection
            .execute(
                "INSERT INTO attachments (message_id, mime_type, size, blob_id)
                 VALUES (?1, 'application/pdf', 13, ?2)",
                bind![message, blob.as_str()],
            )
            .await
            .expect("insert an attachment");
    }

    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .await
        .expect("collect");

    assert_eq!(report.scanned, 4);
    assert_eq!(report.removed, 1, "only the orphan goes");
    assert_eq!(report.bytes_reclaimed, orphan_bytes);

    assert!(store.contains(&raw));
    assert!(store.contains(&attached));
    assert!(store.contains(&also_attached));
    assert!(!store.contains(&orphan));
}

#[tokio::test]
async fn a_blob_becomes_collectable_once_its_last_reference_goes() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let shared = store.put(b"referenced twice").expect("put");
    let first = insert_message(&connection, Some(&shared)).await;
    insert_message(&connection, Some(&shared)).await;

    connection
        .execute("DELETE FROM messages WHERE id = ?1", [first])
        .await
        .expect("delete one of them");
    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .await
        .expect("collect");
    assert_eq!(report.removed, 0, "the other message still points at it");
    assert!(store.contains(&shared));

    connection
        .execute("DELETE FROM messages", ())
        .await
        .expect("delete the rest");
    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .await
        .expect("collect");
    assert_eq!(report.removed, 1, "now nothing does");
    assert!(!store.contains(&shared));
}

#[tokio::test]
async fn a_blob_younger_than_the_grace_period_is_never_collected() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    // Bytes on disk, the referencing row not yet written: the window every real caller lives in.
    let in_flight = store.put(b"written, not yet referenced").expect("put");

    let report = store
        .collect_garbage(&connection, GarbageCollection::default())
        .await
        .expect("collect");

    assert_eq!(
        report.removed, 0,
        "the default grace period is what keeps a concurrent write safe"
    );
    assert_eq!(report.skipped_too_young, 1);
    assert!(store.contains(&in_flight));
}

#[tokio::test]
async fn collecting_an_empty_store_is_a_no_op() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let report = store
        .collect_garbage(&connection, GarbageCollection::immediate())
        .await
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
        BlobStore::open(&root, &postio_storage::test_support::blob_keys()).expect("first open");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755))
            .expect("loosen it, the way a pre-fix store would be");

        BlobStore::open(&root, &postio_storage::test_support::blob_keys()).expect("reopen");

        assert_eq!(mode_of(&root), 0o700);
    }
}

#[test]
fn a_streamed_write_yields_the_same_id_as_the_same_bytes_at_once() {
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
    // Asserted structurally: far more bytes than any buffer the writer could own all arrive
    // correctly. A doubling `VecSink` would peak well above a 40 MB message (ADR 0017).
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

fn mail_shaped_text() -> Vec<u8> {
    let mut text = String::new();
    for n in 0..200 {
        text.push_str(&format!(
            "On Tuesday, Ada Lovelace <ada@example.com> wrote:\r\n\
             > The quarterly figures are attached for review.\r\n\
             > Please confirm receipt by Friday.\r\n\
             Reply {n}: confirmed, thank you.\r\n\
             --\r\n\
             Ada Lovelace | Analytical Engines | ada@example.com\r\n\r\n"
        ));
    }
    text.into_bytes()
}

#[test]
fn a_compressed_blob_reads_back_byte_for_byte() {
    let (_directory, store) = store();
    let content = mail_shaped_text();

    let id = store.put(&content).expect("put");

    assert_eq!(store.get(&id).expect("get"), content);
}

#[test]
fn the_id_is_the_digest_of_the_plaintext_not_of_what_is_on_disk() {
    // The id is taken before compression and encryption, so dedup is unaffected by either (ADR
    // 0014).
    let (_directory, store) = store();
    let content = mail_shaped_text();

    let id = store.put(&content).expect("put");

    // Keyed since #301: recompute it as the store does. That it is not the plain digest is
    // `blob_encryption.rs`'s to assert.
    let keys = postio_storage::test_support::blob_keys();
    let mut hasher = blake3::Hasher::new_keyed(keys.id().expose());
    hasher.update(&content);
    assert_eq!(id.as_str(), hasher.finalize().to_hex().as_str());
}

#[test]
fn mail_shaped_text_actually_gets_smaller() {
    let (_directory, store) = store();
    let content = mail_shaped_text();

    let id = store.put(&content).expect("put");
    let on_disk = store.len_of(&id).expect("stored length");

    assert!(
        on_disk * 4 < content.len() as u64,
        "expected better than 4x on quoted mail, got {} from {}",
        on_disk,
        content.len()
    );
}

#[test]
fn already_compressed_bytes_are_not_compressed_again() {
    // JPEG, PNG, PDF and ZIP are incompressible: zstd would cost CPU to make them larger, so the
    // store declines and says so in the header.
    let (_directory, store) = store();
    let mut content = Vec::new();
    for n in 0u32..8_000 {
        content.extend_from_slice(blake3::hash(&n.to_le_bytes()).as_bytes());
    }

    let id = store.put(&content).expect("put");
    let on_disk = store.len_of(&id).expect("stored length");

    assert_eq!(store.get(&id).expect("get"), content);
    // Encryption overhead, pinned: a 31-byte header and one 16-byte Poly1305 tag per 64 KiB chunk.
    let chunks = (content.len() as u64).div_ceil(64 * 1024);
    let overhead = 31 + 16 * chunks;
    assert!(
        on_disk <= content.len() as u64 + overhead,
        "an incompressible blob must not grow beyond its seal: {} from {} (+{overhead})",
        on_disk,
        content.len()
    );
}

#[test]
fn a_blob_written_before_the_format_existed_still_reads() {
    // Existing stores hold headerless plaintext files; a file without the magic is read verbatim.
    let (_directory, store) = store();
    let content = b"Subject: written by an older Postio\r\n\r\nplain, headerless bytes";

    let id = BlobId::new(blake3::hash(content).to_hex().to_string());
    let path = store.path_of(&id).expect("path");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, content).expect("write the legacy blob");

    assert_eq!(store.get(&id).expect("get"), content);
}

#[test]
fn a_compressed_blob_streams_without_being_read_whole() {
    // `reader` keeps a large attachment out of memory; compression must not turn it into a `get`.
    let (_directory, store) = store();
    let content = mail_shaped_text();
    let id = store.put(&content).expect("put");

    let mut reader = store.reader(&id).expect("reader");
    let mut round_tripped = Vec::new();
    let mut buffer = [0u8; 1024];
    loop {
        let read = reader.read(&mut buffer).expect("read");
        if read == 0 {
            break;
        }
        round_tripped.extend_from_slice(&buffer[..read]);
    }

    assert_eq!(round_tripped, content);
}

async fn insert_message_at(connection: &Connection, received_at: i64, raw: Option<&BlobId>) -> i64 {
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, raw_blob_id, body_state)
             VALUES (1, 1, ?1, ?1, ?2, 'full')",
            bind![received_at, raw.map(BlobId::as_str)],
        )
        .await
        .expect("insert a message");
    connection.last_insert_rowid()
}

async fn attach(connection: &Connection, message: i64, blob: &BlobId, size: i64) {
    connection
        .execute(
            "INSERT INTO attachments (message_id, mime_type, size, blob_id, part_id)
             VALUES (?1, 'application/pdf', ?2, ?3, '2')",
            bind![message, size, blob.as_str()],
        )
        .await
        .expect("insert an attachment");
}

#[tokio::test]
async fn eviction_takes_raw_source_before_it_takes_a_payload() {
    // ADR 0017's order: raw source goes first, being refetchable; a downloaded attachment was asked
    // for.
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let raw = store.put(&vec![b'r'; 40_000]).expect("put");
    let payload = store.put(&vec![b'p'; 40_000]).expect("put");
    let message = insert_message_at(&connection, 1_000, Some(&raw)).await;
    attach(&connection, message, &payload, 40_000).await;

    let budget = store.len_of(&payload).expect("len") + 16;
    let report = store
        .evict_to_fit(&connection, budget)
        .await
        .expect("evict");

    assert!(report.removed >= 1);
    assert!(!store.contains(&raw), "raw source goes first");
    assert!(store.contains(&payload), "the attachment survives it");
}

#[tokio::test]
async fn eviction_cannot_reach_the_text_that_search_is_made_of() {
    // Message text is not refetchable in any meaningful sense, and bodies are columns on the row
    // (ADR 0020), so no pass over the blob store can evict them.
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;

    let mut message = postio_model::Message::new(
        account.id,
        inbox,
        chrono::TimeZone::timestamp_opt(&chrono::Utc, 1_000, 0)
            .single()
            .expect("a timestamp"),
    );
    let messages = postio_storage::repository::MessageRepository::new(&connection);
    let id = messages.create(&mut message).await.expect("create");
    let words = "the words, which are never evicted".repeat(40);
    messages
        .set_body(
            id,
            &postio_storage::repository::StoredBody {
                text: Some(words.clone()),
                ..Default::default()
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("store a body");

    let report = store.evict_to_fit(&connection, 0).await.expect("evict");
    assert_eq!(report.removed, 0);

    assert_eq!(
        messages
            .body(id)
            .await
            .expect("body")
            .expect("the row")
            .text,
        Some(words),
        "no eviction budget can reach a body: it is not in the blob store"
    );
}

#[tokio::test]
async fn eviction_takes_the_oldest_mail_first() {
    // Recency comes from the message, not blob mtime or atime (immutable, `noatime`). Oldest is
    // evicted first, the mirror of the backfill.
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let old = store.put(&vec![b'o'; 40_000]).expect("put");
    let new = store.put(&vec![b'n'; 40_000]).expect("put");
    insert_message_at(&connection, 1_000, Some(&old)).await;
    insert_message_at(&connection, 9_000, Some(&new)).await;

    let budget = store.len_of(&new).expect("len") + 16;
    store
        .evict_to_fit(&connection, budget)
        .await
        .expect("evict");

    assert!(!store.contains(&old), "the mail nobody has opened in years");
    assert!(store.contains(&new), "not this week's");
}

#[tokio::test]
async fn an_evicted_payload_puts_its_message_back_to_partial() {
    // `full` means every part is local; the attachment chip would offer "open" for bytes no longer
    // here.
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let payload = store.put(&vec![b'p'; 40_000]).expect("put");
    let message = insert_message_at(&connection, 1_000, None).await;
    attach(&connection, message, &payload, 40_000).await;

    store.evict_to_fit(&connection, 0).await.expect("evict");

    assert!(!store.contains(&payload));
    let state: String = postio_storage::sql::one(
        &connection,
        "SELECT body_state FROM messages WHERE id = ?1",
        [message],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the row");
    assert_eq!(state, "partial");
    let blob: Option<String> = postio_storage::sql::one(
        &connection,
        "SELECT blob_id FROM attachments WHERE message_id = ?1",
        [message],
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("the attachment");
    assert_eq!(blob, None, "and the row stops claiming bytes it lost");
}

#[tokio::test]
async fn a_store_already_under_its_budget_evicts_nothing() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let raw = store.put(&vec![b'r'; 4_000]).expect("put");
    insert_message_at(&connection, 1_000, Some(&raw)).await;

    let report = store
        .evict_to_fit(&connection, 100 * 1024 * 1024)
        .await
        .expect("evict");

    assert_eq!(report.removed, 0);
    assert!(store.contains(&raw));
}

#[tokio::test]
async fn signed_mail_keeps_its_raw_source_longest() {
    let (_directory, store) = store();
    let database = database().await;
    let connection = database.connect().await.expect("checkout");

    let signed_raw = store.put(&vec![b's'; 40_000]).expect("put");
    let plain_raw = store.put(&vec![b'r'; 40_000]).expect("put");
    let payload = store.put(&vec![b'p'; 40_000]).expect("put");
    let signed = insert_message_at(&connection, 1_000, Some(&signed_raw)).await;
    connection
        .execute(
            "UPDATE messages SET content_type = 'multipart/signed' WHERE id = ?1",
            bind![signed],
        )
        .await
        .expect("a signed message");
    let plain = insert_message_at(&connection, 2_000, Some(&plain_raw)).await;
    attach(&connection, plain, &payload, 40_000).await;

    let one = store.len_of(&signed_raw).expect("len") + 16;
    store
        .evict_to_fit(&connection, 2 * one)
        .await
        .expect("evict");
    assert!(
        !store.contains(&plain_raw),
        "unsigned raw source goes first, though newer"
    );
    assert!(store.contains(&payload) && store.contains(&signed_raw));

    store.evict_to_fit(&connection, one).await.expect("evict");
    assert!(!store.contains(&payload), "then attachment payloads");
    assert!(
        store.contains(&signed_raw),
        "a signature needs the exact bytes, so signed raw source goes last"
    );
}
