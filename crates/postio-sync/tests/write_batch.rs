//! The write half of a sync pass, on its own.
//!
//! `commit_batch` is what `enumerate` does once it has a batch of headers in
//! hand: upsert, thread, record correspondents, one transaction per write
//! unit. It is public so that a bench can measure the write path a first sync
//! actually runs rather than a reimplementation of it that can drift away
//! from it silently (#726).
//!
//! These tests are that seam's contract. `tests/initial.rs` covers the same
//! behaviour reached through a whole pass; this covers it reached directly,
//! which is how the bench reaches it.

use std::collections::BTreeSet;

use postio_model::{
    Account, EmailAddress, Generation, Mailbox, Message, RfcMessageId, Uid, UidValidity,
};
use postio_storage::repository::{ContactRepository, MessageRepository};
use postio_storage::test_support;
use postio_sync::commit_batch;
use rusqlite::Connection;

const INBOX: &str = "INBOX";

/// The UID space these messages belong to. Any value will do; it only has to
/// be the same one `uids_in` is asked about. Two newtypes over the same
/// counter: a message records the `UIDVALIDITY` it was seen under, and the
/// engine asks about a mailbox's naming generation.
const UID_VALIDITY: UidValidity = UidValidity::new(1);
const GENERATION: Generation = Generation::new(1);

/// An account and an empty local `INBOX`.
fn local(connection: &Connection) -> (Account, Mailbox) {
    let account = test_support::account(connection);
    let inbox = test_support::mailbox(connection, &account, INBOX);
    (account, inbox)
}

/// A message from Ada with `uid`, its own `Message-ID`, and `subject`.
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

#[test]
fn a_committed_batch_is_stored_threaded_and_its_correspondents_recorded() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = local(&connection);

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
    .expect("the batch commits");

    assert_eq!(report.inserted, 2, "both messages are new");
    assert_eq!(report.threaded, 2, "both were filed into a thread");

    // The ids the upsert assigned belong to the caller's messages, so a caller
    // can go on using them — `enumerate` relies on this to report progress
    // against what it just wrote.
    //
    // `thread_id` is deliberately *not* asserted on the caller's copy: the
    // threading pass writes it to the row, not back into the struct it was
    // handed. So the thread has to be read back from the store, which is also
    // where anything that cares about it will look.
    let messages = MessageRepository::new(&connection);
    for written in &batch {
        assert!(written.id.get() > 0, "the upsert assigned an id");
        let stored = messages
            .get(written.id)
            .expect("read the message back")
            .expect("the message is in the store");
        assert!(
            stored.thread_id.is_some(),
            "the stored message was filed into a thread"
        );
    }

    let stored = messages.uids_in(inbox.id, GENERATION).expect("uids_in");
    assert_eq!(stored.len(), 2, "both messages reached the store");

    // Recorded: the correspondent list is built by the sync path and nothing
    // else, so a batch that writes mail without recording anyone leaves the
    // finder and the composer's completion empty however much mail arrives.
    let contacts = ContactRepository::new(&connection)
        .list(Some(account.id))
        .expect("list contacts");
    let addresses: Vec<String> = contacts.iter().map(|c| c.address.normalized()).collect();
    assert!(
        addresses.contains(&"ada@example.com".to_string()),
        "the sender was recorded: {addresses:?}"
    );
    assert!(
        addresses.contains(&"bob@example.com".to_string()),
        "the recipient was recorded: {addresses:?}"
    );
}

#[test]
fn a_uid_already_known_is_written_again_but_its_correspondents_are_not() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = local(&connection);

    // The pass that first wrote UID 1.
    let mut first = vec![message(&account, &inbox, 1, "Note one")];
    commit_batch(
        &connection,
        &inbox,
        Some(&account),
        &BTreeSet::new(),
        &mut first,
    )
    .expect("the first batch commits");

    // A re-enumeration: UID 1 is known going in, so it is refreshed but not
    // counted as a new sighting. Recording it again would inflate `times_seen`
    // without a new message ever having arrived.
    let before = sightings_of(&connection, &account, "ada@example.com");
    let mut again = vec![message(&account, &inbox, 1, "Note one")];
    let report = commit_batch(
        &connection,
        &inbox,
        Some(&account),
        &BTreeSet::from([1]),
        &mut again,
    )
    .expect("the second batch commits");

    assert_eq!(report.inserted, 0, "nothing was new");
    assert_eq!(report.updated, 1, "the known message was written again");
    assert_eq!(
        sightings_of(&connection, &account, "ada@example.com"),
        before,
        "a known UID records no second sighting"
    );
}

/// How many times `address` has been seen for `account`.
fn sightings_of(connection: &Connection, account: &Account, address: &str) -> u32 {
    ContactRepository::new(connection)
        .list(Some(account.id))
        .expect("list contacts")
        .iter()
        .find(|contact| contact.address.normalized() == address)
        .map(|contact| contact.times_seen)
        .unwrap_or(0)
}
