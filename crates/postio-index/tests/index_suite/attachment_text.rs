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
        extraction_of(&connection, ids[0])
            .await
            .expect("its record"),
        Some(Extraction {
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
        extraction_of(&connection, ids[1])
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

#[tokio::test]
async fn named_units_are_read_back_by_where_they_are_whatever_a_sheet_is_called() {
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
    let odd = Location::Sheet {
        name: "Q3 \"final\" \\ v2".into(),
        row: 2,
    };
    let extracted = Extracted {
        units: vec![
            sheet(1, "Atlas budget template"),
            Unit {
                location: odd.clone(),
                text: "Kestrel survey".into(),
            },
        ],
        outcome: Outcome::Complete,
    };
    index::index_attachment_text(&connection, ids[0], &extracted)
        .await
        .expect("indexed");
    assert_eq!(
        index::attachment_units(&connection, &[(ids[0], odd.clone())])
            .await
            .expect("read"),
        vec![(ids[0], odd, "Kestrel survey".to_owned())],
        "one read, by attachment and location; the sheet's name quoted whatever it holds"
    );
    assert!(
        index::attachment_units(&connection, &[(ids[0], Location::Page(9))])
            .await
            .expect("read")
            .is_empty(),
        "a unit that is not there is left out"
    );
}

/// The parts the search still owes a reading, as the index keeps them for
/// it (`attachment_text_owed`), beside what the indexer's queue would say
/// from scratch: every downloaded part with no row from this extractor.
async fn owed_and_queued(connection: &Checkout) -> (Vec<(i64, i64)>, Vec<(i64, i64)>) {
    let pairs = |row: &turso::Row| Ok((row.col(0)?, row.col(1)?));
    let owed = sql::all(
        connection,
        "SELECT content_id, position FROM attachment_text_owed ORDER BY 1, 2",
        (),
        pairs,
    )
    .await
    .expect("what is owed");
    let queued = sql::all(
        connection,
        "SELECT DISTINCT m.content_id, a.position
           FROM attachments a JOIN messages m ON m.id = a.message_id
          WHERE a.blob_id IS NOT NULL AND m.content_id IS NOT NULL
            AND NOT EXISTS (SELECT 1 FROM attachment_extraction e
                             WHERE e.content_id = m.content_id AND e.position = a.position
                               AND e.version = ?1)
          ORDER BY 1, 2",
        [i64::from(EXTRACTOR_VERSION)],
        pairs,
    )
    .await
    .expect("what the queue holds");
    (owed, queued)
}

async fn assert_owed_is_queued(connection: &Checkout, step: &str, expected: usize) {
    let (owed, queued) = owed_and_queued(connection).await;
    assert_eq!(
        owed, queued,
        "after {step}: the kept set drifted from the queue"
    );
    assert_eq!(owed.len(), expected, "after {step}: {owed:?}");
}

fn read(text: &str) -> Extracted {
    Extracted {
        units: vec![Unit {
            location: Location::Line(1),
            text: text.to_owned(),
        }],
        outcome: Outcome::Complete,
    }
}

/// The search asks "is any downloaded attachment still unread?" on every
/// query, so it cannot ask the queue -- a walk of every attachment when the
/// answer is no. The index keeps the answer as a set the writes maintain,
/// and every write that moves the queue must move the set the same way.
#[tokio::test]
async fn what_the_search_is_owed_follows_the_queue_through_every_write() {
    let (_store, connection, account, inbox) = world().await;
    assert_owed_is_queued(&connection, "a fresh store", 0).await;

    let (first, ids) = message_with(
        &connection,
        account,
        inbox,
        None,
        &[
            ("here.txt", "text/plain", true),
            ("later.txt", "text/plain", false),
        ],
    )
    .await;
    assert_owed_is_queued(&connection, "a message with one part on disk", 1).await;

    index::index_attachment_text(&connection, ids[0], &read("harbor"))
        .await
        .expect("indexed");
    assert_owed_is_queued(&connection, "reading it", 0).await;

    MessageRepository::new(&connection)
        .set_attachment_blob(first, "3", &BlobId::new("blob-later.txt"))
        .await
        .expect("downloaded");
    assert_owed_is_queued(&connection, "downloading the second part", 1).await;

    // A second occurrence of one payload owes one reading, not two.
    let identity = Some(ContentIdentity::new("jmap-email", "email-9"));
    let parts = [("budget.txt", "text/plain", true)];
    let (shared, shared_ids) =
        message_with(&connection, account, inbox, identity.clone(), &parts).await;
    let (_, other_ids) = message_with(&connection, account, inbox, identity, &parts).await;
    assert_owed_is_queued(&connection, "two occurrences of one payload", 2).await;

    // Evicting one occurrence's bytes leaves the other's to read.
    sql::execute(
        &connection,
        "UPDATE attachments SET blob_id = NULL WHERE id = ?1",
        [shared_ids[0].get()],
    )
    .await
    .expect("evict one");
    assert_owed_is_queued(&connection, "evicting one occurrence", 2).await;
    sql::execute(
        &connection,
        "UPDATE attachments SET blob_id = NULL WHERE id = ?1",
        [other_ids[0].get()],
    )
    .await
    .expect("evict the other");
    assert_owed_is_queued(&connection, "evicting both", 1).await;
    sql::execute(
        &connection,
        "UPDATE attachments SET blob_id = 'blob-budget.txt' WHERE id = ?1",
        [shared_ids[0].get()],
    )
    .await
    .expect("download again");
    assert_owed_is_queued(&connection, "downloading it again", 2).await;

    index::index_attachment_text(&connection, shared_ids[0], &read("kestrel"))
        .await
        .expect("indexed");
    index::index_attachment_text(&connection, ids[1], &read("gannet"))
        .await
        .expect("indexed");
    assert_owed_is_queued(&connection, "reading everything", 0).await;

    // A row from an older extractor is owed again, and reading it again
    // settles it.
    sql::execute(
        &connection,
        "UPDATE attachment_extraction SET version = version - 1",
        (),
    )
    .await
    .expect("age them");
    assert_owed_is_queued(&connection, "an older extractor's rows", 3).await;
    index::index_attachment_text(&connection, ids[0], &read("harbor"))
        .await
        .expect("indexed");
    assert_owed_is_queued(&connection, "reading one of them again", 2).await;

    // A message going takes what it owed, unless another occurrence still
    // holds the bytes.
    sql::execute(
        &connection,
        "UPDATE attachments SET blob_id = 'blob-budget.txt' WHERE id = ?1",
        [other_ids[0].get()],
    )
    .await
    .expect("the other occurrence downloads too");
    assert_owed_is_queued(&connection, "both occurrences on disk", 2).await;
    MessageRepository::new(&connection)
        .delete(&[shared])
        .await
        .expect("delete one occurrence");
    assert_owed_is_queued(&connection, "deleting one occurrence", 2).await;
    MessageRepository::new(&connection)
        .delete(&[first])
        .await
        .expect("delete");
    assert_owed_is_queued(&connection, "deleting the first message", 1).await;

    // A half rebuilt from nothing owes everything on disk.
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
    assert_owed_is_queued(&connection, "rebuilding the half", 1).await;

    // And a store last opened by another extractor is counted again.
    sql::execute(
        &connection,
        "UPDATE search_schema SET version = 0 WHERE half = 'extractor'",
        (),
    )
    .await
    .expect("another extractor");
    sql::execute(&connection, "DELETE FROM attachment_text_owed", ())
        .await
        .expect("forget");
    index::ensure_schema(&connection)
        .await
        .expect("schema again");
    assert_owed_is_queued(&connection, "a new extractor", 1).await;
}
