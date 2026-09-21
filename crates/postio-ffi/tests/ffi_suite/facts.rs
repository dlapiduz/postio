//! Everything the pane asks about one open message, answered in one read
//! (#1589, first half).
//!
//! Opening a message used to be four boundary calls — notice, caveat,
//! unsubscribe offer, recipients — and between them they loaded and
//! decompressed the body **three times** and ran the sanitizer twice, once
//! purely to count blocked images. `messageFacts` is those four answers off
//! one row read, one body load and one render, which is the shape a 16 ms
//! interaction budget can actually afford.
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

    let notice = facts.notice.expect("two remote images were held back");
    assert!(
        notice.summary.starts_with("2 remote images"),
        "the notice names the count: {}",
        notice.summary
    );
    assert_eq!(notice.sender, "digest@lists.example.org");
    assert!(!notice.allowed);

    assert!(
        facts.caveat.is_some(),
        "the body was flagged and the caveat says nothing"
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
    assert!(facts.notice.is_none(), "nothing was held back");
    assert!(facts.caveat.is_none(), "nothing went wrong decoding");
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
    assert!(facts.notice.is_none());
    assert!(facts.caveat.is_none());
    assert!(facts.offer.is_none());
    assert!(facts.recipients.is_none());
    session.shutdown();
}
