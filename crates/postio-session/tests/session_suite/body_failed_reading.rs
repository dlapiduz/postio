//! A message whose body will not download says so, instead of promising it
//! for ever (#1918).

use chrono::Utc;
use postio_model::{BodyState, EmailAddress, Message};
use postio_session::reading::{Body, load_with_row};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_ui::reader::document::Absent;

async fn a_headers_only_message() -> (postio_storage::Store, postio_model::MessageId) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut message = Message::new(account.id, inbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.sync.body_state = BodyState::HeadersOnly;
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("a message");
    (database, id)
}

#[tokio::test]
async fn a_body_still_on_its_way_is_downloading_and_a_refused_one_is_not() {
    let (database, id) = a_headers_only_message().await;
    let connection = database.connect().await.expect("a connection");

    let (before, _) = load_with_row(&connection, id, false).await;
    assert!(matches!(before, Body::Absent(Absent::Partial)));

    MessageRepository::new(&connection)
        .mark_body_failed(id)
        .await
        .expect("mark");
    let (after, row) = load_with_row(&connection, id, false).await;
    assert!(
        matches!(after, Body::Absent(Absent::Failed)),
        "a spinner that never ends is a lie"
    );
    assert!(row.is_some(), "the headers still come with it");
}
