use std::time::Duration;

use postio_account::backend::{FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, Mailbox, MessageId, RemoteId, UidValidity};
use postio_storage::repository::{FlagSource, MessageRepository};
use postio_storage::test_support::gate_log::{self, Event};
use postio_storage::{Store, WritePriority, test_support};
use postio_sync::{resync_mailbox, sync_mailbox};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

const MESSAGES: u32 = 600;

const WRITE_UNIT: u32 = 25;

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\
         Message-ID: <note-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

async fn fixture() -> (test_support::TempStore, Mailbox, MessageId) {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    let drafts = test_support::mailbox(&connection, &account, "Drafts").await;
    let mut message = postio_model::Message::new(account.id, drafts.id, chrono::Utc::now());
    message.subject = Some("Being typed".into());
    let scratch = MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("the fixture writes");
    drop(connection);
    (database, inbox, scratch)
}

async fn server() -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=MESSAGES {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    backend
}

fn background_grants(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|event| **event == Event::Granted(WritePriority::Background))
        .count()
}

async fn keystroke_mid_pass(database: &Store, scratch: MessageId) -> usize {
    let started = std::time::Instant::now();
    let patience = Duration::from_secs(120);
    while background_grants(&gate_log::events()) < 2 {
        assert!(
            started.elapsed() < patience,
            "the pass never took a second background permit: {:?}",
            gate_log::events()
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mark = gate_log::events().len();
    let (connection, _permit) = database
        .interactive_write()
        .await
        .expect("the keystroke's checkout");
    let mut flags = FlagSet::default();
    flags.insert(Flag::Flagged);
    MessageRepository::new(&connection)
        .set_flags(scratch, &flags, FlagSource::Local)
        .await
        .expect("the keystroke's write");
    mark
}

fn assert_the_pass_stood_aside(mark: usize, what: &str) {
    let events = gate_log::events();
    assert_eq!(
        gate_log::background_grants_while_interactive_waited(mark),
        Some(0),
        "{what}: a background unit began while the keystroke's write was \
         waiting -- it waited for the pass, not for one unit: {events:?}"
    );
    let interactive_granted = events
        .iter()
        .position(|event| *event == Event::Granted(WritePriority::Interactive))
        .expect("the keystroke was served");
    assert!(
        background_grants(&events[interactive_granted + 1..]) > 0,
        "{what}: no background unit ran after the keystroke, so this run \
         never measured a pass in progress: {events:?}"
    );
    let units = background_grants(&events);
    assert!(
        units >= (MESSAGES / WRITE_UNIT) as usize,
        "{what}: the pass took {units} background permits for {MESSAGES} \
         messages -- fewer than one per {WRITE_UNIT}-message unit, so a \
         keystroke waits for more than one unit"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_sync_stands_aside_for_a_keystroke_at_every_write_unit() {
    let (database, inbox, scratch) = fixture().await;
    let backend = server().await;
    gate_log::reset();

    let pass = tokio::spawn({
        let database = database.clone();
        let inbox = inbox.clone();
        async move {
            let connection = database.connect().await.expect("the pass's checkout");
            sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
                .await
                .expect("the first sync");
        }
    });
    let mark = keystroke_mid_pass(&database, scratch).await;
    pass.await.expect("the pass task");

    assert_the_pass_stood_aside(mark, "first sync");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resync_stands_aside_for_a_keystroke_at_every_write_unit() {
    let (database, inbox, scratch) = fixture().await;
    let backend = server().await;
    {
        let connection = database.connect().await.expect("checkout");
        sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
            .await
            .expect("bootstrap sync");
    }
    let everything: Vec<RemoteId> = (1..=MESSAGES)
        .map(|uid| RemoteId::new(format!("{VALIDITY}:{uid}")))
        .collect();
    backend
        .store_flags(
            INBOX,
            &everything,
            &FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("the server-side flag change");
    gate_log::reset();

    let pass = tokio::spawn({
        let database = database.clone();
        let inbox = inbox.clone();
        async move {
            let connection = database.connect().await.expect("the pass's checkout");
            resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
                .await
                .expect("the resync");
        }
    });
    let mark = keystroke_mid_pass(&database, scratch).await;
    pass.await.expect("the pass task");

    assert_the_pass_stood_aside(mark, "resync");
}
