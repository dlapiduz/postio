//! Content sharing must never change the identity of a mailbox occurrence.
use chrono::Utc;
use postio_model::{
    BodyState, ContentIdentity, Flag, Mailbox, Message, RemoteId, RfcMessageId, Uid, UidValidity,
};
use postio_storage::{
    repository::{MailboxRepository, MessageRepository, StoredBody},
    test_support,
};

#[tokio::test]
async fn account_wide_identity_shares_body_and_preserves_occurrences() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut archive = Mailbox::new(account.id, "Archive", Some('/'));
    MailboxRepository::new(&connection)
        .create(&mut archive)
        .await
        .unwrap();
    let repo = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, Utc::now());
    first.server.remote_id = Some(RemoteId::new("native-1"));
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "native-1"));
    first.server.uid = Some(Uid::new(7));
    first.server.uid_validity = Some(UidValidity::new(9));
    first.sync.body_state = BodyState::HeadersOnly;
    repo.create(&mut first).await.unwrap();
    let body = StoredBody {
        text: Some("one immutable body".into()),
        html: None,
        headers: None,
        headers_truncated: false,
        encoding_problems: false,
    };
    repo.set_body(first.id, &body, BodyState::Full)
        .await
        .unwrap();
    let mut second = first.clone();
    second.mailbox_id = archive.id;
    second.server.uid = Some(Uid::new(11));
    second.server.uid_validity = Some(UidValidity::new(42));
    second.flags.insert(Flag::Seen);
    second.sync.body_state = BodyState::HeadersOnly;
    repo.create(&mut second).await.unwrap();
    assert_ne!(first.id, second.id);
    // The reader resolves one content owner by primary keys; sharing must not
    // turn opening a message into a mailbox-sized read.
    let counting = postio_storage::test_support::counting::here;
    postio_storage::test_support::counting::record();
    postio_storage::test_support::counting::reset();
    assert_eq!(repo.body(second.id).await.unwrap().unwrap().text, body.text);
    assert!(repo.has_reusable_content(second.id).await.unwrap());
    assert_eq!(counting().statements, 2);
    assert_eq!(counting().rows, 2);
    for query in postio_storage::test_support::counting::recorded().keys() {
        let plan = postio_storage::test_support::counting::plan_steps(&connection, query)
            .await
            .expect("the actual reader query has a plan");
        let scans: Vec<_> = plan
            .iter()
            .filter(|step| step.starts_with("SCAN") && !step.contains("CONSTANT ROW"))
            .collect();
        assert!(scans.is_empty(), "reader scans: {scans:?}");
    }
    assert_eq!(
        repo.get(second.id).await.unwrap().unwrap().sync.body_state,
        BodyState::Full
    );
    assert!(!repo.get(first.id).await.unwrap().unwrap().flags.is_seen());
    assert!(repo.get(second.id).await.unwrap().unwrap().flags.is_seen());
    assert_eq!(
        repo.get(first.id).await.unwrap().unwrap().server.uid,
        Some(Uid::new(7))
    );
    assert_eq!(
        repo.get(second.id).await.unwrap().unwrap().server.uid,
        Some(Uid::new(11))
    );
    assert_eq!(
        repo.get(second.id)
            .await
            .unwrap()
            .unwrap()
            .server
            .uid_validity,
        Some(UidValidity::new(42))
    );
    postio_storage::actions::set_flag(
        &connection,
        account.id,
        &[&first],
        &Flag::Flagged,
        true,
        Utc::now(),
    )
    .await
    .unwrap();
    repo.set_headers(
        first.id,
        Some(&postio_model::headers::Block {
            text: "Subject: shared\r\n".into(),
            truncated: false,
        }),
    )
    .await
    .unwrap();
    let flagged = repo.get(first.id).await.unwrap().unwrap();
    assert!(flagged.flags.is_flagged());
    assert!(flagged.sync.flags_dirty);
    assert!(flagged.sync.has_pending_operations);
    assert!(
        !repo
            .get(second.id)
            .await
            .unwrap()
            .unwrap()
            .flags
            .is_flagged()
    );
    let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
    assert!(
        queue
            .pending_for(postio_model::OperationTarget::Message(first.id))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        queue
            .pending_for(postio_model::OperationTarget::Message(second.id))
            .await
            .unwrap()
            .is_none()
    );
    let contents: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM message_contents", ())
            .await
            .unwrap();
    assert_eq!(contents, 1, "body storage is owned once");
    repo.delete(&[first.id]).await.unwrap();
    assert_eq!(repo.body(second.id).await.unwrap().unwrap().text, body.text);
    repo.delete(&[second.id]).await.unwrap();
    let contents: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM message_contents", ())
            .await
            .unwrap();
    assert_eq!(contents, 0, "last membership collects content");
}

#[tokio::test]
async fn matching_rfc_id_and_imap_coordinates_do_not_share_content() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut archive = Mailbox::new(account.id, "Archive", Some('/'));
    MailboxRepository::new(&connection)
        .create(&mut archive)
        .await
        .unwrap();
    let repo = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, Utc::now());
    first.rfc_message_id = Some(RfcMessageId::new("<same@example.test>"));
    first.server.remote_id = Some(RemoteId::new("9:7"));
    first.server.uid = Some(Uid::new(7));
    first.server.uid_validity = Some(UidValidity::new(9));
    repo.create(&mut first).await.unwrap();
    repo.set_body(
        first.id,
        &StoredBody {
            text: Some("first bytes".into()),
            html: None,
            headers: None,
            headers_truncated: false,
            encoding_problems: false,
        },
        BodyState::Full,
    )
    .await
    .unwrap();
    let mut second = first.clone();
    second.mailbox_id = archive.id;
    repo.create(&mut second).await.unwrap();
    assert_eq!(repo.body(second.id).await.unwrap().unwrap().text, None);
}

#[tokio::test]
async fn content_keys_are_scoped_to_account_and_namespace() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let (other_account, other_inbox) = test_support::account_with_inbox(&connection).await;
    let repo = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, Utc::now());
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "native-1"));
    repo.create(&mut first).await.unwrap();
    repo.set_body(
        first.id,
        &StoredBody {
            text: Some("private account body".into()),
            html: None,
            headers: None,
            headers_truncated: false,
            encoding_problems: false,
        },
        BodyState::Full,
    )
    .await
    .unwrap();
    for (account_id, mailbox, namespace) in [
        (other_account.id, other_inbox, "jmap-email"),
        (account.id, inbox, "gmail-message"),
    ] {
        let mut other = Message::new(account_id, mailbox, Utc::now());
        other.server.content_identity = Some(ContentIdentity::new(namespace, "native-1"));
        repo.create(&mut other).await.unwrap();
        assert_eq!(repo.body(other.id).await.unwrap().unwrap().text, None);
        assert_eq!(
            repo.get(other.id)
                .await
                .unwrap()
                .unwrap()
                .server
                .content_identity,
            other.server.content_identity
        );
    }
}

#[tokio::test]
async fn existing_memberships_and_late_memberships_reuse_inline_parts() {
    use postio_model::{Attachment, BlobId, Disposition, MessageId};
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let repo = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, Utc::now());
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "native-1"));
    first.sync.body_state = BodyState::HeadersOnly;
    let mut earlier = first.clone();
    repo.create(&mut first).await.unwrap();
    repo.create(&mut earlier).await.unwrap();
    first.content_type = Some("multipart/related".into());
    let mut inline = Attachment::new(MessageId::UNASSIGNED, "image/png", 10);
    inline.part_id = Some("2".into());
    inline.content_id = Some("image@example.test".into());
    inline.disposition = Disposition::Inline;
    first.attachments.push(inline);
    repo.update(&mut first).await.unwrap();
    let blob = BlobId::new("sha256:fixture");
    repo.set_attachment_blob(first.id, "2", &blob)
        .await
        .unwrap();
    let body = StoredBody {
        text: None,
        html: Some("<img src=\"cid:image@example.test\">".into()),
        headers: None,
        headers_truncated: false,
        encoding_problems: false,
    };
    repo.set_body(first.id, &body, BodyState::Full)
        .await
        .unwrap();
    let mut late = Message::new(account.id, inbox, Utc::now());
    late.server.content_identity = first.server.content_identity.clone();
    late.sync.body_state = BodyState::HeadersOnly;
    repo.create(&mut late).await.unwrap();
    repo.delete(&[first.id]).await.unwrap();
    for id in [earlier.id, late.id] {
        let loaded = repo.get(id).await.unwrap().unwrap();
        assert_eq!(loaded.sync.body_state, BodyState::Full);
        assert_eq!(loaded.content_type.as_deref(), Some("multipart/related"));
        assert_eq!(loaded.attachments.len(), 1);
        assert_eq!(loaded.attachments[0].blob_id.as_ref(), Some(&blob));
        assert_eq!(repo.body(id).await.unwrap().unwrap().html, body.html);
    }
}

#[tokio::test]
async fn empty_content_identity_cannot_merge_messages() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let repo = MessageRepository::new(&connection);
    for (namespace, key) in [("", "native-1"), ("jmap-email", "")] {
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.server.content_identity = Some(ContentIdentity::new(namespace, key));
        assert!(
            repo.create(&mut message).await.is_err(),
            "an empty identity is not a backend guarantee"
        );
    }
}

#[tokio::test]
async fn evicting_shared_blobs_cannot_restore_missing_bytes_on_a_late_membership() {
    use postio_model::{Attachment, MessageId};
    use postio_storage::BlobStore;
    let store = test_support::temp().await;
    let connection = store.connect().await.unwrap();
    let blobs =
        BlobStore::open(store.directory().join("blobs"), &test_support::blob_keys()).unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let repo = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, Utc::now());
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "native-1"));
    first.content_type = Some("multipart/mixed".into());
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/octet-stream", 10);
    attachment.part_id = Some("2".into());
    first.attachments.push(attachment);
    repo.create(&mut first).await.unwrap();
    let raw = blobs.put(b"raw message fixture").unwrap();
    let payload = blobs.put(b"payload fixture").unwrap();
    repo.set_fetched(first.id, None, Some(&raw)).await.unwrap();
    repo.set_attachment_blob(first.id, "2", &payload)
        .await
        .unwrap();
    repo.set_body(
        first.id,
        &StoredBody {
            text: Some("retained words".into()),
            html: None,
            headers: None,
            headers_truncated: false,
            encoding_problems: false,
        },
        BodyState::Full,
    )
    .await
    .unwrap();
    postio_storage::test_support::counting::record();
    blobs.evict_to_fit(&connection, 0).await.unwrap();
    // Forgetting one physical blob must seek its references, not scan all
    // contents for every eviction candidate.
    for query in postio_storage::test_support::counting::recorded()
        .keys()
        .filter(|query| {
            query.contains("WHERE raw_blob_id = ?1") || query.contains("WHERE blob_id = ?1")
        })
    {
        let scans = postio_storage::test_support::counting::unbounded(
            &connection,
            query,
            postio_storage::test_support::counting::GROWING_TABLES,
        )
        .await;
        assert!(scans.is_empty(), "eviction scans: {scans:?}: {query}");
    }
    let mut late = Message::new(account.id, inbox, Utc::now());
    late.server.content_identity = first.server.content_identity.clone();
    repo.create(&mut late).await.unwrap();
    assert_eq!(late.raw_blob_id, None);
    assert_eq!(late.attachments[0].blob_id, None);
    assert_eq!(late.sync.body_state, BodyState::Partial);
    assert_eq!(
        repo.body(late.id).await.unwrap().unwrap().text.as_deref(),
        Some("retained words")
    );
}

#[tokio::test]
async fn occurrences_have_no_body_columns() {
    use postio_storage::sql::RowExt;
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let columns: Vec<String> =
        postio_storage::sql::all(&connection, "PRAGMA table_info(messages)", (), |row| {
            row.col(1)
        })
        .await
        .unwrap();
    for body in ["body_text", "body_html", "body_headers"] {
        assert!(
            !columns.iter().any(|column| column == body),
            "{body} belongs only to content"
        );
    }
}

#[tokio::test]
async fn payload_completion_survives_a_later_header_write() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let repo = MessageRepository::new(&connection);
    let mut message = Message::new(account.id, inbox, Utc::now());
    repo.create(&mut message).await.unwrap();
    repo.set_body(
        message.id,
        &StoredBody {
            text: Some("local words".into()),
            html: None,
            headers: None,
            headers_truncated: false,
            encoding_problems: false,
        },
        BodyState::Partial,
    )
    .await
    .unwrap();
    repo.set_body_state(message.id, BodyState::Full)
        .await
        .unwrap();
    repo.set_headers(
        message.id,
        Some(&postio_model::headers::Block {
            text: "Subject: retained\r\n".into(),
            truncated: false,
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        repo.get(message.id).await.unwrap().unwrap().sync.body_state,
        BodyState::Full
    );
}
