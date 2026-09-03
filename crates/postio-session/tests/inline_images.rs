//! An inline image, from the wire to the bytes the pane would draw. #751.
//!
//! Every layer under this one passed while the image did not appear, which is
//! the shape of bug `mailbox_roles.rs` and `cid_scoping.rs` both name.
//! `postio-body` proved `cid:` survives sanitisation, `postio-ui` proved the
//! CSP allows the local scheme, `postio-gtk`'s reader test proved the WebView
//! asks for all three ids, and `cid_scoping.rs` proved resolution is scoped to
//! the open message — and none of them could see that nothing ever fetched the
//! part's bytes, or that the id the IMAP path stored had angle brackets round
//! it and could match no `cid:` URL at all.
//!
//! So this test refuses the two shortcuts that hid it. The parts map is not
//! built by hand: the message arrives through `BODYSTRUCTURE` as a server
//! reports it, brackets included, and is backfilled under the *default*
//! policy. And the assertion is on what `postio_session::reading` hands the
//! scheme handler — the bytes that become pixels — rather than on what some
//! layer was handed.
//!
//! Nothing here touches the network: a mock backend, a temporary store, and a
//! blob directory beside it.

use std::cell::Cell;
use std::rc::Rc;

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

/// Base64 for `PNGBYTES`, so a failure names the bytes it got.
const LOGO: &str = "UE5HQllURVM=";

/// The whole message, exactly as a server would describe and serve it.
///
/// The `Content-ID` carries its angle brackets because that is what RFC 3501
/// puts in `BODYSTRUCTURE`'s id field, and the HTML references it without
/// them because that is what RFC 2392 puts in a `cid:` URL. Reconciling those
/// two spellings is half of what this test exists to prove.
fn message_with_an_inline_logo() -> MockMessage {
    let structure = BodyStructure::from_parts(
        "multipart/related",
        [
            PartNode::new("1", "text/html", 64)
                .with_charset("utf-8")
                .with_encoding("7bit")
                // A sender who marks the body itself `inline` -- common, and
                // enough on its own to leave the pane with no HTML at all.
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

#[tokio::test(flavor = "current_thread")]
async fn an_inline_image_synced_from_a_server_resolves_to_its_bytes() {
    let database = test_support::temp();
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);

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
        .expect("uids")
        .first()
        .expect("the message synced");
    let stored = messages
        .by_uid(inbox.id, postio_model::Generation::new(VALIDITY), uid)
        .expect("look up")
        .expect("stored");

    // The default policy, on purpose: payloads stay on the server until
    // somebody asks. An inline image under the cap is not a payload.
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
            received_at: stored.received_at,
            want: Want::Text,
        },
        policy.max_inline_bytes,
        &CancelToken::new(),
    )
    .await
    .expect("backfill the text axis");

    // ── the body is the HTML, not the alternative ────────────────────────
    let body = messages.body(stored.id).expect("body").expect("the row");
    let html = body
        .html
        .expect("a text/html part disposed `inline` is still the message");
    assert!(
        html.contains("cid:logo@example.com"),
        "the pane has no <img> to draw at all: {html}"
    );

    let settled = messages.get(stored.id).expect("get").expect("row");
    assert_eq!(
        settled.sync.body_state,
        BodyState::Full,
        "every part of this message is local once the text axis has run"
    );
    drop(connection);

    // ── and the id in that <img> resolves to real bytes ──────────────────
    let showing = Rc::new(Cell::new(Some(settled.id)));
    let source = postio_session::reading::cid_source(
        move || showing.get(),
        (*database).clone(),
        blobs.clone(),
    );

    let (bytes, mime) = source.resolve("logo@example.com").expect(
        "the scheme handler got nothing to draw, so the pane shows a broken \
         box -- which is #751 exactly: either the bytes were never fetched, \
         or the stored Content-ID still has its angle brackets",
    );
    assert_eq!(
        bytes, b"PNGBYTES",
        "the wrong part's bytes reached the pane"
    );
    assert_eq!(mime, "image/png");

    // ── and a cid: the message does not declare still draws nothing ──────
    assert!(
        source.resolve("missing@example.com").is_none(),
        "a dangling cid: must stay a 404; nothing here may reach the network \
         to go looking for it"
    );
}
