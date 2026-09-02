//! A `Content-ID` resolves only inside the message that declares it. #608.
//!
//! `cid_source` is fifteen lines carrying a security property, which is why
//! it moved out of `postio-app` rather than being written a second time for
//! the macOS frontend: a `Content-ID` is chosen by whoever sent the message,
//! and nothing stops two senders choosing the same one. If resolution were
//! global — "find the part with this id" — a message could reference a part
//! of *another* message and the reader would render it, so a sender could
//! address bytes they were never sent.
//!
//! The scoping is one line in the implementation (`showing()` names the
//! message, and the lookup starts from that message's parts), which is
//! exactly the kind of line a reimplementation drops without noticing.
//!
//! No display and no network: a store, a blob directory, and a closure. It
//! runs on both hosts, which is the point of it living here.

use std::cell::Cell;
use std::rc::Rc;

use postio_model::ids::MessageId;
use postio_model::{Attachment, BodyState, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::{BlobStore, test_support};

/// File a message carrying one inline part with `content_id`, and hand back
/// the message's id.
fn message_with_part(
    connection: &postio_storage::PooledConnection,
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
        .expect("file the message");
    message.id
}

#[test]
fn a_content_id_from_another_message_does_not_resolve() {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let mine = message_with_part(
        &connection,
        &blobs,
        account.id,
        inbox,
        "The message on screen",
        "logo@example.com",
        b"MINE",
    );
    let theirs = message_with_part(
        &connection,
        &blobs,
        account.id,
        inbox,
        "Somebody else's",
        "secret@example.invalid",
        b"THEIRS",
    );
    assert_ne!(mine, theirs, "the fixture needs two distinct messages");
    drop(connection);

    // The pane is showing `mine`, and that is the only thing the resolver is
    // told. Everything else it has -- the store, the blob directory -- can
    // see both messages' parts.
    let showing = Rc::new(Cell::new(Some(mine)));
    let source = postio_session::reading::cid_source(
        {
            let showing = Rc::clone(&showing);
            move || showing.get()
        },
        (*database).clone(),
        blobs.clone(),
    );

    // ── the open message's own part resolves ─────────────────────────────
    let (bytes, mime) = source
        .resolve("logo@example.com")
        .expect("a message's own inline part must resolve, or nothing renders");
    assert_eq!(bytes, b"MINE", "resolved the wrong part's bytes");
    assert_eq!(mime, "image/png");

    // ── the other message's does not ─────────────────────────────────────
    assert!(
        source.resolve("secret@example.invalid").is_none(),
        "a Content-ID declared by a different message resolved. Resolution \
         is scoped to the message on screen precisely so a sender cannot \
         address bytes they were never sent."
    );

    // ── and the scope follows the pane ───────────────────────────────────
    // The same resolver, the same store: only which message is open changed.
    // Asserted so a rewrite cannot pass by hardcoding one message's parts.
    showing.set(Some(theirs));
    assert!(
        source.resolve("logo@example.com").is_none(),
        "the resolver kept answering for the message that is no longer open"
    );
    let (bytes, _) = source
        .resolve("secret@example.invalid")
        .expect("the newly-open message's own part must resolve");
    assert_eq!(bytes, b"THEIRS");

    // ── nothing open resolves nothing ────────────────────────────────────
    showing.set(None);
    assert!(
        source.resolve("logo@example.com").is_none(),
        "with no message on screen there is no scope to resolve within"
    );
}
