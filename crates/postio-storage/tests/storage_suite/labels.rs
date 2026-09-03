//! Labels: the account's set, and which messages carry them (#780).
//!
//! The schema had `labels` and `message_labels` from migration 0001 and
//! `MessageRepository` wrote a message's whole label set on create and update,
//! but nothing listed an account's labels, created one, or moved a single
//! label on and off a message that already exists. That last shape is the one
//! a command needs: `AddLabel` is incremental, undoable and queued, the way
//! `Flag` is, and rewriting a whole message row to add one label would race
//! every other write to it.

use postio_model::{AccountId, Label, LabelId};
use postio_storage::repository::{LabelRepository, MessageRepository};
use postio_storage::test_support;

fn a_message(
    connection: &rusqlite::Connection,
    account: AccountId,
    mailbox: postio_model::MailboxId,
) -> postio_model::MessageId {
    let mut message = postio_model::Message::new(account, mailbox, chrono::Utc::now());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create a message")
}

/// A second account, so "this account's labels" means something.
fn another_account(connection: &rusqlite::Connection) -> postio_model::Account {
    let mut account = postio_model::Account::new(
        "Quinn",
        postio_model::EmailAddress::new(Some("Quinn Abara"), "quinn@example.net"),
    );
    account.incoming.host = "imap.example.net".to_owned();
    account.outgoing.host = "smtp.example.net".to_owned();
    postio_storage::repository::AccountRepository::new(connection)
        .create(&mut account)
        .expect("create a second account");
    account
}

#[test]
fn an_account_lists_the_labels_it_owns_and_no_others() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, _inbox) = test_support::account_with_inbox(&connection);
    let labels = LabelRepository::new(&connection);

    let mut work = Label::new(account.id, "Work");
    let mut receipts = Label::new(account.id, "Receipts");
    labels.create(&mut work).expect("create");
    labels.create(&mut receipts).expect("create");
    assert!(work.id.is_assigned(), "create assigns the id");

    let listed = labels.list(account.id).expect("list");
    assert_eq!(
        listed
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["Receipts", "Work"],
        "a picker shows them in a stable order a person can scan"
    );

    // A second account's labels are its own: the picker for one account must
    // never offer another's, which is the mistake a missing scope makes.
    let other = another_account(&connection);
    let mut theirs = Label::new(other.id, "Theirs");
    labels.create(&mut theirs).expect("create");
    let listed = labels.list(account.id).expect("list");
    assert!(
        !listed.iter().any(|label| label.name == "Theirs"),
        "one account's picker offered another account's label: {listed:?}"
    );
}

#[test]
fn a_label_name_is_unique_per_account_whatever_its_case() {
    // `idx_labels_account_name` is `COLLATE NOCASE`, so this is the schema's
    // rule; the repository has to answer it as something a caller can act on
    // rather than a raw constraint violation.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, _inbox) = test_support::account_with_inbox(&connection);
    let labels = LabelRepository::new(&connection);

    let mut first = Label::new(account.id, "Work");
    labels.create(&mut first).expect("create");

    let mut again = Label::new(account.id, "work");
    assert!(
        labels.create(&mut again).is_err(),
        "two labels differing only in case would look like one to a person \
         and like two to the picker"
    );

    // The same name under a different account is a different label.
    let other = another_account(&connection);
    let mut theirs = Label::new(other.id, "Work");
    labels
        .create(&mut theirs)
        .expect("a second account may use the name");
}

#[test]
fn a_label_goes_on_and_off_one_message_without_rewriting_it() {
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let labels = LabelRepository::new(&connection);
    let message = a_message(&connection, account.id, inbox);

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).expect("create");

    assert!(labels.attach(message, work.id).expect("attach"), "newly on");
    assert_eq!(
        labels.for_message(message).expect("read back"),
        vec![work.id]
    );
    // The message row itself agrees, which is what the list and the reader
    // render from.
    assert_eq!(
        MessageRepository::new(&connection)
            .get(message)
            .expect("get")
            .expect("the message")
            .labels,
        vec![work.id]
    );

    // Attaching again is not an error and not a second row: `message_labels`
    // is keyed on the pair, and a command that ran twice must be harmless.
    assert!(
        !labels.attach(message, work.id).expect("attach again"),
        "the second attach reports that nothing changed"
    );
    assert_eq!(labels.for_message(message).expect("read back").len(), 1);

    assert!(labels.detach(message, work.id).expect("detach"), "came off");
    assert!(labels.for_message(message).expect("read back").is_empty());
    assert!(
        !labels.detach(message, work.id).expect("detach again"),
        "detaching what is not there reports that nothing changed, so an \
         undo that runs twice does not claim to have done something"
    );
}

#[test]
fn deleting_a_label_takes_it_off_every_message_carrying_it() {
    // `ON DELETE CASCADE` on `message_labels`, asserted because a label that
    // outlived its rows would leave messages pointing at nothing.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let labels = LabelRepository::new(&connection);
    let message = a_message(&connection, account.id, inbox);

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).expect("create");
    labels.attach(message, work.id).expect("attach");

    assert!(labels.delete(work.id).expect("delete"));
    assert!(labels.for_message(message).expect("read back").is_empty());
    assert!(labels.get(work.id).expect("get").is_none());
    assert!(
        !labels
            .delete(LabelId::new(9999))
            .expect("delete a stranger"),
        "deleting a label that is not there is not a change"
    );
}

#[test]
fn a_resync_does_not_take_a_label_off_a_message() {
    // The hazard that decides how labels are designed (#780). `write_update`
    // -- which `upsert_batch` uses for every message a sync already knows --
    // replaces a message's whole label set from `message.labels`, and a
    // `Message` built from the wire carries none. So a label attached locally
    // was deleted by the next resync of its mailbox: a feature that works, is
    // tested, and is quietly undone by another layer, which is this
    // repository's characteristic bug.
    let database = test_support::memory();
    let connection = database.connection().expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let labels = LabelRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    message.server.uid = Some(postio_model::Uid::new(1));
    message.server.uid_validity = Some(postio_model::UidValidity::new(100));
    message.server.remote_id = Some(postio_model::RemoteId::new("100:1"));
    messages.create(&mut message).expect("create");

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).expect("create");
    labels.attach(message.id, work.id).expect("attach");

    // The same message coming back from the server, as a sync builds it:
    // flags and coordinates, and no idea about labels.
    let mut fetched = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    fetched.server = message.server.clone();
    messages
        .upsert_batch(&mut vec![fetched])
        .expect("the resync");

    assert_eq!(
        labels.for_message(message.id).expect("read back"),
        vec![work.id],
        "the resync took the label off; a label a person put on a message \
         must survive the next sync of its mailbox"
    );
}
