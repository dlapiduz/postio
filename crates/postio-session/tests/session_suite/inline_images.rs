use postio_account::backend::{
    BodyStructure, Disposition, MailBackend, MockBackend, MockMailbox, MockMessage, PartNode,
};
use postio_account::cancel::CancelToken;
use postio_model::{BodyState, Uid, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};
use postio_sync::backfill::{BackfillPolicy, BodyRequest, Want, fetch_body};
use postio_sync::sync_mailbox;

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

const LOGO: &str = "UE5HQllURVM=";

fn message_with_an_inline_logo() -> MockMessage {
    let structure = BodyStructure::from_parts(
        "multipart/related",
        [
            PartNode::new("1", "text/html", 64)
                .with_charset("utf-8")
                .with_encoding("7bit")
                .with_disposition(Disposition::Inline),
            PartNode::new("2", "image/png", 8)
                .with_encoding("base64")
                .with_content_id("<logo@example.com>")
                .with_disposition(Disposition::Inline),
        ],
    );
    MockMessage::new(
        b"From: Ada Lovelace <ada@example.com>\r\n\
          Subject: The new sign over the north entrance\r\n\
          Message-ID: <inline-logo@example.com>\r\n\
          Content-Type: multipart/related; boundary=rel\r\n\
          \r\n\
          --rel\r\n\
          Content-Type: text/html; charset=utf-8\r\n\
          \r\n\
          <p><img src=\"cid:logo@example.com\" alt=\"\"></p>\r\n\
          --rel--\r\n"
            .to_vec(),
    )
    .with_structure(structure)
    .with_part(
        "1",
        &b"<p><img src=\"cid:logo@example.com\" alt=\"\"></p>"[..],
    )
    .with_part("2", LOGO.as_bytes())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inline_image_synced_from_a_server_resolves_to_its_bytes() {
    let database = test_support::temp().await;
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;

    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(message_with_an_inline_logo()),
        )
        .build();
    backend.connect().await.expect("connect");

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("header sync");

    let messages = MessageRepository::new(&connection);
    let uid = *messages
        .uids_in(inbox.id, postio_model::Generation::new(VALIDITY))
        .await
        .expect("uids")
        .first()
        .expect("the message synced");
    let stored = messages
        .by_uid(inbox.id, postio_model::Generation::new(VALIDITY), uid)
        .await
        .expect("look up")
        .expect("stored");

    let policy = BackfillPolicy::default();
    fetch_body(
        &connection,
        &blobs,
        &backend,
        &BodyRequest {
            message: stored.id,
            mailbox: inbox.id,
            path: inbox.path.clone(),
            uid: Uid::new(uid.get()),
            remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:{}", uid.get())),
            size: stored.size,
            text_size: stored.size,
            rank: postio_sync::order::sync_priority(inbox.role),
            received_at: stored.received_at,
            want: Want::Text,
        },
        policy.max_inline_bytes,
        None,
        &CancelToken::new(),
    )
    .await
    .expect("backfill the text axis");

    let body = messages
        .body(stored.id)
        .await
        .expect("body")
        .expect("the row");
    let html = body
        .html
        .expect("a text/html part disposed `inline` is still the message");
    assert!(
        html.contains("cid:logo@example.com"),
        "the pane has no <img> to draw at all: {html}"
    );

    let settled = messages.get(stored.id).await.expect("get").expect("row");
    assert_eq!(
        settled.sync.body_state,
        BodyState::Full,
        "every part of this message is local once the text axis has run"
    );
    drop(connection);

    let (bytes, mime) =
        postio_session::reading::resolve_cid(&database, &blobs, settled.id, "logo@example.com")
            .await
            .expect(
                "the scheme handler got nothing to draw, so the pane shows a broken \
         box -- which is #751 exactly: either the bytes were never fetched, \
         or the stored Content-ID still has its angle brackets",
            );
    assert_eq!(
        bytes, b"PNGBYTES",
        "the wrong part's bytes reached the pane"
    );
    assert_eq!(mime, "image/png");

    assert!(
        postio_session::reading::resolve_cid(&database, &blobs, settled.id, "missing@example.com")
            .await
            .is_none(),
        "a dangling cid: must stay a 404; nothing here may reach the network \
         to go looking for it"
    );
}
