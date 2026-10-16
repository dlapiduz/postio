//! What an account's mail costs, before a byte of it is fetched — ADR 0017.

use postio_storage::repository::{MessageRepository, StorageFootprint};
use postio_storage::sql::bind;
use postio_storage::test_support;

/// A headers-only message, the state the footprint is computed from.
fn message(
    mailbox: postio_model::MailboxId,
    account: postio_model::AccountId,
    uid: u32,
) -> postio_model::Message {
    let mut message = postio_model::Message::new(
        account,
        mailbox,
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 3, 1, 9, 0, uid).unwrap(),
    );
    message.server.uid = Some(postio_model::Uid::new(uid));
    message.server.uid_validity = Some(postio_model::UidValidity::new(1));
    message.sync.body_state = postio_model::BodyState::HeadersOnly;
    message
}

#[tokio::test]
async fn the_footprint_is_known_from_headers_alone() {
    // The nicest property of the whole measurement: `BODYSTRUCTURE` arrives
    // with the header sync, so `messages.size` and `attachments.size` are
    // populated for mail nobody has downloaded. Postio can say "1.4 GB of
    // mail, 11 GB of attachments" having spent nothing.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    // A message that is all words, and one that is mostly a PDF.
    let mut plain = message(inbox, account.id, 1);
    plain.size = 2_000;
    messages.create(&mut plain).await.expect("create");

    let mut with_payload = message(inbox, account.id, 2);
    with_payload.size = 1_000_000;
    with_payload.attachments = vec![{
        let mut attachment = postio_model::Attachment::new(
            postio_model::MessageId::UNASSIGNED,
            "application/pdf",
            990_000,
        );
        attachment.filename = Some("statement.pdf".to_owned());
        attachment.part_id = Some("2".to_owned());
        attachment
    }];
    messages.create(&mut with_payload).await.expect("create");

    let footprint = messages.footprint(account.id).await.expect("footprint");

    assert_eq!(footprint.messages, 2);
    assert_eq!(footprint.total_bytes, 1_002_000);
    assert_eq!(footprint.attachment_bytes, 990_000);
    // What the text axis will actually pull: everything that is not payload.
    assert_eq!(footprint.text_bytes(), 12_000);
    assert_eq!(footprint.local_bytes, 0, "nothing is downloaded yet");
}

#[tokio::test]
async fn an_account_with_no_mail_has_an_empty_footprint() {
    // The empty state owes an answer too, and it must not be a division by
    // zero or a claim of "0 B of 0 B" that reads like a bug.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, _inbox) = test_support::account_with_inbox(&connection).await;

    let footprint = MessageRepository::new(&connection)
        .footprint(account.id)
        .await
        .expect("footprint");

    assert_eq!(footprint, StorageFootprint::default());
    assert!(footprint.is_empty());
}

#[tokio::test]
async fn a_footprint_is_a_lower_bound_until_every_folder_has_synced_headers() {
    // The honesty this issue exists for. While the header pass is still
    // running the total climbs, and a number that grows every few seconds
    // looks broken -- so the surface must be able to say "over 1.4 GB" rather
    // than a total it is about to contradict.
    //
    // Complete means every selectable mailbox has a `last_full_sync_at`.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = message(inbox, account.id, 1);
    message.size = 2_000;
    messages.create(&mut message).await.expect("create");

    let footprint = messages.footprint(account.id).await.expect("footprint");
    assert!(
        !footprint.complete,
        "no folder has finished a header pass, so this is a floor"
    );

    connection
        .execute(
            "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, last_full_sync_at)
             VALUES (?1, ?2, 1, 1000)
             ON CONFLICT (mailbox_id) DO UPDATE SET last_full_sync_at = 1000",
            bind![inbox.get(), account.id.get()],
        )
        .await
        .expect("record a completed header pass");

    let footprint = messages.footprint(account.id).await.expect("footprint");
    assert!(footprint.complete, "now the total is a total");
}

#[tokio::test]
async fn local_bytes_count_only_what_is_actually_downloaded() {
    // What the progress line divides by. A message whose text is local but
    // whose payload is not has contributed its text and not its payload --
    // which is the ordinary steady state under ADR 0017, not a special case.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = message(inbox, account.id, 1);
    message.size = 100_000;
    message.sync.body_state = postio_model::BodyState::Partial;
    message.attachments = vec![postio_model::Attachment::new(
        postio_model::MessageId::UNASSIGNED,
        "application/pdf",
        90_000,
    )];
    messages.create(&mut message).await.expect("create");

    let footprint = messages.footprint(account.id).await.expect("footprint");
    assert_eq!(
        footprint.local_bytes, 10_000,
        "its words are here, its attachment is not"
    );
}

/// What the footprint and the read-receipt figure were before the store
/// kept them (#1508): the aggregates over every message an account holds.
/// The oracle the kept row is held to.
async fn recounted(
    connection: &postio_storage::Connection,
    account: postio_model::AccountId,
) -> (u64, u64, u64, u64, u64) {
    postio_storage::sql::one(
        connection,
        "SELECT
           (SELECT count(*) FROM messages
             WHERE account_id = ?1 AND deleted_locally = 0),
           (SELECT coalesce(sum(size), 0) FROM messages
             WHERE account_id = ?1 AND deleted_locally = 0),
           (SELECT coalesce(sum(a.size), 0) FROM attachments a
              JOIN messages m ON m.id = a.message_id
             WHERE m.account_id = ?1 AND m.deleted_locally = 0),
           (SELECT coalesce(sum(CASE m.body_state
                      WHEN 'full' THEN m.size
                      WHEN 'partial' THEN m.size - coalesce(
                          (SELECT sum(a.size) FROM attachments a
                            WHERE a.message_id = m.id AND a.blob_id IS NULL), 0)
                      ELSE 0 END), 0)
              FROM messages m WHERE m.account_id = ?1 AND m.deleted_locally = 0),
           (SELECT count(*) FROM messages
             WHERE account_id = ?1 AND read_receipt_requested = 1
               AND deleted_locally = 0)",
        [account.get()],
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok((
                row.col::<i64>(0)? as u64,
                row.col::<i64>(1)? as u64,
                row.col::<i64>(2)? as u64,
                row.col::<i64>(3)? as u64,
                row.col::<i64>(4)? as u64,
            ))
        },
    )
    .await
    .expect("the recount")
}

/// The kept figures, in [`recounted`]'s order.
async fn kept(
    messages: &MessageRepository<'_>,
    account: postio_model::AccountId,
) -> (u64, u64, u64, u64, u64) {
    let footprint = messages.footprint(account).await.expect("footprint");
    let receipts = messages
        .read_receipt_requested_count(account)
        .await
        .expect("read receipts");
    (
        footprint.messages,
        footprint.total_bytes,
        footprint.attachment_bytes,
        footprint.local_bytes,
        receipts,
    )
}

fn with_pdf(mut message: postio_model::Message, size: u64) -> postio_model::Message {
    let mut attachment =
        postio_model::Attachment::new(postio_model::MessageId::UNASSIGNED, "application/pdf", size);
    attachment.part_id = Some("2".to_owned());
    message.attachments = vec![attachment];
    message
}

#[tokio::test]
async fn the_kept_footprint_follows_every_write_that_moves_it() {
    // A kept figure is only as good as the writes that keep it. Each step
    // below is one a person or a sync makes, and after each the kept row
    // must say what counting every message would.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive").await;
    let messages = MessageRepository::new(&connection);

    let mut ids = Vec::new();
    for (uid, state, size, payload, receipt) in [
        (1, postio_model::BodyState::HeadersOnly, 2_000, None, false),
        (
            2,
            postio_model::BodyState::Partial,
            100_000,
            Some(90_000),
            true,
        ),
        (
            3,
            postio_model::BodyState::Full,
            50_000,
            Some(40_000),
            false,
        ),
        (4, postio_model::BodyState::NotFetched, 7_000, None, true),
    ] {
        let mut created = message(inbox, account.id, uid);
        created.size = size;
        created.sync.body_state = state;
        created.read_receipt_requested = receipt;
        if let Some(payload) = payload {
            created = with_pdf(created, payload);
        }
        ids.push(messages.create(&mut created).await.expect("create"));
    }
    let expect = |step: &str, kept: (u64, u64, u64, u64, u64), counted| {
        assert_eq!(
            kept, counted,
            "after {step}, the kept figures disagree with a recount"
        );
    };
    expect(
        "filing four",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );
    assert_eq!(
        kept(&messages, account.id).await.0,
        4,
        "four messages are counted"
    );

    messages
        .set_attachment_blob(ids[1], "2", &postio_model::BlobId::new("sha256:fetched"))
        .await
        .expect("the payload arrives");
    expect(
        "a payload arriving",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );

    messages
        .set_body_state(ids[0], postio_model::BodyState::Full)
        .await
        .expect("a body arrives");
    expect(
        "a body arriving",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );

    messages
        .move_to(&[ids[2]], archive.id)
        .await
        .expect("a move");
    expect(
        "a move",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );

    messages
        .set_deleted_locally(&[ids[1]], true)
        .await
        .expect("a local delete");
    expect(
        "a local delete",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );

    messages
        .set_deleted_locally(&[ids[1]], false)
        .await
        .expect("an undo");
    expect(
        "an undo",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );

    messages
        .delete(&[ids[1], ids[3]])
        .await
        .expect("an expunge");
    expect(
        "an expunge",
        kept(&messages, account.id).await,
        recounted(&connection, account.id).await,
    );
    assert_eq!(
        kept(&messages, account.id).await,
        (2, 52_000, 40_000, 52_000, 0),
        "two full messages are left, 2k and 50k, one carrying a 40k payload, \
         and neither asked for a receipt"
    );
}

#[tokio::test]
async fn an_account_that_does_not_exist_weighs_nothing() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let messages = MessageRepository::new(&connection);
    let nobody = postio_model::AccountId::new(999_999);
    assert_eq!(
        messages.footprint(nobody).await.expect("footprint"),
        StorageFootprint {
            complete: true,
            ..StorageFootprint::default()
        }
    );
    assert_eq!(
        messages
            .read_receipt_requested_count(nobody)
            .await
            .expect("count"),
        0
    );
}

/// A store from before the footprint was kept: an account with mail in
/// every body state, a payload fetched and one not, a message hidden by a
/// local delete, and a read-receipt request.
const BEFORE_IT_WAS_KEPT: &str = "
INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                      incoming_username, outgoing_host, outgoing_port,
                      outgoing_username, created_at)
     VALUES (7, 'Ada', 'ada@example.com', 'imap.example.com', 993,
             'ada@example.com', 'smtp.example.com', 587, 'ada@example.com', 1);
INSERT INTO mailboxes (id, account_id, name, path, role) VALUES (3, 7, 'INBOX', 'INBOX', 'inbox');
INSERT INTO messages (id, account_id, mailbox_id, received_at, sort_at, size, body_state,
                      read_receipt_requested, deleted_locally)
     VALUES (10, 7, 3, 1, 1, 100000, 'partial', 1, 0),
            (11, 7, 3, 2, 2, 50000, 'full', 0, 0),
            (12, 7, 3, 3, 3, 2000, 'headers_only', 1, 0),
            (13, 7, 3, 4, 4, 9000, 'full', 1, 1);
INSERT INTO attachments (message_id, position, mime_type, size, blob_id)
     VALUES (10, 0, 'application/pdf', 60000, NULL),
            (10, 1, 'application/pdf', 30000, 'sha256:here'),
            (11, 0, 'application/pdf', 40000, 'sha256:also-here'),
            (13, 0, 'application/pdf', 8000, NULL);
";

#[tokio::test]
async fn a_migrated_store_keeps_the_footprint_it_used_to_count() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    postio_storage::Store::create_at_schema(
        &path,
        &test_support::key(),
        include_str!("../schemas/804e6e62.sql"),
        BEFORE_IT_WAS_KEPT,
    )
    .await
    .expect("a store at the schema before the footprint was kept");
    let store = postio_storage::Store::open(&path, &test_support::key())
        .await
        .expect("it opens, migrated");
    let connection = store.connect().await.expect("a connection");
    let messages = MessageRepository::new(&connection);
    let account = postio_model::AccountId::new(7);

    let migrated = kept(&messages, account).await;
    assert_eq!(
        migrated,
        recounted(&connection, account).await,
        "the migration fills the kept row from what the store already held"
    );
    // Spelled out, so the oracle cannot agree with a migration that filled
    // nothing because it also counts nothing.
    assert_eq!(
        migrated,
        (3, 152_000, 130_000, 40_000 + 50_000, 2),
        "three visible messages; 100k + 50k + 2k; every payload of theirs; \
         the partial one's 40k of text and fetched payload plus the full one; \
         two read-receipt requests"
    );

    // And the triggers are there to keep it.
    messages
        .delete(&[postio_model::MessageId::new(10)])
        .await
        .expect("an expunge after the migration");
    assert_eq!(
        kept(&messages, account).await,
        recounted(&connection, account).await,
        "a write after the migration keeps the row"
    );
}
