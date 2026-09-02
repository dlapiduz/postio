//! The reader document, and the inline parts it may reference.
//!
//! ADR 0019 Q6 names this the highest risk in the whole port: two readers,
//! two content security policies, two link policies, and the drift is
//! invisible until somebody's mail phones home. The answer is that there is
//! one implementation — so these tests assert against `postio_ui`'s own
//! functions rather than against strings typed out here, because a copy of the
//! CSP in a test would drift in exactly the same way as a copy in the code.

use chrono::Utc;
use postio_body::RemoteImages;
use postio_ffi::{RemoteImagesFfi, Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_ui::reader::document as shared;

/// A session over a store holding one message whose HTML body is `html`.
fn with_body(html: &str) -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let id = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        let id = repository.create(&mut message).expect("a message");

        repository
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    text: None,
                    html: Some(html.to_owned()),
                    headers: None,
                },
                postio_model::message::BodyState::Full,
            )
            .expect("the body is stored");
        id
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    (session, id.into())
}

#[test]
fn the_document_carries_the_shared_content_security_policy() {
    // Byte-for-byte against `postio_ui`'s own answer. "contains a CSP" would
    // pass against a wrong one, which is the failure mode that matters: a
    // policy that is present and permissive looks exactly like a policy that
    // works.
    let (session, id) = with_body("<p>hello</p>");

    let blocked = session.reader_document(id, RemoteImagesFfi::Blocked);
    assert!(
        blocked.contains(&shared::content_security_policy(RemoteImages::Blocked)),
        "the blocked document does not carry the shared policy"
    );

    let allowed = session.reader_document(id, RemoteImagesFfi::Allowed);
    assert!(
        allowed.contains(&shared::content_security_policy(RemoteImages::Allowed)),
        "the allowed document does not carry the shared policy"
    );
    assert_ne!(
        blocked, allowed,
        "blocking remote images changed nothing about the document"
    );
    session.shutdown();
}

#[test]
fn the_document_is_the_one_the_gtk_reader_would_render() {
    // Not "looks similar": identical. Both frontends compose through the same
    // `postio_ui` functions, so a change to any of them moves both readers at
    // once or fails here.
    let (session, id) = with_body("<p>hello</p>");
    let body = postio_model::MessageBody {
        text: None,
        html: Some("<p>hello</p>".to_string()),
    };
    let (content, _held) = shared::body_html(&body, RemoteImages::Blocked);
    let expected = shared::document_for(&content, RemoteImages::Blocked);

    assert_eq!(
        session.reader_document(id, RemoteImagesFfi::Blocked),
        expected
    );
    session.shutdown();
}

#[test]
fn the_senders_markup_is_bounded_and_carries_no_script() {
    // `.postio-body` is a security affordance rather than styling (#323): a
    // visible edge between what Postio wrote and what arrived in the message,
    // so markup imitating application chrome has a harder time. A frontend
    // that forgot it would look fine and be wrong.
    let (session, id) = with_body("<p>hi</p><script>alert(1)</script>");
    let document = session.reader_document(id, RemoteImagesFfi::Blocked);

    assert!(
        document.contains("postio-body"),
        "the sender's content is not inside its container"
    );
    assert!(
        !document.contains("<script"),
        "a script tag survived into the document"
    );
    session.shutdown();
}

#[test]
fn a_message_with_no_body_gets_a_state_plate_not_a_blank_page() {
    // #70 Cause A: four different "no body" situations all rendering as an
    // empty column. The boundary must carry the reason, not an empty string.
    let database = test_support::memory();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");
    let id = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .expect("a message")
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");

    let document = session.reader_document(id.into(), RemoteImagesFfi::Blocked);
    assert!(
        document.len() > 200,
        "a body-less message produced an empty document rather than a state plate"
    );
    assert!(
        document.contains(&shared::content_security_policy(RemoteImages::Blocked)),
        "even a state plate is served under the policy"
    );
    session.shutdown();
}

/// Two messages, each with one inline part, in one store.
///
/// Two rather than one, because the property worth asserting is not "a part
/// resolves" but "a part resolves *only* for the message that declared it".
fn two_messages_with_inline_parts() -> (std::sync::Arc<Session>, i64, i64) {
    let database = test_support::memory();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let (first, second) = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);

        let make = |cid: &str, bytes: &[u8]| {
            let blob = blobs.put(bytes).expect("a part blob");
            let mut message = Message::new(account.id, inbox, Utc::now());
            let mut part = postio_model::Attachment::new(
                postio_model::MessageId::UNASSIGNED,
                "image/png",
                bytes.len() as u64,
            );
            part.content_id = Some(cid.to_string());
            part.blob_id = Some(blob);
            part.disposition = postio_model::attachment::Disposition::Inline;
            message.attachments = vec![part];
            repository.create(&mut message).expect("a message")
        };

        (
            make("first@example.com", b"first-bytes"),
            make("second@example.com", b"second-bytes"),
        )
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session");
    (session, first.into(), second.into())
}

#[test]
fn an_inline_part_resolves_for_the_message_that_declared_it() {
    let (session, first, _second) = two_messages_with_inline_parts();
    let part = session
        .resolve_cid(first, "first@example.com".to_string())
        .expect("the part its own message declared");
    assert_eq!(part.bytes, b"first-bytes");
    assert_eq!(part.mime_type, "image/png");
    session.shutdown();
}

#[test]
fn a_content_id_from_another_message_does_not_resolve() {
    // The security property, asserted rather than described. A `Content-ID` is
    // meaningful only inside the message that declares it; resolving one
    // globally would let a sender's markup address another sender's parts, so
    // a crafted `cid:` referencing a colleague's attachment would render it.
    let (session, first, second) = two_messages_with_inline_parts();
    assert!(
        session
            .resolve_cid(first, "second@example.com".to_string())
            .is_none(),
        "one message resolved another message's part"
    );
    // ...and the other way round, so the test cannot pass by resolving nothing.
    assert!(
        session
            .resolve_cid(second, "second@example.com".to_string())
            .is_some(),
        "the part does exist -- the scoping check above proved nothing"
    );
    session.shutdown();
}

#[test]
fn a_content_id_nothing_declared_does_not_resolve() {
    // A miss is `None` rather than a stall: the corpus fixture `inline-image-cid`
    // is a `cid:` with no matching part, and the reader must show a broken
    // image instead of waiting for bytes that are never coming.
    let (session, first, _second) = two_messages_with_inline_parts();
    assert!(
        session
            .resolve_cid(first, "nobody@example.com".to_string())
            .is_none()
    );
    session.shutdown();
}
