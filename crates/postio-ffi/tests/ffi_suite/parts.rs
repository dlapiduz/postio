//! Getting an attachment's bytes out of a message.
//!
//! The part of #1572 that survives the three-pane shell: `part_bytes` is
//! how the reader hands a named part to whatever asked for it. The parts
//! panel -- listing, saving, exporting -- went with that shell, and Focus's
//! reader brings its own surface (specs/009-focus-macos).

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::{Attachment, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// How a part is described to the seeder: its MIME path, type, size, the name
/// the sender gave it, and the bytes -- when the bytes are on this machine.
struct Seed<'a> {
    part_id: &'a str,
    mime: &'a str,
    filename: Option<&'a str>,
    bytes: Option<&'a [u8]>,
}

impl<'a> Seed<'a> {
    fn new(part_id: &'a str, mime: &'a str) -> Self {
        Seed {
            part_id,
            mime,
            filename: None,
            bytes: None,
        }
    }

    fn named(mut self, filename: &'a str) -> Self {
        self.filename = Some(filename);
        self
    }

    fn downloaded(mut self, bytes: &'a [u8]) -> Self {
        self.bytes = Some(bytes);
        self
    }
}

/// A session over a store holding one message made of `seeds`.
///
/// `content_type` is the message's own — what the tree hangs off. Every part
/// that names bytes has them written into the blob store first, so
/// "downloaded" in these tests means the same thing it means in the
/// application: the bytes are here, and reading them reaches no network.
async fn with_parts(content_type: &str, seeds: &[Seed<'_>]) -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let id = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.content_type = Some(content_type.to_owned());
        message.attachments = seeds
            .iter()
            .map(|seed| {
                let size = seed.bytes.map(|bytes| bytes.len() as u64).unwrap_or(4_096);
                let mut part =
                    Attachment::new(postio_model::MessageId::UNASSIGNED, seed.mime, size);
                part.part_id = Some(seed.part_id.to_owned());
                part.filename = seed.filename.map(str::to_owned);
                part.blob_id = seed
                    .bytes
                    .map(|bytes| blobs.put(bytes).expect("a part blob"));
                part
            })
            .collect();
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message")
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    (session, id.into())
}

// -- bytes -------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_downloaded_parts_bytes_cross_exactly() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"words"),
            Seed::new("2", "image/png")
                .named("cold.png")
                .downloaded(b"\x89PNG\r\n\x1a\n"),
        ],
    )
    .await;

    let bytes = session
        .part_bytes(message, "2".to_string())
        .await
        .expect("the part's own bytes");
    assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_part_that_is_not_on_this_machine_and_cannot_be_fetched_says_so() {
    // No engine — nothing is syncing this store — so there is no route to the
    // bytes at all. A sentence rather than an empty file: a zero-byte
    // attachment on disk looks like a saved file and is not one.
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "application/zip").named("archive.zip")],
    )
    .await;

    let refused = session
        .part_bytes(message, "1".to_string())
        .await
        .expect_err("bytes that are not here cannot be handed over")
        .to_string();
    assert!(
        refused.contains("syncing"),
        "the refusal has to name the reason, and this reason in particular: a \
         store that is not open and an account that is not syncing send the \
         user to two different places, and the one sentence they both get is \
         the one that sends them to the wrong one. Got: {refused:?}"
    );
    session.shutdown();
}
