//! The columns a search walk reads instead of three correlated lookups per
//! matched message (spec 010 D30): `messages.people_ids`, `label_ids` and
//! `attachment_count`, kept by triggers on `recipients`, `message_labels`
//! and `attachments`.
//!
//! A maintained answer is only worth its write cost while it is the answer,
//! so the test is the one `attachment_text_owed` has: after every kind of
//! write, the columns say exactly what the lookups they replace would say.

use chrono::Utc;
use postio_model::{
    AccountId, Attachment, BlobId, BodyState, ContentIdentity, EmailAddress, Label, MailboxId,
    Message, MessageId,
};
use postio_storage::repository::{LabelRepository, MessageRepository, StoredBody};
use postio_storage::sql::{self, RowExt as _};
use postio_storage::{Connection, Store, test_support};

/// What the walk used to ask per message, and what the columns hold, each
/// as a sorted list so the order a trigger appended in does not matter.
#[derive(Debug, PartialEq, Eq)]
struct Walked {
    id: i64,
    people: Vec<i64>,
    labels: Vec<i64>,
    attachments: i64,
}

fn sorted(list: Option<String>) -> Vec<i64> {
    let mut ids: Vec<i64> = list
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    ids.sort_unstable();
    ids
}

/// Every message's columns, and the lookups they stand for.
async fn walked(connection: &Connection) -> (Vec<Walked>, Vec<Walked>) {
    let rows = sql::all_unbounded(
        connection,
        "SELECT m.id, m.people_ids, m.label_ids, m.attachment_count,
                (SELECT group_concat(CASE r.kind WHEN 'from' THEN -r.address_id
                                                 ELSE r.address_id END)
                   FROM recipients r
                  WHERE r.message_id = m.id AND r.kind IN ('from', 'to', 'cc', 'bcc')),
                (SELECT group_concat(ml.label_id) FROM message_labels ml
                  WHERE ml.message_id = m.id),
                (SELECT count(*) FROM attachments f WHERE f.message_id = m.id)
           FROM messages m ORDER BY m.id",
        (),
        |row| {
            Ok((
                Walked {
                    id: row.int(0)?,
                    people: sorted(row.opt_text(1)?),
                    labels: sorted(row.opt_text(2)?),
                    attachments: row.int(3)?,
                },
                Walked {
                    id: row.int(0)?,
                    people: sorted(row.opt_text(4)?),
                    labels: sorted(row.opt_text(5)?),
                    attachments: row.int(6)?,
                },
            ))
        },
    )
    .await
    .expect("the columns and the lookups read");
    rows.into_iter().unzip()
}

async fn assert_kept(connection: &Connection, step: &str) {
    let (columns, lookups) = walked(connection).await;
    assert_eq!(
        columns, lookups,
        "after {step}, the walk's columns have to say what the lookups say"
    );
}

fn address(name: &str) -> EmailAddress {
    EmailAddress::new(Some(name), format!("{}@example.com", name.to_lowercase()))
}

fn attachment(position: usize, stored: bool) -> Attachment {
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 1_000);
    attachment.filename = Some(format!("part-{position}.pdf"));
    attachment.part_id = Some(format!("{}", position + 2));
    attachment.blob_id = stored.then(|| BlobId::new(format!("blob-{position}")));
    attachment
}

fn message(account: AccountId, mailbox: MailboxId) -> Message {
    let mut message = Message::new(account, mailbox, Utc::now());
    message.subject = Some("Atlas budget".into());
    message.from = vec![address("Ada")];
    message.sender = Some(address("Assistant"));
    message.reply_to = vec![address("Desk")];
    message.to = vec![address("Bob"), address("Cleo")];
    message.cc = vec![address("Dan")];
    message.bcc = vec![address("Eve"), address("Bob")];
    message
}

#[tokio::test]
async fn the_walk_columns_follow_every_write() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    let labels = LabelRepository::new(&connection);
    assert_kept(&connection, "a fresh store").await;

    let mut atlas = Label::new(account.id, "Atlas");
    labels.create(&mut atlas).await.expect("a label");
    let mut q3 = Label::new(account.id, "Q3 close");
    labels.create(&mut q3).await.expect("a second label");

    // Every kind of person, two labels and two attachments.
    let mut first = message(account.id, inbox);
    first.labels = vec![atlas.id, q3.id];
    first.attachments = vec![attachment(0, true), attachment(1, false)];
    messages.create(&mut first).await.expect("create");
    let (columns, _) = walked(&connection).await;
    assert_eq!(
        columns[0].people.len(),
        6,
        "from, to, cc and bcc, not sender or reply-to"
    );
    assert!(columns[0].people[0] < 0, "a sender is spelled negated");
    assert_eq!(columns[0].labels.len(), 2);
    assert_eq!(columns[0].attachments, 2);
    assert_kept(&connection, "a message with people, labels and files").await;

    // Nobody, nothing.
    let mut bare = Message::new(account.id, inbox, Utc::now());
    messages
        .create(&mut bare)
        .await
        .expect("create a bare message");
    assert_kept(&connection, "a message with no people, labels or files").await;

    // A resync rewrites every child row.
    first.to = vec![address("Fay")];
    first.cc.clear();
    first.labels = vec![q3.id];
    first.attachments = vec![attachment(0, true)];
    messages.update(&mut first).await.expect("update");
    assert_kept(&connection, "an update that rewrites the child rows").await;

    // A label on and off by hand, and a label gone from every message.
    labels.attach(bare.id, atlas.id).await.expect("attach");
    labels
        .attach(bare.id, atlas.id)
        .await
        .expect("attach again, ignored");
    assert_kept(&connection, "attaching a label").await;
    labels.detach(first.id, q3.id).await.expect("detach");
    assert_kept(&connection, "detaching a label").await;
    labels.attach(first.id, q3.id).await.expect("attach");
    labels.delete(atlas.id).await.expect("delete a label");
    assert_kept(&connection, "deleting a label every message loses").await;

    // A recipient row moved to another message, and an attachment too:
    // nothing writes either today, and a trigger that misses it would be
    // wrong the day something does.
    sql::execute(
        &connection,
        "UPDATE recipients SET message_id = ?2 WHERE message_id = ?1 AND kind = 'to'",
        [first.id.get(), bare.id.get()],
    )
    .await
    .expect("move a recipient");
    sql::execute(
        &connection,
        "UPDATE recipients SET kind = 'cc' WHERE message_id = ?1 AND kind = 'from'",
        [first.id.get()],
    )
    .await
    .expect("change a recipient's kind");
    sql::execute(
        &connection,
        "UPDATE attachments SET message_id = ?2 WHERE message_id = ?1",
        [first.id.get(), bare.id.get()],
    )
    .await
    .expect("move an attachment");
    assert_kept(&connection, "rows moved between messages").await;

    // A downloaded part: no change to any count.
    sql::execute(
        &connection,
        "UPDATE attachments SET blob_id = 'blob-late' WHERE blob_id IS NULL",
        (),
    )
    .await
    .expect("a download");
    assert_kept(&connection, "downloading an attachment").await;

    // Two occurrences of one payload (#1780): the second is given its
    // attachments by the content projection, not by its own write.
    let identity = Some(ContentIdentity::new("jmap-email", "email-7"));
    let mut shared = message(account.id, inbox);
    shared.server.content_identity = identity.clone();
    shared.attachments = vec![attachment(0, true), attachment(1, true)];
    shared.sync.body_state = BodyState::HeadersOnly;
    messages
        .create(&mut shared)
        .await
        .expect("the first occurrence");
    messages
        .set_body(
            shared.id,
            &StoredBody {
                text: Some("the shared body".into()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("its body");
    let mut again = message(account.id, inbox);
    again.server.content_identity = identity;
    again.sync.body_state = BodyState::HeadersOnly;
    messages
        .create(&mut again)
        .await
        .expect("the second occurrence");
    assert_kept(&connection, "a second occurrence of one payload").await;
    let (columns, _) = walked(&connection).await;
    let again_row = columns
        .iter()
        .find(|walked| walked.id == again.id.get())
        .expect("the second occurrence");
    assert_eq!(
        again_row.attachments, 2,
        "the second occurrence carries the payload's parts"
    );

    // Gone.
    messages
        .delete(&[first.id, shared.id])
        .await
        .expect("delete");
    assert_kept(&connection, "deleting messages").await;
}

/// What a store at the previous schema held: two messages with people,
/// labels and attachments, one with none.
const HELD: &str = "
INSERT INTO accounts (id, display_name, address, incoming_host, incoming_port,
                      incoming_username, outgoing_host, outgoing_port,
                      outgoing_username, created_at)
     VALUES (7, 'Ada', 'ada@example.com', 'imap.example.com', 993,
             'ada@example.com', 'smtp.example.com', 587, 'ada@example.com', 1);
INSERT INTO mailboxes (id, account_id, name, path, role) VALUES (3, 7, 'INBOX', 'INBOX', 'inbox');
INSERT INTO messages (id, account_id, mailbox_id, received_at, sort_at, has_attachments)
     VALUES (10, 7, 3, 1, 1, 1), (11, 7, 3, 2, 2, 0), (12, 7, 3, 3, 3, 0);
INSERT INTO addresses (id, address, address_normalized)
     VALUES (1, 'ada@example.com', 'ada@example.com'),
            (2, 'bob@example.com', 'bob@example.com'),
            (3, 'cleo@example.com', 'cleo@example.com');
INSERT INTO recipients (message_id, kind, position, address_id)
     VALUES (10, 'from', 0, 1), (10, 'to', 0, 2), (10, 'cc', 0, 3), (10, 'reply_to', 0, 3),
            (11, 'from', 0, 2), (11, 'bcc', 0, 1);
INSERT INTO labels (id, account_id, name) VALUES (5, 7, 'Atlas'), (6, 7, 'Q3 close');
INSERT INTO message_labels (message_id, label_id) VALUES (10, 5), (10, 6), (11, 6);
INSERT INTO attachments (message_id, position, mime_type) VALUES (10, 0, 'text/plain'),
                                                                 (10, 1, 'text/plain');
";

#[tokio::test]
async fn a_migrated_store_has_its_walk_columns_filled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("postio.db");
    Store::create_at_schema(
        &path,
        &test_support::key(),
        include_str!("../schemas/3235e866.sql"),
        HELD,
    )
    .await
    .expect("a store at the schema before the walk's columns");
    let store = Store::open(&path, &test_support::key())
        .await
        .expect("it opens, migrated");
    let connection = store.connect().await.expect("a connection");

    let (columns, _) = walked(&connection).await;
    assert_eq!(
        columns,
        vec![
            Walked {
                id: 10,
                people: vec![-1, 2, 3],
                labels: vec![5, 6],
                attachments: 2,
            },
            Walked {
                id: 11,
                people: vec![-2, 1],
                labels: vec![6],
                attachments: 0,
            },
            Walked {
                id: 12,
                people: vec![],
                labels: vec![],
                attachments: 0,
            },
        ],
        "the migration fills the columns from what the store already held"
    );
    assert_kept(&connection, "the migration").await;

    // And the triggers are there to keep them.
    sql::execute(
        &connection,
        "INSERT INTO message_labels (message_id, label_id) VALUES (12, 5)",
        (),
    )
    .await
    .expect("a label on the bare message");
    assert_kept(&connection, "a write after the migration").await;
}
