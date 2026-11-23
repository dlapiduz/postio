//! A sync batch racing UI-thread writes: `BEGIN DEFERRED` promoting a read lock gets `SQLITE_BUSY`
//! without the busy handler, `BEGIN IMMEDIATE` waits (`postio-79`).
//! Timing-dependent on purpose, so it hammers; needs a file-backed database since in-memory fails
//! differently (`SQLITE_LOCKED`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, Message};
use postio_storage::repository::{FlagSource, MessageRepository};
use postio_storage::test_support;
use postio_sync::{Progress, sync_mailbox_with_batch_size};

const INBOX: &str = "INBOX";

const MESSAGES: u32 = 120;

const WRITERS: usize = 3;

#[tokio::test(flavor = "multi_thread")]
async fn a_sync_batch_survives_the_ui_thread_writing_underneath_it() {
    let database = test_support::temp().await;

    let (account, inbox, scratch) = {
        let connection = database.connect().await.expect("checkout");
        let account = test_support::account(&connection).await;
        let inbox = test_support::mailbox(&connection, &account, INBOX).await;
        let drafts = test_support::mailbox(&connection, &account, "Drafts").await;
        let mut message = Message::new(account.id, drafts.id, chrono::Utc::now());
        message.subject = Some("Being typed".into());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("the fixture writes");
        (account, inbox, id)
    };
    let _ = account;

    let mut mailbox = MockMailbox::new(INBOX);
    for n in 1..=MESSAGES {
        mailbox = mailbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 Subject: Note {n}\r\n\r\nBody {n}.\r\n"
            )
            .into_bytes(),
        ));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

    let stop = Arc::new(AtomicBool::new(false));
    let commits = Arc::new(AtomicU64::new(0));
    // Every writer waits here until all are ready, or a green run could mean nothing was writing.
    let ready = Arc::new(tokio::sync::Barrier::new(WRITERS + 1));
    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let database = database.clone();
            let stop = Arc::clone(&stop);
            let commits = Arc::clone(&commits);
            let ready = Arc::clone(&ready);
            tokio::spawn(async move {
                let connection = database.connect().await.map_err(|e| e.to_string())?;
                let messages = MessageRepository::new(&connection);
                let mut flagged = false;
                let warm = |flagged: bool| {
                    let mut flags = FlagSet::default();
                    if flagged {
                        flags.insert(Flag::Flagged);
                    }
                    flags
                };
                messages
                    .set_flags(scratch, &warm(false), FlagSource::Local)
                    .await
                    .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                ready.wait().await;
                while !stop.load(Ordering::Relaxed) {
                    flagged = !flagged;
                    messages
                        .set_flags(scratch, &warm(flagged), FlagSource::Local)
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
    let outcome = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        1,
        &CancelToken::new(),
        |_progress: Progress| {},
    )
    .await;

    stop.store(true, Ordering::Relaxed);
    for writer in writers {
        writer
            .await
            .expect("a writer task panicked")
            .expect("the writers must succeed too");
    }

    let commits = commits.load(Ordering::Relaxed);
    let report = outcome.unwrap_or_else(|error| {
        panic!(
            "a sync batch lost to a concurrent write after {commits} of them: {error}\n\
             If this is a `database is locked` that arrived far sooner than \
             busy_timeout, some write path has gone back to a deferred \
             transaction: it takes a read lock on its first SELECT, and SQLite \
             refuses to let a promotion wait for the write lock rather than \
             risk a deadlock, so the busy handler is never invoked. The two \
             places that decide this are postio_storage's `Scope::open` and \
             the batch transactions in postio_sync::initial and \
             postio_sync::resync."
        )
    });
    assert_eq!(
        report.inserted, MESSAGES as usize,
        "every message should have landed"
    );

    // A canary, not the proof: the barrier guarantees writers were committing; this catches a
    // future edit that leaves them idle.
    assert!(
        commits >= WRITERS as u64,
        "the pass survived, but only {commits} writes landed under it — the \
         writer threads are not running, so this run proved nothing"
    );
}
