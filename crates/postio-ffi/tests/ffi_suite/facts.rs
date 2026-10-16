//! What one open message costs to ask about (#1589).
//!
//! Opening a message used to be four boundary calls that loaded and
//! decompressed the body **three times** between them, once purely to count
//! blocked images. Now the split is by what each fact needs: `messageFacts`
//! is the row's own facts (offer, recipients) with **no body load at all**,
//! and the two body facts — the notice and the caveat — ride
//! `readerDocument`'s answer as by-products of the render the pane pays for
//! anyway. One load, one render, per message open.
//!
//! Asserted against expected values, never against the four older calls:
//! those delegate to this now, so an equivalence test would be a tautology.
//! The single-fact suites (`notice.rs`, `unsubscribe.rs`) remain the
//! behavioural record of each fact's own rules.

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// One message with everything the facts cover: two remote images to hold
/// back, a `List-Id` to offer leaving, recipients to name, and a body the
/// decoder flagged.
async fn a_message_with_everything() -> (std::sync::Arc<Session>, i64) {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let id = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.from = vec![EmailAddress::new(
            Some("Weekly Digest"),
            "digest@lists.example.org",
        )];
        message.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.cc = vec![EmailAddress::new(
            Some("Grace Hopper"),
            "grace@example.test",
        )];
        message.list_id = Some("news.lists.example.org".to_owned());
        let id = repository.create(&mut message).await.expect("a message");
        repository
            .set_body(
                id,
                &StoredBody {
                    text: None,
                    html: Some(
                        "<p>hello <img src=\"https://tracker.example/1.png\">\
                         <img src=\"https://tracker.example/2.png\"></p>"
                            .to_owned(),
                    ),
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: true,
                },
                postio_model::message::BodyState::Full,
            )
            .await
            .expect("the body is stored");
        id.get()
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session over the store");
    (session, id)
}

#[tokio::test(flavor = "multi_thread")]
async fn one_call_answers_everything_the_pane_asks() {
    let (session, message) = a_message_with_everything().await;
    let facts = session.message_facts(message).await;
    let answers = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Blocked, false)
        .await;

    let notice = answers.notice.expect("two remote images were held back");
    assert!(
        notice.summary.starts_with("2 remote images"),
        "the notice names the count: {}",
        notice.summary
    );
    assert_eq!(notice.sender, "digest@lists.example.org");
    assert!(!notice.allowed);

    assert!(
        answers.caveat.is_some(),
        "the body was flagged and the caveat says nothing"
    );
    assert!(
        !answers.html.is_empty(),
        "and the document itself still came along"
    );

    let offer = facts.offer.expect("a List-Id is an offer to leave");
    assert_eq!(offer.list_identifier, "news.lists.example.org");

    let recipients = facts.recipients.expect("the message names people");
    assert!(
        recipients
            .to
            .as_deref()
            .is_some_and(|to| to.contains("Ada")),
        "the To line names its recipient: {:?}",
        recipients.to
    );
    assert!(
        recipients.cc_label.is_some(),
        "one Cc means a disclosure to offer"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plain_personal_message_has_almost_no_facts() {
    // The common case: no pictures held back, nothing to unsubscribe from,
    // nothing wrong with the body. Every absent fact must be `None` rather
    // than an empty something — a blank banner is still a banner.
    let (session, message) = {
        let database = test_support::memory().await;
        let id = {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            let repository = MessageRepository::new(&connection);
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
            message.to = vec![EmailAddress::new(Some("Bo"), "bo@example.com")];
            let id = repository.create(&mut message).await.expect("a message");
            repository
                .set_body(
                    id,
                    &StoredBody {
                        text: Some("Six is fine.".to_owned()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::message::BodyState::Full,
                )
                .await
                .expect("the body is stored");
            id.get()
        };
        let session = Session::open(SessionOptions::in_memory_with(database))
            .expect("a session over the store");
        (session, id)
    };

    let facts = session.message_facts(message).await;
    let answers = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Blocked, false)
        .await;
    assert!(answers.notice.is_none(), "nothing was held back");
    assert!(answers.caveat.is_none(), "nothing went wrong decoding");
    // Not `None`: with no `List-Id` the identifier deliberately falls back
    // to the sender's domain (`postio_ui::unsubscribe::list_identifier`),
    // which is how a newsletter that never sets the header still gets a
    // banner. The gate that matters is `send_state` (#1525), and a received
    // message passes it.
    assert_eq!(
        facts
            .offer
            .as_ref()
            .map(|offer| offer.list_identifier.as_str()),
        Some("example.com"),
        "a bare sender still identifies by domain"
    );
    assert!(facts.recipients.is_some(), "and the To line still reads");
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_that_is_gone_answers_nothing_at_all() {
    let (session, _message) = a_message_with_everything().await;
    let facts = session.message_facts(9_999).await;
    assert!(facts.offer.is_none());
    assert!(facts.recipients.is_none());
    let answers = session
        .reader_answers(9_999, postio_ffi::RemoteImagesFfi::Blocked, false)
        .await;
    assert!(answers.notice.is_none());
    assert!(answers.caveat.is_none());
    assert!(
        !answers.html.is_empty(),
        "a missing message is a state plate, and a plate is a document"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_view_and_original_agree_on_what_was_held_back() {
    // The load-bearing assumption behind folding the notice into the
    // document: `held_back` is counted during the sanitize pass, which runs
    // *before* reader view's reduce, so the two render modes must report
    // the same counts. If reduce ever started dropping images from the
    // count, the banner would understate — silently, and only in reader
    // view — and this is the test that says so out loud instead.
    let (session, message) = a_message_with_everything().await;

    let reduced = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Blocked, true)
        .await;
    let original = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Blocked, false)
        .await;

    let (a, b) = (
        reduced.notice.expect("held back in reader view"),
        original.notice.expect("held back in the original"),
    );
    assert_eq!(
        (a.remote_images, a.trackers),
        (b.remote_images, b.trackers),
        "the two render modes disagree about what was held back"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_allowed_render_claims_no_notice() {
    // "What would be loaded" cannot be asked of a render that loaded it.
    // The caller keeps the notice it already has; this answer saying `None`
    // is what stops a grant flip from erasing the banner's numbers.
    let (session, message) = a_message_with_everything().await;
    let answers = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Allowed, false)
        .await;
    assert!(answers.notice.is_none());
    assert!(
        answers.caveat.is_some(),
        "the caveat rides the load, not the render, so it survives the grant"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_open_is_one_body_read_counted_at_the_sql_seam() {
    // #1589's acceptance line: counted, not timed. The row facts plus the
    // document — everything a message open asks — inside a statement budget
    // that a second body load or render pass cannot fit under. The budget
    // is deliberately snug: the regression this guards added six-plus
    // statements per extra load, so a comfortable margin would let one back
    // in quietly.
    let (session, message) = a_message_with_everything().await;
    // Warm the session's own one-time work so the count is about the open.
    let _ = session.message_facts(message).await;

    postio_storage::test_support::counting::reset();
    let _ = session.message_facts(message).await;
    let _ = session
        .reader_answers(message, postio_ffi::RemoteImagesFfi::Blocked, false)
        .await;
    let counts = postio_storage::test_support::counting::here();

    assert!(
        counts.statements <= 20,
        "a message open issued {} statements — a second body load has crept          back into the path #1589 closed",
        counts.statements
    );
    session.shutdown();
}

/// The Mac's message window opens a message with one call that reads its
/// body once and sanitises it once (#1609, #1589). It used to be two calls
/// side by side -- the chrome around the body, and the document -- each with
/// a store read of its own, so every open decompressed the body twice and
/// read the row four times. Counted at the storage seam, which sees the
/// store owner's thread as well as this one.
#[tokio::test(flavor = "multi_thread")]
async fn opening_a_message_in_focus_reads_and_sanitises_its_body_once() {
    let (session, message) = a_message_with_everything().await;

    let loads = test_support::counting::body_loads();
    let sanitised = postio_ui::test_support::bodies_sanitised();
    let open = session.focus_message_open(
        message,
        0,
        1,
        postio_ffi::RemoteImagesFfi::Blocked,
        None,
        1440,
    );
    let sanitised = postio_ui::test_support::bodies_sanitised() - sanitised;
    let loads = test_support::counting::body_loads() - loads;

    // What the person sees came along whole: the chrome, the body, and the
    // facts the render paid for.
    assert_eq!(open.view.message, message);
    assert_eq!(
        open.view.fields[0].people[0].address, "digest@lists.example.org",
        "the sender block is the message's"
    );
    assert!(open.document.html.contains("hello"), "the body is drawn");
    let notice = open
        .document
        .notice
        .expect("two remote images were held back");
    assert!(
        notice.summary.starts_with("2 remote images"),
        "{}",
        notice.summary
    );
    assert!(open.document.caveat.is_some(), "the body was flagged");

    assert_eq!(loads, 1, "one open decoded the body {loads} times");
    assert_eq!(
        sanitised, 1,
        "one open sanitised the body {sanitised} times"
    );
    session.shutdown();
}

/// The standing grants are read once and kept, not read from disk for every
/// message drawn (#1589) -- and kept current by the session's own writes,
/// which are the only ones there are, so a revoke in Settings is true for the
/// next message drawn.
#[tokio::test(flavor = "multi_thread")]
async fn the_allow_list_is_kept_in_memory_and_current_through_its_writes() {
    let (session, message) = a_message_with_everything().await;
    let allowed = |session: &Session| {
        session
            .focus_reader_document(message, postio_ffi::RemoteImagesFfi::Blocked, None, 1440)
            .notice
            .expect("still held back: the render asked for is a blocked one")
            .allowed
    };
    assert!(!allowed(&session));

    session.allow_sender("digest@lists.example.org".to_owned());
    assert!(allowed(&session), "a grant is true at once");

    // Gone from disk behind the session's back: the session does not go
    // back to the file to draw the next message.
    let path = std::env::temp_dir();
    let files: Vec<_> = std::fs::read_dir(&path)
        .expect("the temporary directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with(&format!("postio-allowed-{}-", std::process::id()))
                })
        })
        // This session's, by the grant it holds, should another session in
        // this process have a file of its own.
        .filter(|path| {
            std::fs::read_to_string(path)
                .is_ok_and(|text| text.contains("digest@lists.example.org"))
        })
        .collect();
    assert!(!files.is_empty(), "the grant was written through to disk");
    for file in &files {
        std::fs::remove_file(file).expect("the grants file goes");
    }
    assert!(
        allowed(&session),
        "drawing a message read the grants from disk again"
    );

    session.revoke_remote_images("digest@lists.example.org".to_owned());
    assert!(!allowed(&session), "a revoke is true at once");
    session.shutdown();
}
