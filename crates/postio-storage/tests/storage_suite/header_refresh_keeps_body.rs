//! A header refresh of a known IMAP row keeps the content already fetched
//! (#1982).
//!
//! `resync::incremental` re-fetches the headers of everything `CHANGEDSINCE`
//! reports and writes each through `upsert_batch`. Reading a message moves
//! its `MODSEQ`, so a body the person had just read went back to
//! `headers_only` and the pane said "Downloading" again. Content is
//! immutable for a message on an IMAP server: a flag, a `MODSEQ` or a move
//! says nothing about it.

use chrono::{TimeZone, Utc};
use postio_model::{
    AccountId, Attachment, BlobId, BodyState, Flag, MailboxId, Message, MessageId, ModSeq,
    RemoteId, RfcMessageId, Uid, UidValidity,
};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support::{self, counting};

fn at(seconds: i64) -> chrono::DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// What `FETCH (UID FLAGS ENVELOPE ...)` hands back for message `n`: headers.
fn header_fetch(account: AccountId, mailbox: MailboxId, n: u32, mod_seq: u64) -> Message {
    let mut message = Message::new(account, mailbox, at(i64::from(n)));
    message.subject = Some(format!("Tide gate {n}"));
    message.rfc_message_id = Some(RfcMessageId::new(format!("<tide-{n}@example.com>")));
    message.server.uid = Some(Uid::new(n));
    message.server.uid_validity = Some(UidValidity::new(7));
    message.server.remote_id = Some(RemoteId::new(format!("7:{n}")));
    message.server.mod_seq = Some(ModSeq::new(mod_seq));
    message.sync.body_state = BodyState::HeadersOnly;
    message
}

/// Message `n` as the store holds it once its body, raw source, preview and
/// one attachment have been fetched.
async fn fetched(messages: &MessageRepository<'_>, mut message: Message) -> MessageId {
    let mut part = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 4_096);
    part.filename = Some("survey.pdf".into());
    part.part_id = Some("2".into());
    message.attachments = vec![part];
    messages.create(&mut message).await.expect("the first sync");
    let body = StoredBody {
        text: Some("The gate closes at six.".into()),
        html: None,
        headers: None,
        headers_truncated: false,
        encoding_problems: false,
    };
    messages
        .set_body(message.id, &body, BodyState::Full)
        .await
        .expect("the body");
    messages
        .set_fetched(
            message.id,
            Some("The gate closes at six."),
            Some(&BlobId::new("sha256:raw")),
        )
        .await
        .expect("the preview and raw source");
    messages
        .set_attachment_blob(message.id, "2", &BlobId::new("sha256:pdf"))
        .await
        .expect("the attachment");
    message.id
}

async fn assert_content_kept(messages: &MessageRepository<'_>, id: MessageId, what: &str) {
    let row = messages.get(id).await.unwrap().expect("the row");
    assert_eq!(
        row.sync.body_state,
        BodyState::Full,
        "{what} threw the fetched body away"
    );
    assert_eq!(
        row.raw_blob_id,
        Some(BlobId::new("sha256:raw")),
        "{what}: raw"
    );
    assert_eq!(
        row.preview.as_deref(),
        Some("The gate closes at six."),
        "{what}: preview"
    );
    assert_eq!(row.attachments.len(), 1, "{what}: attachment rows");
    assert_eq!(
        row.attachments[0].blob_id,
        Some(BlobId::new("sha256:pdf")),
        "{what}: attachment blob"
    );
    assert_eq!(
        row.attachments[0].part_id.as_deref(),
        Some("2"),
        "{what}: part map"
    );
    assert_eq!(
        messages
            .body(id)
            .await
            .unwrap()
            .expect("a body")
            .text
            .as_deref(),
        Some("The gate closes at six."),
        "{what}: text"
    );
}

#[tokio::test]
async fn another_clients_flag_change_keeps_the_fetched_body() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let id = fetched(&messages, header_fetch(account.id, inbox, 1, 10)).await;

    let mut refreshed = header_fetch(account.id, inbox, 1, 11);
    refreshed.flags.insert(Flag::Flagged);
    let report = messages
        .upsert_batch(&mut vec![refreshed])
        .await
        .expect("the resync");

    assert_eq!(report.updated, 1);
    assert_content_kept(&messages, id, "a flag change").await;
    let row = messages.get(id).await.unwrap().unwrap();
    assert!(
        row.flags.contains(&Flag::Flagged),
        "the refresh still lands"
    );
    assert_eq!(row.server.mod_seq, Some(ModSeq::new(11)));
}

#[tokio::test]
async fn our_own_seen_store_coming_back_keeps_the_fetched_body() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let id = fetched(&messages, header_fetch(account.id, inbox, 1, 10)).await;

    let mut refreshed = header_fetch(account.id, inbox, 1, 12);
    refreshed.flags.insert(Flag::Seen);
    messages
        .upsert_batch(&mut vec![refreshed])
        .await
        .expect("the resync");

    assert_content_kept(&messages, id, "the MODSEQ of our own \\Seen").await;
    assert!(
        messages
            .get(id)
            .await
            .unwrap()
            .unwrap()
            .flags
            .contains(&Flag::Seen)
    );
}

#[tokio::test]
async fn adopting_a_moved_row_at_its_destination_keeps_the_fetched_body() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    // The archive move nulled the server coordinates and kept the row.
    let mut ours = header_fetch(account.id, inbox, 1, 10);
    ours.server.uid = None;
    ours.server.uid_validity = None;
    ours.server.remote_id = None;
    ours.server.mod_seq = None;
    let id = fetched(&messages, ours).await;

    // The destination's pass names it: a new uid, the same Message-ID.
    let mut named = header_fetch(account.id, inbox, 41, 3);
    named.rfc_message_id = Some(RfcMessageId::new("<tide-1@example.com>"));
    let report = messages
        .upsert_batch(&mut vec![named])
        .await
        .expect("the destination's pass");

    assert_eq!(
        (report.updated, report.inserted),
        (1, 0),
        "adopted, not copied"
    );
    assert_content_kept(&messages, id, "adopting the moved row").await;
    assert_eq!(
        messages.get(id).await.unwrap().unwrap().server.uid,
        Some(Uid::new(41))
    );
}

#[tokio::test]
async fn a_row_never_fetched_is_written_as_the_refresh_has_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let mut first = header_fetch(account.id, inbox, 1, 10);
    messages.create(&mut first).await.expect("the first sync");

    let mut refreshed = header_fetch(account.id, inbox, 1, 11);
    refreshed.preview = Some("a snippet from the envelope".into());
    messages
        .upsert_batch(&mut vec![refreshed])
        .await
        .expect("the resync");

    let row = messages.get(first.id).await.unwrap().unwrap();
    assert_eq!(row.sync.body_state, BodyState::HeadersOnly);
    assert_eq!(row.preview.as_deref(), Some("a snippet from the envelope"));
}

/// The rows and statements a refresh of `known` fetched rows reads and runs.
async fn cost_of_refreshing(known: u32) -> (usize, usize) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    for n in 1..=known {
        fetched(&messages, header_fetch(account.id, inbox, n, 10)).await;
    }
    counting::install(&connection);
    let mut batch: Vec<Message> = (1..=known)
        .map(|n| header_fetch(account.id, inbox, n, 11))
        .collect();
    let counts = counting::counted_async(|| async {
        messages.upsert_batch(&mut batch).await.expect("the resync");
    })
    .await;
    (counts.rows, counts.statements)
}

#[tokio::test]
async fn keeping_the_body_costs_a_fixed_number_of_reads_per_row() {
    let (rows_1, statements_1) = cost_of_refreshing(1).await;
    let (rows_2, statements_2) = cost_of_refreshing(2).await;
    let (rows_9, statements_9) = cost_of_refreshing(9).await;
    assert_eq!(
        rows_9 - rows_1,
        8 * (rows_2 - rows_1),
        "rows read per refreshed message is not constant"
    );
    assert_eq!(
        statements_9 - statements_1,
        8 * (statements_2 - statements_1),
        "statements per refreshed message is not constant"
    );
}
