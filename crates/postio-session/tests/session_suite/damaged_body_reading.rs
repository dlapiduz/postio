//! A body the parser could not read is not "no body" (#287, #277).
//!
//! A message whose MIME structure the parser had to guess at can come back
//! with no text and no HTML part. Calling that `Absent::Empty` tells the
//! person "that is the whole message, not a fault", which is the one thing
//! known to be untrue; the decode caveat is what is true.

use chrono::Utc;
use postio_model::{BodyState, EmailAddress, Message};
use postio_session::reading::{Body, load_with_row};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_ui::reader::document::Absent;

async fn a_message_whose_body_is(
    stored: StoredBody,
) -> (postio_storage::Store, postio_model::MessageId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut message = Message::new(account.id, inbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("a message");
    MessageRepository::new(&connection)
        .set_body(id, &stored, BodyState::Full)
        .await
        .expect("a body");
    (database, id)
}

#[tokio::test]
async fn an_empty_body_the_parser_flagged_carries_the_caveat_not_the_empty_plate() {
    let (database, id) = a_message_whose_body_is(StoredBody {
        encoding_problems: true,
        ..StoredBody::default()
    })
    .await;
    let connection = database.connect().await.expect("a connection");

    let (body, _) = load_with_row(&connection, id, false).await;
    assert!(
        matches!(
            body,
            Body::Ready {
                encoding_problems: true,
                ..
            }
        ),
        "a damaged message read as \"This message has no body\""
    );
}

#[tokio::test]
async fn an_empty_body_that_parsed_cleanly_is_still_no_body() {
    let (database, id) = a_message_whose_body_is(StoredBody::default()).await;
    let connection = database.connect().await.expect("a connection");

    let (body, _) = load_with_row(&connection, id, false).await;
    assert!(matches!(body, Body::Absent(Absent::Empty)));
}
