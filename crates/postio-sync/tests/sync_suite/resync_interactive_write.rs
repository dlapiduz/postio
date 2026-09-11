//! A resync batch does not lock the UI thread's write out.
//!
//! # What went wrong
//!
//! #425 established that an interactive write must never queue behind bulk
//! sync work, and `postio_storage::WriteGate` is the queue-with-a-priority
//! that makes it so. `initial.rs` takes a `WritePriority::Background` permit
//! ahead of every `BEGIN IMMEDIATE`, and re-takes it per `WRITE_UNIT`, so a
//! person waits for one unit at most however long the sync runs.
//!
//! `resync.rs` — the path that runs on *every* start, for every folder, and
//! therefore the one a person actually meets — took no permit at all, and
//! wrote its whole changed set inside a single transaction. Its own comment
//! says it "meets the UI thread's local-first writes more often than the
//! first-sync one does", which is exactly right and exactly why the omission
//! mattered.
//!
//! Observed: a draft autosaved as it was typed while an ordinary resync ran,
//! and the write lost. `PRAGMA busy_timeout` is a retry loop, not a queue, so
//! a writer that is still waiting when it expires does not queue — it fails.
//! The draft was not saved.
//!
//! # Why the writers shorten their own timeout
//!
//! The production timeout is five seconds, so reproducing the loss at its
//! real size needs a changed set big enough to hold the lock for longer than
//! that — thousands of messages, and a slow test that measures the machine as
//! much as the code.
//!
//! Shrinking the writers' timeout instead keeps the *shape* and drops the
//! scale: the question is whether an interactive write waits for the whole
//! batch or for one write unit, and any timeout between those two answers
//! separates them. [`WRITER_BUSY_TIMEOUT`] sits between the two by a wide
//! margin at both ends.
//!
//! Note what the fix makes true, which is stronger than the assertion below:
//! a writer that holds the gate meets no background transaction at all,
//! because a background unit cannot *begin* while an interactive writer is
//! waiting. The timeout only has to cover the one unit that was already in
//! flight when the writer arrived.

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

/// Enough changed messages that one ungated transaction over the lot is
/// plainly longer than [`WRITER_BUSY_TIMEOUT`], and few enough that the
/// bootstrap sync in front of it stays quick.
///
/// Measured on this workspace: at four thousand the unfixed code loses the
/// write every run, and the whole test — bootstrap sync included — takes
/// about five seconds. Six hundred was tried first and was *not* enough: the
/// single transaction finished inside the writer's timeout, so the writer
/// waited and then succeeded, and the test passed against the very bug it
/// was written for.
const MESSAGES: u32 = 4_000;

/// How long a writer waits for the lock before giving up, in milliseconds.
///
/// Far longer than one `WRITE_UNIT` of 25 messages, which is what the fix
/// leaves it waiting for; far shorter than a single transaction over all
/// [`MESSAGES`], which is what it waited for before. See the module docs.
const WRITER_BUSY_TIMEOUT: u32 = 250;

/// One, and exactly one: a second writer would contend with the *first* for
/// the gate as well as with the resync, and a failure could then be two
/// interactive writers colliding rather than the thing this is about. One
/// writer is also what the bug was — a person typing into a composer.
const WRITERS: usize = 1;

/// A pause between one writer's commits, so that the writers are *people*.
///
/// Without it this test hangs, and deservedly: `WriteGate` holds a background
/// writer back while any interactive writer is waiting, and says in as many
/// words that it "does not try to be fair in that direction" because
/// interactive writers are human-paced. A thread looping with no gap at all
/// is not human-paced — it leaves no instant with nobody waiting, so the
/// resync never gets a permit, and the first version of this test hung for
/// four minutes starving the background lane it came to measure.
///
/// Five milliseconds is two hundred keystrokes a second per writer, which is
/// still far faster than a person and still leaves the gate its gaps.
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
    // File-backed, for the reason `concurrent_writers.rs` records: an
    // in-memory database shares a cache between the pool's connections and
    // fails in a different model entirely (#204).
    let database = test_support::temp();

    let (inbox, scratch) = {
        let connection = database.connection().expect("checkout");
        let account = test_support::account(&connection);
        let inbox = test_support::mailbox(&connection, &account, INBOX);
        // The row the writers hammer lives in a mailbox of its own, so
        // nothing they do can be mistaken for something the resync did.
        let drafts = test_support::mailbox(&connection, &account, "Drafts");
        let mut message = postio_model::Message::new(account.id, drafts.id, chrono::Utc::now());
        message.subject = Some("Being typed".into());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .expect("the fixture writes");
        (inbox, id)
    };

    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=MESSAGES {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

    // The store matches the server, so what follows is a resync and not a
    // first sync — which is the whole point: this is the ordinary path.
    {
        let connection = database.connection().expect("checkout");
        sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
            .await
            .expect("bootstrap sync");
    }

    // Another client reads every message, so the resync has all of them to
    // write back.
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

    // ── the UI thread, as far as SQLite is concerned ─────────────────────
    let stop = Arc::new(AtomicBool::new(false));
    let commits = Arc::new(AtomicU64::new(0));
    // Every writer waits here until all of them — and the resync — are ready,
    // so a green run cannot mean "nothing was writing yet".
    let ready = Arc::new(std::sync::Barrier::new(WRITERS + 1));
    let writers: Vec<_> = (0..WRITERS)
        .map(|_| {
            let database = database.clone();
            let stop = Arc::clone(&stop);
            let commits = Arc::clone(&commits);
            let ready = Arc::clone(&ready);
            std::thread::spawn(move || -> Result<(), String> {
                // Connection first and permit second, per `WriteGate`'s rules
                // for callers. Held across the loop rather than re-checked
                // out, because the timeout below is a property of the
                // connection and the permit is what has to be re-taken.
                let connection = database.connection().map_err(|e| e.to_string())?;
                connection
                    .pragma_update(None, "busy_timeout", WRITER_BUSY_TIMEOUT)
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
                // The pool checkout and the first statement prepare happen
                // before the barrier, so these threads are already writing
                // by the time the resync starts.
                messages
                    .set_flags(scratch, &flags(false), FlagSource::Local)
                    .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                ready.wait();
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(WRITER_PACE);
                    flagged = !flagged;
                    let _permit = database.write_gate().acquire(WritePriority::Interactive);
                    // One statement, one commit — what `f` on a focused row
                    // costs, and what a draft's autosave costs.
                    messages
                        .set_flags(scratch, &flags(flagged), FlagSource::Local)
                        .map_err(|error| format!("the UI thread's own write failed: {error}"))?;
                    commits.fetch_add(1, Ordering::Relaxed);
                }
                Ok(())
            })
        })
        .collect();

    // ── the resync ───────────────────────────────────────────────────────
    let connection = database.connection().expect("checkout");
    ready.wait();
    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {}).await;

    stop.store(true, Ordering::Relaxed);
    let results: Vec<_> = writers
        .into_iter()
        .map(|writer| writer.join().expect("a writer thread panicked"))
        .collect();

    // The writers first: a lost interactive write *is* the bug, and it is
    // what a person loses a draft to.
    for result in results {
        result.expect("an interactive write must not lose its place to a resync");
    }
    outcome.expect("the resync itself must still succeed");

    // Non-vacuity: a run where the writers never got going proves nothing.
    let commits = commits.load(Ordering::Relaxed);
    assert!(
        commits > 0,
        "no interactive write completed during the resync, so this run never \
         exercised the contention it is about"
    );
}
