//! Fetched bytes going straight to disk — ADR 0017, axis 2.

use postio_imap::backend::BodyPart;
use postio_imap::backend::{BodySink, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_imap::cancel::CancelToken;
use postio_model::UidValidity;
use postio_storage::BlobStore;
use postio_sync::blob_sink::BlobSink;

/// A blob store in a directory that lives as long as the returned guard.
///
/// `postio_storage::test_support::temp` rather than `tempfile` directly: it is
/// what every other test in this crate uses to get a directory, and a second
/// way of doing it would be a second thing to keep in step.
fn store() -> (postio_storage::test_support::TempDatabase, BlobStore) {
    let database = postio_storage::test_support::temp();
    let store = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("open");
    (database, store)
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
        .expect("read the store")
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
    out
}

#[tokio::test]
async fn a_fetch_through_the_sink_lands_in_the_blob_store() {
    // The bytes never exist whole in this process: they go socket, chunk,
    // file. What is asserted is that they arrived intact anyway, and that the
    // sink hands back the id the rest of the system stores.
    let (_database, blobs) = store();
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new("INBOX")
                .uid_validity(UidValidity::new(1))
                .message(MockMessage::new(
                    &b"Subject: streamed\r\n\r\nthe body, arriving in pieces"[..],
                )),
        )
        .build();
    backend.connect().await.expect("connect");

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    backend
        .fetch_body(
            "INBOX",
            &postio_model::RemoteId::new("1:1"),
            &mut sink,
            &CancelToken::new(),
        )
        .await
        .expect("fetch");

    let id = sink
        .finished_blob()
        .expect("the fetch completed, so there is a blob");
    assert_eq!(
        blobs.get(&id).expect("read"),
        b"Subject: streamed\r\n\r\nthe body, arriving in pieces"
    );
}

#[tokio::test]
async fn a_cancelled_fetch_leaves_neither_a_blob_nor_a_temp_file() {
    // The sink contract: without `finish` the bytes are a fragment, and a
    // fragment stored as a message is worse than no message.
    //
    // Nothing is published -- but the temporary file does not survive either,
    // because `BlobWriter`'s drop removes it. Worth pinning: a cancelled fetch
    // is an *ordinary* event here (the user closed the message, the link
    // dropped, `max_body_bytes` changed its mind), so if it leaked a `.part`
    // file every time, the store would grow debris under normal use and
    // `purge_temporary` would be load-bearing rather than a backstop.
    //
    // What `purge_temporary` is actually for is the case a drop cannot cover:
    // a power cut or a kill -9, where no destructor runs at all.
    let (_database, blobs) = store();

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"half of a message").await.expect("a chunk");
    // No `finish` -- exactly what a cancelled or torn-off fetch leaves.
    drop(sink);

    assert!(blob_files(&blobs).is_empty(), "nothing was published");
    assert_eq!(
        blobs.purge_temporary().expect("purge"),
        0,
        "and nothing was left for the backstop to sweep"
    );
}

#[tokio::test]
async fn a_sink_that_never_finished_has_no_blob_to_offer() {
    // `finished_blob` is the only way to get the id, and it is `None` until
    // `finish` -- so a caller cannot accidentally treat a fragment as a body.
    let (_database, blobs) = store();

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"partial").await.expect("a chunk");

    assert!(sink.finished_blob().is_none());
}

#[tokio::test]
async fn the_same_bytes_streamed_and_stored_at_once_are_one_blob() {
    // Content addressing has to survive the push form, or a body fetched by
    // the sync path and the same body written by any other path would be two
    // files. `BodyPart::Whole` is the shape the backfill uses.
    let (_database, blobs) = store();
    let raw = &b"Subject: twice\r\n\r\nthe very same bytes"[..];
    let at_once = blobs.put(raw).expect("put");

    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new("INBOX")
                .uid_validity(UidValidity::new(1))
                .message(MockMessage::new(raw)),
        )
        .build();
    backend.connect().await.expect("connect");

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    backend
        .fetch_part(
            "INBOX",
            &postio_model::RemoteId::new("1:1"),
            &BodyPart::Whole,
            &mut sink,
            &CancelToken::new(),
        )
        .await
        .expect("fetch");

    assert_eq!(sink.finished_blob().expect("a blob"), at_once);
    assert_eq!(blob_files(&blobs).len(), 1);
}
