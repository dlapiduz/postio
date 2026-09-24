//! Opening a message reads its row once.
//!
//! The reading pane asked `load_body_or_reason` for the body -- which reads
//! the message row, with its recipients, attachments and labels, to decide
//! whether there is a body at all -- and then read the same row again for
//! the header and the parts tree. `load_with_row` hands back the row it
//! already read.

use chrono::Utc;
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_storage::test_support::counting::counted_async;

#[tokio::test]
async fn the_body_and_the_row_cost_what_the_body_alone_did() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut message = Message::new(account.id, inbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.to = vec![EmailAddress::new(None::<String>, "postio@example.net")];
    let id = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("a message");
    MessageRepository::new(&connection)
        .set_body(
            id,
            &StoredBody {
                text: Some("The readings.".to_owned()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .await
        .expect("a body");

    let body_alone = counted_async(async || {
        let _ = postio_session::reading::load_body_or_reason(&connection, id, false).await;
    })
    .await;
    let mut row = None;
    let both = counted_async(async || {
        let (body, fetched) = postio_session::reading::load_with_row(&connection, id, false).await;
        assert!(matches!(body, postio_session::reading::Body::Ready { .. }));
        row = fetched;
    })
    .await;
    assert_eq!(
        row.map(|row| row.id),
        Some(id),
        "the row comes back with the body"
    );
    assert!(
        both.statements <= body_alone.statements,
        "the body and its row took {} statements where the body alone took {}",
        both.statements,
        body_alone.statements
    );
}
