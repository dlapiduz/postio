use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use postio_account::backend::{FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, RemoteId, UidValidity};
use postio_storage::repository::{FlagSource, MessageRepository};
use postio_storage::{WritePriority, test_support};
use postio_sync::{resync_mailbox, sync_mailbox};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

const MESSAGES: u32 = 4_000;

const WRITER_BUSY_TIMEOUT: u32 = 250;

const WRITERS: usize = 1;

/// Without a gap this test hangs: `WriteGate` holds a background writer back while any interactive
/// writer waits, and a tight loop leaves no instant with nobody waiting.
const WRITER_PACE: Duration = Duration::from_millis(5);

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\
         Message-ID: <note-{n}@example.com>\r\n\
         Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resync_batch_does_not_lock_out_an_interactive_write() {
    let database = test_support::temp().await;

    let (inbox, scratch) = {
        let connection = database.connect().await.expect("checkout");
        let account = test_support::account(&connection).await;
        let inbox = test_support::mailbox(&connection, &account, INBOX).await;
        let drafts = test_support::mailbox(&connection, &account, "Drafts").await;
        let mut message = postio_model::Message::new(account.id, drafts.id, chrono::Utc::now());
        message.subject = Some("Being typed".into());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("the fixture writes");
        (inbox, id)
    };

    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=MESSAGES {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

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

    let stop = Arc::new(AtomicBool::new(false));
    let commits = Arc::new(AtomicU64::new(0));
    let ready = Arc::new(tokio::sync::Barrier::new(WRITERS + 1));
    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let database = database.clone();
            let stop = Arc::clone(&stop);
            let commits = Arc::clone(&commits);
            let ready = Arc::clone(&ready);
            tokio::spawn(async move {
                let connection = database.connect().await.map_err(|e| e.to_string())?;
                connection
                    .busy_timeout(std::time::Duration::from_millis(u64::from(
                        WRITER_BUSY_TIMEOUT,
                    )))
                    .map_err(|e| e.to_string())?;
                let messages = MessageRepository::new(&connection);
                let mut flagged = false;
                let flags = |flagged: bool| {
                    let mut flags = FlagSet::default();
                    if flagged {
                        flags.insert(Flag::Flagged);
                    }
                    flags
                };
                messages
                    .set_flags(scratch, &flags(false), FlagSource::Local)
                    .await
                    .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                ready.wait().await;
                while !stop.load(Ordering::Relaxed) {
                    tokio::time::sleep(WRITER_PACE).await;
                    flagged = !flagged;
                    let _permit = database
                        .write_gate()
                        .acquire(WritePriority::Interactive)
                        .await;
                    messages
                        .set_flags(scratch, &flags(flagged), FlagSource::Local)
                        .await
                        .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                    commits.fetch_add(1, Ordering::Relaxed);
                }
                Ok::<(), String>(())
            })
        })
        .collect();

    let connection = database.connect().await.expect("checkout");
    ready.wait().await;
    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {}).await;

    stop.store(true, Ordering::Relaxed);
    for writer in writers {
        writer
            .await
            .expect("a writer task panicked")
            .expect("an interactive write must not lose its place to a resync");
    }
    outcome.expect("the resync itself must still succeed");

    let commits = commits.load(Ordering::Relaxed);
    assert!(
        commits > 0,
        "no interactive write completed during the resync, so this run never \
         exercised the contention it is about"
    );
}
