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

// -- opening ------------------------------------------------------------------

/// What `o` offers for `message`, as the Mac asks for it.
fn offered(session: &Session, message: i64) -> postio_ffi::OpenChoicesFfi {
    session.open_choices(message)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_with_attachments_offers_them_by_name_and_size() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[
            Seed::new("1", "text/plain").downloaded(b"words"),
            Seed::new("2", "application/pdf")
                .named("report.pdf")
                .downloaded(b"%PDF-1.4"),
        ],
    )
    .await;

    let choices = offered(&session, message);
    let names: Vec<&str> = choices
        .parts
        .iter()
        .map(|part| part.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["report.pdf"],
        "the body's own text part is not a chip"
    );
    assert_eq!(choices.nothing, None, "there is something to open");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_with_nothing_to_open_says_so_in_the_shared_words() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "text/plain").downloaded(b"hi")],
    )
    .await;

    let choices = offered(&session, message);
    assert!(choices.parts.is_empty());
    assert_eq!(
        choices.nothing.as_deref(),
        Some(postio_ui::focus_target::NOTHING_TO_OPEN)
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_a_downloaded_part_writes_a_private_copy_under_its_name() {
    use std::os::unix::fs::PermissionsExt;
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("2", "application/pdf")
            .named("report.pdf")
            .downloaded(b"%PDF-1.4")],
    )
    .await;
    let part = offered(&session, message).parts[0].id;

    let path = session
        .open_attachment(message, part)
        .await
        .expect("a part that is here opens");
    let path = std::path::PathBuf::from(path);
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some("report.pdf")
    );
    assert_eq!(std::fs::read(&path).expect("the copy"), b"%PDF-1.4");
    let mode = std::fs::metadata(path.parent().expect("a directory"))
        .expect("its directory")
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "nobody but this user can read the copy");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hostile_file_name_cannot_write_outside_the_copy_s_directory() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("2", "text/plain")
            .named("../../evil.txt")
            .downloaded(b"x")],
    )
    .await;
    let part = offered(&session, message).parts[0].id;
    let path = std::path::PathBuf::from(
        session
            .open_attachment(message, part)
            .await
            .expect("it opens under a safe name"),
    );
    assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("evil.txt"));
    assert!(!path.to_string_lossy().contains(".."));
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_part_that_cannot_be_fetched_opens_nothing_and_says_why() {
    let (session, message) = with_parts(
        "multipart/mixed",
        &[Seed::new("1", "application/zip").named("archive.zip")],
    )
    .await;
    let part = offered(&session, message).parts[0].id;
    let refused = session
        .open_attachment(message, part)
        .await
        .expect_err("no bytes, no file")
        .to_string();
    assert!(refused.contains("syncing"), "got {refused:?}");
    session.shutdown();
}

// -- two accounts -------------------------------------------------------------

/// A server holding one inbox message with a PDF attachment whose bytes are
/// `payload`, so which server answered is readable from the bytes.
fn server_with_report(payload: &str) -> postio_account::backend::MockBackend {
    use postio_account::backend::{BodyStructure, MockBackend, MockMailbox, MockMessage, PartNode};
    let text = format!("See attached: {payload}");
    let raw = format!(
        "Message-ID: <report@example.com>\r\nFrom: ada@example.com\r\n\
         Subject: Report\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n\
         --parts\r\nContent-Type: text/plain\r\n\r\n{text}\r\n\
         --parts\r\nContent-Type: application/pdf\r\n\
         Content-Disposition: attachment; filename=report.pdf\r\n\
         Content-Transfer-Encoding: 7bit\r\n\r\n{payload}\r\n--parts--\r\n"
    );
    MockBackend::builder()
        .mailbox(
            MockMailbox::new("INBOX").message(
                MockMessage::new(raw.into_bytes())
                    .with_structure(BodyStructure::from_parts(
                        "multipart/mixed",
                        [
                            PartNode::new("1", "text/plain", text.len() as u64)
                                .with_encoding("7bit"),
                            PartNode::new("2", "application/pdf", payload.len() as u64)
                                .with_encoding("7bit")
                                .with_filename("report.pdf"),
                        ],
                    ))
                    .with_part("1", text.as_bytes())
                    .with_part("2", payload.as_bytes()),
            ),
        )
        .build()
}

/// A session with two syncing accounts, each over its own server. Returns the
/// session, and the id and attachment of the message that lives on the
/// second account's server; the first account's server holds a message of its
/// own, so that an engine asked for the wrong message has something to
/// answer with.
async fn two_accounts() -> (std::sync::Arc<Session>, i64, i64) {
    use postio_model::{Account, EmailAddress};
    use postio_storage::repository::AccountRepository;
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");
    let mut accounts = Vec::new();
    {
        let connection = database.connect().await.expect("a connection");
        for name in ["first", "second"] {
            let mut account = Account::new(
                name,
                EmailAddress::new(None::<String>, format!("{name}@example.com")),
            );
            AccountRepository::new(&connection)
                .create(&mut account)
                .await
                .expect("an account");
            accounts.push(account.id);
        }
    }
    let session = Session::open(
        SessionOptions::in_memory_with(database.clone()).with_blobs_for_test(blobs, scratch),
    )
    .expect("a session");
    session.adopt_mock_engine_for(
        accounts[0],
        std::sync::Arc::new(server_with_report("%PDF first account")),
    );
    session.adopt_mock_engine_for(
        accounts[1],
        std::sync::Arc::new(server_with_report("%PDF second account")),
    );
    // Wait for the second account's sync to bring its message in.
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    let message = loop {
        let connection = database.read().await.expect("a read connection");
        let found = postio_storage::sql::first(
            &connection,
            "SELECT id FROM messages WHERE account_id = ?1",
            [accounts[1].get()],
            |row| postio_storage::sql::RowExt::col::<i64>(row, 0),
        )
        .await
        .expect("a query");
        if let Some(id) = found {
            break id;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the second account never synced its message"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let part = offered(&session, message).parts[0].id;
    (session, message, part)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_part_of_the_second_account_comes_from_the_second_accounts_server() {
    let (session, message, part) = two_accounts().await;
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        session.part_bytes(message, "2".to_string()),
    )
    .await
    .expect("the fetch asked the engine that can answer it")
    .expect("the part's bytes");
    assert_eq!(bytes, b"%PDF second account");

    let path = session
        .open_attachment(message, part)
        .await
        .expect("an attachment of the second account opens");
    assert_eq!(std::fs::read(path).unwrap(), b"%PDF second account");
    session.shutdown();
}
