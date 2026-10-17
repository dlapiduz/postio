//! A full resync of unchanged mail writes nothing (#1719).
//!
//! A backend with no MODSEQ -- a Maildir, any IMAP server without CONDSTORE
//! -- plans `Full(NoModSeq)` on every pass and re-reads every header in the
//! folder. Each pass used to upsert and re-thread every one of them, holding
//! the store's write lock for seconds over a folder where nothing had
//! changed, and reporting every row as updated, which redraws every list.
//!
//! What is counted is the writes themselves: a trigger logs every row the
//! pass writes to `messages`, and `counting` records every statement it
//! issued, so a pass that threads anything is seen doing it.

use postio_account::backend::{FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, FullResyncReason, Generation, Mailbox, Uid, UidValidity};
use postio_storage::repository::MessageRepository;
use postio_storage::sql::{self, RowExt as _};
use postio_storage::test_support::{self, counting};
use postio_storage::{Checkout, Connection};
use postio_sync::{Outcome, initial::Report, resync_mailbox, sync_mailbox};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

/// A server of `count` messages with no CONDSTORE, so its statuses carry no
/// MODSEQ and every pass after the first is a full one.
async fn modseq_less_server(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder()
        .capabilities(["IMAP4rev1"])
        .mailbox(mailbox)
        .build();
    backend.connect().await.expect("connect");
    backend
}

/// A store holding `backend`'s inbox after its first sync, with a trigger
/// logging every row written to `messages` from here on.
async fn synced(connection: &Checkout, backend: &MockBackend) -> Mailbox {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, INBOX).await;
    sync_mailbox(connection, backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");
    sql::batch(
        connection,
        "CREATE TABLE message_writes (id INTEGER PRIMARY KEY, message_id INTEGER);
         CREATE TRIGGER count_message_inserts AFTER INSERT ON messages
         BEGIN INSERT INTO message_writes (message_id) VALUES (new.id); END;
         CREATE TRIGGER count_message_updates AFTER UPDATE ON messages
         BEGIN INSERT INTO message_writes (message_id) VALUES (new.id); END;",
    )
    .await
    .expect("the counting triggers");
    inbox
}

/// The full pass a modseq-less backend runs, with what it did and every
/// statement it issued.
async fn full_pass(
    connection: &Checkout,
    backend: &MockBackend,
    inbox: &Mailbox,
) -> (Report, std::collections::BTreeMap<String, usize>) {
    counting::record();
    let outcome = resync_mailbox(connection, backend, inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the resync");
    let issued = counting::recorded();
    match outcome {
        Outcome::Full {
            reason: FullResyncReason::NoModSeq,
            report,
        } => (report, issued),
        other => panic!("the fixture is about the modseq-less plan: {other:?}"),
    }
}

/// The ids of the `messages` rows written since the triggers went in.
async fn rows_written(connection: &Connection) -> Vec<i64> {
    sql::all(
        connection,
        "SELECT DISTINCT message_id FROM message_writes ORDER BY message_id",
        (),
        |row| row.col(0),
    )
    .await
    .expect("the write log")
}

/// The statements that thread a message: every one of them reads or writes
/// `thread_links`.
fn threading(issued: &std::collections::BTreeMap<String, usize>) -> Vec<&String> {
    issued
        .keys()
        .filter(|sql| sql.contains("thread_links"))
        .collect()
}

#[tokio::test]
async fn a_full_resync_of_unchanged_mail_writes_nothing() {
    let backend = modseq_less_server(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let inbox = synced(&connection, &backend).await;

    let (report, issued) = full_pass(&connection, &backend, &inbox).await;

    assert_eq!(
        (report.inserted, report.updated, report.threaded),
        (0, 0, 0),
        "nothing changed on the server, so nothing changed here, and a host \
         raises no MessagesChanged: {report:?}"
    );
    assert_eq!(
        rows_written(&connection).await,
        Vec::<i64>::new(),
        "an unchanged folder rewrote its rows"
    );
    assert_eq!(
        threading(&issued),
        Vec::<&String>::new(),
        "an unchanged folder was threaded again"
    );
}

#[tokio::test]
async fn a_flag_changed_on_the_server_still_reaches_its_row() {
    let backend = modseq_less_server(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let inbox = synced(&connection, &backend).await;

    // Another client reads message 2 between passes.
    backend
        .store_flags(
            INBOX,
            &[postio_model::RemoteId::new(format!("{VALIDITY}:2"))],
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");

    let (report, _issued) = full_pass(&connection, &backend, &inbox).await;

    let read = MessageRepository::new(&connection)
        .by_uid(inbox.id, Generation::new(VALIDITY), Uid::new(2))
        .await
        .expect("read message 2")
        .expect("message 2 is stored");
    assert!(
        read.flags.is_seen(),
        "the server's read never reached the row"
    );
    assert_eq!(
        (report.inserted, report.updated),
        (0, 1),
        "only the message that changed is written: {report:?}"
    );
    assert_eq!(
        rows_written(&connection).await,
        vec![read.id.get()],
        "rows other than the one whose flag changed were rewritten"
    );

    // And the pass after it, with nothing changed again, writes nothing.
    let (report, _issued) = full_pass(&connection, &backend, &inbox).await;
    assert_eq!((report.updated, report.threaded), (0, 0), "{report:?}");
}
