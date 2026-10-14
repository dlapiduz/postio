//! The attachment indexer: text out of attachments that are already on
//! this machine, and never a byte fetched to get it (spec 010 D10, FR-050,
//! US8 scenarios 1 and 2; T124).
//!
//! The search seed is the honest case: six files whose bytes are stored,
//! each saying "Atlas budget" where its format keeps words, and two PDFs
//! whose bytes were never downloaded. The rest are the ways a pass can go
//! wrong: a server that would happily serve the bytes if asked, a file the
//! extractor cannot read, a blob the store has lost, and bytes that arrive
//! after the pass has run.

use std::time::Duration;

use postio_account::backend::{
    BodyStructure, Disposition, MailBackend, MockBackend, MockMailbox, MockMessage, PartNode,
};
use postio_account::cancel::CancelToken;
use postio_core::bridge::EventHub;
use postio_extract::Location;
use postio_index::index;
use postio_model::{AttachmentId, BlobId, Message, MessageId, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::sql::{self, RowExt as _};
use postio_storage::{BlobStore, Checkout, test_support};

/// Every attachment row with this file name.
async fn attachments_named(connection: &Checkout, name: &str) -> Vec<AttachmentId> {
    sql::all(
        connection,
        "SELECT id FROM attachments WHERE filename = ?1 ORDER BY id",
        [name],
        |row| Ok(AttachmentId::new(row.col(0)?)),
    )
    .await
    .expect("attachments by name")
}

/// How an attachment's extraction went, read straight from its record.
#[derive(Debug, PartialEq, Eq)]
struct Extraction {
    version: u32,
    outcome: String,
    units: u32,
}

async fn extraction_of(
    connection: &Checkout,
    attachment: AttachmentId,
) -> postio_storage::Result<Option<Extraction>> {
    sql::first(
        connection,
        "SELECT e.version, e.outcome, e.units
           FROM attachment_extraction e
           JOIN messages m ON m.content_id = e.content_id
           JOIN attachments a ON a.message_id = m.id AND a.position = e.position
          WHERE a.id = ?1",
        [attachment.get()],
        |row| {
            Ok(Extraction {
                version: u32::try_from(row.col::<i64>(0)?).unwrap_or(0),
                outcome: row.col(1)?,
                units: u32::try_from(row.col::<i64>(2)?).unwrap_or(0),
            })
        },
    )
    .await
}

/// Poll `check` until it holds or the patience dial runs out.
async fn eventually<F, Fut>(mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + postio_test_support::scaled(Duration::from_secs(10));
    loop {
        if check().await {
            return true;
        }
        if std::time::Instant::now() > deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_downloaded_attachment_of_the_search_seed_is_read_where_it_says_it() {
    let (database, _account) = postio_demo::search_demo().await;
    let scratch = tempfile::tempdir().expect("a blob directory");
    let blobs = BlobStore::open(scratch.path().to_path_buf(), &test_support::blob_keys())
        .expect("a blob store");
    postio_demo::store_search_blobs(&blobs).expect("the seed's files");

    let indexed = postio_session::index_local_attachments(&database, &blobs)
        .await
        .expect("the pass runs");
    assert!(indexed >= 6, "every stored file at least once: {indexed}");

    let connection = database.connect().await.expect("a connection");
    let expectations: [(&str, Location, &str); 6] = [
        (
            "Atlas-budget-template.xlsx",
            Location::Sheet {
                name: "Summary".to_owned(),
                row: 14,
            },
            "Total Atlas budget FY26",
        ),
        (
            "Atlas-Q3-budget.xlsx",
            Location::Sheet {
                name: "Q3".to_owned(),
                row: 6,
            },
            "Total Atlas budget Q3",
        ),
        (
            "Atlas-Sep-actuals.pdf",
            Location::Page(2),
            "Spend to date against Atlas budget: 71%",
        ),
        (
            "Atlas-budget-memo.docx",
            Location::Paragraph(2),
            "The Atlas budget holds the platform roles",
        ),
        (
            "Atlas-budget-review.pptx",
            Location::Slide(2),
            "Where the Atlas budget went in Q3",
        ),
        (
            "atlas-budget-notes.txt",
            Location::Line(2),
            "platform roles move the Atlas budget up",
        ),
    ];
    for (name, location, words) in expectations {
        let ids = attachments_named(&connection, name).await;
        assert!(!ids.is_empty(), "the seed attaches {name}");
        for id in ids {
            let text = index::attachment_text(&connection, id)
                .await
                .expect("its text");
            assert!(
                text.iter()
                    .any(|(at, said)| *at == location && said.contains(words)),
                "{name}: expected {words:?} at {location:?}, read {text:?}"
            );
            assert_eq!(
                extraction_of(&connection, id)
                    .await
                    .expect("its record")
                    .map(|record| record.outcome),
                Some("complete".to_owned()),
                "{name}"
            );
        }
    }

    // The two PDFs whose bytes never came down are not read, not recorded,
    // and not queued: there is nothing on this machine to read.
    for name in ["Contractor-invoices-Sep.pdf", "Invoice-2026-08.pdf"] {
        for id in attachments_named(&connection, name).await {
            assert_eq!(
                extraction_of(&connection, id).await.expect("record"),
                None,
                "{name} was never downloaded"
            );
        }
    }
    assert!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("the queue")
            .is_empty()
    );
    drop(connection);

    assert_eq!(
        postio_session::index_local_attachments(&database, &blobs)
            .await
            .expect("a second pass"),
        0,
        "a caught-up store costs one query and no writes"
    );
}

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

/// A message whose PDF stays on the server: the structure says it is
/// there, and the mock would serve its bytes to anyone who asked.
fn message_with_a_remote_pdf() -> MockMessage {
    let structure = BodyStructure::from_parts(
        "multipart/mixed",
        [
            PartNode::new("1", "text/plain", 32)
                .with_charset("utf-8")
                .with_encoding("7bit"),
            PartNode::new("2", "application/pdf", 48_000)
                .with_encoding("base64")
                .with_filename("Harbor-survey.pdf")
                .with_disposition(Disposition::Attachment),
        ],
    );
    MockMessage::new(
        b"From: Lena Park <lena@example.com>\r\n\
          Subject: Harbor survey\r\n\
          Message-ID: <harbor-survey@example.com>\r\n\
          Content-Type: multipart/mixed; boundary=mix\r\n\
          \r\n\
          --mix\r\n\
          Content-Type: text/plain\r\n\
          \r\n\
          The survey is attached.\r\n\
          --mix--\r\n"
            .to_vec(),
    )
    .with_structure(structure)
    .with_part("1", &b"The survey is attached."[..])
    .with_part("2", &b"JVBERi0xLjQK"[..])
}

/// One message in `mailbox` with one attachment, its bytes stored.
async fn local_attachment(
    connection: &Checkout,
    blobs: &BlobStore,
    account: postio_model::AccountId,
    mailbox: postio_model::MailboxId,
    name: &str,
    mime: &str,
    bytes: Option<&[u8]>,
) -> (MessageId, AttachmentId) {
    let mut message = Message::new(account, mailbox, chrono::Utc::now());
    message.subject = Some(format!("About {name}"));
    let mut attachment = postio_model::Attachment::new(MessageId::UNASSIGNED, mime, 100);
    attachment.filename = Some(name.to_owned());
    attachment.part_id = Some("2".to_owned());
    attachment.blob_id = bytes.map(|bytes| blobs.put(bytes).expect("stored"));
    message.attachments.push(attachment);
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    let id = attachments_named(connection, name).await[0];
    (message.id, id)
}

#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_that_was_never_downloaded_is_never_fetched() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;

    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(message_with_a_remote_pdf()),
        )
        .build();
    backend.connect().await.expect("connect");
    postio_sync::sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("header sync");
    let remote = attachments_named(&connection, "Harbor-survey.pdf").await;
    assert_eq!(remote.len(), 1, "the structure put the PDF on the message");
    let remote_message: i64 = sql::first(
        &connection,
        "SELECT message_id FROM attachments WHERE id = ?1",
        [remote[0].get()],
        |row| row.col(0),
    )
    .await
    .expect("its message")
    .expect("a row");
    // A sentinel whose bytes *are* here, so the pass has something to do
    // and the test something to wait for.
    let (local_message, local) = local_attachment(
        &connection,
        &blobs,
        account.id,
        inbox.id,
        "local-notes.txt",
        "text/plain",
        Some(b"notes kept on this machine\n"),
    )
    .await;
    drop(connection);
    let calls_before = backend.calls();

    let hub = EventHub::new();
    let sink = hub.sink();
    let indexer = postio_session::spawn_attachment_indexer(
        database.clone(),
        blobs.clone(),
        Some(hub.subscribe("attachments")),
        &tokio::runtime::Handle::current(),
    );
    for message in [MessageId::new(remote_message), local_message] {
        sink.emit(postio_core::Event::BodyLoaded {
            account: account.id,
            message,
        });
    }

    let database_for_check = database.clone();
    assert!(
        eventually(|| {
            let database = database_for_check.clone();
            async move {
                let connection = database.connect().await.expect("checkout");
                extraction_of(&connection, local)
                    .await
                    .expect("record")
                    .is_some()
            }
        })
        .await,
        "the downloaded attachment was indexed"
    );
    // And once more after a full debounce, so a late request would have
    // been made by now.
    tokio::time::sleep(postio_test_support::scaled(Duration::from_secs(1))).await;
    indexer.abort();

    assert_eq!(
        backend.calls(),
        calls_before,
        "the indexer asked the server for something: FR-050 says it may not"
    );
    assert!(
        backend.body_fetches().is_empty(),
        "{:?}",
        backend.body_fetches()
    );
    let connection = database.connect().await.expect("checkout");
    assert_eq!(
        extraction_of(&connection, remote[0]).await.expect("record"),
        None,
        "nothing on this machine to read, so nothing recorded"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_cannot_be_read_is_recorded_and_the_pass_moves_on() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (_, broken) = local_attachment(
        &connection,
        &blobs,
        account.id,
        inbox,
        "broken.pdf",
        "application/pdf",
        Some(b"%PDF-1.7 and then nothing a reader can use"),
    )
    .await;
    // A row that says its bytes are here when the store has lost them.
    let (lost_message, lost) = local_attachment(
        &connection,
        &blobs,
        account.id,
        inbox,
        "lost.pdf",
        "application/pdf",
        None,
    )
    .await;
    MessageRepository::new(&connection)
        .set_attachment_blob(lost_message, "2", &BlobId::new("0".repeat(64)))
        .await
        .expect("a dangling blob id");
    let (_, fine) = local_attachment(
        &connection,
        &blobs,
        account.id,
        inbox,
        "fine.txt",
        "text/plain",
        Some(b"Harbor survey notes\n"),
    )
    .await;
    drop(connection);

    postio_session::index_local_attachments(&database, &blobs)
        .await
        .expect("the pass runs over all three");

    let connection = database.connect().await.expect("checkout");
    let outcome = |id| {
        let connection = &connection;
        async move {
            extraction_of(connection, id)
                .await
                .expect("record")
                .map(|record| record.outcome)
        }
    };
    assert_eq!(outcome(broken).await.as_deref(), Some("failed"));
    assert_eq!(outcome(lost).await.as_deref(), Some("failed"));
    assert_eq!(outcome(fine).await.as_deref(), Some("complete"));
    assert!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("queue")
            .is_empty(),
        "a failure is recorded, so the next pass does not try it again"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bytes_stored_later_are_indexed_on_the_event_that_says_so() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (message, attachment) = local_attachment(
        &connection,
        &blobs,
        account.id,
        inbox,
        "later.txt",
        "text/plain",
        None,
    )
    .await;
    drop(connection);

    let hub = EventHub::new();
    let sink = hub.sink();
    let indexer = postio_session::spawn_attachment_indexer(
        database.clone(),
        blobs.clone(),
        Some(hub.subscribe("attachments")),
        &tokio::runtime::Handle::current(),
    );

    // The bytes land the way a payload fetch lands them: stored, recorded
    // against the part, then announced.
    let blob = blobs
        .put(b"the Atlas figures arrived late\n")
        .expect("stored");
    {
        let connection = database.connect().await.expect("checkout");
        MessageRepository::new(&connection)
            .set_attachment_blob(message, "2", &blob)
            .await
            .expect("recorded");
    }
    sink.emit(postio_core::Event::BodyLoaded {
        account: account.id,
        message,
    });

    let check = database.clone();
    assert!(
        eventually(|| {
            let database = check.clone();
            async move {
                let connection = database.connect().await.expect("checkout");
                index::attachment_text(&connection, attachment)
                    .await
                    .expect("text")
                    .iter()
                    .any(|(at, said)| *at == Location::Line(1) && said.contains("Atlas"))
            }
        })
        .await,
        "the attachment was not indexed after the event that said its bytes arrived"
    );
    indexer.abort();
}
