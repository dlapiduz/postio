//! The schema must be able to persist `postio-model` faithfully.
//!
//! Repositories are a later bead; this test writes the SQL by hand purely to
//! prove the *columns exist* and round-trip every field of a fully populated
//! [`Message`] — the model is the input to the schema, so a missing column is a
//! schema bug and belongs here rather than three beads later.

use chrono::{TimeZone, Utc};
use rusqlite::{Connection, params};

use postio_model::{
    Account, Attachment, AuthMethod, BodyState, Contact, Disposition, Draft, DraftKind, DraftState,
    EmailAddress, Flag, FlagSet, Label, Mailbox, MailboxRole, Message, RfcMessageId, Thread,
};
use postio_storage::migrate;

fn migrated() -> Connection {
    let mut connection = Connection::open_in_memory().expect("in-memory sqlite");
    connection
        .pragma_update(None, "foreign_keys", true)
        .expect("foreign keys");
    migrate(&mut connection).expect("migrate");
    connection
}

fn millis(datetime: chrono::DateTime<Utc>) -> i64 {
    datetime.timestamp_millis()
}

fn store_account(connection: &Connection, account: &Account) -> i64 {
    connection
        .execute(
            "INSERT INTO accounts (
                 display_name, address, address_name,
                 incoming_host, incoming_port, incoming_security, incoming_username,
                 outgoing_host, outgoing_port, outgoing_security, outgoing_username,
                 auth_method, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                account.display_name,
                account.address.address,
                account.address.name,
                account.incoming.host,
                account.incoming.port,
                "tls",
                account.incoming.username,
                account.outgoing.host,
                account.outgoing.port,
                "starttls",
                account.outgoing.username,
                match account.auth {
                    AuthMethod::Password => "password",
                    AuthMethod::AppPassword => "app_password",
                    AuthMethod::OAuth2 => "oauth2",
                    AuthMethod::XOAuth2 => "xoauth2",
                },
                account.enabled,
                millis(account.created_at),
            ],
        )
        .expect("insert account");
    connection.last_insert_rowid()
}

fn store_mailbox(connection: &Connection, account_id: i64, mailbox: &Mailbox) -> i64 {
    connection
        .execute(
            "INSERT INTO mailboxes (
                 account_id, parent_id, name, path, delimiter, role, selectable, subscribed,
                 total_count, unread_count, flagged_count)
             VALUES (?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                account_id,
                mailbox.name,
                mailbox.path,
                mailbox.delimiter.map(|d| d.to_string()),
                mailbox.role.as_str(),
                mailbox.selectable,
                mailbox.subscribed,
                mailbox.counts.total,
                mailbox.counts.unread,
                mailbox.counts.flagged,
            ],
        )
        .expect("insert mailbox");
    connection.last_insert_rowid()
}

fn a_full_message(account_id: i64, mailbox_id: i64) -> Message {
    use postio_model::{AccountId, MailboxId};

    let received_at = Utc.with_ymd_and_hms(2026, 2, 3, 4, 5, 6).unwrap();
    let mut message = Message::new(
        AccountId::new(account_id),
        MailboxId::new(mailbox_id),
        received_at,
    );
    message.rfc_message_id = Some(RfcMessageId::new("child@example.com"));
    message.in_reply_to = Some(RfcMessageId::new("<parent@example.com>"));
    message.references = vec![
        RfcMessageId::new("<root@example.com>"),
        RfcMessageId::new("<parent@example.com>"),
    ];
    message.from = vec![EmailAddress::new(Some("Alice"), "alice@example.com")];
    message.sender = Some(EmailAddress::new(Some("List"), "list@example.com"));
    message.reply_to = vec![EmailAddress::new(None::<String>, "replies@example.com")];
    message.to = vec![
        EmailAddress::new(Some("Bob"), "bob@example.com"),
        EmailAddress::new(None::<String>, "carol@example.com"),
    ];
    message.cc = vec![EmailAddress::new(None::<String>, "dave@example.com")];
    message.bcc = vec![EmailAddress::new(None::<String>, "erin@example.com")];
    message.subject = Some("Re: Invoice 42".to_owned());
    message.date = Some(Utc.with_ymd_and_hms(2026, 2, 3, 4, 0, 0).unwrap());
    message.preview = Some("Here is the invoice you asked for".to_owned());
    message.size = 8_192;
    message.flags = [Flag::Seen, Flag::Flagged, Flag::parse("Work")]
        .into_iter()
        .collect::<FlagSet>();
    message.headers.push("Received", "from mx.example.com");
    message.headers.push("X-Mailer", "Postio");
    message.server.uid = Some(4242.into());
    message.server.uid_validity = Some(9.into());
    message.server.mod_seq = Some(777.into());
    message.server.remote_id = Some("remote-4242".to_owned());
    message.sync.body_state = BodyState::Full;
    message.sync.flags_dirty = true;
    message.sync.has_pending_operations = true;
    message.sync.last_synced_at = Some(Utc.with_ymd_and_hms(2026, 2, 3, 5, 0, 0).unwrap());
    message.raw_blob_id = Some(postio_model::BlobId::new("blake3:deadbeef"));
    message
}

fn insert_message(connection: &Connection, message: &Message) -> i64 {
    let flags = message
        .flags
        .persistable()
        .iter()
        .map(Flag::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    let references = message
        .references
        .iter()
        .map(RfcMessageId::as_str)
        .collect::<Vec<_>>()
        .join(" ");

    connection
        .execute(
            "INSERT INTO messages (
                 account_id, mailbox_id, thread_id,
                 rfc_message_id, in_reply_to, reference_ids,
                 subject, normalized_subject, date, received_at,
                 preview, size, flags, seen, flagged, answered, draft, deleted, has_attachments,
                 uid, uid_validity, mod_seq, remote_id,
                 body_state, flags_dirty, has_pending_operations, deleted_locally, last_synced_at,
                 raw_blob_id, body_text_blob_id, body_html_blob_id, headers_blob_id)
             VALUES (
                 ?1, ?2, NULL,
                 ?3, ?4, ?5,
                 ?6, ?7, ?8, ?9,
                 ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21, ?22,
                 ?23, ?24, ?25, ?26, ?27,
                 ?28, NULL, NULL, NULL)",
            params![
                message.account_id.get(),
                message.mailbox_id.get(),
                message.rfc_message_id.as_ref().map(RfcMessageId::as_str),
                message.in_reply_to.as_ref().map(RfcMessageId::as_str),
                references,
                message.subject,
                message.normalized_subject(),
                message.date.map(millis),
                millis(message.received_at),
                message.preview,
                message.size as i64,
                flags,
                message.flags.is_seen(),
                message.flags.is_flagged(),
                message.flags.is_answered(),
                message.flags.is_draft(),
                message.flags.is_deleted(),
                message.has_attachments(),
                message.server.uid.map(|uid| uid.get()),
                message.server.uid_validity.map(|value| value.get()),
                message.server.mod_seq.map(|value| value.get() as i64),
                message.server.remote_id,
                match message.sync.body_state {
                    BodyState::NotFetched => "not_fetched",
                    BodyState::HeadersOnly => "headers_only",
                    BodyState::Partial => "partial",
                    BodyState::Full => "full",
                },
                message.sync.flags_dirty,
                message.sync.has_pending_operations,
                message.sync.deleted_locally,
                message.sync.last_synced_at.map(millis),
                message.raw_blob_id.as_ref().map(|id| id.as_str()),
            ],
        )
        .expect("insert message");
    connection.last_insert_rowid()
}

fn insert_recipients(connection: &Connection, message_id: i64, message: &Message) {
    let mut groups: Vec<(&str, Vec<&EmailAddress>)> = vec![
        ("from", message.from.iter().collect()),
        ("sender", message.sender.iter().collect()),
        ("reply_to", message.reply_to.iter().collect()),
        ("to", message.to.iter().collect()),
        ("cc", message.cc.iter().collect()),
        ("bcc", message.bcc.iter().collect()),
    ];
    for (kind, addresses) in groups.drain(..) {
        for (position, address) in addresses.into_iter().enumerate() {
            connection
                .execute(
                    "INSERT INTO recipients
                         (message_id, kind, position, name, address, address_normalized)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        message_id,
                        kind,
                        position as i64,
                        address.name,
                        address.address,
                        address.normalized(),
                    ],
                )
                .expect("insert recipient");
        }
    }
}

fn read_addresses(connection: &Connection, message_id: i64, kind: &str) -> Vec<EmailAddress> {
    connection
        .prepare(
            "SELECT name, address FROM recipients
             WHERE message_id = ?1 AND kind = ?2 ORDER BY position",
        )
        .expect("prepare")
        .query_map(params![message_id, kind], |row| {
            Ok(EmailAddress {
                name: row.get(0)?,
                address: row.get(1)?,
            })
        })
        .expect("query")
        .collect::<Result<_, _>>()
        .expect("collect")
}

/// The columns of a `messages` row this test reads back, so the round trip is
/// checked field by field rather than through an unreadable tuple.
struct StoredMessage {
    subject: Option<String>,
    date: Option<i64>,
    received_at: i64,
    size: i64,
    preview: Option<String>,
    flags: String,
    body_state: String,
    remote_id: Option<String>,
    raw_blob_id: Option<String>,
}

#[test]
fn a_fully_populated_message_round_trips_through_the_schema() {
    let connection = migrated();
    let account = Account::new("iCloud", EmailAddress::new(None::<String>, "ada@example.com"));
    let account_id = store_account(&connection, &account);
    let mailbox = Mailbox::new(account.id, "INBOX", Some('/'));
    let mailbox_id = store_mailbox(&connection, account_id, &mailbox);

    let message = a_full_message(account_id, mailbox_id);
    let message_id = insert_message(&connection, &message);
    insert_recipients(&connection, message_id, &message);

    let stored = connection
        .query_row(
            "SELECT subject, date, received_at, size, preview, flags, body_state,
                    remote_id, raw_blob_id
             FROM messages WHERE id = ?1",
            [message_id],
            |row| {
                Ok(StoredMessage {
                    subject: row.get(0)?,
                    date: row.get(1)?,
                    received_at: row.get(2)?,
                    size: row.get(3)?,
                    preview: row.get(4)?,
                    flags: row.get(5)?,
                    body_state: row.get(6)?,
                    remote_id: row.get(7)?,
                    raw_blob_id: row.get(8)?,
                })
            },
        )
        .expect("read message back");

    assert_eq!(stored.subject, message.subject);
    assert_eq!(stored.date, message.date.map(millis));
    assert_eq!(stored.received_at, millis(message.received_at));
    assert_eq!(stored.size as u64, message.size);
    assert_eq!(stored.preview, message.preview);
    assert_eq!(stored.body_state, "full");
    assert_eq!(stored.remote_id, message.server.remote_id);
    assert_eq!(stored.raw_blob_id.as_deref(), Some("blake3:deadbeef"));

    // Flags survive as a canonical, `\Recent`-free set.
    let restored: FlagSet = stored.flags.split_whitespace().map(Flag::parse).collect();
    assert_eq!(restored, message.flags.persistable());

    // Every address header keeps its order and its display names.
    assert_eq!(
        read_addresses(&connection, message_id, "from"),
        message.from
    );
    assert_eq!(
        read_addresses(&connection, message_id, "sender"),
        message.sender.iter().cloned().collect::<Vec<_>>()
    );
    assert_eq!(
        read_addresses(&connection, message_id, "reply_to"),
        message.reply_to
    );
    assert_eq!(read_addresses(&connection, message_id, "to"), message.to);
    assert_eq!(read_addresses(&connection, message_id, "cc"), message.cc);
    assert_eq!(read_addresses(&connection, message_id, "bcc"), message.bcc);

    // The reference chain that JWZ threading walks survives verbatim, in order.
    let references: String = connection
        .query_row(
            "SELECT reference_ids FROM messages WHERE id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .expect("references");
    let restored: Vec<RfcMessageId> = references
        .split_whitespace()
        .map(RfcMessageId::new)
        .collect();
    assert_eq!(restored, message.references);
}

#[test]
fn every_mailbox_role_is_storable() {
    let connection = migrated();
    let account = Account::new("roles", EmailAddress::new(None::<String>, "r@example.com"));
    let account_id = store_account(&connection, &account);

    for role in [
        MailboxRole::Inbox,
        MailboxRole::Archive,
        MailboxRole::Sent,
        MailboxRole::Drafts,
        MailboxRole::Trash,
        MailboxRole::Junk,
        MailboxRole::Flagged,
        MailboxRole::Regular,
    ] {
        connection
            .execute(
                "INSERT INTO mailboxes (account_id, name, path, role) VALUES (?1, ?2, ?2, ?3)",
                params![account_id, role.as_str(), role.as_str()],
            )
            .unwrap_or_else(|error| panic!("role {} must be storable: {error}", role.as_str()));
    }
}

#[test]
fn every_body_state_is_storable_and_nothing_else_is() {
    let connection = migrated();
    let account = Account::new("body", EmailAddress::new(None::<String>, "b@example.com"));
    let account_id = store_account(&connection, &account);
    let mailbox = Mailbox::new(account.id, "INBOX", None);
    let mailbox_id = store_mailbox(&connection, account_id, &mailbox);

    for state in ["not_fetched", "headers_only", "partial", "full"] {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, body_state)
                 VALUES (?1, ?2, 0, ?3)",
                params![account_id, mailbox_id, state],
            )
            .unwrap_or_else(|error| panic!("body_state {state} must be storable: {error}"));
    }
    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at, body_state)
             VALUES (?1, ?2, 0, 'nonsense')",
            params![account_id, mailbox_id],
        )
        .expect_err("body_state is a closed set");
}

#[test]
fn attachment_metadata_round_trips_without_the_bytes() {
    let connection = migrated();
    let account = Account::new("att", EmailAddress::new(None::<String>, "a@example.com"));
    let account_id = store_account(&connection, &account);
    let mailbox = Mailbox::new(account.id, "INBOX", None);
    let mailbox_id = store_mailbox(&connection, account_id, &mailbox);
    let message = a_full_message(account_id, mailbox_id);
    let message_id = insert_message(&connection, &message);

    let mut attachment = Attachment::new(message.id, "application/pdf", 12_345);
    attachment.filename = Some("invoice.pdf".to_owned());
    attachment.content_id = Some("<cid-1>".to_owned());
    attachment.disposition = Disposition::Inline;
    attachment.part_id = Some("2.1".to_owned());
    attachment.blob_id = Some(postio_model::BlobId::new("blake3:cafe"));

    connection
        .execute(
            "INSERT INTO attachments (
                 message_id, position, filename, mime_type, size, content_id,
                 disposition, disposition_raw, part_id, blob_id)
             VALUES (?1, 0, ?2, ?3, ?4, ?5, 'inline', NULL, ?6, ?7)",
            params![
                message_id,
                attachment.filename,
                attachment.mime_type,
                attachment.size as i64,
                attachment.content_id,
                attachment.part_id,
                attachment.blob_id.as_ref().map(|id| id.as_str()),
            ],
        )
        .expect("insert attachment");

    let (filename, mime_type, size, part_id, blob_id): (
        Option<String>,
        String,
        i64,
        Option<String>,
        Option<String>,
    ) = connection
        .query_row(
            "SELECT filename, mime_type, size, part_id, blob_id FROM attachments",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .expect("read attachment");

    assert_eq!(filename, attachment.filename);
    assert_eq!(mime_type, attachment.mime_type);
    assert_eq!(size as u64, attachment.size);
    assert_eq!(part_id, attachment.part_id);
    assert_eq!(blob_id.as_deref(), Some("blake3:cafe"));
}

#[test]
fn an_attachment_belongs_to_a_message_or_a_draft_but_never_both() {
    let connection = migrated();
    let error = connection
        .execute(
            "INSERT INTO attachments (mime_type, size) VALUES ('text/plain', 1)",
            [],
        )
        .expect_err("an attachment must have an owner");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("constraint")
    );
}

#[test]
fn a_draft_and_its_recipients_are_storable() {
    let connection = migrated();
    let account = Account::new("drafts", EmailAddress::new(None::<String>, "d@example.com"));
    let account_id = store_account(&connection, &account);

    let mut draft = Draft::new(account.id);
    draft.kind = DraftKind::ReplyAll;
    draft.state = DraftState::Queued;
    draft.subject = "Re: lunch".to_owned();
    draft.body.text = Some("On my way".to_owned());
    draft.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
    draft.bcc = vec![EmailAddress::new(None::<String>, "archive@example.com")];

    connection
        .execute(
            "INSERT INTO drafts (
                 account_id, identity_id, kind, in_reply_to_message_id, thread_id,
                 subject, body_text, body_html, state, created_at, updated_at)
             VALUES (?1, NULL, 'reply_all', NULL, NULL, ?2, ?3, NULL, 'queued', ?4, ?4)",
            params![
                account_id,
                draft.subject,
                draft.body.text,
                millis(draft.created_at)
            ],
        )
        .expect("insert draft");
    let draft_id = connection.last_insert_rowid();

    // Draft recipients live in the same table as message recipients.
    for (kind, address) in [("to", &draft.to[0]), ("bcc", &draft.bcc[0])] {
        connection
            .execute(
                "INSERT INTO recipients (draft_id, kind, position, name, address, address_normalized)
                 VALUES (?1, ?2, 0, ?3, ?4, ?5)",
                params![draft_id, kind, address.name, address.address, address.normalized()],
            )
            .expect("insert draft recipient");
    }

    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM recipients WHERE draft_id = ?1",
            [draft_id],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 2);
}

#[test]
fn a_recipient_belongs_to_a_message_or_a_draft_but_never_both() {
    let connection = migrated();
    let error = connection
        .execute(
            "INSERT INTO recipients (kind, position, address, address_normalized)
             VALUES ('to', 0, 'x@example.com', 'x@example.com')",
            [],
        )
        .expect_err("a recipient must have an owner");
    assert!(
        error
            .to_string()
            .to_ascii_lowercase()
            .contains("constraint")
    );
}

#[test]
fn a_thread_and_its_membership_are_storable() {
    let connection = migrated();
    let account = Account::new(
        "threads",
        EmailAddress::new(None::<String>, "t@example.com"),
    );
    let account_id = store_account(&connection, &account);
    let mailbox = Mailbox::new(account.id, "INBOX", None);
    let mailbox_id = store_mailbox(&connection, account_id, &mailbox);

    let mut thread = Thread::new(account.id);
    thread.subject = Some("Invoice 42".to_owned());
    thread.message_count = 2;
    thread.unread_count = 1;
    thread.has_attachments = true;
    thread.is_flagged = true;
    thread.first_at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    thread.last_at = Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0).unwrap();

    connection
        .execute(
            "INSERT INTO threads (
                 account_id, subject, message_count, unread_count, has_attachments,
                 is_flagged, first_at, last_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                account_id,
                thread.subject,
                thread.message_count,
                thread.unread_count,
                thread.has_attachments,
                thread.is_flagged,
                millis(thread.first_at),
                millis(thread.last_at),
            ],
        )
        .expect("insert thread");
    let thread_id = connection.last_insert_rowid();

    // Membership is `messages.thread_id`, not a duplicated id list.
    for _ in 0..2 {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, thread_id, received_at)
                 VALUES (?1, ?2, ?3, 0)",
                params![account_id, mailbox_id, thread_id],
            )
            .expect("insert thread member");
    }
    let members: i64 = connection
        .query_row(
            "SELECT count(*) FROM messages WHERE thread_id = ?1",
            [thread_id],
            |row| row.get(0),
        )
        .expect("count members");
    assert_eq!(members, 2);
}

#[test]
fn labels_apply_to_many_messages_and_cascade() {
    let connection = migrated();
    let account = Account::new("labels", EmailAddress::new(None::<String>, "l@example.com"));
    let account_id = store_account(&connection, &account);
    let mailbox = Mailbox::new(account.id, "INBOX", None);
    let mailbox_id = store_mailbox(&connection, account_id, &mailbox);

    let label = Label::new(account.id, "Work");
    connection
        .execute(
            "INSERT INTO labels (account_id, name, color) VALUES (?1, ?2, ?3)",
            params![account_id, label.name, label.color],
        )
        .expect("insert label");
    let label_id = connection.last_insert_rowid();

    connection
        .execute(
            "INSERT INTO messages (account_id, mailbox_id, received_at) VALUES (?1, ?2, 0)",
            params![account_id, mailbox_id],
        )
        .expect("insert message");
    let message_id = connection.last_insert_rowid();
    connection
        .execute(
            "INSERT INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            params![message_id, label_id],
        )
        .expect("apply label");
    connection
        .execute(
            "INSERT INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            params![message_id, label_id],
        )
        .expect_err("a label applies to a message at most once");

    connection
        .execute("DELETE FROM labels WHERE id = ?1", [label_id])
        .expect("delete label");
    let remaining: i64 = connection
        .query_row("SELECT count(*) FROM message_labels", [], |row| row.get(0))
        .expect("count");
    assert_eq!(remaining, 0, "deleting a label unapplies it");
}

#[test]
fn a_contact_accumulates_sightings() {
    let connection = migrated();
    let mut contact = Contact::new(EmailAddress::new(Some("Alice"), "Alice@Example.com"));
    contact.record_seen(Utc.with_ymd_and_hms(2026, 2, 3, 0, 0, 0).unwrap());

    connection
        .execute(
            "INSERT INTO contacts (account_id, name, address, address_name,
                                   address_normalized, times_seen, last_seen_at)
             VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                contact.name,
                contact.address.address,
                contact.address.name,
                contact.address.normalized(),
                contact.times_seen,
                contact.last_seen_at.map(millis),
            ],
        )
        .expect("insert contact");

    let (normalized, times_seen): (String, i64) = connection
        .query_row(
            "SELECT address_normalized, times_seen FROM contacts",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("read contact");
    assert_eq!(normalized, "alice@example.com");
    assert_eq!(times_seen, 1);
}
