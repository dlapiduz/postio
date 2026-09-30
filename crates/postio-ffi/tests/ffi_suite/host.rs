//! A verb issued through the boundary reaches the store (specs/005-tui-frontend
//! T019, ADR 0041).
//!
//! A session opened the ordinary way -- no bus supplied by the caller -- used
//! to wire a handler that took every command and dropped it, so `invoke`
//! reached nothing: the macOS app's `a` archived nothing at all. The session
//! is a client of `postio-host` now, and its commands run on the host's verbs
//! over the store it opened. These hold that, with no bus of the test's own.

use chrono::Utc;
use postio_ffi::{ScopeFfi, Session, SessionOptions};
use postio_model::{Mailbox, MailboxRole, Message};
use postio_storage::repository::{MailboxRepository, MessageRepository};
use postio_storage::test_support;

/// Where the store says `message` lives now.
async fn home_of(database: &postio_storage::Store, message: i64) -> Option<i64> {
    let connection = database.connect().await.expect("a connection");
    MessageRepository::new(&connection)
        .get(postio_model::ids::MessageId::new(message))
        .await
        .expect("a read")
        .map(|message| message.mailbox_id.get())
}

#[tokio::test(flavor = "multi_thread")]
async fn archiving_through_the_boundary_moves_the_message() {
    let database = test_support::memory().await;
    let (inbox, archive, message) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut archive = Mailbox::new(account.id, "Archive", Some('/'));
        archive.role = MailboxRole::Archive;
        MailboxRepository::new(&connection)
            .create(&mut archive)
            .await
            .expect("an Archive folder");
        let mut message = Message::new(account.id, inbox, Utc::now());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        (inbox, archive.id.get(), message.id.get())
    };

    // No `on_bridge`: the bus is whatever the session wires for itself,
    // which is what `Session::open_at` -- the Swift constructor -- gets.
    let session =
        Session::open(SessionOptions::in_memory_with(database.clone())).expect("a session");
    session.open_scope(ScopeFfi::Mailbox {
        mailbox: inbox.into(),
    });
    let _ = session.row_at(0);
    session.settle_for_test();
    let row = session.row_at(0).expect("the message's row is resident");
    assert_eq!(row.id, message, "the fixture's one row is its one message");

    session.set_cursor(Some(row.id));
    session.invoke("archive");

    let deadline =
        std::time::Instant::now() + postio_test_support::scaled(std::time::Duration::from_secs(10));
    let mut home = home_of(&database, message).await;
    while home != Some(archive) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        home = home_of(&database, message).await;
    }
    assert_eq!(
        home,
        Some(archive),
        "`archive` went through the boundary and the message never left the \
         inbox (mailbox {}) -- the session's commands reach no handler",
        inbox.get()
    );
    session.shutdown();
}
