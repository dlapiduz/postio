use std::collections::BTreeSet;

use postio_model::{
    Account, EmailAddress, Generation, Mailbox, Message, RfcMessageId, Uid, UidValidity,
};
use postio_storage::Connection;
use postio_storage::repository::{ContactRepository, MessageRepository};
use postio_storage::test_support;
use postio_sync::commit_batch;

const INBOX: &str = "INBOX";

const UID_VALIDITY: UidValidity = UidValidity::new(1);
const GENERATION: Generation = Generation::new(1);

async fn local(connection: &Connection) -> (Account, Mailbox) {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, INBOX).await;
    (account, inbox)
}

fn message(account: &Account, mailbox: &Mailbox, uid: u32, subject: &str) -> Message {
    let mut message = Message::new(account.id, mailbox.id, chrono::Utc::now());
    message.subject = Some(subject.to_string());
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.to = vec![EmailAddress::new(Some("Bob"), "bob@example.com")];
    message.server.uid = Some(Uid::new(uid));
    message.server.uid_validity = Some(UID_VALIDITY);
    message.rfc_message_id = Some(RfcMessageId::new(format!("<note-{uid}@example.com>")));
    message
}

#[tokio::test]
async fn a_committed_batch_is_stored_threaded_and_its_correspondents_recorded() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = local(&connection).await;

    let mut batch = vec![
        message(&account, &inbox, 1, "Note one"),
        message(&account, &inbox, 2, "Note two"),
    ];

    let report = commit_batch(
        &connection,
        &inbox,
        Some(&account),
        &BTreeSet::new(),
        &mut batch,
    )
    .await
    .expect("the batch commits");

    assert_eq!(report.inserted, 2, "both messages are new");
    assert_eq!(report.threaded, 2, "both were filed into a thread");

    let messages = MessageRepository::new(&connection);
    for written in &batch {
        assert!(written.id.get() > 0, "the upsert assigned an id");
        let stored = messages
            .get(written.id)
            .await
            .expect("read the message back")
            .expect("the message is in the store");
        assert!(
            stored.thread_id.is_some(),
            "the stored message was filed into a thread"
        );
    }

    let stored = messages
        .uids_in(inbox.id, GENERATION)
        .await
        .expect("uids_in");
    assert_eq!(stored.len(), 2, "both messages reached the store");

    let addresses: Vec<String> = ContactRepository::new(&connection)
        .people(10_000)
        .await
        .expect("list people")
        .iter()
        .flat_map(|person| &person.addresses)
        .map(|owned| owned.address.normalized())
        .collect();
    assert!(
        addresses.contains(&"ada@example.com".to_string()),
        "the sender was recorded: {addresses:?}"
    );
    assert!(
        addresses.contains(&"bob@example.com".to_string()),
        "the recipient was recorded: {addresses:?}"
    );
}

#[tokio::test]
async fn a_uid_already_known_is_written_again_but_its_correspondents_are_not() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = local(&connection).await;

    let mut first = vec![message(&account, &inbox, 1, "Note one")];
    commit_batch(
        &connection,
        &inbox,
        Some(&account),
        &BTreeSet::new(),
        &mut first,
    )
    .await
    .expect("the first batch commits");

    let before = sightings_of(&connection, &account, "ada@example.com").await;
    let mut again = vec![message(&account, &inbox, 1, "Note one")];
    again[0].flags.insert(postio_model::Flag::Seen);
    let report = commit_batch(
        &connection,
        &inbox,
        Some(&account),
        &BTreeSet::from([1]),
        &mut again,
    )
    .await
    .expect("the second batch commits");

    assert_eq!(report.inserted, 0, "nothing was new");
    assert_eq!(report.updated, 1, "the known message was written again");
    assert_eq!(
        sightings_of(&connection, &account, "ada@example.com").await,
        before,
        "a known UID records no second sighting"
    );
}

async fn sightings_of(connection: &Connection, _account: &Account, address: &str) -> u32 {
    ContactRepository::new(connection)
        .by_address(address)
        .await
        .expect("look the address up")
        .and_then(|person| {
            person
                .addresses
                .into_iter()
                .find(|owned| owned.address.normalized() == address)
        })
        .map(|owned| owned.times_seen)
        .unwrap_or(0)
}
