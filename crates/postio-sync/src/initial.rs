//! Initial sync: enumerating a mailbox for the first time, newest mail first.
//!
//! # Why newest first
//!
//! A mailbox with years of history can hold tens of thousands of messages, and
//! nobody opens Postio to read the oldest one. CLAUDE.md's performance budget
//! makes this structural rather than a nicety: the first screenful has to be
//! visible in seconds, which means the *order* messages are fetched in is the
//! whole of the perceived-speed story. So this module takes the mailbox's UIDs
//! newest first, in batches, committing and threading each batch before asking
//! for the next.
//!
//! Which UIDs those are comes from the server when it will say
//! ([`MailBackend::existing_uids`]), and otherwise from walking
//! `1..=`[`uid_next`](postio_account::backend::MailboxStatus::uid_next)`-1` as
//! this always did. The difference matters more than it looks: a long-lived
//! folder's UID space is mostly gaps, and walking it costs a round trip per
//! chunk of *UIDs* rather than per chunk of mail (#727).
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
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use chrono::Utc;
use postio_account::backend::{
    BackendError, BackendResult, FetchedMessage, MailBackend, SelectMode, UidSet,
};
use postio_model::{Account, Mailbox, MailboxId, MailboxStatus, Message, Uid};
use postio_storage::repository::{
    AccountRepository, MessageRepository, SyncStateRepository, ThreadingRepository,
};
use postio_storage::{Checkout, WritePriority};

use crate::drain::SyncError;
use postio_account::cancel::CancelToken;

/// This module's result type.
pub type Result<T> = std::result::Result<T, SyncError>;

/// How many **messages** one `FETCH` asks for at a time.
///
/// Small enough that the first batch — and therefore the first screenful —
/// commits and is visible in well under a second; large enough that a
/// ten-thousand-message inbox does not need ten thousand round trips.
///
/// Messages, not UIDs, and the distinction used to be invisible because the
/// two were the same thing: the pass chunked the UID *space*, so this was
/// "UIDs per round trip" and a folder whose UIDs were mostly expunged gaps
/// paid for every empty chunk. Since #727 the pass asks the server which UIDs
/// exist ([`MailBackend::existing_uids`]) and chunks that, so this number now
/// governs only how much mail arrives per round trip — which is what its
/// value was chosen for, and what #78 measured 200 as still being right for.
pub const DEFAULT_BATCH_SIZE: usize = 200;

/// How many messages one background write transaction covers.
///
/// A *batch* is what the network is asked for; a **write unit** is what
/// SQLite's write lock is held for. They used to be the same thing, and #425
/// is why they are not any more.
///
/// The write gate ([`postio_storage::WriteGate`]) guarantees a person's write
/// never waits for more than the background unit already in progress. That
/// guarantee is only worth what the unit costs, so the unit has to be small:
/// a batch of two hundred held the lock for 90–200 ms, measured, which is an
/// archive keystroke visibly lagging its keypress. Twenty-five holds it for
/// 8–9 ms, inside CLAUDE.md's 16 ms interaction budget with room for a slower
/// disk.
///
/// Smaller still would buy nothing: the cost of subdividing is one commit per
/// unit, measured at 0.2–0.4 ms against 8 ms of work, and that ratio gets
/// worse as the unit shrinks while the latency it buys is already under
/// budget. Twenty-five is where those two curves cross for this schema.
///
/// This does not change how much a pass fetches, how it batches its `FETCH`es,
/// or where an interrupted pass resumes — `uids_in` counts what committed, so
/// a finer unit resumes at a finer grain.
///
/// **Since #1587 this is the ceiling, not the size.** Twenty-five was chosen
/// when a unit cost 8–9 ms; on the Turso store a row measured ~118 ms to
/// write, which put a twenty-five-row unit at ~3 s — the guarantee's
/// mechanism intact and its number inflated ~350x, so every mark-read,
/// archive and draft autosave waited seconds behind the backfill. The unit
/// is sized from what the last one actually cost now ([`unit_rows`]), and
/// this constant is the most it may grow back to when rows are cheap again.
pub(crate) const WRITE_UNIT: usize = 25;

/// What one write unit is allowed to cost, wall-clock.
///
/// The number the old constant was implicitly built on: twenty-five rows at
/// 8–9 ms, "inside CLAUDE.md's 16 ms interaction budget with room for a
/// slower disk". Making it explicit is what lets the unit keep meaning that
/// when the per-row cost moves under it.
const UNIT_BUDGET: std::time::Duration = std::time::Duration::from_millis(9);

/// How many rows the next write unit should cover.
///
/// Learned from the last unit's measured cost, machine-wide: the cost is a
/// property of this store on this disk, not of any one pass, and fifteen
/// folders each rediscovering it with one three-second unit apiece would be
/// fifteen stalls. Starts at [`WRITE_UNIT`] and can only shrink below it —
/// growing past it would loosen the #425 guarantee this exists to keep.
pub(crate) fn unit_rows() -> usize {
    LEARNED_UNIT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Teach the sizing what the unit just written actually cost.
///
/// `held` is the time the write lock was held — after the gate permit,
/// through the commit. Never the wait for the permit: queueing time says how
/// busy the gate is, not how expensive a row is, and feeding it back would
/// make a busy moment shrink the unit for no reason.
pub(crate) fn unit_wrote(rows: usize, held: std::time::Duration) {
    LEARNED_UNIT.store(
        next_unit_rows(rows, held),
        std::sync::atomic::Ordering::Relaxed,
    );
}

static LEARNED_UNIT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(WRITE_UNIT);

/// The sizing rule, pure so it can be pinned.
///
/// Rows the budget affords at the measured per-row cost, floored at one —
/// a unit is at least a row, so on a store where one row costs more than
/// the whole budget the bound honestly becomes "one row's cost" — and
/// ceilinged at [`WRITE_UNIT`], because past the measured sweet spot more
/// rows buy nothing and cost latency.
fn next_unit_rows(rows: usize, held: std::time::Duration) -> usize {
    let per_row = held.as_secs_f64() / rows.max(1) as f64;
    if per_row <= f64::EPSILON {
        return WRITE_UNIT;
    }
    ((UNIT_BUDGET.as_secs_f64() / per_row) as usize).clamp(1, WRITE_UNIT)
}

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
    /// whole of a pass (`postio-qhz.9`). The enumeration no longer walks that
    /// ceiling either — since #727 it asks the server which UIDs exist — but
    /// this stays `EXISTS` regardless: the fraction a person reads should
    /// count mail, not UID space, whichever the pass happens to enumerate.
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
    connection: &Checkout,
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
    connection: &Checkout,
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
    connection: &Checkout,
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
        MailboxStatus::new(selected.generation).with_uid_next(selected.uid_next);
    if let Some(mod_seq) = selected.highest_mod_seq {
        server_status = server_status.with_highest_mod_seq(mod_seq);
    }

    let now = Utc::now();
    // Whether this pass is the folder's first to finish: asked before
    // `observe` stamps anything, and read again below, where only a first
    // sync repairs the conversations its own filing order split.
    let first_sync = !SyncStateRepository::new(connection)
        .get(mailbox.id)
        .await?
        .is_some_and(|state| state.has_synced());
    SyncStateRepository::new(connection)
        .observe(mailbox.id, &server_status, now)
        .await?;

    // The UID ceiling: the highest UID this pass could reach, and the range
    // it enumerates. Not what progress is reported against — see
    // `Progress::target`.
    let highest_uid = selected.uid_next.get().saturating_sub(1);
    let mut report = Report::default();

    if highest_uid < 1 {
        SyncStateRepository::new(connection)
            .complete_full_sync(mailbox.id, now)
            .await?;
        return Ok(report);
    }

    let known: BTreeSet<u32> = MessageRepository::new(connection)
        .uids_in(mailbox.id, selected.generation)
        .await?
        .into_iter()
        .map(Uid::get)
        .collect();

    // Fetched once per pass, not per message: who a contact-sighting is
    // recorded against never changes mid-pass. `None` (an orphaned mailbox
    // row) just means no sightings are recorded, rather than failing sync
    // over a nicety.
    let account = AccountRepository::new(connection)
        .get(mailbox.account_id)
        .await?;

    // What the server actually holds, when it will say — otherwise every UID
    // below the ceiling, which is what this did for every backend before
    // `existing_uids` existed.
    //
    // The difference is the whole of #727. A long-lived folder's UID space is
    // mostly gaps: everything expunged over the years still counts toward
    // `UIDNEXT`, so walking `1..=highest_uid` costs a round trip per
    // `batch_size` *UIDs* whether or not any message lives among them. An
    // empty chunk fetches nothing, commits nothing and logs nothing, so the
    // cost is also invisible — #78 measured a real first sync spending 46% of
    // its wall clock in exactly that silence, and `Progress::target`'s note
    // records the same sparseness from the other side (an inbox of ninety-two
    // messages whose UID ceiling was 63,022).
    let present = existing_uids(backend, mailbox, cancel).await?;

    // A listing shorter than `EXISTS` is not believed.
    //
    // `UID SEARCH ALL` is the optimisation above; `EXISTS` is what the
    // `SELECT` that opened this mailbox already reported, so the cross-check
    // is free. A server that names fewer UIDs than it just said it holds has
    // under-reported, and believing it is the expensive mistake: the pass
    // enumerates the few it was told about, *completes*, and
    // `complete_full_sync` stamps `last_full_sync_at` — after which
    // `SyncState::plan` sees `has_synced()` and every later pass is
    // incremental against a mailbox that was never enumerated. The backlog is
    // then not slow, it is unreachable, and no restart recovers it.
    //
    // Discarding the listing costs a slower pass and nothing else: `None` is
    // the path a refused `SEARCH` already takes, walking the UID space as
    // every backend did before #727.
    //
    // Only the short direction. A listing *longer* than `EXISTS` is the
    // ordinary race — mail delivered between the `SELECT` and the `SEARCH` —
    // and the ceiling filter below already handles it.
    let present = match present {
        Some(uids) if (uids.len() as u32) < selected.exists => {
            tracing::warn!(
                mailbox = mailbox.id.get(),
                listed = uids.len(),
                exists = selected.exists,
                "the server listed fewer UIDs than it says the mailbox holds; \
                 walking the UID space instead"
            );
            None
        }
        other => other,
    };

    let mut missing: Vec<u32> = match present {
        Some(uids) => uids
            .into_iter()
            .map(Uid::get)
            // A server is entitled to name a UID at or above the ceiling it
            // just reported — it may have accepted a delivery between the
            // SELECT and this call. Such a message is not this pass's job:
            // the pass is resumable and the next one sees a higher ceiling.
            .filter(|uid| *uid <= highest_uid)
            .filter(|uid| coverage == Coverage::Everything || !known.contains(uid))
            .collect(),
        None => (1..=highest_uid)
            .filter(|uid| coverage == Coverage::Everything || !known.contains(uid))
            .collect(),
    };
    // Descending: the newest UID in the mailbox is fetched, threaded and
    // visible before the oldest one is even asked for.
    missing.sort_unstable_by_key(|&uid| std::cmp::Reverse(uid));

    let mut fetched_so_far = match coverage {
        Coverage::Missing => known.len() as u32,
        Coverage::Everything => 0,
    };

    // Every batch's UID set up front. A fetch that is still outstanding while
    // the previous batch is being written has to borrow its set from
    // something that outlives the iteration that started it.
    let batches: Vec<UidSet> = missing
        .chunks(batch_size)
        .map(|chunk| chunk.iter().map(|&uid| Uid::new(uid)).collect())
        .collect();

    // The batch that has been asked for but not yet folded in: either still
    // on the wire, or already answered while it was being primed.
    let mut ahead: Option<ReadAhead<'_>> = None;

    for index in 0..batches.len() {
        if cancel.is_cancelled() {
            return Err(SyncError::Backend(BackendError::Cancelled));
        }

        let asked_at = std::time::Instant::now();
        let mut fetched = match ahead.take() {
            Some(ReadAhead::Answered(answer)) => answer?,
            Some(ReadAhead::OnTheWire(fetching)) => fetching.await?,
            None => {
                backend
                    .fetch_headers(&mailbox.path, &batches[index], None, cancel)
                    .await?
            }
        };
        let fetch_took = asked_at.elapsed();

        // Ask for the next batch *now*, before taking SQLite's write lock:
        // one poll is what puts the FETCH on the wire, and the server then
        // works on it for the whole of the local write instead of waiting for
        // it (#77). At most one fetch is ever outstanding, so a pass still
        // wants exactly one pooled connection.
        //
        // Nothing from a read-ahead is committed until the iteration that
        // consumes it, so cancelling simply drops it — an interrupted pass
        // resumes from `uids_in` exactly as it always has.
        if index + 1 < batches.len() && !cancel.is_cancelled() {
            ahead = Some(read_ahead(backend, &mailbox.path, &batches[index + 1], cancel).await);
        }
        fetched.sort_unstable_by_key(|message| std::cmp::Reverse(message.uid));

        let mut messages: Vec<Message> = fetched
            .into_iter()
            .map(|message| message.into_message(mailbox.account_id, mailbox.id))
            .collect();
        if messages.is_empty() {
            continue;
        }

        let wrote_from = std::time::Instant::now();
        let batch =
            commit_batch(connection, mailbox, account.as_ref(), &known, &mut messages).await?;
        report.inserted += batch.inserted;
        report.updated += batch.updated;
        report.threaded += batch.threaded;

        // One real yield per batch, and it is load-bearing. The read-ahead
        // above primes the next fetch *before* the commit, so by the time the
        // commit finishes the fetch's timer may already have elapsed — its
        // await then never returns `Pending`, and a pass whose commits run
        // longer than its fetches walks every batch of the folder inside a
        // single poll. The engine's whole interruption story assumes a pass
        // yields: `sync_wave`'s cancel arm and refill both wait their turn at
        // a `select!`, and a pass that never yields deafens the wave to the
        // user for the length of the folder — which is what
        // `docs/notes/2026-09-13-a-slow-pass-stops-every-folder-behind-it.md`
        // measured live, with the engine's fts merges as the slow commits.
        yield_once().await;

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

    SyncStateRepository::new(connection)
        .complete_full_sync(mailbox.id, now)
        .await?;
    if first_sync {
        rethread_orphans(connection, mailbox).await;
    }
    Ok(report)
}

/// Joins the replies a first sync filed before the messages they answer.
///
/// A first sync files newest first, so a reply routinely arrives before its
/// original. One that names its parent is claimed and waits for it; one
/// with no `In-Reply-To` and no `References` can only be placed by its
/// subject, and when it was filed there was nothing yet to place it with.
/// Once the whole folder is local, every original such a reply could join
/// is here. After the first sync only: later passes fetch what is new, and
/// what is new arrives after what it answers.
///
/// Background, like every write a pass makes, and never fatal: a folder
/// whose repair failed is a folder with a split conversation, which is
/// what it was before this existed.
async fn rethread_orphans(connection: &Checkout, mailbox: &Mailbox) {
    let _permit = connection
        .write_gate()
        .acquire(WritePriority::Background)
        .await;
    match postio_storage::repository::ThreadingRepository::new(connection, mailbox.account_id)
        .rethread_orphans(mailbox.id)
        .await
    {
        Ok(0) => {}
        Ok(moved) => tracing::debug!(mailbox = mailbox.id.get(), moved, "rethreaded replies"),
        Err(error) => {
            tracing::warn!(mailbox = mailbox.id.get(), %error, "could not rethread replies");
        }
    }
}

/// Asks the backend which UIDs exist, and treats a refusal as "it will not
/// say" rather than as a failed pass.
///
/// Three ways to get nothing back, and all three mean the same thing to the
/// caller — walk the UID space instead:
///
/// * the backend does not implement it (the trait default), which is every
///   backend but IMAP today;
/// * the server has no `SEARCH`, or refused this one;
/// * the call failed outright.
///
/// The last is the interesting one. A first sync that cannot happen because
/// an *optimisation* failed would be a worse bug than the slowness this
/// exists to fix, and nothing is masked for long: a transport that is really
/// broken fails again on the very next `FETCH`, which is not optional and
/// does propagate. Cancellation is the one thing that must not be swallowed —
/// a cancelled pass has to stop, not quietly fall back to the slow path it
/// was cancelled out of.
async fn existing_uids(
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    cancel: &CancelToken,
) -> Result<Option<Vec<Uid>>> {
    match backend.existing_uids(&mailbox.path, cancel).await {
        Ok(uids) => Ok(uids),
        Err(BackendError::Cancelled) => Err(SyncError::Backend(BackendError::Cancelled)),
        Err(error) => {
            tracing::debug!(
                mailbox = mailbox.id.get(),
                %error,
                "the server would not list its UIDs; walking the UID space"
            );
            Ok(None)
        }
    }
}

/// Writes one batch of headers to the store, the way a sync pass does.
///
/// Upsert, thread, record correspondents — one transaction per
/// [`WRITE_UNIT`], with the write gate re-taken per unit. `known` is the set
/// of UIDs the mailbox already held when the pass started, which decides
/// which messages count as newly seen; `account` is the account
/// correspondents are recorded against, and `None` (an orphaned mailbox row)
/// simply records none.
///
/// `messages` is updated in place with the ids the upsert assigned, so the
/// caller can go on using them.
///
/// # Why this is public
///
/// So that the write path a first sync runs can be *measured* rather than
/// re-implemented. #78 established that a first sync is write-bound — a 1:12
/// fetch-to-write ratio against a real account — which makes per-message
/// write cost the number that decides how long one takes, and #726 is the
/// bench that watches it. A bench that assembled its own upsert-thread-record
/// sequence would measure a copy that drifts away from this one silently, and
/// a budget over a copy guards nothing.
///
/// # One transaction per write unit
///
/// A slice of the batch, not the whole of it. See [`WRITE_UNIT`] for the
/// second half of #425's fix, and the gate below for the first.
///
/// Every repository call below opens a savepoint of its own and releases it,
/// and a release is a real commit when nothing encloses it. Nothing did: a
/// batch of two hundred messages committed once for the upserts and then once
/// *per message* for threading and once more per message for the contact
/// sighting, so it paid four hundred-odd fsyncs where it needed one. Measured
/// on a real account, that was 2.25 ms of local write per message — the same
/// order as the network transfer it was supposedly waiting on
/// (`postio-0d9.7`).
///
/// Enclosing them turns those savepoints into nested ones, which cost nothing
/// to release, and leaves one commit per write unit. The durability story is
/// unchanged: an interrupted pass resumes from `uids_in`, which counts
/// whatever actually committed, so a unit smaller than a batch resumes at a
/// finer grain rather than a worse one.
///
/// # Why IMMEDIATE
///
/// Not DEFERRED, and this is the load-bearing part (#79). The first statement
/// below is a SELECT, so a deferred transaction would be holding a *read*
/// lock by the time it wrote and would have to promote — and SQLite refuses
/// to make a promotion wait, because blocking a connection that already holds
/// a read lock could deadlock against the writer it is waiting for. It
/// returns SQLITE_BUSY without invoking the busy handler at all, so
/// `busy_timeout` never gets a say. The other writer is not another sync pass
/// (the engine is single-threaded and nothing awaits between BEGIN and
/// COMMIT) but the UI thread, which writes local-first on every flag, archive
/// and draft autosave through this same pool. Taking the write lock up front
/// is what puts this back inside the five-second timeout.
pub async fn commit_batch(
    connection: &Checkout,
    mailbox: &Mailbox,
    account: Option<&Account>,
    known: &BTreeSet<u32>,
    messages: &mut [Message],
) -> Result<Report> {
    let mut report = Report::default();

    let mut rest: &mut [Message] = messages;
    while !rest.is_empty() {
        // Sized from what the last unit cost, not from a constant: #425's
        // guarantee is "a person waits one unit at most", and a unit is only
        // small if it is small *in time* on this store, today (#1587).
        let take = unit_rows().min(rest.len());
        let (slice, tail) = rest.split_at_mut(take);
        rest = tail;
        // The permit, the transaction and the sizing clock, and another try
        // when the engine says busy: see [`write_unit`].
        let account_id = mailbox.account_id;
        let known_uids = &known;
        let ((upsert, written), held) = write_unit(connection, || {
            // `BEGIN IMMEDIATE`, which is what `transaction` opens at the
            // outermost level, and for the reason #79 records: the first
            // statement inside is a read, and a deferred transaction that then
            // has to promote its read lock to a write lock is refused outright
            // rather than waiting.
            let source: Vec<Message> = slice.to_vec();
            postio_storage::transaction(connection, move |connection| async move {
                let mut written = source;
                let upsert = MessageRepository::new(&connection)
                    .upsert_batch(&mut written)
                    .await?;

                let threading = ThreadingRepository::new(&connection, account_id);
                for message in &written {
                    threading.thread(message).await?;
                }

                // Only messages that were not already known before this pass:
                // a `Coverage::Everything` re-enumeration re-fetches messages
                // already stored (that is its whole point, refreshing what an
                // untrustworthy incremental pull may have missed), and
                // recording those again would count the same correspondent
                // twice for one message.
                if let Some(account) = account {
                    for message in &written {
                        let is_new = message
                            .server
                            .uid
                            .is_some_and(|uid| !known_uids.contains(&uid.get()));
                        if is_new {
                            crate::contacts::record(&connection, account, message).await?;
                        }
                    }
                }

                Ok::<_, SyncError>((upsert, written))
            })
        })
        .await?;

        report.inserted += upsert.inserted;
        report.updated += upsert.updated;
        report.threaded += written.len();

        // Teach the sizing what this unit cost while the numbers are in
        // hand — the next unit, whichever pass writes it, starts right.
        unit_wrote(slice.len(), held);
        // The ids `upsert_batch` assigned belong to the caller's messages, not
        // to this unit's copy of them.
        copy_back(slice, &written);
        // And a yield with the permit down, once per unit. The gate is
        // first-come: released and re-taken in the same poll, it never
        // changes hands, and the one yield per *batch* above was the only
        // point at which a body being fetched beside this pass could write
        // (#631) -- twenty inbox bodies took as long as two thousand archive
        // headers. A yield here lets a waiter that the release woke take its
        // turn between units; the pass is back on the queue behind it.
        yield_once().await;
    }

    Ok(report)
}

/// Carry the ids `upsert_batch` assigned back onto the caller's messages.
///
/// # Why not `clone_from_slice`
///
/// It was, and it panicked against a real account:
///
/// ```text
/// destination and source slices have different lengths
///   postio_sync::initial::commit_batch
///   postio_sync::resync::enumerate_the_shortfall
/// ```
///
/// `upsert_batch` does not always return what it was given. It `retain`s
/// away two kinds of row on purpose --- the server's copy of a draft Postio
/// itself wrote, and a message the user has already moved out of this mailbox
/// while the server has not been told --- so `written` is shorter whenever
/// either applies. Drafts is where it showed up, re-enumerating 27 local rows
/// against the 28 the server reported.
///
/// That has always been true; nothing reached it until a mailbox short of
/// `EXISTS` started re-enumerating itself. A positional copy was wrong the
/// whole time and merely unreachable, which is the worse kind of wrong.
///
/// # The shape that makes this cheap
///
/// `retain` preserves order, so `written` is a *subsequence* of `slice`: same
/// messages, same order, some missing. One walk down both, matching on the
/// remote id --- the same key both `retain`s decide on, and `None` is never
/// dropped by either, so those align by position within the run.
///
/// A message that was dropped keeps whatever it arrived with, which is right:
/// nothing was stored for it, so there is no id to carry back.
fn copy_back(slice: &mut [Message], written: &[Message]) {
    let mut stored = written.iter();
    let mut next = stored.next();
    for slot in slice.iter_mut() {
        let Some(candidate) = next else {
            return;
        };
        if candidate.server.remote_id == slot.server.remote_id {
            *slot = candidate.clone();
            next = stored.next();
        }
    }
}

/// A batch asked for ahead of time. See the read-ahead in [`enumerate`].
enum ReadAhead<'a> {
    /// The `FETCH` is out and the answer has not arrived yet.
    OnTheWire(Pin<Box<dyn Future<Output = BackendResult<Vec<FetchedMessage>>> + Send + 'a>>),
    /// The backend answered while the request was being primed — a mock, a
    /// cache, or a server that was simply quick.
    Answered(BackendResult<Vec<FetchedMessage>>),
}

/// The most times one write unit is tried when the engine reports the
/// database busy, before the pass gives up (#1594).
const BUSY_RETRIES: usize = 3;

/// Run one write unit under the gate, and again if the engine says busy.
///
/// The gate orders Postio's own writers; the engine's `busy_timeout` covers
/// whatever it does not. A unit that outlived that timeout -- two lanes, the
/// body writer and the indexer contending, at the row costs #1587 measured
/// -- came back as `database is locked`, and the `?` on it ended the pass: a
/// small folder's pass died eight seconds in for exactly that (#1594), and a
/// failed pass was not requeued the way a cancelled one is. So a busy unit is
/// tried again, [`BUSY_RETRIES`] times. The permit is *released* between
/// tries, so a waiter the failure woke takes its turn rather than watching
/// this pass hold the gate while it waits on a lock the gate does not order.
/// No sleep of its own: the wait is the engine's `busy_timeout`, so a try
/// costs nothing when the lock frees at once, and this crate has no executor
/// to sleep on -- one real yield, as every unit already takes.
///
/// Returns what the unit produced and how long the lock was held for it,
/// timed from the permit rather than from the call: queueing is not holding,
/// and the sizing in [`unit_wrote`] must not learn from a queue.
pub(crate) async fn write_unit<T, Fut>(
    connection: &Checkout,
    unit: impl Fn() -> Fut,
) -> Result<(T, std::time::Duration)>
where
    Fut: Future<Output = Result<T>>,
{
    let mut tries = 0;
    loop {
        // Ahead of `BEGIN IMMEDIATE`, never after: the permit is what stands
        // this aside for a keystroke's write, and standing aside after taking
        // the engine's lock would be standing aside too late. Taken per unit
        // rather than held across a batch, so a person waits for one unit at
        // most (#425).
        let permit = connection
            .write_gate()
            .acquire(WritePriority::Background)
            .await;
        let held_from = std::time::Instant::now();
        match unit().await {
            Ok(value) => {
                let held = held_from.elapsed();
                drop(permit);
                return Ok((value, held));
            }
            Err(SyncError::Storage(error)) if error.is_busy() && tries < BUSY_RETRIES => {
                drop(permit);
                tries += 1;
                // A count, which is all a log may carry about a write.
                tracing::debug!(
                    tries,
                    "the database was busy under a write unit; trying it again"
                );
                yield_once().await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Start a fetch and poll it once, so the request reaches the server before
/// the caller goes off to do something blocking.
///
/// The single poll is the whole mechanism: a future does nothing until it is
/// polled, so a fetch merely *created* before a write would still be sitting
/// unsent when the write finished. Polling with the caller's own waker —
/// rather than a throwaway one — means the later `await` picks it up exactly
/// as if it had been awaited all along.
/// Return `Pending` exactly once, waking immediately.
///
/// What `tokio::task::yield_now` is, without naming an executor — this crate
/// runs under whichever runtime the caller picked. The single `Pending` is
/// the entire point: it hands the enclosing `select!` one poll, which is the
/// turn the engine's cancel arm and lane refill take theirs on.
pub(crate) async fn yield_once() {
    let mut yielded = false;
    std::future::poll_fn(move |context| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            context.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

async fn read_ahead<'a>(
    backend: &'a dyn MailBackend,
    mailbox: &'a str,
    uids: &'a UidSet,
    cancel: &'a CancelToken,
) -> ReadAhead<'a> {
    let mut fetching = backend.fetch_headers(mailbox, uids, None, cancel);
    let primed = std::future::poll_fn(|context| Poll::Ready(fetching.as_mut().poll(context))).await;
    match primed {
        Poll::Ready(answer) => ReadAhead::Answered(answer),
        Poll::Pending => ReadAhead::OnTheWire(fetching),
    }
}

#[cfg(test)]
mod unit_sizing_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_cheap_store_gets_the_full_unit() {
        // The measured sweet spot: twenty-five rows in 8 ms is the world the
        // constant was sized for, and the sizing must hand it straight back.
        assert_eq!(next_unit_rows(25, Duration::from_millis(8)), WRITE_UNIT);
    }

    #[test]
    fn a_slow_store_shrinks_the_unit_to_fit_the_budget() {
        // #1587's world: ~118 ms per row. Twenty-five rows held the lock for
        // ~3 s, which made #425's "one unit at most" guarantee mean three
        // seconds. The budget affords no whole row, so the floor holds.
        assert_eq!(next_unit_rows(25, Duration::from_millis(2950)), 1);
    }

    #[test]
    fn the_floor_is_one_row_because_a_unit_is_at_least_a_row() {
        // On a store where one row costs more than the whole budget, the
        // bound honestly becomes "one row's cost". There is nothing finer to
        // subdivide into.
        assert_eq!(next_unit_rows(1, Duration::from_millis(118)), 1);
    }

    #[test]
    fn recovery_is_immediate_once_rows_are_cheap_again() {
        // The shrink must not be a ratchet: the moment a unit measures cheap
        // -- the backfill ended, or #1587's root cause is fixed -- the next
        // unit is the full twenty-five again, not a slow climb.
        assert_eq!(next_unit_rows(1, Duration::from_micros(300)), WRITE_UNIT);
    }

    #[test]
    fn the_unit_never_grows_past_the_measured_sweet_spot() {
        // Past twenty-five, more rows buy nothing and cost latency -- the
        // constant's own doc records where the curves cross. An absurdly
        // fast measurement must not loosen the #425 guarantee.
        assert_eq!(next_unit_rows(25, Duration::from_nanos(1)), WRITE_UNIT);
        assert_eq!(next_unit_rows(25, Duration::ZERO), WRITE_UNIT);
    }

    #[test]
    fn a_middling_store_lands_in_between() {
        // 45 ms for twenty-five rows is 1.8 ms a row; a 9 ms budget affords
        // five of those.
        assert_eq!(next_unit_rows(25, Duration::from_millis(45)), 5);
    }
}

#[cfg(test)]
mod carrying_ids_back_onto_a_shortened_batch {
    use super::copy_back;
    use postio_model::{AccountId, MailboxId, Message, MessageId, RemoteId};

    fn message(remote: Option<&str>, id: i64) -> Message {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), chrono::Utc::now());
        message.server.remote_id = remote.map(|remote| RemoteId::new(remote.to_owned()));
        message.id = MessageId::new(id);
        message
    }

    /// The crash, as a batch.
    ///
    /// `upsert_batch` drops the server's copy of a draft Postio wrote, so what
    /// comes back is shorter than what went in and `clone_from_slice` panicked
    /// with "destination and source slices have different lengths". Drafts is
    /// where it happened: 27 rows held against the 28 the server reported, a
    /// re-enumeration fetched the 28th, and it was the user's own draft.
    #[test]
    fn a_dropped_row_does_not_panic_and_does_not_shift_the_others() {
        let mut slice = [
            message(Some("1:10"), 0),
            message(Some("1:11"), 0),
            message(Some("1:12"), 0),
        ];
        // The middle one was retained away; the others came back with ids.
        let written = [message(Some("1:10"), 101), message(Some("1:12"), 103)];

        copy_back(&mut slice, &written);

        assert_eq!(slice[0].id, MessageId::new(101), "the first got its id");
        assert_eq!(
            slice[1].id,
            MessageId::new(0),
            "the dropped one keeps what it arrived with -- nothing was stored \
             for it, so there is no id to carry back"
        );
        assert_eq!(
            slice[2].id,
            MessageId::new(103),
            "and the one after the gap got its own id, not the gap's neighbour's"
        );
    }

    /// The ordinary case still copies straight across.
    #[test]
    fn nothing_dropped_is_a_plain_copy() {
        let mut slice = [message(Some("1:10"), 0), message(Some("1:11"), 0)];
        let written = [message(Some("1:10"), 101), message(Some("1:11"), 102)];

        copy_back(&mut slice, &written);

        assert_eq!(slice[0].id, MessageId::new(101));
        assert_eq!(slice[1].id, MessageId::new(102));
    }

    /// A message with no remote id is never dropped by either `retain`, so
    /// runs of them align by position.
    #[test]
    fn rows_without_a_remote_id_still_line_up() {
        let mut slice = [message(None, 0), message(Some("1:11"), 0), message(None, 0)];
        let written = [message(None, 201), message(None, 203)];

        copy_back(&mut slice, &written);

        assert_eq!(slice[0].id, MessageId::new(201));
        assert_eq!(slice[1].id, MessageId::new(0), "the dropped one is left");
        assert_eq!(slice[2].id, MessageId::new(203));
    }
}

#[cfg(test)]
mod busy_retry_tests {
    use std::cell::Cell;

    use postio_account::backend::BackendError;
    use postio_storage::test_support;

    use super::*;

    #[tokio::test]
    async fn a_busy_unit_is_tried_again_and_succeeds() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let tries = Cell::new(0);
        let (value, _held) = write_unit(&connection, || {
            let n = tries.get() + 1;
            tries.set(n);
            async move {
                if n <= 2 {
                    Err(SyncError::Storage(test_support::busy()))
                } else {
                    Ok(n)
                }
            }
        })
        .await
        .expect("the third try succeeds");
        assert_eq!(value, 3);
    }

    #[tokio::test]
    async fn a_unit_that_stays_busy_fails_after_the_last_try() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let tries = Cell::new(0);
        let error = write_unit(&connection, || {
            tries.set(tries.get() + 1);
            async { Err::<(), _>(SyncError::Storage(test_support::busy())) }
        })
        .await
        .expect_err("a lock that never frees ends the unit");
        assert!(matches!(error, SyncError::Storage(error) if error.is_busy()));
        assert_eq!(tries.get(), BUSY_RETRIES + 1);
    }

    #[tokio::test]
    async fn any_other_failure_is_not_tried_again() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        let tries = Cell::new(0);
        let error = write_unit(&connection, || {
            tries.set(tries.get() + 1);
            async {
                Err::<(), _>(SyncError::Backend(BackendError::Protocol {
                    reason: "not busy".to_owned(),
                }))
            }
        })
        .await
        .expect_err("the failure is returned as it was");
        assert!(matches!(error, SyncError::Backend(_)));
        assert_eq!(tries.get(), 1);
    }
}
