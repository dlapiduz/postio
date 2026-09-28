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
use postio_storage::Connection;
use postio_storage::repository::{LabelRepository, MessageRepository};
use postio_storage::test_support;

async fn a_message(
    connection: &Connection,
    account: AccountId,
    mailbox: postio_model::MailboxId,
) -> postio_model::MessageId {
    let mut message = postio_model::Message::new(account, mailbox, chrono::Utc::now());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create a message")
}

/// A second account, so "this account's labels" means something.
async fn another_account(connection: &Connection) -> postio_model::Account {
    let mut account = postio_model::Account::new(
        "Quinn",
        postio_model::EmailAddress::new(Some("Quinn Abara"), "quinn@example.net"),
    );
    account.incoming.host = "imap.example.net".to_owned();
    account.outgoing.host = "smtp.example.net".to_owned();
    postio_storage::repository::AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("create a second account");
    account
}

#[tokio::test]
async fn an_account_lists_the_labels_it_owns_and_no_others() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, _inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);

    let mut work = Label::new(account.id, "Work");
    let mut receipts = Label::new(account.id, "Receipts");
    labels.create(&mut work).await.expect("create");
    labels.create(&mut receipts).await.expect("create");
    assert!(work.id.is_assigned(), "create assigns the id");

    let listed = labels.list(account.id).await.expect("list");
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
    let other = another_account(&connection).await;
    let mut theirs = Label::new(other.id, "Theirs");
    labels.create(&mut theirs).await.expect("create");
    let listed = labels.list(account.id).await.expect("list");
    assert!(
        !listed.iter().any(|label| label.name == "Theirs"),
        "one account's picker offered another account's label: {listed:?}"
    );
}

#[tokio::test]
async fn a_label_name_is_unique_per_account_whatever_its_case() {
    // `idx_labels_account_name` is `COLLATE NOCASE`, so this is the schema's
    // rule; the repository has to answer it as something a caller can act on
    // rather than a raw constraint violation.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, _inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);

    let mut first = Label::new(account.id, "Work");
    labels.create(&mut first).await.expect("create");

    let mut again = Label::new(account.id, "work");
    assert!(
        labels.create(&mut again).await.is_err(),
        "two labels differing only in case would look like one to a person \
         and like two to the picker"
    );

    // The same name under a different account is a different label.
    let other = another_account(&connection).await;
    let mut theirs = Label::new(other.id, "Work");
    labels
        .create(&mut theirs)
        .await
        .expect("a second account may use the name");
}

#[tokio::test]
async fn a_label_goes_on_and_off_one_message_without_rewriting_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);
    let message = a_message(&connection, account.id, inbox).await;

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).await.expect("create");

    assert!(
        labels.attach(message, work.id).await.expect("attach"),
        "newly on"
    );
    assert_eq!(
        labels.for_message(message).await.expect("read back"),
        vec![work.id]
    );
    // The message row itself agrees, which is what the list and the reader
    // render from.
    assert_eq!(
        MessageRepository::new(&connection)
            .get(message)
            .await
            .expect("get")
            .expect("the message")
            .labels,
        vec![work.id]
    );

    // Attaching again is not an error and not a second row: `message_labels`
    // is keyed on the pair, and a command that ran twice must be harmless.
    assert!(
        !labels.attach(message, work.id).await.expect("attach again"),
        "the second attach reports that nothing changed"
    );
    assert_eq!(
        labels.for_message(message).await.expect("read back").len(),
        1
    );

    assert!(
        labels.detach(message, work.id).await.expect("detach"),
        "came off"
    );
    assert!(
        labels
            .for_message(message)
            .await
            .expect("read back")
            .is_empty()
    );
    assert!(
        !labels.detach(message, work.id).await.expect("detach again"),
        "detaching what is not there reports that nothing changed, so an \
         undo that runs twice does not claim to have done something"
    );
}

#[tokio::test]
async fn deleting_a_label_takes_it_off_every_message_carrying_it() {
    // `ON DELETE CASCADE` on `message_labels`, asserted because a label that
    // outlived its rows would leave messages pointing at nothing.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);
    let message = a_message(&connection, account.id, inbox).await;

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).await.expect("create");
    labels.attach(message, work.id).await.expect("attach");

    assert!(labels.delete(work.id).await.expect("delete"));
    assert!(
        labels
            .for_message(message)
            .await
            .expect("read back")
            .is_empty()
    );
    assert!(labels.get(work.id).await.expect("get").is_none());
    assert!(
        !labels
            .delete(LabelId::new(9999))
            .await
            .expect("delete a stranger"),
        "deleting a label that is not there is not a change"
    );
}

#[tokio::test]
async fn a_resync_does_not_take_a_label_off_a_message() {
    // The hazard that decides how labels are designed (#780). `write_update`
    // -- which `upsert_batch` uses for every message a sync already knows --
    // replaces a message's whole label set from `message.labels`, and a
    // `Message` built from the wire carries none. So a label attached locally
    // was deleted by the next resync of its mailbox: a feature that works, is
    // tested, and is quietly undone by another layer, which is this
    // repository's characteristic bug.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    message.server.uid = Some(postio_model::Uid::new(1));
    message.server.uid_validity = Some(postio_model::UidValidity::new(100));
    message.server.remote_id = Some(postio_model::RemoteId::new("100:1"));
    messages.create(&mut message).await.expect("create");

    let mut work = Label::new(account.id, "Work");
    labels.create(&mut work).await.expect("create");
    labels.attach(message.id, work.id).await.expect("attach");

    // The same message coming back from the server, as a sync builds it:
    // flags and coordinates, and no idea about labels.
    let mut fetched = postio_model::Message::new(account.id, inbox, chrono::Utc::now());
    fetched.server = message.server.clone();
    messages
        .upsert_batch(&mut vec![fetched])
        .await
        .expect("the resync");

    assert_eq!(
        labels.for_message(message.id).await.expect("read back"),
        vec![work.id],
        "the resync took the label off; a label a person put on a message \
         must survive the next sync of its mailbox"
    );
}

/// Files a message into `mailbox` under `rfc`, threaded, carrying `labels`.
async fn a_threaded_message(
    connection: &Connection,
    account: AccountId,
    mailbox: postio_model::MailboxId,
    rfc: &str,
    references: &[&str],
    labels: &[LabelId],
) -> postio_model::ThreadId {
    let mut message = postio_model::Message::new(account, mailbox, chrono::Utc::now());
    message.rfc_message_id = Some(postio_model::RfcMessageId::new(rfc));
    message.references = references
        .iter()
        .map(postio_model::RfcMessageId::new)
        .collect();
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create a message");
    for label in labels {
        LabelRepository::new(connection)
            .attach(message.id, *label)
            .await
            .expect("attach");
    }
    postio_storage::repository::ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("threaded")
        .thread_id
}

#[tokio::test]
async fn a_page_s_labels_are_one_statement_that_reads_them_and_scans_no_table() {
    // A Focus page draws each conversation's label pills (spec 007 T043),
    // read for the whole page at once: a conversation's labels are every
    // label any of its messages carries, each once, in the order they were
    // made -- and never one statement a row.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);
    let mut made = Vec::new();
    for name in ["Atlas", "Harbor", "Home"] {
        let mut label = Label::new(account.id, name);
        labels.create(&mut label).await.expect("create");
        made.push(label);
    }
    let (atlas, harbor, home) = (made[0].id, made[1].id, made[2].id);
    // A conversation of two carrying all three between its messages, one
    // label on both; another carrying one; a third carrying none.
    let three = a_threaded_message(
        &connection,
        account.id,
        inbox,
        "<a@x.test>",
        &[],
        &[atlas, harbor],
    )
    .await;
    a_threaded_message(
        &connection,
        account.id,
        inbox,
        "<b@x.test>",
        &["<a@x.test>"],
        &[harbor, home],
    )
    .await;
    let one = a_threaded_message(&connection, account.id, inbox, "<c@x.test>", &[], &[home]).await;
    let none = a_threaded_message(&connection, account.id, inbox, "<d@x.test>", &[], &[]).await;

    let _ = labels.for_threads(&[three]).await.expect("warm");
    let mut found = Vec::new();
    let counts = counted_async(|| async {
        found = labels
            .for_threads(&[three, one, none])
            .await
            .expect("the page's labels");
    })
    .await;
    let named: Vec<(postio_model::ThreadId, &str)> = found
        .iter()
        .map(|(thread, label)| (*thread, label.name.as_str()))
        .collect();
    let mut expected = vec![
        (three, "Atlas"),
        (three, "Harbor"),
        (three, "Home"),
        (one, "Home"),
    ];
    expected.sort_by_key(|(thread, _)| *thread);
    assert_eq!(
        named, expected,
        "each conversation's labels once each, in the order they were made"
    );
    assert_eq!(
        counts.statements, 1,
        "one statement for the page: {counts:?}"
    );
    assert_eq!(counts.rows, 4, "every row read is a pill: {counts:?}");
    assert!(
        scans(&connection, &LabelRepository::explain_for_threads(3))
            .await
            .is_empty(),
        "a page's labels are sought through the conversations' index, never a walk"
    );
}

#[tokio::test]
async fn each_label_counts_its_conversations_in_one_statement() {
    // Focus's label picker and folders popover show each label with how
    // many conversations carry it (screens 10 and 13): every label of the
    // account at once, a conversation counted once however many of its
    // messages carry the label, and a label nobody uses counted as none.
    use postio_storage::test_support::counting::{counted_async, scans};

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let labels = LabelRepository::new(&connection);
    let mut made = Vec::new();
    for name in ["Atlas", "Harbor", "Home", "Unused"] {
        let mut label = Label::new(account.id, name);
        labels.create(&mut label).await.expect("create");
        made.push(label.id);
    }
    let (atlas, harbor, home, unused) = (made[0], made[1], made[2], made[3]);
    a_threaded_message(
        &connection,
        account.id,
        inbox,
        "<a@x.test>",
        &[],
        &[atlas, harbor],
    )
    .await;
    a_threaded_message(
        &connection,
        account.id,
        inbox,
        "<b@x.test>",
        &["<a@x.test>"],
        &[harbor, home],
    )
    .await;
    a_threaded_message(&connection, account.id, inbox, "<c@x.test>", &[], &[home]).await;
    let other = another_account(&connection).await;
    let mut theirs = Label::new(other.id, "Atlas");
    labels.create(&mut theirs).await.expect("create");

    let _ = labels.counts(account.id).await.expect("warm");
    let mut found = Vec::new();
    let counts = counted_async(|| async {
        found = labels.counts(account.id).await.expect("the counts");
    })
    .await;
    found.sort();
    let mut expected = vec![(atlas, 1), (harbor, 1), (home, 2)];
    expected.sort();
    assert_eq!(
        found, expected,
        "conversations per label, the unused one absent"
    );
    assert!(!found.iter().any(|(label, _)| *label == unused));
    assert_eq!(counts.statements, 1, "one statement: {counts:?}");
    assert!(
        scans(&connection, LabelRepository::explain_counts())
            .await
            .is_empty(),
        "sought through the account's labels and each label's index"
    );
}
