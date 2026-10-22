//! The person at a sender's address, for the Mac's card and `i`
//! (specs/011-focus-contacts S1): made on demand from the stored mail when
//! the catch-up has not reached them, and shaped in Rust so Swift only draws.

use chrono::Utc;
use postio_ffi::{ContactKindFfi, Session, SessionOptions};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// A session over a store holding mail from one sender, and no people.
async fn old_mail() -> std::sync::Arc<Session> {
    let database = test_support::memory().await;
    {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut message = Message::new(account.id, inbox, Utc::now());
        message.from = vec![EmailAddress::new(Some("Old Friend"), "friend@example.org")];
        message.to = vec![account.address.clone()];
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("old mail");
        test_support::mark_contacts_history(&connection).await;
    }
    Session::open(SessionOptions::in_memory_with(database)).expect("a session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_with_mail_and_no_person_comes_back_as_one() {
    let session = old_mail().await;
    let found = tokio::task::spawn_blocking(move || {
        (
            session.contact_for_address("Friend@Example.org".into()),
            session.contact_for_address("stranger@example.org".into()),
        )
    })
    .await
    .expect("the blocking call");

    let person = found.0.expect("a person, made on demand");
    assert_eq!(person.name, "Old Friend");
    assert_eq!(person.kind, ContactKindFfi::Person);
    assert!(!person.saved, "being asked about is not being saved");
    assert_eq!(person.addresses, ["friend@example.org"]);
    assert_eq!(person.times_seen, 1);
    assert!(
        found.1.is_none(),
        "an address the store never saw is nobody"
    );
}
