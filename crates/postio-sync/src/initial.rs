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
use postio_storage::repository::{MessageRepository, SyncStateRepository, ThreadingRepository};
use rusqlite::Connection;

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
    /// The server's `UIDNEXT` minus one: the highest UID this pass could ever
    /// reach. Some UIDs in range may not exist — expunged messages leave
    /// gaps — so this is an upper bound on the total, not a promise of it.
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

    let target = selected.uid_next.get().saturating_sub(1);
    let mut report = Report::default();

    if target < 1 {
        SyncStateRepository::new(connection).complete_full_sync(mailbox.id, now)?;
        return Ok(report);
    }

    let known: BTreeSet<u32> = MessageRepository::new(connection)
        .uids_in(mailbox.id, selected.uid_validity)?
        .into_iter()
        .map(Uid::get)
        .collect();

    let mut missing: Vec<u32> = (1..=target).filter(|uid| !known.contains(uid)).collect();
    // Descending: the newest UID in the mailbox is fetched, threaded and
    // visible before the oldest one is even asked for.
    missing.sort_unstable_by_key(|&uid| std::cmp::Reverse(uid));

    let mut fetched_so_far = known.len() as u32;

    for chunk in missing.chunks(batch_size) {
        if cancel.is_cancelled() {
            return Err(SyncError::Backend(BackendError::Cancelled));
        }

        let uids: UidSet = chunk.iter().map(|&uid| Uid::new(uid)).collect();
        let mut fetched = backend
            .fetch_headers(&mailbox.path, &uids, None, cancel)
            .await?;
        fetched.sort_unstable_by_key(|message| std::cmp::Reverse(message.uid));

        let mut messages: Vec<Message> = fetched
            .into_iter()
            .map(|message| message.into_message(mailbox.account_id, mailbox.id))
            .collect();
        if messages.is_empty() {
            continue;
        }

        let upsert = MessageRepository::new(connection).upsert_batch(&mut messages)?;
        report.inserted += upsert.inserted;
        report.updated += upsert.updated;

        let threading = ThreadingRepository::new(connection, mailbox.account_id);
        for message in &messages {
            threading.thread(message)?;
            report.threaded += 1;
        }

        fetched_so_far += messages.len() as u32;
        on_progress(Progress {
            mailbox_id: mailbox.id,
            fetched: fetched_so_far,
            target,
        });
    }

    SyncStateRepository::new(connection).complete_full_sync(mailbox.id, now)?;
    Ok(report)
}
