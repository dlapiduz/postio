//! Initial sync: enumerating a mailbox for the first time, newest mail first.
//!
//! # Why newest first
//!
//! A mailbox with years of history can hold tens of thousands of messages, and
//! nobody opens Postio to read the oldest one. CLAUDE.md's performance budget
//! makes this structural rather than a nicety: the first screenful has to be
//! visible in seconds, which means the *order* messages are fetched in is the
//! whole of the perceived-speed story. So this module walks the mailbox's UID
//! space from [`MailboxStatus::uid_next`](postio_imap::backend::MailboxStatus)
//! downwards, in batches, committing and threading each batch before asking
//! for the next.
//!
//! # Resumability, for free
//!
//! There is no separate "how far did we get" counter to keep in step with the
//! messages it describes. Instead, every pass asks
//! [`MessageRepository::uids_in`] what is already stored under the mailbox's
//! current `UIDVALIDITY` and only fetches what is missing. A crash mid-sync
//! leaves whatever was committed in the database, and the next call to
//! [`sync_mailbox`] sees exactly that and picks up where it left off — no
//! watermark to persist, and no watermark to get out of sync with the rows it
//! was supposed to describe.
//!
//! [`SyncState::complete_full_sync`](postio_model::SyncState::complete_full_sync)
//! is the marker that a pass ran to completion; it is written last, after
//! every batch, so an interrupted pass is indistinguishable from one that
//! never started and simply resumes.
//!
//! # Threading as messages arrive
//!
//! A reply routinely has a lower `Date` than the message it answers but can
//! easily have a *higher* UID (it was received later), so newest-first order
//! means a thread's replies are seen before the message that started it. That
//! is exactly the case [`postio_model::threading`] is built for: filing a
//! reply claims its parent's `Message-ID` immediately, and when the parent
//! turns up in a later batch it finds the thread that was already waiting for
//! it. See `ThreadingRepository`'s module docs for the mechanism.
//!
//! # What this does not do
//!
//! Nothing here decides *whether* a mailbox needs this treatment, or wipes a
//! mailbox whose `UIDVALIDITY` changed — that decision is
//! [`SyncState::plan`](postio_model::SyncState::plan), and the wipe is the
//! caller's job before this function is ever called. This module only knows
//! how to fill a mailbox that is, or is becoming, empty of a UID range.

use std::collections::BTreeSet;

use chrono::Utc;
use postio_imap::backend::{BackendError, MailBackend, SelectMode, UidSet};
use postio_model::{Mailbox, MailboxId, MailboxStatus, Message, Uid};
use postio_storage::repository::{
    AccountRepository, MessageRepository, SyncStateRepository, ThreadingRepository,
};
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::drain::SyncError;
use postio_imap::cancel::CancelToken;

/// This module's result type.
pub type Result<T> = std::result::Result<T, SyncError>;

/// How many UIDs one `FETCH` asks for at a time.
///
/// Small enough that the first batch — and therefore the first screenful —
/// commits and is visible in well under a second; large enough that a
/// ten-thousand-message inbox does not need ten thousand round trips.
pub const DEFAULT_BATCH_SIZE: usize = 200;

/// What one committed batch reports, so the caller can drive a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// The mailbox this batch belongs to.
    pub mailbox_id: MailboxId,
    /// Messages written to the local store so far this pass, counting
    /// whatever was already there when it started.
    pub fetched: u32,
    /// How many messages the server says the mailbox holds — its `EXISTS`.
    ///
    /// The same kind of thing as [`fetched`](Self::fetched), which is what
    /// makes the pair a fraction anyone can read. It is deliberately *not*
    /// `UIDNEXT - 1`: that is the width of the UID space, which counts every
    /// message ever expunged from the folder, and a long-lived inbox of
    /// ninety-two messages reported `61 / 63022` and rendered as `0%` for the
    /// whole of a pass (`postio-qhz.9`). The enumeration still walks the UID
    /// ceiling — it has to — but nobody has to look at it.
    pub target: u32,
}

/// What a completed (or resumed) pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Messages that were not known locally before this pass.
    pub inserted: usize,
    /// Messages already present that this pass wrote again — resumed rows
    /// from an earlier interrupted pass land here, harmlessly.
    pub updated: usize,
    /// Messages filed into a thread during this pass.
    pub threaded: usize,
}

/// Enumerates `mailbox`, newest `UID` first, writing headers as they land.
///
/// `on_progress` is called once per committed batch. Cancelling `cancel`
/// between batches stops the pass without losing anything already
/// committed — the next call resumes exactly where this one stopped.
///
/// Call this once the caller has established that `mailbox` needs a full
/// enumeration (see [`SyncState::plan`](postio_model::SyncState::plan)) and,
/// if its `UIDVALIDITY` just changed, after the caller has wiped its stale
/// rows. This function does not check either.
pub async fn sync_mailbox(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Report> {
    sync_mailbox_with_batch_size(
        connection,
        backend,
        mailbox,
        DEFAULT_BATCH_SIZE,
        cancel,
        on_progress,
    )
    .await
}

/// [`sync_mailbox`] with an explicit batch size.
///
/// Exists mostly so a test can force several batches over a handful of
/// messages rather than needing hundreds of fixtures to see more than one.
/// `batch_size` is clamped to at least one.
pub async fn sync_mailbox_with_batch_size(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    batch_size: usize,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Report> {
    enumerate(
        connection,
        backend,
        mailbox,
        batch_size,
        Coverage::Missing,
        cancel,
        on_progress,
    )
    .await
}

/// Which UIDs an enumeration asks the server about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    /// Only the UIDs the mailbox does not already hold.
    ///
    /// What filling an empty mailbox needs, and what makes a sync that was
    /// interrupted halfway cheap to resume.
    Missing,
    /// Every UID in the mailbox, the ones already stored included.
    ///
    /// For a pass that has to *refresh* rather than fill: when an incremental
    /// pull could not be trusted, what it lost was most likely a flag on a
    /// message that is already here, and skipping those would leave the very
    /// thing that went missing missing.
    Everything,
}

/// The body of an enumeration pass. See [`sync_mailbox_with_batch_size`].
pub(crate) async fn enumerate(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    batch_size: usize,
    coverage: Coverage,
    cancel: &CancelToken,
    mut on_progress: impl FnMut(Progress),
) -> Result<Report> {
    let batch_size = batch_size.max(1);
    let selected = backend.select(&mailbox.path, SelectMode::ReadWrite).await?;

    let mut server_status =
        MailboxStatus::new(selected.uid_validity).with_uid_next(selected.uid_next);
    if let Some(mod_seq) = selected.highest_mod_seq {
        server_status = server_status.with_highest_mod_seq(mod_seq);
    }

    let now = Utc::now();
    SyncStateRepository::new(connection).observe(mailbox.id, &server_status, now)?;

    // The UID ceiling: the highest UID this pass could reach, and the range
    // it enumerates. Not what progress is reported against — see
    // `Progress::target`.
    let highest_uid = selected.uid_next.get().saturating_sub(1);
    let mut report = Report::default();

    if highest_uid < 1 {
        SyncStateRepository::new(connection).complete_full_sync(mailbox.id, now)?;
        return Ok(report);
    }

    let known: BTreeSet<u32> = MessageRepository::new(connection)
        .uids_in(mailbox.id, selected.uid_validity)?
        .into_iter()
        .map(Uid::get)
        .collect();

    // Fetched once per pass, not per message: who a contact-sighting is
    // recorded against never changes mid-pass. `None` (an orphaned mailbox
    // row) just means no sightings are recorded, rather than failing sync
    // over a nicety.
    let account = AccountRepository::new(connection).get(mailbox.account_id)?;

    let mut missing: Vec<u32> = (1..=highest_uid)
        .filter(|uid| coverage == Coverage::Everything || !known.contains(uid))
        .collect();
    // Descending: the newest UID in the mailbox is fetched, threaded and
    // visible before the oldest one is even asked for.
    missing.sort_unstable_by_key(|&uid| std::cmp::Reverse(uid));

    let mut fetched_so_far = match coverage {
        Coverage::Missing => known.len() as u32,
        Coverage::Everything => 0,
    };

    for chunk in missing.chunks(batch_size) {
        if cancel.is_cancelled() {
            return Err(SyncError::Backend(BackendError::Cancelled));
        }

        let uids: UidSet = chunk.iter().map(|&uid| Uid::new(uid)).collect();
        let asked_at = std::time::Instant::now();
        let mut fetched = backend
            .fetch_headers(&mailbox.path, &uids, None, cancel)
            .await?;
        let fetch_took = asked_at.elapsed();
        fetched.sort_unstable_by_key(|message| std::cmp::Reverse(message.uid));

        let mut messages: Vec<Message> = fetched
            .into_iter()
            .map(|message| message.into_message(mailbox.account_id, mailbox.id))
            .collect();
        if messages.is_empty() {
            continue;
        }

        let wrote_from = std::time::Instant::now();
        // One transaction for the whole batch, which is what this module's own
        // documentation has always claimed it does — "committing and threading
        // each batch" — and did not.
        //
        // Every repository call below opens a savepoint of its own and
        // releases it, and a release is a real commit when nothing encloses
        // it. Nothing did: a batch of two hundred messages committed once for
        // the upserts and then once *per message* for threading and once more
        // per message for the contact sighting, so it paid four hundred-odd
        // fsyncs where it needed one. Measured on a real account, that was
        // 2.25 ms of local write per message — the same order as the network
        // transfer it was supposedly waiting on (`postio-0d9.7`).
        //
        // Enclosing them turns those savepoints into nested ones, which cost
        // nothing to release, and leaves exactly one commit per batch. The
        // durability story is unchanged: a batch was already the unit an
        // interrupted pass resumed from.
        //
        // IMMEDIATE, not DEFERRED, and this is the load-bearing part (#79).
        // The first statement below is a SELECT, so a deferred transaction
        // would be holding a *read* lock by the time it wrote and would have
        // to promote — and SQLite refuses to make a promotion wait, because
        // blocking a connection that already holds a read lock could deadlock
        // against the writer it is waiting for. It returns SQLITE_BUSY without
        // invoking the busy handler at all, so `busy_timeout` never gets a
        // say. The other writer is not another sync pass (the engine is
        // single-threaded and nothing awaits between BEGIN and COMMIT) but the
        // UI thread, which writes local-first on every flag, archive and draft
        // autosave through this same pool. Taking the write lock up front is
        // what puts this back inside the five-second timeout.
        let batch = Transaction::new_unchecked(connection, TransactionBehavior::Immediate)
            .map_err(postio_storage::Error::from)?;
        let connection: &Connection = &batch;

        let upsert = MessageRepository::new(connection).upsert_batch(&mut messages)?;
        report.inserted += upsert.inserted;
        report.updated += upsert.updated;

        let threading = ThreadingRepository::new(connection, mailbox.account_id);
        for message in &messages {
            threading.thread(message)?;
            report.threaded += 1;
        }

        // Only messages that were not already known before this pass: a
        // `Coverage::Everything` re-enumeration re-fetches messages already
        // stored (that is its whole point, refreshing what an untrustworthy
        // incremental pull may have missed), and recording those again would
        // count the same correspondent twice for one message.
        if let Some(account) = &account {
            for message in &messages {
                let is_new = message
                    .server
                    .uid
                    .is_some_and(|uid| !known.contains(&uid.get()));
                if is_new {
                    crate::contacts::record(connection, account, message)?;
                }
            }
        }

        batch.commit().map_err(postio_storage::Error::from)?;

        // Where a first sync's wall clock actually goes, per batch: waiting on
        // the server, or writing to SQLite. `postio-0d9.7` asks for several
        // different optimisations — more connections, pipelined FETCH, bigger
        // batches, bigger transactions — and which of them is worth anything
        // depends entirely on this ratio. Counts and durations only; a log
        // never carries mail.
        tracing::debug!(
            messages = messages.len(),
            fetch_ms = fetch_took.as_millis() as u64,
            write_ms = wrote_from.elapsed().as_millis() as u64,
            "sync batch committed"
        );

        fetched_so_far += messages.len() as u32;
        on_progress(Progress {
            mailbox_id: mailbox.id,
            fetched: fetched_so_far,
            target: selected.exists,
        });
    }

    SyncStateRepository::new(connection).complete_full_sync(mailbox.id, now)?;
    Ok(report)
}
