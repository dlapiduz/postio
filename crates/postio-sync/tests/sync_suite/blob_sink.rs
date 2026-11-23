use postio_account::backend::BodyPart;
use postio_account::backend::{BodySink, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::UidValidity;
use postio_storage::BlobStore;
use postio_sync::blob_sink::BlobSink;

async fn store() -> (postio_storage::test_support::TempStore, BlobStore) {
    let database = postio_storage::test_support::temp().await;
    let store = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("open");
    (database, store)
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
    let (_database, blobs) = store().await;
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
    // A cancelled fetch is ordinary, so `BlobWriter`'s drop must remove the temp file;
    // `purge_temporary` is only the backstop for a power cut or kill -9.
    let (_database, blobs) = store().await;

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"half of a message").await.expect("a chunk");
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
    let (_database, blobs) = store().await;

    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"partial").await.expect("a chunk");

    assert!(sink.finished_blob().is_none());
}

#[tokio::test]
async fn a_chunk_after_finish_is_an_error() {
    let (_database, blobs) = store().await;
    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"the whole body").await.expect("a chunk");
    sink.finish().await.expect("finish");

    let error = sink
        .chunk(b"arrives too late")
        .await
        .expect_err("a chunk after finish must be refused");
    assert!(
        format!("{error}").contains("arrived after the fetch was finished"),
        "{error}"
    );
}

#[tokio::test]
async fn finishing_twice_is_an_error() {
    let (_database, blobs) = store().await;
    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"the whole body").await.expect("a chunk");
    sink.finish().await.expect("the first finish");

    let error = sink
        .finish()
        .await
        .expect_err("a second finish must be refused");
    assert!(format!("{error}").contains("finished twice"), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_storage_failure_while_finishing_reaches_the_caller_as_a_fetch_failure() {
    use std::os::unix::fs::PermissionsExt;

    let (_database, blobs) = store().await;
    let mut sink = BlobSink::new(&blobs).expect("a sink");
    sink.chunk(b"a body that never gets to land")
        .await
        .expect("a chunk");

    // The root loses its write bit so the shard directory cannot be created; restored before
    // cleanup.
    let root = blobs.root().to_path_buf();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o500))
        .expect("remove the store's write bit");

    let result = sink.finish().await;

    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
        .expect("restore the write bit so the temp directory can be cleaned up");

    let error = result.expect_err("a store that refuses the write must not report success");
    assert!(
        format!("{error}").contains("the blob store refused the fetched bytes"),
        "{error}"
    );
}

#[tokio::test]
async fn the_same_bytes_streamed_and_stored_at_once_are_one_blob() {
    let (_database, blobs) = store().await;
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
