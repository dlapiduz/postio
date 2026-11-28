//! Initial sync: enumerates a mailbox newest first in batches, committing and threading each before
//! the next so the first screenful appears fast.
//! Resumable with no watermark: each pass fetches only what `MessageRepository::uids_in` says is
//! missing; `complete_full_sync` is written last.
//! Does not decide whether a mailbox needs this, nor wipe on a `UIDVALIDITY` change; the caller
//! does both.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;

use chrono::Utc;
use postio_account::backend::{
    BackendError, BackendResult, FetchedMessage, MailBackend, SelectMode, UidSet,
};
use postio_model::{Account, Generation, Mailbox, MailboxId, MailboxStatus, Message, Uid};
use postio_storage::repository::{
    AccountRepository, MessageRepository, SyncStateRepository, ThreadingRepository,
};
use postio_storage::{Checkout, WritePriority};

use crate::drain::SyncError;
use crate::filing::{FiledMessage, FilingPass};
use postio_account::cancel::CancelToken;
use postio_model::MessageId;

pub type Result<T> = std::result::Result<T, SyncError>;

/// Messages per `FETCH`: small enough that the first batch is visible in well under a second, large
/// enough to avoid a round trip per few messages (#78).
pub const DEFAULT_BATCH_SIZE: usize = 200;

/// Messages per write transaction. The write gate bounds a person's wait to one background unit, so
/// the unit must be small:
/// 200 held the lock 90-200 ms, 25 holds it 8-9 ms, inside the 16 ms budget (#425).
/// This is the ceiling: the actual size is learned from the last unit's cost by `unit_rows`
/// (#1587).
pub(crate) const WRITE_UNIT: usize = 25;

/// What one write unit may cost wall-clock: 25 rows at 8-9 ms, inside the 16 ms interaction budget.
const UNIT_BUDGET: std::time::Duration = std::time::Duration::from_millis(9);

/// Learned from the last unit's measured cost, machine-wide, since cost is a property of the store
/// and disk, not a pass.
/// Starts at `WRITE_UNIT` and only shrinks below it, or the #425 guarantee loosens.
pub(crate) fn unit_rows() -> usize {
    LEARNED_UNIT.load(std::sync::atomic::Ordering::Relaxed)
}

/// `held` is the lock-hold time after the gate permit through the commit, never the wait for the
/// permit: queueing says how busy the gate is, not how costly a row is.
pub(crate) fn unit_wrote(rows: usize, held: std::time::Duration) {
    LEARNED_UNIT.store(
        next_unit_rows(rows, held),
        std::sync::atomic::Ordering::Relaxed,
    );
}

static LEARNED_UNIT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(WRITE_UNIT);

/// Pure sizing rule: rows the budget affords at the measured per-row cost, at least one, at most
/// `WRITE_UNIT`.
fn next_unit_rows(rows: usize, held: std::time::Duration) -> usize {
    let per_row = held.as_secs_f64() / rows.max(1) as f64;
    if per_row <= f64::EPSILON {
        return WRITE_UNIT;
    }
    ((UNIT_BUDGET.as_secs_f64() / per_row) as usize).clamp(1, WRITE_UNIT)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub mailbox_id: MailboxId,
    pub fetched: u32,
    /// The server's `EXISTS`, not `UIDNEXT - 1`: the UID space counts every expunged message, and a
    /// 92-message inbox rendered `0%` (`postio-qhz.9`).
    pub target: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub inserted: usize,
    /// Ids a `Coverage::Everything` pass inserted, empty otherwise: for a backend with no MODSEQ
    /// these are the new arrivals (#1704).
    pub arrived: Vec<MessageId>,
    pub filed_away: Vec<MessageId>,
    pub updated: usize,
    pub threaded: usize,
    pub thread_arrivals: crate::threaded::ThreadArrivals,
}

/// `on_progress` is called once per committed batch. Cancelling between batches loses nothing
/// committed; the next call resumes.
/// Does not check that `mailbox` needs a full enumeration, nor wipe stale rows after a
/// `UIDVALIDITY` change; the caller does.
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
        None,
        cancel,
        on_progress,
    )
    .await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    Missing,
    /// For a pass that must refresh: a lost flag is most likely on a message already here.
    Everything,
}

/// `filing` is `Some` only on a modseq-less backend's steady-state pass, where what a
/// re-enumeration inserts is new mail.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn enumerate(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    batch_size: usize,
    coverage: Coverage,
    filing: Option<&dyn FilingPass>,
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
    let first_sync = !SyncStateRepository::new(connection)
        .get(mailbox.id)
        .await?
        .is_some_and(|state| state.has_synced());
    SyncStateRepository::new(connection)
        .observe(mailbox.id, &server_status, now)
        .await?;

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

    // `None` (an orphaned mailbox row) records no sightings rather than failing sync.
    let account = AccountRepository::new(connection)
        .get(mailbox.account_id)
        .await?;

    // What the server actually holds, else every UID below the ceiling. A long-lived folder's UID
    // space is mostly gaps, which cost a round trip per chunk of UIDs (#727).
    let present = existing_uids(backend, mailbox, cancel).await?;

    // A listing shorter than `EXISTS` is not believed: completing on it would stamp
    // `last_full_sync_at` and leave the backlog unreachable.
    // Discarding it costs only a slower pass. A longer listing is the ordinary delivery race,
    // handled by the ceiling filter.
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
            // A server may name a UID at or above the ceiling (a delivery between SELECT and this
            // call); the next pass sees it.
            .filter(|uid| *uid <= highest_uid)
            .filter(|uid| coverage == Coverage::Everything || !known.contains(uid))
            .collect(),
        None => (1..=highest_uid)
            .filter(|uid| coverage == Coverage::Everything || !known.contains(uid))
            .collect(),
    };
    missing.sort_unstable_by_key(|&uid| std::cmp::Reverse(uid));

    let mut fetched_so_far = match coverage {
        Coverage::Missing => known.len() as u32,
        Coverage::Everything => 0,
    };

    // Built up front: an outstanding fetch borrows its set across iterations.
    let batches: Vec<UidSet> = missing
        .chunks(batch_size)
        .map(|chunk| chunk.iter().map(|&uid| Uid::new(uid)).collect())
        .collect();

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

        // Ask for the next batch before taking the write lock, so the server works while we write
        // (#77). At most one fetch is outstanding.
        // A read-ahead commits nothing until consumed, so cancelling just drops it.
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
        let batch = commit_batch_filing(
            connection,
            mailbox,
            account.as_ref(),
            &known,
            &mut messages,
            filing,
        )
        .await?;
        report.inserted += batch.inserted;
        report.updated += batch.updated;
        report.threaded += batch.threaded;
        if coverage == Coverage::Everything {
            report.thread_arrivals.extend(batch.thread_arrivals);
            report.arrived.extend(batch.arrived);
            report.filed_away.extend(batch.filed_away);
        }

        // One real yield per batch, load-bearing: read-ahead priming can make every await Ready,
        // and a pass that never yields deafens `sync_wave`'s cancel and refill arms to the user.
        yield_once().await;

        // Per-batch fetch versus write time, to see where a first sync goes (`postio-0d9.7`).
        // Counts and durations only.
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

/// Joins replies a first sync filed before their originals: one with no `In-Reply-To` or
/// `References` can only be placed by subject.
/// First sync only, since later passes fetch mail that arrives after what it answers. Background
/// and never fatal.
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

/// Asks which UIDs exist. A refusal, an unimplemented backend or an outright failure all mean "walk
/// the UID space": a failed optimisation must not fail the first sync.
/// Cancellation is not swallowed.
pub(crate) async fn existing_uids(
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

/// Writes one batch of headers: upsert, thread, record correspondents, one transaction per
/// `WRITE_UNIT`, gate re-taken per unit.
/// `known` is the UIDs held when the pass started, deciding what counts as newly seen. `messages`
/// is updated in place with assigned ids.
/// Public so benches (#726) measure the real write path rather than a copy.
/// One transaction per unit turns the per-message savepoints into nested ones (one commit instead
/// of hundreds of fsyncs).
/// IMMEDIATE, not DEFERRED (#79): the first statement is a SELECT, and a deferred transaction
/// promoting a read lock to write gets SQLITE_BUSY without the busy handler.
pub async fn commit_batch(
    connection: &Checkout,
    mailbox: &Mailbox,
    account: Option<&Account>,
    known: &BTreeSet<u32>,
    messages: &mut [Message],
) -> Result<Report> {
    commit_batch_filing(connection, mailbox, account, known, messages, None).await
}

/// `commit_batch`, handing each unit's inserted rows to `filing` in the unit's transaction
/// (`crate::filing`).
/// A row already held as fetched is not written at all (#1719): a modseq-less backend re-reads
/// every header each pass. See `unchanged`.
pub(crate) async fn commit_batch_filing(
    connection: &Checkout,
    mailbox: &Mailbox,
    account: Option<&Account>,
    known: &BTreeSet<u32>,
    messages: &mut [Message],
    filing: Option<&dyn FilingPass>,
) -> Result<Report> {
    let skip = unchanged(connection, mailbox, known, messages).await?;
    if skip.is_empty() {
        return write_batch(connection, mailbox, account, known, messages, filing).await;
    }
    let to_write: Vec<usize> = (0..messages.len())
        .filter(|index| !skip.contains(index))
        .collect();
    let mut changed: Vec<Message> = to_write
        .iter()
        .map(|&index| messages[index].clone())
        .collect();
    let report = write_batch(connection, mailbox, account, known, &mut changed, filing).await?;
    for (index, message) in to_write.into_iter().zip(changed) {
        messages[index] = message;
    }
    Ok(report)
}

/// Positions in `messages` whose rows the store already holds exactly as fetched, each given its
/// row's id and thread.
/// Only known uids are asked about. A never-threaded row is written again; a row differing only by
/// a queued local change is written and `upsert_batch` keeps the change (#317).
async fn unchanged(
    connection: &Checkout,
    mailbox: &Mailbox,
    known: &BTreeSet<u32>,
    messages: &mut [Message],
) -> Result<BTreeSet<usize>> {
    let mut skip = BTreeSet::new();
    let mut by_generation: std::collections::BTreeMap<u32, Vec<Uid>> = Default::default();
    for message in messages.iter() {
        if let (Some(uid), Some(validity)) = (message.server.uid, message.server.uid_validity)
            && known.contains(&uid.get())
        {
            by_generation.entry(validity.get()).or_default().push(uid);
        }
    }
    let repository = MessageRepository::new(connection);
    for (validity, uids) in by_generation {
        let stored: std::collections::HashMap<Uid, _> = repository
            .server_states(mailbox.id, Generation::new(validity), &uids)
            .await?
            .into_iter()
            .map(|state| (state.uid, state))
            .collect();
        for (index, message) in messages.iter_mut().enumerate() {
            let Some(state) = message
                .server
                .uid
                .filter(|_| message.server.uid_validity.map(|v| v.get()) == Some(validity))
                .and_then(|uid| stored.get(&uid))
            else {
                continue;
            };
            let same = state.thread_id.is_some()
                && state.remote_id == message.server.remote_id
                && state.mod_seq == message.server.mod_seq
                && state.flags == message.flags.persistable();
            if same {
                message.id = state.id;
                message.thread_id = state.thread_id;
                skip.insert(index);
            }
        }
    }
    Ok(skip)
}

async fn write_batch(
    connection: &Checkout,
    mailbox: &Mailbox,
    account: Option<&Account>,
    known: &BTreeSet<u32>,
    messages: &mut [Message],
    filing: Option<&dyn FilingPass>,
) -> Result<Report> {
    let mut report = Report::default();

    let mut rest: &mut [Message] = messages;
    while !rest.is_empty() {
        // Sized from the last unit's cost so a person waits one small unit at most (#425, #1587).
        let take = unit_rows().min(rest.len());
        let (slice, tail) = rest.split_at_mut(take);
        rest = tail;
        let account_id = mailbox.account_id;
        let mailbox_id = mailbox.id;
        let role = mailbox.role;
        let known_uids = &known;
        let ((upsert, written, effects, joined), held) = write_unit(connection, || {
            // `BEGIN IMMEDIATE` (#79): the first statement is a read, and a deferred transaction
            // promoting to write is refused rather than waiting.
            let source: Vec<Message> = slice.to_vec();
            postio_storage::transaction(connection, move |connection| async move {
                let mut written = source;
                let upsert = upsert_indexed(&connection, &mut written).await?;

                let threading = ThreadingRepository::new(&connection, account_id);
                let mut threads = Vec::with_capacity(written.len());
                for message in &written {
                    threads.push(threading.thread(message).await?.thread_id);
                }

                // Only messages not known before the pass, or a `Coverage::Everything`
                // re-enumeration would count a correspondent twice.
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
                crate::correspondents::record(&connection, role, account_id, &upsert).await?;

                // Inserted rows go to the filing pass in this transaction; refreshed rows are not
                // arrivals.
                let mut effects = crate::filing::FilingEffects::default();
                if let Some(filing) = filing
                    && !upsert.inserted_ids.is_empty()
                {
                    let filed: Vec<FiledMessage<'_>> = written
                        .iter()
                        .zip(&threads)
                        .filter(|(message, _)| upsert.inserted_ids.contains(&message.id))
                        .map(|(message, thread)| FiledMessage {
                            message,
                            thread: Some(*thread),
                            role,
                        })
                        .collect();
                    effects =
                        crate::filing::file_arrivals(&connection, filing, mailbox_id, &filed).await;
                }

                // Said after the commit; the engine emits `ThreadChanged` from the report.
                let mut joined = crate::threaded::ThreadArrivals::default();
                for (message, thread) in written.iter().zip(&threads) {
                    if upsert.inserted_ids.contains(&message.id) {
                        joined.add(*thread, message.id);
                    }
                }

                Ok::<_, SyncError>((upsert, written, effects, joined))
            })
        })
        .await?;

        report.inserted += upsert.inserted;
        report.arrived.extend(&upsert.inserted_ids);
        report.thread_arrivals.extend(joined);
        report.filed_away.extend(effects.filtered);
        report.filed_away.extend(effects.held);
        report.updated += upsert.updated;
        report.threaded += written.len();

        unit_wrote(slice.len(), held);
        copy_back(slice, &written);
        // Yield with the permit down, once per unit: the gate is first-come and re-taken in the
        // same poll it never changes hands, starving body writes (#631).
        yield_once().await;
    }

    Ok(report)
}

/// Upserts a write unit's messages, indexing each once (#1587): the search triggers stand aside and
/// each `search_documents` row is written once at the end, or not at all if unchanged.
/// Inside the unit's transaction, so a failure rolls it all back.
pub(crate) async fn upsert_indexed(
    connection: &postio_storage::Connection,
    batch: &mut Vec<Message>,
) -> postio_storage::Result<postio_storage::repository::UpsertReport> {
    let deferred = postio_index::index::defer_documents(connection).await?;
    let report = MessageRepository::new(connection)
        .upsert_batch(batch)
        .await?;
    if deferred {
        let ids: Vec<_> = batch.iter().map(|message| message.id).collect();
        postio_index::index::write_documents(connection, &ids).await?;
    }
    Ok(report)
}

/// Carry assigned ids back by remote id, not position: `upsert_batch` drops own drafts and
/// locally moved messages, so `written` is an ordered subsequence of `slice`.
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

enum ReadAhead<'a> {
    OnTheWire(Pin<Box<dyn Future<Output = BackendResult<Vec<FetchedMessage>>> + Send + 'a>>),
    Answered(BackendResult<Vec<FetchedMessage>>),
}

const BUSY_RETRIES: usize = 3;

/// Runs one write unit under the gate, retrying `BUSY_RETRIES` times when the engine reports busy
/// (#1594): a failed pass is not requeued the way a cancelled one is.
/// The permit is released between tries so a woken waiter takes its turn. No sleep of its own: the
/// wait is the engine's `busy_timeout`.
/// Returns the unit's result and the lock-hold time timed from the permit, since queueing must not
/// feed the sizing in `unit_wrote`.
pub(crate) async fn write_unit<T, Fut>(
    connection: &Checkout,
    unit: impl Fn() -> Fut,
) -> Result<(T, std::time::Duration)>
where
    Fut: Future<Output = Result<T>>,
{
    let mut tries = 0;
    loop {
        // Ahead of `BEGIN IMMEDIATE`: the permit stands this aside for a keystroke's write. Per
        // unit, so a person waits one unit at most (#425).
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

/// Starts a fetch and polls it once so the request reaches the server before the caller blocks: a
/// future does nothing until polled.
/// Polled with the caller's own waker so the later `await` picks it up.
/// Returns `Pending` exactly once, waking immediately: `tokio::task::yield_now` without naming an
/// executor.
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
        assert_eq!(next_unit_rows(25, Duration::from_millis(8)), WRITE_UNIT);
    }

    #[test]
    fn a_slow_store_shrinks_the_unit_to_fit_the_budget() {
        assert_eq!(next_unit_rows(25, Duration::from_millis(2950)), 1);
    }

    #[test]
    fn the_floor_is_one_row_because_a_unit_is_at_least_a_row() {
        assert_eq!(next_unit_rows(1, Duration::from_millis(118)), 1);
    }

    #[test]
    fn recovery_is_immediate_once_rows_are_cheap_again() {
        assert_eq!(next_unit_rows(1, Duration::from_micros(300)), WRITE_UNIT);
    }

    #[test]
    fn the_unit_never_grows_past_the_measured_sweet_spot() {
        assert_eq!(next_unit_rows(25, Duration::from_nanos(1)), WRITE_UNIT);
        assert_eq!(next_unit_rows(25, Duration::ZERO), WRITE_UNIT);
    }

    #[test]
    fn a_middling_store_lands_in_between() {
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

    #[test]
    fn a_dropped_row_does_not_panic_and_does_not_shift_the_others() {
        let mut slice = [
            message(Some("1:10"), 0),
            message(Some("1:11"), 0),
            message(Some("1:12"), 0),
        ];
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

    #[test]
    fn nothing_dropped_is_a_plain_copy() {
        let mut slice = [message(Some("1:10"), 0), message(Some("1:11"), 0)];
        let written = [message(Some("1:10"), 101), message(Some("1:11"), 102)];

        copy_back(&mut slice, &written);

        assert_eq!(slice[0].id, MessageId::new(101));
        assert_eq!(slice[1].id, MessageId::new(102));
    }

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
