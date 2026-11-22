//! Keeps a synced mailbox current by executing `SyncState::plan`'s answer: up to date, incremental,
//! or full. Every full reason wipes local rows first, since an unwiped `UIDVALIDITY` change
//! silently corrupts state (old UID, different message). Vanished messages are found by count
//! arithmetic (ADR 0001: no `VANISHED`). Arrivals have a second witness, `UIDNEXT`, so a lying
//! change feed cannot hide new mail; the server's `HIGHESTMODSEQ` is recorded last so a failed
//! sweep is retried.

use std::collections::BTreeSet;

use chrono::Utc;
use postio_account::backend::{
    BackendError, MailBackend, MailboxStatus as ServerStatus, SelectMode, UidSet,
};
use postio_account::cancel::CancelToken;
use postio_model::{
    FlagPass, FullResyncReason, Generation, Mailbox, MailboxId, MailboxStatus, Message, MessageId,
    ResyncPlan, SweepPolicy, Uid,
};
use postio_storage::Checkout;
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, MessageRepository, OperationQueueRepository, SyncStateRepository,
    ThreadingRepository,
};

use crate::drain::SyncError;
use crate::filing::{FiledMessage, FilingEffects, FilingPass};
use crate::initial::{self, Progress};

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    UpToDate,
    Full {
        reason: FullResyncReason,
        report: initial::Report,
    },
    /// The wire was untrustworthy, not the local state: `io-imap` drops an undecodable untagged
    /// line and completes `Ok` (ADR 0001), so the mailbox was re-enumerated.
    Rebuilt {
        report: initial::Report,
    },
    Incremental {
        changed: usize,
        vanished: usize,
        /// Of `changed`, genuine arrivals rather than flag changes; what a desktop notification is
        /// about.
        arrived: Vec<MessageId>,
        /// Of `arrived`, those a filing pass took out of the inbox's view (filtered or held); Focus
        /// never announces these (FR-153).
        filed_away: Vec<MessageId>,
        /// Of `arrived`, the thread each joined, including filed-away ones.
        thread_arrivals: crate::threaded::ThreadArrivals,
    },
}

/// Whether the mailbox is still short of `EXISTS` after an incremental pass reconciled it.
/// Checked after, not before: new mail makes it short on every delivery, which would turn each into
/// a full re-enumeration.
/// The queue is consulted because a locally archived row the server has not heard about also leaves
/// it short; a `failed` row counts as unsettled, or that message resurrects every pass.
async fn still_short_of_exists(
    connection: &Checkout,
    mailbox: &Mailbox,
    previous: &postio_model::SyncState,
    exists: u32,
) -> Result<bool> {
    let Some(generation) = previous.generation else {
        return Ok(false);
    };
    let messages = MessageRepository::new(connection);
    let held = messages.count_in(mailbox.id, generation).await?;
    // Rows the store refuses however often fetched (the server's copy of a draft this client wrote)
    // count in `EXISTS` but never in ours;
    // without this Drafts re-enumerated on every pass.
    let refused = messages.refused_rows_in(mailbox.id).await?;
    if held.saturating_add(refused) >= exists {
        return Ok(false);
    }
    if OperationQueueRepository::new(connection)
        .has_unsettled_in(mailbox.id)
        .await?
    {
        return Ok(false);
    }
    tracing::warn!(
        mailbox = mailbox.id.get(),
        held,
        exists,
        "the mailbox holds less than the server says it does after an \
         incremental pass; re-enumerating"
    );
    Ok(true)
}

/// `Coverage::Missing` because nothing renumbered: the repair costs only the missing mail and
/// resumes if cut short.
async fn enumerate_the_shortfall(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let report = initial::enumerate(
        connection,
        backend,
        mailbox,
        initial::DEFAULT_BATCH_SIZE,
        initial::Coverage::Missing,
        None,
        cancel,
        on_progress,
    )
    .await?;
    Ok(Outcome::Full {
        reason: FullResyncReason::ShortOfExists,
        report,
    })
}

/// `on_progress` is only called when a full pass runs.
pub async fn resync_mailbox(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    resync_mailbox_filing(connection, backend, mailbox, None, cancel, on_progress).await
}

/// With `filing` handed what an incremental pass files, inside the transaction that files it (spec
/// 007); only a host in Focus mode runs this.
pub async fn resync_mailbox_filing(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    filing: Option<&dyn FilingPass>,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    resync_mailbox_with_policy(
        connection,
        backend,
        mailbox,
        filing,
        SweepPolicy::DEFAULT,
        cancel,
        on_progress,
    )
    .await
}

/// With the flag sweep's thresholds supplied, so a test can hold a different policy than the
/// shipped two-thousand-message one.
#[allow(clippy::too_many_arguments)]
pub async fn resync_mailbox_with_policy(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    filing: Option<&dyn FilingPass>,
    policy: SweepPolicy,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let sync_state = SyncStateRepository::new(connection);
    let previous = sync_state.require(mailbox.id).await?;

    let selected = match backend.select(&mailbox.path, SelectMode::ReadWrite).await {
        Ok(selected) => selected,
        // `UidValidityChanged` is the rebuild signal, not a failed pass; branch on the predicate,
        // not the variant.
        Err(error) if error.requires_full_resync() => {
            return rebuild(
                connection,
                backend,
                mailbox,
                previous.generation,
                cancel,
                on_progress,
            )
            .await;
        }
        Err(error) => return Err(error.into()),
    };
    let reported = to_model_status(&selected);
    let flag_pass = policy.decide(
        selected.exists,
        previous.passes_since_flag_sweep,
        backend.change_feed(),
    );

    match previous.plan(&reported) {
        ResyncPlan::Full(reason) => {
            // `warn`: a full re-enumeration is correct but degraded, and the reason says whether
            // the server renumbered or we lost the basis.
            tracing::warn!(
                mailbox = mailbox.id.get(),
                reason = ?reason,
                "falling back to a full resync"
            );
            // Only a renumbering invalidates cached rows (#564), as in `rebuild`. A modseq-less
            // backend plans this on every pass, so wiping would refetch the world.
            // `NeverSynced` is not wiped (#1176): an interrupted pass leaves rows and no
            // `last_full_sync_at`, and wiping restarted a too-big mailbox from zero for ever.
            let coverage = match reason {
                FullResyncReason::GenerationChanged => {
                    if let Some(generation) = previous.generation {
                        wipe_mailbox(connection, mailbox.id, generation).await?;
                    }
                    initial::Coverage::Missing
                }
                FullResyncReason::NeverSynced => initial::Coverage::Missing,
                // Never planned: `still_short_of_exists` decides it after an incremental pass.
                FullResyncReason::ShortOfExists => initial::Coverage::Missing,
                FullResyncReason::NoModSeq | FullResyncReason::ModSeqWentBackwards => {
                    initial::Coverage::Everything
                }
            };
            // Recorded before the re-read, except `ModSeqWentBackwards`, which records last:
            // storing the lower value first makes a cut-short pass look `UpToDate`.
            // `GenerationChanged` must record first: the observation drops the old counters so a
            // cut-short pass resumes as `NeverSynced`.
            let record_after = reason == FullResyncReason::ModSeqWentBackwards;
            if !record_after {
                sync_state
                    .observe(mailbox.id, &reported, Utc::now())
                    .await?;
            }
            // A backend with no MODSEQ re-reads on every pass, so what it inserts is new mail and
            // goes to filing (spec 007); other full passes file the backlog, i.e. none (FR-118).
            let filing = match reason {
                FullResyncReason::NoModSeq => filing,
                _ => None,
            };
            let report = initial::enumerate(
                connection,
                backend,
                mailbox,
                initial::DEFAULT_BATCH_SIZE,
                coverage,
                filing,
                cancel,
                on_progress,
            )
            .await?;
            if record_after {
                sync_state
                    .observe(mailbox.id, &reported, Utc::now())
                    .await?;
            }
            Ok(Outcome::Full { reason, report })
        }
        ResyncPlan::Incremental { since } => {
            tracing::debug!(
                mailbox = mailbox.id.get(),
                since = since.get(),
                "resyncing incrementally"
            );
            // One unit: a failed pull or sweep leaves `HIGHESTMODSEQ` unrecorded.
            let outcome = async {
                let mut outcome = incremental(
                    connection,
                    backend,
                    mailbox,
                    &selected,
                    since,
                    previous.uid_next,
                    filing,
                    cancel,
                )
                .await?;
                let swept = sweep_flags(
                    connection,
                    backend,
                    mailbox,
                    selected.generation,
                    flag_pass,
                    cancel,
                )
                .await?;
                if let Outcome::Incremental { changed, .. } = &mut outcome {
                    *changed += swept.corrected;
                }
                Ok::<_, SyncError>((outcome, swept.swept))
            }
            .await;

            match outcome {
                Ok((outcome, swept)) => {
                    sync_state
                        .observe(mailbox.id, &reported, Utc::now())
                        .await?;
                    sync_state.record_flag_pass(mailbox.id, swept).await?;
                    if still_short_of_exists(connection, mailbox, &previous, selected.exists)
                        .await?
                    {
                        return enumerate_the_shortfall(
                            connection,
                            backend,
                            mailbox,
                            cancel,
                            on_progress,
                        )
                        .await;
                    }
                    Ok(outcome)
                }
                // The pull cannot be trusted and asking again would lose the same line
                // (`Outcome::Rebuilt`). `reported` is not recorded first: it would say nothing is
                // left to catch up on.
                Err(SyncError::Backend(error)) if error.requires_full_resync() => {
                    // The integrity guard: io-imap dropped a line the pull counted on, so a delta
                    // was silently lost. See `postio_account::imap`'s `undecoded`.
                    tracing::error!(
                        mailbox = mailbox.id.get(),
                        %error,
                        "the incremental pull cannot be trusted; rebuilding: {error}"
                    );
                    rebuild(
                        connection,
                        backend,
                        mailbox,
                        previous.generation,
                        cancel,
                        on_progress,
                    )
                    .await
                }
                Err(other) => Err(other),
            }
        }
        ResyncPlan::UpToDate => {
            tracing::debug!(mailbox = mailbox.id.get(), "already up to date");
            // A server that under-reports may not have moved `HIGHESTMODSEQ`, so "up to date" is
            // exactly when a sweep is needed.
            let swept = match sweep_flags(
                connection,
                backend,
                mailbox,
                selected.generation,
                flag_pass,
                cancel,
            )
            .await
            {
                Ok(swept) => swept,
                Err(SyncError::Backend(error)) if error.requires_full_resync() => {
                    tracing::error!(
                        mailbox = mailbox.id.get(),
                        %error,
                        "the flag sweep cannot be trusted; rebuilding: {error}"
                    );
                    return rebuild(
                        connection,
                        backend,
                        mailbox,
                        previous.generation,
                        cancel,
                        on_progress,
                    )
                    .await;
                }
                Err(other) => return Err(other),
            };
            sync_state
                .observe(mailbox.id, &reported, Utc::now())
                .await?;
            sync_state.record_flag_pass(mailbox.id, swept.swept).await?;
            // An idle server never moves `MODSEQ`, so a mailbox stamped synced but never enumerated
            // would plan `UpToDate` for ever; "up to date" must mean "holds what the server holds".
            // A CONDSTORE-only server may delete without raising `HIGHESTMODSEQ` (RFC 7162 §3.2),
            // so more rows than `EXISTS` is the witness.
            if let Some(generation) = previous.generation {
                let messages = MessageRepository::new(connection);
                if messages.count_in(mailbox.id, generation).await? > selected.exists {
                    let known = messages.uids_in(mailbox.id, generation).await?;
                    let expected_gone = known.len() - selected.exists as usize;
                    let vanished = reconcile_vanished(
                        connection,
                        backend,
                        mailbox,
                        generation,
                        known,
                        expected_gone,
                        cancel,
                    )
                    .await?;
                    if vanished > 0 {
                        return Ok(Outcome::Incremental {
                            changed: swept.corrected,
                            vanished,
                            arrived: Vec::new(),
                            filed_away: Vec::new(),
                            thread_arrivals: Default::default(),
                        });
                    }
                }
            }
            if still_short_of_exists(connection, mailbox, &previous, selected.exists).await? {
                return enumerate_the_shortfall(connection, backend, mailbox, cancel, on_progress)
                    .await;
            }
            if swept.corrected > 0 {
                return Ok(Outcome::Incremental {
                    changed: swept.corrected,
                    vanished: 0,
                    arrived: Vec::new(),
                    filed_away: Vec::new(),
                    thread_arrivals: Default::default(),
                });
            }
            Ok(Outcome::UpToDate)
        }
    }
}

async fn sweep_flags(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    generation: Generation,
    pass: FlagPass,
    cancel: &CancelToken,
) -> Result<crate::flag_sweep::Swept> {
    match pass {
        FlagPass::Trust => Ok(crate::flag_sweep::Swept::default()),
        FlagPass::Sweep => {
            crate::flag_sweep::sweep(connection, backend, mailbox, generation, cancel).await
        }
    }
}

/// Re-enumerates a mailbox whose incremental basis the backend would not stand behind, after asking
/// the server what it is now.
/// A moved generation wipes the rows first, reported as the planned path reports it. A held
/// generation wipes nothing: only the pull was untrustworthy, and a full enumeration refreshes
/// every flag and inserts every arrival.
async fn rebuild(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    known_generation: Option<Generation>,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let selected = backend.select(&mailbox.path, SelectMode::ReadWrite).await?;
    let reported = to_model_status(&selected);

    let renumbered = known_generation.is_some_and(|known| known != selected.generation);
    if renumbered && let Some(stale) = known_generation {
        wipe_mailbox(connection, mailbox.id, stale).await?;
    }

    SyncStateRepository::new(connection)
        .observe(mailbox.id, &reported, Utc::now())
        .await?;

    // A wiped mailbox has nothing to skip; an intact one is re-read, since the lost delta was
    // likely a flag on a stored message.
    let coverage = if renumbered {
        initial::Coverage::Missing
    } else {
        initial::Coverage::Everything
    };
    let report = initial::enumerate(
        connection,
        backend,
        mailbox,
        initial::DEFAULT_BATCH_SIZE,
        coverage,
        None,
        cancel,
        on_progress,
    )
    .await?;

    Ok(if renumbered {
        Outcome::Full {
            reason: FullResyncReason::GenerationChanged,
            report,
        }
    } else {
        Outcome::Rebuilt { report }
    })
}

/// Fetches what changed since `since` and reconciles what vanished; vanish detection runs only when
/// the count arithmetic demands it, and arrivals have a second witness (module docs).
#[allow(clippy::too_many_arguments)]
async fn incremental(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    selected: &ServerStatus,
    since: postio_model::ModSeq,
    previous_uid_next: Option<Uid>,
    filing: Option<&dyn FilingPass>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let known = messages.uids_in(mailbox.id, selected.generation).await?;
    let known_set: UidSet = known.iter().copied().collect();
    let known_count = known.len() as u32;

    // Both fetches that can bring new mail ask for the headers the filing pass reads before a body
    // exists (spec 007, research R8).
    let mut changed = backend
        .fetch_headers_for_filing(&mailbox.path, &UidSet::all(), Some(since), cancel)
        .await?;

    if let Some(floor) = unaccounted_arrivals(&changed, selected, previous_uid_next) {
        let arrivals = backend
            .fetch_headers_for_filing(
                &mailbox.path,
                &UidSet::from_uid_onwards(floor),
                None,
                cancel,
            )
            .await?;
        changed.extend(arrivals);
    }

    let newly_arrived = changed
        .iter()
        .filter(|message| !known_set.contains(message.uid))
        .count() as u32;
    let changed_count = changed.len();

    let mut arrived: Vec<MessageId> = Vec::new();
    let mut filed_away: Vec<MessageId> = Vec::new();
    let mut thread_arrivals = crate::threaded::ThreadArrivals::default();
    if !changed.is_empty() {
        let batch: Vec<Message> = changed
            .into_iter()
            .map(|message| message.into_message(mailbox.account_id, mailbox.id))
            .collect();
        let account = AccountRepository::new(connection)
            .get(mailbox.account_id)
            .await?;

        // `initial::WRITE_UNIT` at a time, as `initial` does: one savepoint release per call is an
        // fsync unless enclosed, and an ungated whole batch held the lock a draft autosave needed
        // (#425).
        let mut rest: &[postio_model::Message] = &batch;
        while !rest.is_empty() {
            let take = initial::unit_rows().min(rest.len());
            let (slice, tail) = rest.split_at(take);
            rest = tail;
            let account_id = mailbox.account_id;
            let mailbox_id = mailbox.id;
            let role = mailbox.role;
            let account_ref = account.as_ref();
            let known_ref = &known_set;
            let ((newly, effects, joined), held) = initial::write_unit(connection, || {
                // IMMEDIATE (#79), as in `initial.rs`. `upsert_batch` takes a `Vec`, so the unit is
                // copied in and read back.
                let source: Vec<Message> = slice.to_vec();
                postio_storage::transaction(connection, move |connection| async move {
                    let mut written = source;
                    let upsert = crate::initial::upsert_indexed(&connection, &mut written).await?;
                    crate::correspondents::record(&connection, role, account_id, &upsert).await?;

                    let threading = ThreadingRepository::new(&connection, account_id);
                    let mut threads = Vec::with_capacity(written.len());
                    for message in &written {
                        threads.push(threading.thread(message).await?.thread_id);
                    }

                    // Only arrivals: a message in `known_set` (read before this fetch) is a flag
                    // change, not a new sighting or new mail.
                    let mut arrivals = Vec::new();
                    if let Some(account) = account_ref {
                        for message in &written {
                            let is_new = message
                                .server
                                .uid
                                .is_some_and(|uid| !known_ref.contains(uid));
                            if is_new {
                                crate::contacts::record(&connection, account, message).await?;
                                arrivals.push(message.id);
                            }
                        }
                    }

                    // Arrivals go to the filing pass in this transaction, after upsert and
                    // threading, before any event (spec 007); a failing pass takes back only its
                    // own writes.
                    let mut effects = FilingEffects::default();
                    if let Some(filing) = filing
                        && !arrivals.is_empty()
                    {
                        let filed: Vec<FiledMessage<'_>> = written
                            .iter()
                            .zip(&threads)
                            .filter(|(message, _)| arrivals.contains(&message.id))
                            .map(|(message, thread)| FiledMessage {
                                message,
                                thread: Some(*thread),
                                role,
                            })
                            .collect();
                        effects =
                            crate::filing::file_arrivals(&connection, filing, mailbox_id, &filed)
                                .await;
                    }
                    let mut joined = crate::threaded::ThreadArrivals::default();
                    for (message, thread) in written.iter().zip(&threads) {
                        if arrivals.contains(&message.id) {
                            joined.add(*thread, message.id);
                        }
                    }
                    Ok::<_, SyncError>((arrivals, effects, joined))
                })
            })
            .await?;
            arrived.extend(newly);
            thread_arrivals.extend(joined);
            filed_away.extend(effects.filtered);
            filed_away.extend(effects.held);
            initial::unit_wrote(slice.len(), held);
            // One real yield per unit, as in `initial.rs`, or a pass whose awaits come back `Ready`
            // cannot be interrupted.
            initial::yield_once().await;
        }
    }

    let mut vanished_count = 0;
    let expected_exists = known_count + newly_arrived;
    if !known_set.is_empty() && selected.exists != expected_exists {
        let expected_gone = (known_count + newly_arrived).saturating_sub(selected.exists);
        vanished_count = reconcile_vanished(
            connection,
            backend,
            mailbox,
            selected.generation,
            known,
            expected_gone as usize,
            cancel,
        )
        .await?;
    }

    Ok(Outcome::Incremental {
        changed: changed_count,
        vanished: vanished_count,
        arrived,
        filed_away,
        thread_arrivals,
    })
}

/// Asks the server which of `known` it still holds and deletes the rest locally; returns how many
/// rows went.
/// `expected_gone` is `known + arrivals - EXISTS`, the integrity check: absence from a fetch is not
/// proof of expunge (io-imap drops undecodable lines, `SEARCH` can come back short),
/// so an answer not accounting for exactly that many vanishings is not believed and nothing is
/// deleted (#1913).
/// One `UID SEARCH` where the backend can list, else the known set in chunks.
async fn reconcile_vanished(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    generation: Generation,
    known: Vec<Uid>,
    expected_gone: usize,
    cancel: &CancelToken,
) -> Result<usize> {
    let present: BTreeSet<Uid> = match initial::existing_uids(backend, mailbox, cancel).await? {
        Some(listing) => listing.into_iter().collect(),
        None => {
            let known_set: UidSet = known.iter().copied().collect();
            let mut present = BTreeSet::new();
            for chunk in known_set.chunks(initial::DEFAULT_BATCH_SIZE) {
                if cancel.is_cancelled() {
                    return Err(SyncError::Backend(BackendError::Cancelled));
                }
                let fetched = backend
                    .fetch_headers(&mailbox.path, &chunk, None, cancel)
                    .await?;
                present.extend(fetched.iter().map(|message| message.uid));
            }
            present
        }
    };
    let vanished: Vec<Uid> = known
        .into_iter()
        .filter(|uid| !present.contains(uid))
        .collect();

    if vanished.len() != expected_gone {
        tracing::warn!(
            mailbox = mailbox.id.get(),
            absent = vanished.len(),
            expected = expected_gone,
            "the presence check disagrees with EXISTS; deleting nothing this pass"
        );
        return Ok(0);
    }

    let messages = MessageRepository::new(connection);
    let ids = messages
        .ids_by_uid(mailbox.id, generation, &vanished)
        .await?;
    Ok(messages.delete(&ids).await?)
}

/// The floor is one past the highest UID the feed reported, so a conforming feed yields an empty
/// range and no second round trip.
fn unaccounted_arrivals(
    reported: &[postio_account::backend::FetchedMessage],
    selected: &ServerStatus,
    previous_uid_next: Option<Uid>,
) -> Option<Uid> {
    let previous = previous_uid_next?;
    let highest = reported
        .iter()
        .map(|message| message.uid.get())
        .max()
        .unwrap_or(0);
    let floor = previous.get().max(highest.saturating_add(1));
    (floor < selected.uid_next.get()).then(|| Uid::new(floor))
}

/// Uses `MessageRepository::uids_in`, not the windowed list query, which hides rows marked for
/// deletion that a `UIDVALIDITY` reset must also wipe.
async fn wipe_mailbox(
    connection: &Connection,
    mailbox_id: MailboxId,
    generation: Generation,
) -> Result<()> {
    let messages = MessageRepository::new(connection);
    let uids = messages.uids_in(mailbox_id, generation).await?;
    let ids = messages.ids_by_uid(mailbox_id, generation, &uids).await?;
    messages.delete(&ids).await?;
    Ok(())
}

fn to_model_status(selected: &ServerStatus) -> MailboxStatus {
    let mut status = MailboxStatus::new(selected.generation).with_uid_next(selected.uid_next);
    if let Some(mod_seq) = selected.highest_mod_seq {
        status = status.with_highest_mod_seq(mod_seq);
    }
    status
}
