//! Keeping an already-synced mailbox current: QRESYNC/CONDSTORE incremental
//! resync, with a `UIDVALIDITY` change as the correctness trap it is.
//!
//! # The decision is not this module's to make
//!
//! [`SyncState::plan`](postio_model::SyncState::plan) is the pure function
//! that decides *how* a mailbox should be brought up to date, from the state
//! this crate already persisted and what the server just reported at
//! `SELECT`. This module is the other half: it executes each of that
//! decision's three answers.
//!
//! * [`ResyncPlan::UpToDate`] — nothing to do beyond recording that the
//!   mailbox was looked at.
//! * [`ResyncPlan::Incremental`] — ask only for what changed since the last
//!   `MODSEQ` we hold, and reconcile what vanished. See "Detecting vanished
//!   messages" below for why that second half needs its own explanation.
//! * [`ResyncPlan::Full`] — re-enumerate from scratch. Per
//!   [`ResyncPlan::Full`]'s own docs this always means *discarding cached
//!   UIDs*, not just the ones that changed generation, so every `Full` reason
//!   — not only [`FullResyncReason::UidValidityChanged`] — wipes this
//!   mailbox's local rows before [`initial::sync_mailbox`] repopulates it.
//!   Getting this specific case wrong is the one CLAUDE.md calls out by name:
//!   a `UIDVALIDITY` change that is not wiped silently corrupts state, because
//!   an old UID means a different message once the generation moves on.
//!
//! # Detecting vanished messages
//!
//! `MailBackend` deliberately does not expose IMAP's `VANISHED` response —
//! ADR 0001 found that `io-imap` discards it when requested through `FETCH`
//! anyway, and the trait stays protocol-neutral rather than growing a QRESYNC-
//! shaped hole for one extension. So this module reconstructs "what vanished"
//! from counting, and only pays for the expensive half of that when the
//! count says it must:
//!
//! 1. `CHANGEDSINCE` finds every message that is new or changed. On a
//!    conforming server new messages are always caught by it: a message
//!    cannot exist before the `MODSEQ` it was created at, and RFC 7162
//!    §3.1.2.1 requires that to exceed the mailbox's previous
//!    `HIGHESTMODSEQ`. See "Arrivals" below for what happens when a server
//!    does not manage that.
//! 2. If `known + newly_arrived` equals what `SELECT` just reported the
//!    mailbox holds, nothing vanished — the arithmetic is exact, not a
//!    heuristic, so a simultaneous arrival-and-deletion that happens to leave
//!    the count unchanged still trips it. This is the common case, and it
//!    costs nothing beyond the `CHANGEDSINCE` fetch itself: "reconnect with
//!    no server changes fetches essentially nothing" falls out of it for free.
//! 3. When the count disagrees, and only then, every previously known UID is
//!    re-fetched with no `CHANGEDSINCE` filter; whichever ones the server no
//!    longer answers for are gone and are deleted locally.
//!
//! # Arrivals, from `UIDNEXT` rather than from the change feed
//!
//! Step 1 above rests on the server assigning every new message a `MODSEQ`
//! above the `HIGHESTMODSEQ` we last recorded. That is what the RFC requires
//! and it is not something to bet the inbox on: a message the change feed
//! never mentions is a message that never appears, with no error anywhere, and
//! "new mail stopped arriving" is the single worst way for a mail client to
//! fail.
//!
//! So arrivals have a second, independent witness — `UIDNEXT`, which means
//! exactly "UIDs below this have been handed out" and cannot be wrong without
//! the server being incoherent. When it has moved past what the change feed
//! accounted for, the gap is fetched directly. The check costs nothing on a
//! conforming server: the change feed already reported those UIDs, so the
//! range is empty and no second `FETCH` is issued.

use std::collections::BTreeSet;

use chrono::Utc;
use postio_imap::backend::{MailBackend, MailboxStatus as ServerStatus, SelectMode, UidSet};
use postio_imap::cancel::CancelToken;
use postio_model::{
    FullResyncReason, Mailbox, MailboxId, MailboxStatus, Message, MessageId, ResyncPlan, Uid,
    UidValidity,
};
use postio_storage::repository::{
    AccountRepository, MessageRepository, SyncStateRepository, ThreadingRepository,
};
use rusqlite::Connection;

use crate::drain::SyncError;
use crate::initial::{self, Progress};

/// This module's result type.
pub type Result<T> = std::result::Result<T, SyncError>;

/// What one resync pass did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The server reported nothing new; only `last_seen_at` advanced.
    UpToDate,
    /// A full re-enumeration ran. See [`ResyncPlan::Full`] for why.
    Full {
        /// Why a full pass was required.
        reason: FullResyncReason,
        /// What [`initial::sync_mailbox`] did while repopulating the mailbox.
        report: initial::Report,
    },
    /// An incremental pull could not be trusted, so the mailbox was
    /// re-enumerated instead.
    ///
    /// Distinct from [`Full`](Self::Full) because nothing was wrong with the
    /// local state: the *wire* was. `io-imap` drops an untagged response it
    /// cannot decode and completes the command `Ok` (ADR 0001), so a pull
    /// that lost a line looks exactly like one that did not; the backend
    /// counts the skips and refuses the result rather than reporting a
    /// gap-free answer. There is no `FullResyncReason` for that, and
    /// inventing one would put the blame in the wrong place.
    Rebuilt {
        /// What the re-enumeration did.
        report: initial::Report,
    },
    /// Only what changed since the last sync was fetched.
    Incremental {
        /// Messages that were new or had a flag change, per `CHANGEDSINCE`.
        changed: usize,
        /// Messages that no longer exist on the server and were removed
        /// locally.
        vanished: usize,
        /// Of `changed`, the ones that were not known before this pass —
        /// genuine arrivals rather than a flag change on mail already here.
        /// This is what a desktop notification is about; `changed` alone
        /// cannot tell the two apart.
        arrived: Vec<MessageId>,
    },
}

/// Brings `mailbox` up to date, choosing full or incremental resync per
/// [`SyncState::plan`](postio_model::SyncState::plan).
///
/// `on_progress` is only called when a full pass runs; see
/// [`initial::sync_mailbox`].
pub async fn resync_mailbox(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let sync_state = SyncStateRepository::new(connection);
    let previous = sync_state.require(mailbox.id)?;

    let selected = match backend.select(&mailbox.path, SelectMode::ReadWrite).await {
        Ok(selected) => selected,
        // A backend that refuses to serve a generation nobody confirmed says
        // so *instead of* handing back a status — see
        // `BackendError::UidValidityChanged`. That refusal is the rebuild
        // signal, not a failed pass: swallowing it here is the difference
        // between re-enumerating a renumbered mailbox and never noticing it
        // was renumbered. Branching on the predicate rather than the variant
        // is what the error type asks of its callers.
        Err(error) if error.requires_full_resync() => {
            return rebuild(
                connection,
                backend,
                mailbox,
                previous.uid_validity,
                cancel,
                on_progress,
            )
            .await;
        }
        Err(error) => return Err(error.into()),
    };
    let reported = to_model_status(&selected);

    match previous.plan(&reported) {
        ResyncPlan::Full(reason) => {
            // Which of the three paths a pass took, and why. The `warn` is
            // deliberate: falling back to a full re-enumeration is correct but
            // degraded, and the reason is what tells you whether the server
            // renumbered or we lost the incremental basis ourselves.
            tracing::warn!(
                mailbox = mailbox.id.get(),
                reason = ?reason,
                "falling back to a full resync"
            );
            if let Some(uid_validity) = previous.uid_validity {
                wipe_mailbox(connection, mailbox.id, uid_validity)?;
            }
            sync_state.observe(mailbox.id, &reported, Utc::now())?;
            let report =
                initial::sync_mailbox(connection, backend, mailbox, cancel, on_progress).await?;
            Ok(Outcome::Full { reason, report })
        }
        ResyncPlan::Incremental { since } => {
            tracing::debug!(
                mailbox = mailbox.id.get(),
                since = since.get(),
                "resyncing incrementally"
            );
            let outcome = incremental(
                connection,
                backend,
                mailbox,
                &selected,
                since,
                previous.uid_next,
                cancel,
            )
            .await;

            match outcome {
                Ok(outcome) => {
                    sync_state.observe(mailbox.id, &reported, Utc::now())?;
                    Ok(outcome)
                }
                // The pull cannot be trusted, and asking the same question
                // again would lose the same answer — see [`Outcome::Rebuilt`].
                // Note that `reported` is deliberately *not* recorded first:
                // a state that says "synchronized up to here" would tell the
                // next pass there was nothing to catch up on.
                Err(SyncError::Backend(error)) if error.requires_full_resync() => {
                    // The integrity guard firing: io-imap dropped a response
                    // line the incremental pull was counting on, so the answer
                    // cannot be trusted. Loud, because it means a delta was
                    // silently lost — see postio-imap's skip counter.
                    tracing::error!(
                        mailbox = mailbox.id.get(),
                        %error,
                        "the incremental pull cannot be trusted; rebuilding"
                    );
                    rebuild(
                        connection,
                        backend,
                        mailbox,
                        previous.uid_validity,
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
            sync_state.observe(mailbox.id, &reported, Utc::now())?;
            Ok(Outcome::UpToDate)
        }
    }
}

/// Re-enumerates a mailbox whose incremental basis the backend would not stand
/// behind.
///
/// Asks the server what it is *now*, because the reason the last answer was
/// refused may well be that the mailbox is not what it was. Two cases come out
/// of that, and they want different things:
///
/// * **The generation moved.** Every local UID is meaningless, so the rows
///   under it are wiped before the mailbox is read again — the
///   [`UIDVALIDITY`](FullResyncReason::UidValidityChanged) rebuild, reported
///   exactly as the planned path reports it, so a caller cannot tell whether
///   the change was discovered by comparing statuses or by being refused one.
/// * **The generation held.** The UIDs are still good and only the *pull* was
///   untrustworthy, so nothing is thrown away: a full enumeration refreshes
///   every flag and inserts every arrival, which is everything a dropped
///   `CHANGEDSINCE` line could have carried. A message expunged during the
///   lost window survives locally until the next pass, whose count check
///   notices that the server holds fewer messages than we do and reconciles —
///   which is a great deal cheaper than discarding rows that are still right.
async fn rebuild(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    known_generation: Option<UidValidity>,
    cancel: &CancelToken,
    on_progress: impl FnMut(Progress),
) -> Result<Outcome> {
    let selected = backend.select(&mailbox.path, SelectMode::ReadWrite).await?;
    let reported = to_model_status(&selected);

    let renumbered = known_generation.is_some_and(|known| known != selected.uid_validity);
    if renumbered && let Some(stale) = known_generation {
        wipe_mailbox(connection, mailbox.id, stale)?;
    }

    SyncStateRepository::new(connection).observe(mailbox.id, &reported, Utc::now())?;

    // A wiped mailbox has nothing left to skip, so the cheaper pass covers
    // it; an intact one has to be re-read rather than filled in, because the
    // delta that went missing was most likely a flag on a message that is
    // already stored.
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
        cancel,
        on_progress,
    )
    .await?;

    Ok(if renumbered {
        Outcome::Full {
            reason: FullResyncReason::UidValidityChanged,
            report,
        }
    } else {
        Outcome::Rebuilt { report }
    })
}

/// Fetches what changed since `since`, and reconciles what vanished.
///
/// See the module docs for why vanish detection is conditional on the
/// arithmetic rather than always run, and why arrivals get a second witness.
async fn incremental(
    connection: &Connection,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    selected: &ServerStatus,
    since: postio_model::ModSeq,
    previous_uid_next: Option<Uid>,
    cancel: &CancelToken,
) -> Result<Outcome> {
    let messages = MessageRepository::new(connection);
    let known = messages.uids_in(mailbox.id, selected.uid_validity)?;
    let known_set: UidSet = known.iter().copied().collect();
    let known_count = known.len() as u32;

    let mut changed = backend
        .fetch_headers(&mailbox.path, &UidSet::all(), Some(since), cancel)
        .await?;

    if let Some(floor) = unaccounted_arrivals(&changed, selected, previous_uid_next) {
        let arrivals = backend
            .fetch_headers(
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
    if !changed.is_empty() {
        let mut batch: Vec<Message> = changed
            .into_iter()
            .map(|message| message.into_message(mailbox.account_id, mailbox.id))
            .collect();
        MessageRepository::new(connection).upsert_batch(&mut batch)?;

        let threading = ThreadingRepository::new(connection, mailbox.account_id);
        for message in &batch {
            threading.thread(message)?;
        }

        // Only the arrivals, by the same test twice over: `known_set` was
        // read before this fetch, so a message already in it is a flag
        // change or similar, not a new correspondent sighting and not new
        // mail to notify about. See `contacts::record`'s docs for the
        // double-counting this also avoids.
        if let Some(account) = AccountRepository::new(connection).get(mailbox.account_id)? {
            for message in &batch {
                let is_new = message
                    .server
                    .uid
                    .is_some_and(|uid| !known_set.contains(uid));
                if is_new {
                    crate::contacts::record(connection, &account, message)?;
                    arrived.push(message.id);
                }
            }
        }
    }

    let mut vanished_count = 0;
    let expected_exists = known_count + newly_arrived;
    if !known_set.is_empty() && selected.exists != expected_exists {
        let still_present = backend
            .fetch_headers(&mailbox.path, &known_set, None, cancel)
            .await?;
        let present: BTreeSet<Uid> = still_present.iter().map(|message| message.uid).collect();
        let vanished: Vec<Uid> = known
            .into_iter()
            .filter(|uid| !present.contains(uid))
            .collect();

        let mut ids = Vec::with_capacity(vanished.len());
        for uid in vanished {
            if let Some(message) = messages.by_uid(mailbox.id, selected.uid_validity, uid)? {
                ids.push(message.id);
            }
        }
        vanished_count = messages.delete(&ids)?;
    }

    Ok(Outcome::Incremental {
        changed: changed_count,
        vanished: vanished_count,
        arrived,
    })
}

/// The first UID of the range `UIDNEXT` says exists and the change feed did
/// not report, or `None` when the two agree.
///
/// The floor is one past the highest UID the feed *did* report rather than the
/// last `UIDNEXT` we recorded, so a feed that accounted for the arrivals — the
/// conforming case, and the common one — produces an empty range and no second
/// round trip.
fn unaccounted_arrivals(
    reported: &[postio_imap::backend::FetchedMessage],
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

/// Removes every locally known message under `uid_validity`.
///
/// Goes through [`MessageRepository::uids_in`] rather than the windowed list
/// query: a `UIDVALIDITY` reset means every row under the old generation is
/// meaningless, including ones a user has marked for deletion but that have
/// not been expunged yet, and the list query hides those.
fn wipe_mailbox(
    connection: &Connection,
    mailbox_id: MailboxId,
    uid_validity: UidValidity,
) -> Result<()> {
    let messages = MessageRepository::new(connection);
    let uids = messages.uids_in(mailbox_id, uid_validity)?;

    let mut ids = Vec::with_capacity(uids.len());
    for uid in uids {
        if let Some(message) = messages.by_uid(mailbox_id, uid_validity, uid)? {
            ids.push(message.id);
        }
    }
    messages.delete(&ids)?;
    Ok(())
}

fn to_model_status(selected: &ServerStatus) -> MailboxStatus {
    let mut status = MailboxStatus::new(selected.uid_validity).with_uid_next(selected.uid_next);
    if let Some(mod_seq) = selected.highest_mod_seq {
        status = status.with_highest_mod_seq(mod_seq);
    }
    status
}
