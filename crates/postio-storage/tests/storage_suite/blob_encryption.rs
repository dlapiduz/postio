use std::io::Read;

use postio_model::test_corpus;
use postio_storage::blob::BlobStore;
use postio_storage::key::{BlobKeys, StoreKey};

fn keys(seed: u8) -> BlobKeys {
    BlobKeys::derive(&StoreKey::from_bytes([seed; 32]))
}

fn store(seed: u8) -> (tempfile::TempDir, BlobStore) {
    let directory = tempfile::tempdir().expect("a directory");
    let store =
        BlobStore::open(directory.path().join("blobs"), &keys(seed)).expect("open the store");
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

/// `len` bytes zstd cannot shrink: cases about byte offsets (chunk boundary, tag) need the payload
/// at its own size.
fn incompressible(len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 32);
    let mut counter = 0u32;
    while out.len() < len {
        out.extend_from_slice(blake3::hash(&counter.to_le_bytes()).as_bytes());
        counter += 1;
    }
    out.truncate(len);
    out
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    needle.len() <= haystack.len()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

#[test]
fn a_blob_round_trips_through_encryption() {
    let (_directory, store) = store(1);
    let content = b"Dear Ada, the frobnicator arrives on Thursday.".repeat(3);

    let id = store.put(&content).expect("store it");
    assert_eq!(store.get(&id).expect("read it back"), content);

    let mut streamed = Vec::new();
    store
        .reader(&id)
        .expect("open it")
        .read_to_end(&mut streamed)
        .expect("stream it");
    assert_eq!(
        streamed, content,
        "the streaming path must agree with `get`"
    );
}

#[test]
fn a_payload_larger_than_one_chunk_round_trips() {
    // The streaming AEAD splits at 64 KiB; a chunk counter or last-block flag can only be wrong
    // across several chunks.
    let (_directory, store) = store(2);
    let content = incompressible(700_000);

    let id = store.put(&content).expect("store it");
    assert_eq!(store.get(&id).expect("read it back"), content);

    let mut streamed = Vec::new();
    store
        .reader(&id)
        .expect("open it")
        .read_to_end(&mut streamed)
        .expect("stream it");
    assert_eq!(streamed, content);
}

#[test]
fn an_empty_blob_round_trips() {
    let (_directory, store) = store(3);
    let id = store.put(b"").expect("store nothing");
    assert_eq!(store.get(&id).expect("read it back"), Vec::<u8>::new());
}

#[test]
fn the_file_on_disk_holds_no_plaintext() {
    let (_directory, store) = store(4);
    // Incompressible, so the marker reaches the file near verbatim; compression would hide it and
    // let a plaintext store pass.
    let marker = b"Zarquon-Vindaloo-Quintessence";
    let mut content = incompressible(200_000);
    content[50_000..50_000 + marker.len()].copy_from_slice(marker);

    let id = store.put(&content).expect("store it");
    let stored = std::fs::read(store.path_of(&id).expect("a path")).expect("read the file");

    assert!(
        !contains(&stored, marker),
        "the payload is sitting on disk in the clear"
    );
    assert!(
        !contains(&stored, &content[..1024]),
        "the opening bytes are sitting on disk in the clear"
    );
}

#[test]
fn a_flipped_byte_in_the_payload_is_refused_rather_than_returned() {
    let (_directory, store) = store(5);
    let content = b"the frobnicator arrives on Thursday".repeat(40);
    let id = store.put(&content).expect("store it");
    let path = store.path_of(&id).expect("a path");

    let mut stored = std::fs::read(&path).expect("read the file");
    let last = stored.len() - 1;
    stored[last] ^= 0x01;
    std::fs::write(&path, &stored).expect("write it back");

    let error = store
        .get(&id)
        .expect_err("a tampered blob must not be read");
    assert!(
        error.to_string().to_ascii_lowercase().contains("authentic"),
        "the error must say what is wrong: {error}"
    );
}

#[test]
fn a_flipped_byte_in_a_middle_chunk_is_refused() {
    // The last chunk's tag cannot cover an earlier one.
    let (_directory, store) = store(6);
    let content = incompressible(400_000);
    let id = store.put(&content).expect("store it");
    let path = store.path_of(&id).expect("a path");

    let mut stored = std::fs::read(&path).expect("read the file");
    stored[4_096] ^= 0x80;
    std::fs::write(&path, &stored).expect("write it back");

    let mut sink = Vec::new();
    let outcome = store
        .reader(&id)
        .expect("the header is intact, so opening succeeds")
        .read_to_end(&mut sink);
    assert!(
        outcome.is_err(),
        "a tampered middle chunk was streamed out as if it were mail"
    );
}

#[test]
fn a_truncated_blob_is_refused_rather_than_returned_short() {
    // Truncation is what the last-block flag exists for: without it a prefix of the chunks
    // verifies.
    let (_directory, store) = store(7);
    let content = incompressible(400_000);
    let id = store.put(&content).expect("store it");
    let path = store.path_of(&id).expect("a path");

    let stored = std::fs::read(&path).expect("read the file");
    std::fs::write(&path, &stored[..stored.len() / 2]).expect("truncate it");

    assert!(
        store.get(&id).is_err(),
        "a truncated blob was returned as if it were whole"
    );
}

#[test]
fn a_blob_written_under_another_key_does_not_decrypt() {
    let directory = tempfile::tempdir().expect("a directory");
    let root = directory.path().join("blobs");
    let content = b"Dear Ada, the frobnicator arrives on Thursday.";

    let mine = BlobStore::open(&root, &keys(8)).expect("open");
    let id = mine.put(content).expect("store it");

    let theirs = BlobStore::open(&root, &keys(9)).expect("open");
    assert!(
        theirs.get(&id).is_err(),
        "another installation's key read the mail"
    );
}

#[test]
fn the_same_content_has_the_same_id_within_one_store() {
    let (_directory, store) = store(10);
    let content = b"one attachment, forwarded around a team";

    let first = store.put(content).expect("store it");
    let second = store.put(content).expect("store it again");

    assert_eq!(first, second, "dedup must survive the keyed id");
    assert_eq!(blob_files(&store).len(), 1, "one file, not two");
}

#[test]
fn the_same_content_has_different_ids_under_different_keys() {
    // A directory listing must not confirm that this mailbox holds a file the reader already has.
    let content = b"a known file somebody is looking for";

    let (_one, mine) = store(11);
    let (_two, theirs) = store(12);

    let here = mine.put(content).expect("store it");
    let there = theirs.put(content).expect("store it");

    assert_ne!(
        here, there,
        "the same bytes are named the same in two stores, so content equality leaks"
    );
}

#[test]
fn a_blob_id_is_not_the_plain_digest_of_its_content() {
    let (_directory, store) = store(13);
    let content = b"a known file somebody is looking for";

    let id = store.put(content).expect("store it");
    assert_ne!(
        id.as_str(),
        blake3::hash(content).to_hex().as_str(),
        "the id is the unkeyed digest, so anybody can recompute it"
    );
}

#[test]
fn every_corpus_message_round_trips_through_the_encrypted_store() {
    let (_directory, store) = store(14);

    for fixture in test_corpus::all() {
        let id = store.put(fixture.bytes()).expect("store the message");
        assert_eq!(
            store.get(&id).expect("read it back"),
            fixture.bytes(),
            "{} did not survive the encrypted store",
            fixture.name()
        );
    }
}
