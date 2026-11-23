use postio_model::ids::MessageId;
use postio_model::{Attachment, BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

async fn message_with_part(
    connection: &postio_storage::Checkout,
    blobs: &BlobStore,
    account: postio_model::ids::AccountId,
    mailbox: postio_model::ids::MailboxId,
    subject: &str,
    content_id: &str,
    bytes: &[u8],
) -> MessageId {
    let blob = blobs.put(bytes).expect("store the part's bytes");
    let mut message = Message::new(account, mailbox, chrono::Utc::now());
    message.subject = Some(subject.to_owned());
    message.sync.body_state = BodyState::Full;

    let mut part = Attachment::new(MessageId::UNASSIGNED, "image/png", bytes.len() as u64);
    part.content_id = Some(content_id.to_owned());
    part.blob_id = Some(blob);
    message.attachments = vec![part];

    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("file the message");
    message.id
}

#[tokio::test(flavor = "multi_thread")]
async fn a_content_id_from_another_message_does_not_resolve() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let mine = message_with_part(
        &connection,
        &blobs,
        account.id,
        inbox,
        "The message on screen",
        "logo@example.com",
        b"MINE",
    )
    .await;
    let theirs = message_with_part(
        &connection,
        &blobs,
        account.id,
        inbox,
        "Somebody else's",
        "secret@example.invalid",
        b"THEIRS",
    )
    .await;
    assert_ne!(mine, theirs, "the fixture needs two distinct messages");
    drop(connection);

    let resolve = |message: MessageId, content_id: &'static str| {
        postio_session::reading::resolve_cid(&database, &blobs, message, content_id)
    };

    let (bytes, mime) = resolve(mine, "logo@example.com")
        .await
        .expect("a message's own inline part must resolve, or nothing renders");
    assert_eq!(bytes, b"MINE", "resolved the wrong part's bytes");
    assert_eq!(mime, "image/png");

    assert!(
        resolve(mine, "secret@example.invalid").await.is_none(),
        "a Content-ID declared by a different message resolved. Resolution \
         is scoped to the message on screen precisely so a sender cannot \
         address bytes they were never sent."
    );

    assert!(
        resolve(theirs, "logo@example.com").await.is_none(),
        "the resolver answered for a message other than the one it was asked about"
    );
    let (bytes, _) = resolve(theirs, "secret@example.invalid")
        .await
        .expect("the other message's own part must resolve");
    assert_eq!(bytes, b"THEIRS");
}
