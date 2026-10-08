//! Focus's engine runs behind the Mac app (specs/009-focus-macos R8).
//!
//! Filing, markers, digests and reminders are the host's, and they run only
//! once a frontend switches Focus on. The GTK app and the terminal always
//! did; the Mac never had, so its inbox had no markers and nothing was ever
//! filed. These assert what a person would see: the header strip's counts.

use std::time::{Duration, Instant};

use chrono::Utc;
use postio_ffi::{Session, SessionOptions};
use postio_model::Message;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A session over a store whose inbox holds one message asking `question`.
async fn asked(question: &str) -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        // A question sent to the person, as the detector needs it: from
        // someone else, to the account's own address, recently.
        let mut message = Message::new(account.id, inbox, Utc::now() - chrono::TimeDelta::hours(1));
        message.subject = Some("Atlas Q3 budget".to_owned());
        message.date = Some(Utc::now() - chrono::TimeDelta::hours(1));
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ada"),
            "ada@example.com",
        )];
        message.to = vec![account.address.clone()];
        message.promoted = Some(postio_model::promoted::PromotedHeaders::default());
        let id = repository.create(&mut message).await.expect("a message");
        repository
            .set_body(
                id,
                &StoredBody {
                    text: Some(question.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::message::BodyState::Full,
            )
            .await
            .expect("the body is stored");
    }
    Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
        .expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_question_in_the_inbox_is_counted_as_needing_action() {
    let session = asked(
        "Hi,\n\nCan you approve these by Friday so finance can close the quarter?\n\nThanks,\nAda",
    )
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let counts = loop {
        let counts = session.focus_counts().expect("the counts");
        if counts.has_action > 0 || Instant::now() > deadline {
            break counts;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(counts.conversations, 1, "{counts:?}");
    assert_eq!(
        counts.has_action, 1,
        "the question is marked, so Has action counts it: {counts:?}"
    );
    session.shutdown();
}
