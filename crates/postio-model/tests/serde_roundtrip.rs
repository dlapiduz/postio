//! Acceptance: every domain type round-trips through serde.

use chrono::{DateTime, TimeZone, Utc};
use postio_model::*;

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .expect("valid timestamp")
}

fn roundtrip<T>(value: &T) -> T
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(value, &back, "round-trip mismatch for {json}");
    back
}

fn sample_account() -> Account {
    Account {
        id: AccountId::new(1),
        display_name: "Personal".into(),
        address: EmailAddress::new(Some("Ada"), "ada@example.com"),
        incoming: ServerConfig {
            host: "imap.example.com".into(),
            port: 993,
            security: TransportSecurity::Tls,
            username: "ada@example.com".into(),
        },
        outgoing: ServerConfig {
            host: "smtp.example.com".into(),
            port: 587,
            security: TransportSecurity::StartTls,
            username: "ada@example.com".into(),
        },
        auth: AuthMethod::AppPassword,
        enabled: true,
        identities: vec![sample_identity()],
        signatures: Vec::new(),
        default_signature_id: Some(SignatureId::new(4)),
        created_at: at(1_000),
        pending_deletion: false,
    }
}

fn sample_identity() -> Identity {
    Identity {
        id: IdentityId::new(7),
        account_id: AccountId::new(1),
        display_name: "Ada Lovelace".into(),
        address: EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        reply_to: Some(EmailAddress::new(None::<String>, "replies@example.com")),
        signature: Some(Signature {
            id: Default::default(),
            name: String::new(),
            text: "— Ada".into(),
            html: Some("<p>— Ada</p>".into()),
        }),
        is_default: true,
    }
}

fn sample_mailbox() -> Mailbox {
    Mailbox {
        id: MailboxId::new(3),
        account_id: AccountId::new(1),
        parent_id: None,
        name: "Sent Messages".into(),
        path: "Sent Messages".into(),
        delimiter: Some('/'),
        role: MailboxRole::Sent,
        selectable: true,
        subscribed: true,
        counts: MailboxCounts {
            total: 42,
            unread: 3,
            flagged: 1,
        },
        uid_validity: Some(UidValidity::new(12)),
        uid_next: Some(Uid::new(900)),
        highest_mod_seq: Some(ModSeq::new(4_000)),
        last_synced_at: Some(at(2_000)),
        signature_id: Some(SignatureId::new(5)),
        backfill_excluded: false,
    }
}

fn sample_attachment() -> Attachment {
    Attachment {
        id: AttachmentId::new(11),
        message_id: MessageId::new(5),
        filename: Some("contract.pdf".into()),
        mime_type: "application/pdf".into(),
        size: 8_192,
        content_id: Some("cid-1".into()),
        disposition: Disposition::Attachment,
        part_id: Some("2.1".into()),
        part_headers: Some("Content-Type: application/pdf\r\n".into()),
        blob_id: Some(BlobId::new("sha256:deadbeef")),
    }
}

fn sample_message() -> Message {
    let mut message = Message::new(AccountId::new(1), MailboxId::new(3), at(3_000));
    message.id = MessageId::new(5);
    message.thread_id = Some(ThreadId::new(9));
    message.rfc_message_id = Some(RfcMessageId::new("<a@example.com>"));
    message.in_reply_to = Some(RfcMessageId::new("<b@example.com>"));
    message.references = vec![
        RfcMessageId::new("<c@example.com>"),
        RfcMessageId::new("<b@example.com>"),
    ];
    message.from = vec![EmailAddress::new(Some("Alice"), "alice@example.com")];
    message.sender = Some(EmailAddress::new(None::<String>, "bounce@example.com"));
    message.reply_to = vec![EmailAddress::new(None::<String>, "list@example.com")];
    message.to = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.cc = vec![EmailAddress::new(None::<String>, "cc@example.com")];
    message.bcc = vec![EmailAddress::new(None::<String>, "bcc@example.com")];
    message.subject = Some("Re: Contract".into());
    message.date = Some(at(2_900));
    message.body = MessageBody {
        text: Some("hello".into()),
        html: Some("<p>hello</p>".into()),
    };
    message.preview = Some("hello".into());
    message.attachments = vec![sample_attachment()];
    message.flags = FlagSet::from_iter([Flag::Seen, Flag::Keyword("Work".into())]);
    message.labels = vec![LabelId::new(2)];
    message.size = 12_345;
    message.headers = Headers::from_iter([("X-Mailer", "postio"), ("Received", "from a")]);
    message.server = ServerIdentifiers {
        uid: Some(Uid::new(881)),
        uid_validity: Some(UidValidity::new(12)),
        mod_seq: Some(ModSeq::new(3_999)),
        remote_id: Some("gm-1234".into()),
    };
    message.sync = LocalSyncState {
        body_state: BodyState::Full,
        flags_dirty: true,
        has_pending_operations: true,
        deleted_locally: false,
        last_synced_at: Some(at(3_100)),
    };
    message.raw_blob_id = Some(BlobId::new("sha256:cafe"));
    message
}

fn sample_thread() -> Thread {
    Thread {
        id: ThreadId::new(9),
        account_id: AccountId::new(1),
        subject: Some("Contract".into()),
        message_ids: vec![MessageId::new(4), MessageId::new(5)],
        participants: vec![EmailAddress::new(Some("Alice"), "alice@example.com")],
        mailbox_ids: vec![MailboxId::new(3)],
        labels: vec![LabelId::new(2)],
        message_count: 2,
        unread_count: 1,
        has_attachments: true,
        is_flagged: false,
        first_at: at(2_800),
        last_at: at(3_000),
    }
}

fn sample_contact() -> Contact {
    Contact {
        id: ContactId::new(21),
        account_id: Some(AccountId::new(1)),
        name: Some("Alice".into()),
        address: EmailAddress::new(Some("Alice"), "alice@example.com"),
        times_seen: 12,
        last_seen_at: Some(at(3_000)),
        source: postio_model::ContactSource::User,
        suppressed: false,
    }
}

fn sample_draft() -> Draft {
    Draft {
        id: DraftId::new(31),
        account_id: AccountId::new(1),
        identity_id: Some(IdentityId::new(7)),
        kind: DraftKind::Reply,
        in_reply_to: Some(MessageId::new(5)),
        thread_id: Some(ThreadId::new(9)),
        to: vec![EmailAddress::new(Some("Alice"), "alice@example.com")],
        cc: vec![],
        bcc: vec![],
        subject: "Re: Contract".into(),
        body: MessageBody {
            text: Some("sure".into()),
            html: None,
        },
        attachments: vec![],
        state: DraftState::Editing,
        server: ServerIdentifiers::default(),
        created_at: at(3_200),
        updated_at: at(3_300),
    }
}

fn sample_label() -> Label {
    Label {
        id: LabelId::new(2),
        account_id: AccountId::new(1),
        name: "Work".into(),
        color: Some("#5980a6".into()),
    }
}

#[test]
fn every_entity_round_trips() {
    roundtrip(&sample_account());
    roundtrip(&sample_identity());
    roundtrip(&sample_mailbox());
    roundtrip(&sample_message());
    roundtrip(&sample_thread());
    roundtrip(&sample_attachment());
    roundtrip(&sample_contact());
    roundtrip(&sample_label());
    roundtrip(&sample_draft());
}

#[test]
fn enums_and_value_objects_round_trip() {
    roundtrip(&Flag::Seen);
    roundtrip(&Flag::Keyword("Work".into()));
    roundtrip(&FlagSet::from_iter([Flag::Seen, Flag::Flagged]));
    roundtrip(&MailboxRole::Junk);
    roundtrip(&Disposition::Inline);
    roundtrip(&TransportSecurity::StartTls);
    roundtrip(&AuthMethod::OAuth2);
    roundtrip(&BodyState::HeadersOnly);
    roundtrip(&DraftState::Failed);
    roundtrip(&DraftKind::Forward);
    roundtrip(&EmailAddress::new(Some("Alice"), "alice@example.com"));
    roundtrip(&Headers::from_iter([("To", "a@b")]));
    roundtrip(&AccountId::new(1));
    roundtrip(&RfcMessageId::new("<a@b>"));
}

#[test]
fn ids_serialize_transparently() {
    assert_eq!(serde_json::to_string(&AccountId::new(4)).unwrap(), "4");
    assert_eq!(serde_json::to_string(&Uid::new(7)).unwrap(), "7");
    assert_eq!(
        serde_json::to_string(&RfcMessageId::new("<a@b>")).unwrap(),
        "\"<a@b>\""
    );
}
