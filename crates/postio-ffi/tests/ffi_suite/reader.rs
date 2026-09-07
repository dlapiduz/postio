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
                    headers_truncated: false,
                    encoding_problems: false,
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

    let blocked = session.reader_document(id, RemoteImagesFfi::Blocked, false);
    assert!(
        blocked.contains(&shared::content_security_policy(RemoteImages::Blocked)),
        "the blocked document does not carry the shared policy"
    );

    let allowed = session.reader_document(id, RemoteImagesFfi::Allowed, false);
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
    // `<p>hello</p>` is not bulk, so both sides draw the original — which is
    // the point of asserting against `body_html` rather than a literal: the
    // FFI document is the shared one, whichever way it was drawn.
    let drawn = shared::body_html(&body, RemoteImages::Blocked, shared::Rendering::Original);
    // Through `sheet_for` rather than naming the sheet: the paper is part of
    // "the document the GTK reader would render", so a test that pinned it to
    // `Theme` would stop noticing if the two frontends ever chose differently.
    let expected = shared::document_for(
        &drawn.html,
        RemoteImages::Blocked,
        shared::sheet_for(drawn.rendering, shared::suits_reader_view(&body)),
    );

    assert_eq!(
        session.reader_document(id, RemoteImagesFfi::Blocked, false),
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
    let document = session.reader_document(id, RemoteImagesFfi::Blocked, false);

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

    let document = session.reader_document(id.into(), RemoteImagesFfi::Blocked, false);
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

/// A session over a store holding one message addressed to `to` and `cc`.
///
/// Recipients are read per open message rather than carried on every list
/// row: a mailbox is never loaded into memory (`PRODUCT.md` §18), and `To`
/// and `Cc` are questions asked about the message in front of you.
fn with_recipients(
    to: &[(Option<&str>, &str)],
    cc: &[(Option<&str>, &str)],
) -> (std::sync::Arc<Session>, i64) {
    use postio_model::address::EmailAddress;

    let database = test_support::memory();
    let addresses = |list: &[(Option<&str>, &str)]| -> Vec<EmailAddress> {
        list.iter()
            .map(|(name, address)| EmailAddress::new(*name, *address))
            .collect()
    };

    let id = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = addresses(to);
        message.cc = addresses(cc);
        repository.create(&mut message).expect("a message")
    };

    let session = Session::open(SessionOptions::in_memory_with(database)).expect("a session");
    (session, id.into())
}

#[test]
fn an_open_message_can_say_who_it_was_addressed_to() {
    // #1259: macOS drew who a message was *from* and never who it was *to*.
    // GTK's reader has drawn both since #319, from four functions it kept
    // private — so the second frontend's choice was to write them again or
    // to share them. This asserts it shares them.
    let (session, id) = with_recipients(
        &[(None, "bob@example.com")],
        &[(Some("Grace Hopper"), "grace@example.com")],
    );

    let drawn = session.recipients(id).expect("a message has recipients");
    let shared = postio_ui::reader::header::MessageHeader::of(
        &[],
        &[postio_model::address::EmailAddress::new(
            None::<&str>,
            "bob@example.com",
        )],
        &[postio_model::address::EmailAddress::new(
            Some("Grace Hopper"),
            "grace@example.com",
        )],
        None,
        Utc::now(),
        chrono::Local::now(),
    );

    assert_eq!(drawn.to, shared.to_line());
    assert_eq!(drawn.cc, shared.cc);
    assert_eq!(drawn.cc_label, shared.cc_toggle_label());
    session.shutdown();
}

#[test]
fn a_message_addressed_to_nobody_offers_no_recipient_lines_at_all() {
    // Not blank lines: a header spends no space on a question this message
    // does not answer, which is what makes the one-recipient case one line.
    let (session, id) = with_recipients(&[], &[]);
    let drawn = session.recipients(id).expect("a message");
    assert_eq!(drawn.to, None);
    assert_eq!(drawn.cc, None);
    assert_eq!(drawn.cc_label, None);
    session.shutdown();
}

#[test]
fn a_message_that_is_not_in_the_store_has_no_recipients_rather_than_empty_ones() {
    let (session, id) = with_recipients(&[(None, "bob@example.com")], &[]);
    assert!(session.recipients(id + 4_242).is_none());
    session.shutdown();
}

#[test]
fn the_reading_pane_offers_the_same_four_verbs_the_keyboard_does() {
    // Reply, reply all and forward were reachable with the pointer; archive
    // was keyboard-only, which is the gap #1221 closed for the list. Which
    // four and in what order is the shared list's call, so the two frontends
    // cannot offer different bars.
    let (session, _) = with_recipients(&[], &[]);

    let offered = session.reader_actions();
    assert_eq!(
        offered.len(),
        postio_ui::reader::header::ReaderAction::ALL.len()
    );
    for (action, shared) in offered
        .iter()
        .zip(postio_ui::reader::header::ReaderAction::ALL)
    {
        assert_eq!(action.command, shared.command().as_str());
        assert_eq!(action.title, shared.title());
        assert_eq!(action.primary, shared.primary());
    }
    assert!(
        offered.iter().any(|action| action.command == "archive"),
        "archive is still unreachable with the pointer"
    );
    session.shutdown();
}

// --- the grants, as the Privacy pane reads them (#1156) ---------------------

/// A session with nothing in it, for the grant tests.
///
/// `in_memory` gives each session an allow-list file of its own under the
/// temp directory — checked rather than assumed, because a shared path once
/// made one test's grant true for the next, and because the equivalent
/// mistake with the keyring reached the developer's real login keychain.
fn a_session() -> std::sync::Arc<Session> {
    Session::open(SessionOptions::in_memory()).expect("an in-memory session")
}

#[test]
fn a_grant_can_be_seen_and_taken_back() {
    // "Blocked until allowed per sender" is only a promise if *allowed* is
    // reviewable: a permission nobody can see is one nobody can withdraw.
    let session = a_session();
    assert!(
        session.remote_image_grants().is_empty(),
        "nothing is allowed until somebody allows it"
    );

    session.allow_sender("ada@example.com".to_owned());
    session.allow_domain("example.net".to_owned());

    let grants = session.remote_image_grants();
    assert_eq!(grants.len(), 2);
    let sender = grants
        .iter()
        .find(|grant| grant.subject == "ada@example.com")
        .expect("the address grant");
    assert!(!sender.whole_domain);
    let domain = grants
        .iter()
        .find(|grant| grant.subject == "example.net")
        .expect("the domain grant");
    assert!(
        domain.whole_domain,
        "a domain grant covers everyone at it, and the pane has to say so"
    );

    session.revoke_remote_images("ada@example.com".to_owned());

    let left = session.remote_image_grants();
    assert_eq!(left.len(), 1, "only the one that was named went");
    assert_eq!(left[0].subject, "example.net");
}

#[test]
fn revoking_a_domain_does_not_need_to_be_told_it_is_one() {
    // One entry point for both kinds. A caller that had to guess which list
    // held a subject would leave a grant in place while reporting it gone.
    let session = a_session();
    session.allow_domain("example.org".to_owned());

    session.revoke_remote_images("example.org".to_owned());

    assert!(session.remote_image_grants().is_empty());
}
