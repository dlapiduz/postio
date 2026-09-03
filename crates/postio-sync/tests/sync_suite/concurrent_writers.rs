//! A sync batch and the UI thread writing at the same time.
//!
//! `postio-79`. The engine is a single-thread tokio runtime with no `await`
//! between a batch's `BEGIN` and its `COMMIT`, so two engine passes cannot
//! interleave inside the write section. The other writer is the one that
//! matters: every mutating action in Postio is local-first, so flagging a
//! message, archiving one, or a draft autosaving as it is typed all write to
//! the same pool from the GTK thread while a sync pass is running.
//!
//! # Why a deferred transaction is the bug
//!
//! `BEGIN DEFERRED` takes no lock. The first statement inside a sync batch is
//! a `SELECT` — `own_draft_copies`, then `find_by_uid` — so the transaction is
//! already holding a *read* lock by the time it tries to write, and has to
//! promote. SQLite will not make a promotion wait: blocking a connection that
//! already holds a read lock could deadlock against the very writer it would
//! be waiting for, so it returns `SQLITE_BUSY` and deliberately **does not
//! invoke the busy handler** — the case `sqlite3_busy_handler`'s own
//! documentation carves out. `PRAGMA busy_timeout = 5000` therefore never gets
//! a say, and the batch, with it the pass, fails instead of waiting.
//!
//! That "instead of waiting" is what it looks like from outside: an unfixed
//! run here dies in about 50ms against a five-second timeout. The extended
//! code is plain `SQLITE_BUSY` (5), *not* the `SQLITE_BUSY_SNAPSHOT` (517)
//! that a stale-snapshot read gives — worth knowing when reading a bug
//! report, because the two are the same shape of problem with the same fix
//! and only one of them names itself.
//!
//! `BEGIN IMMEDIATE` takes the write lock up front, before any read has
//! happened, so there is no promotion to refuse: the busy handler applies and
//! the five-second timeout means what it says.
//!
//! # This test is timing-dependent, on purpose
//!
//! There is no way to make the race deterministic without a hook in the
//! production write path, which would be a worse trade than a test that has to
//! hammer. So it hammers: several writer threads committing continuously
//! against a batch size of one, which is the smallest batch and therefore the
//! most read-then-write windows per message fetched. Against the unfixed code
//! it does not merely reproduce sometimes — it loses the *first* batch, every
//! run, before the writers have managed five commits between them.
//!
//! It needs a *file-backed* database. An in-memory one shares a cache between
//! the pool's connections and fails differently (`SQLITE_LOCKED`), which is a
//! different bug from the one this is about.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{Flag, FlagSet, Message};
use postio_storage::repository::{FlagSource, MessageRepository};
use postio_storage::test_support;
use postio_sync::{Progress, sync_mailbox_with_batch_size};

const INBOX: &str = "INBOX";

/// Enough messages that a batch size of one gives the race many chances.
const MESSAGES: u32 = 120;

/// More than one, so a commit is in flight most of the time rather than some
/// of it. Fewer than the core count, so the sync thread is never starved.
const WRITERS: usize = 3;

#[tokio::test]
async fn a_sync_batch_survives_the_ui_thread_writing_underneath_it() {
    // File-backed: see the module docs on why in-memory proves something else.
    let database = test_support::temp();

    let (account, inbox, scratch) = {
        let connection = database.connection().expect("checkout");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, INBOX);
        // The row the writers hammer, in a mailbox of its own so nothing they
        // do can be mistaken for something the sync pass did.
        let drafts = test_support::mailbox(&connection, &account, "Drafts");
        let mut message = Message::new(account.id, drafts.id, chrono::Utc::now());
        message.subject = Some("Being typed".into());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
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

    // -- the UI thread, as far as SQLite is concerned ----------------------
    let stop = Arc::new(AtomicBool::new(false));
    let commits = Arc::new(AtomicU64::new(0));
    // Every writer waits here until all of them — and the sync — are ready.
    // Without it the pass can be most of the way through its 120 batches
    // before a thread has finished checking a connection out of the pool, and
    // a green run then means "nothing was writing", which proves nothing.
    let ready = Arc::new(std::sync::Barrier::new(WRITERS + 1));
    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let database = database.clone();
            let stop = Arc::clone(&stop);
            let commits = Arc::clone(&commits);
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || -> Result<(), String> {
                let connection = database.connection().map_err(|e| e.to_string())?;
                let messages = MessageRepository::new(&connection);
                let mut flagged = false;
                // Everything expensive — the pool checkout, the first
                // statement prepare — happens before the barrier, so that
                // once the sync starts these threads are already committing.
                let warm = |flagged: bool| {
                    let mut flags = FlagSet::default();
                    if flagged {
                        flags.insert(Flag::Flagged);
                    }
                    flags
                };
                messages
                    .set_flags(scratch, &warm(false), FlagSource::Local)
                    .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                ready.wait();
                while !stop.load(Ordering::Relaxed) {
                    flagged = !flagged;
                    // One statement, one commit — exactly what `f` on a
                    // focused row costs, and every one of them advances the
                    // WAL under whatever the sync pass is holding.
                    messages
                        .set_flags(scratch, &warm(flagged), FlagSource::Local)
                        .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                    commits.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            })
        })
        .collect();

    // -- a sync pass, batch by batch ---------------------------------------
    let connection = database.connection().expect("checkout");
    ready.wait();
    let outcome = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        // One message per batch: the most `BEGIN`/first-`SELECT`/first-write
        // windows this fixture can produce.
        1,
        &CancelToken::new(),
        |_progress: Progress| {},
    )
    .await;

    stop.store(true, Ordering::Relaxed);
    for writer in writers {
        writer
            .join()
            .expect("a writer thread panicked")
            .expect("the writers must succeed too");
    }

    // The outcome first, and the canary second. A failed pass *is* the bug
    // reproducing, and unfixed it fails on the very first batch — so a guard
    // on "were the writers really writing" would fire before the real
    // assertion ever ran and report the wrong thing entirely.
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

    // A canary, not the proof. The barrier above is what actually guarantees
    // the writers were committing for the whole of the pass — they are past it
    // before the sync starts and are only stopped after it returns — so this
    // just catches a future edit that leaves them doing nothing at all and
    // takes the green run with it. Deliberately not a threshold proportional
    // to `MESSAGES`: once the pass holds the write lock per batch the writers
    // get fewer turns, and a run that committed 22 times raced exactly as
    // honestly as one that committed 2000. What proves this test is worth
    // having is that it fails every run without the fix, which is the module
    // docs' claim and not something an assertion here can check.
    assert!(
        commits >= WRITERS as u64,
        "the pass survived, but only {commits} writes landed under it — the \
         writer threads are not running, so this run proved nothing"
    );
}
