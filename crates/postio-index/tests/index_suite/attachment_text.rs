//! The `attachments` half of the index: located text from attachments
//! already on this machine (spec 010 D10, T122).
//!
//! Keyed by content, not by the attachment row. A message's attachment rows
//! are a projection a refetch replaces (`set_attachment_blob` says why the
//! row id does not survive one), and since #1805 the immutable payload is a
//! `message_contents` row that every occurrence of the same message shares.
//! The text of an attachment is a fact about that payload: keyed by
//! `(content_id, position)` it survives a refetch, is extracted once however
//! many folders hold the message, and goes when the last occurrence goes.

use chrono::Utc;
use postio_extract::{EXTRACTOR_VERSION, Extracted, Limit, Location, Outcome, Unit};
use postio_index::index;
use postio_model::{
    AccountId, Attachment, AttachmentId, BlobId, ContentIdentity, MailboxId, Message, MessageId,
};
use postio_storage::Checkout;
use postio_storage::repository::MessageRepository;
use postio_storage::sql::{self, RowExt as _};
use postio_storage::test_support;

/// One message carrying `parts`: (file name, MIME type, blob on disk?).
async fn message_with(
    connection: &Checkout,
    account: AccountId,
    mailbox: MailboxId,
    identity: Option<ContentIdentity>,
    parts: &[(&str, &str, bool)],
) -> (MessageId, Vec<AttachmentId>) {
    let mut message = Message::new(account, mailbox, Utc::now());
    message.subject = Some("Atlas budget".into());
    message.server.content_identity = identity;
    for (position, (name, mime, stored)) in parts.iter().enumerate() {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, *mime, 1_000);
        attachment.filename = Some((*name).to_owned());
        attachment.part_id = Some(format!("{}", position + 2));
        attachment.blob_id = stored.then(|| BlobId::new(format!("blob-{name}")));
        message.attachments.push(attachment);
    }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create");
    let ids = sql::all(
        connection,
        "SELECT id FROM attachments WHERE message_id = ?1 ORDER BY position",
        [message.id.get()],
        |row| Ok(AttachmentId::new(row.col(0)?)),
    )
    .await
    .expect("the attachment ids");
    (message.id, ids)
}

fn sheet(row: u32, text: &str) -> Unit {
    Unit {
        location: Location::Sheet {
            name: "Summary".to_owned(),
            row,
        },
        text: text.to_owned(),
    }
}

async fn world() -> (postio_storage::Store, Checkout, AccountId, MailboxId) {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (store, connection, account.id, inbox)
}

async fn count(connection: &Checkout, sql_text: &str) -> i64 {
    sql::first(connection, sql_text, (), |row| row.col(0))
        .await
        .expect("a count")
        .unwrap_or(0)
}

#[tokio::test]
async fn the_half_creates_its_tables() {
    let (_store, connection, _, _) = world().await;
    for table in ["attachment_passages", "attachment_extraction"] {
        assert_eq!(
            count(
                &connection,
                &format!(
                    "SELECT count(*) FROM sqlite_schema WHERE type = 'table' AND name = '{table}'"
                )
            )
            .await,
            1,
            "{table}"
        );
    }
}

#[tokio::test]
async fn one_row_per_unit_as_extracted_and_folded_for_the_index() {
    let (_store, connection, account, inbox) = world().await;
    let (_, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[(
            "budget.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            true,
        )],
    )
    .await;
    let extracted = Extracted {
        units: vec![
            sheet(1, "Atlas budget template"),
            sheet(14, "Total Café budget FY26"),
        ],
        outcome: Outcome::Complete,
    };
    assert!(
        index::index_attachment_text(&connection, ids[0], &extracted)
            .await
            .expect("indexed")
    );

    assert_eq!(
        index::attachment_text(&connection, ids[0])
            .await
            .expect("read back"),
        vec![
            (
                Location::Sheet {
                    name: "Summary".into(),
                    row: 1
                },
                "Atlas budget template".to_owned()
            ),
            (
                Location::Sheet {
                    name: "Summary".into(),
                    row: 14
                },
                "Total Café budget FY26".to_owned()
            ),
        ],
        "the text as written, in order, each with where it was"
    );
    // Folded on the way in, as bodies are: "cafe" meets "Café".
    let matched = count(
        &connection,
        "SELECT count(*) FROM attachment_passages WHERE fts_match(text_search, 'cafe')",
    )
    .await;
    assert_eq!(matched, 1);

    // Indexing again replaces rather than adds.
    assert!(
        index::index_attachment_text(&connection, ids[0], &extracted)
            .await
            .expect("again")
    );
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_passages").await,
        2
    );
    assert_eq!(
        index::extraction_of(&connection, ids[0])
            .await
            .expect("its record"),
        Some(index::Extraction {
            version: EXTRACTOR_VERSION,
            outcome: "complete".to_owned(),
            units: 2,
        })
    );
}

#[tokio::test]
async fn a_failed_or_empty_extraction_is_recorded_so_it_is_not_tried_again() {
    let (_store, connection, account, inbox) = world().await;
    let (_, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[
            ("broken.pdf", "application/pdf", true),
            ("long.pdf", "application/pdf", true),
        ],
    )
    .await;
    index::index_attachment_text(
        &connection,
        ids[0],
        &Extracted {
            units: Vec::new(),
            outcome: Outcome::Failed,
        },
    )
    .await
    .expect("recorded");
    index::index_attachment_text(
        &connection,
        ids[1],
        &Extracted {
            units: vec![Unit {
                location: Location::Page(1),
                text: "first page".into(),
            }],
            outcome: Outcome::Truncated(Limit::Time),
        },
    )
    .await
    .expect("recorded");
    assert!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("the queue")
            .is_empty(),
        "tried is not missing, however it went (#500's lesson)"
    );
    assert_eq!(
        index::extraction_of(&connection, ids[1])
            .await
            .expect("record")
            .map(|e| e.outcome),
        Some("truncated".to_owned())
    );
}

#[tokio::test]
async fn deleting_the_message_takes_its_text_with_it() {
    let (_store, connection, account, inbox) = world().await;
    let (message, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[("notes.txt", "text/plain", true)],
    )
    .await;
    index::index_attachment_text(
        &connection,
        ids[0],
        &Extracted {
            units: vec![Unit {
                location: Location::Line(1),
                text: "harbor notes".into(),
            }],
            outcome: Outcome::Complete,
        },
    )
    .await
    .expect("indexed");
    MessageRepository::new(&connection)
        .delete(&[message])
        .await
        .expect("delete");
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_passages").await,
        0
    );
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_extraction").await,
        0
    );
}

#[tokio::test]
async fn a_second_occurrence_of_the_same_content_shares_one_extraction() {
    let (_store, connection, account, inbox) = world().await;
    let identity = Some(ContentIdentity::new("jmap-email", "email-7"));
    let parts = [("budget.pdf", "application/pdf", true)];
    let (first, first_ids) =
        message_with(&connection, account, inbox, identity.clone(), &parts).await;
    let (_, second_ids) = message_with(&connection, account, inbox, identity, &parts).await;
    let queue = index::attachments_missing_text(&connection, 10)
        .await
        .expect("the queue");
    assert_eq!(queue.len(), 1, "one payload, one extraction: {queue:?}");

    index::index_attachment_text(
        &connection,
        first_ids[0],
        &Extracted {
            units: vec![Unit {
                location: Location::Page(2),
                text: "Spend to date".into(),
            }],
            outcome: Outcome::Complete,
        },
    )
    .await
    .expect("indexed");
    assert!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("queue")
            .is_empty()
    );
    assert_eq!(
        index::attachment_text(&connection, second_ids[0])
            .await
            .expect("read")
            .len(),
        1,
        "the other occurrence reads the same text"
    );

    // Removing one occurrence keeps the text for the other.
    MessageRepository::new(&connection)
        .delete(&[first])
        .await
        .expect("delete");
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_passages").await,
        1
    );
}

#[tokio::test]
async fn the_queue_is_attachments_on_disk_with_no_current_row() {
    let (_store, connection, account, inbox) = world().await;
    let (message, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[
            ("here.pdf", "application/pdf", true),
            ("never-fetched.pdf", "application/pdf", false),
            ("done.txt", "text/plain", true),
        ],
    )
    .await;
    index::index_attachment_text(
        &connection,
        ids[2],
        &Extracted {
            units: Vec::new(),
            outcome: Outcome::Complete,
        },
    )
    .await
    .expect("indexed");

    let queue = index::attachments_missing_text(&connection, 10)
        .await
        .expect("the queue");
    assert_eq!(queue.len(), 1, "{queue:?}");
    let missing = &queue[0];
    assert_eq!(missing.attachment, ids[0]);
    assert_eq!(missing.message, message);
    assert_eq!(missing.blob, BlobId::new("blob-here.pdf"));
    assert_eq!(missing.mime_type, "application/pdf");
    assert_eq!(missing.name.as_deref(), Some("here.pdf"));

    // The event path asks about named messages only.
    assert_eq!(
        index::attachments_missing_text_of(&connection, &[message])
            .await
            .expect("named")
            .len(),
        1
    );
    assert!(
        index::attachments_missing_text_of(&connection, &[MessageId::new(message.get() + 999)])
            .await
            .expect("named")
            .is_empty()
    );

    // A row made by an older extractor is missing again.
    sql::execute(
        &connection,
        "UPDATE attachment_extraction SET version = version - 1",
        (),
    )
    .await
    .expect("age it");
    assert_eq!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("queue")
            .len(),
        2
    );
}

#[tokio::test]
async fn a_schema_version_bump_drops_the_half_and_rebuilds_it_empty() {
    let (_store, connection, account, inbox) = world().await;
    let (_, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[("notes.txt", "text/plain", true)],
    )
    .await;
    index::index_attachment_text(
        &connection,
        ids[0],
        &Extracted {
            units: vec![Unit {
                location: Location::Line(1),
                text: "harbor".into(),
            }],
            outcome: Outcome::Complete,
        },
    )
    .await
    .expect("indexed");
    sql::execute(
        &connection,
        "UPDATE search_schema SET version = 0 WHERE half = 'attachments'",
        (),
    )
    .await
    .expect("an old store");
    index::ensure_schema(&connection)
        .await
        .expect("schema again");
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_passages").await,
        0
    );
    assert_eq!(
        count(&connection, "SELECT count(*) FROM attachment_extraction").await,
        0
    );
    assert_eq!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("queue")
            .len(),
        1
    );
    assert_eq!(
        count(
            &connection,
            "SELECT version FROM search_schema WHERE half = 'attachments'"
        )
        .await,
        index::ATTACHMENTS_SCHEMA_VERSION
    );
}

#[tokio::test]
async fn an_account_cleared_for_reindexing_is_extracted_again() {
    let (_store, connection, account, inbox) = world().await;
    let (_, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[("notes.txt", "text/plain", true)],
    )
    .await;
    index::index_attachment_text(
        &connection,
        ids[0],
        &Extracted {
            units: Vec::new(),
            outcome: Outcome::Complete,
        },
    )
    .await
    .expect("indexed");
    assert_eq!(
        index::clear_account_attachment_index(&connection, account.get())
            .await
            .expect("cleared"),
        1
    );
    assert_eq!(
        index::attachments_missing_text(&connection, 10)
            .await
            .expect("queue")
            .len(),
        1
    );
}

#[test]
fn a_location_survives_its_column() {
    for location in [
        Location::Page(2),
        Location::Sheet {
            name: "Q3: final".into(),
            row: 14,
        },
        Location::Slide(3),
        Location::Paragraph(7),
        Location::Line(1),
        Location::Table { index: 1, row: 4 },
        Location::ImageText,
    ] {
        let encoded = index::encode_location(&location);
        assert_eq!(
            index::decode_location(&encoded),
            Some(location),
            "{encoded}"
        );
    }
    assert_eq!(index::encode_location(&Location::Page(2)), "page:2");
    assert_eq!(index::decode_location("nonsense"), None);
}
