//! Drains the mutation queue against a server that may have moved. Every step ends Applied,
//! Obsolete (mailbox goes in `needs_resync`),
//! Deferred (backoff) or Failed (loud: a silently vanished operation is a message the user cannot
//! find).
//! A vanished message is detected by a `STORE` that updates nothing. Once SMTP accepts a send it is
//! never retried or reported failed.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_account::backend::{BackendError, Capabilities, FlagChange, MailBackend, UidMapping};
use postio_model::{
    AccountId, Flag, FlagSet, MailboxId, MessageId, Operation, OperationId, OperationTarget,
};
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{MailboxRepository, MessageRepository, OperationQueueRepository};

use crate::coalesce::{Step, coalesce};
use crate::retry::RetryPolicy;
use crate::send::SmtpContext;

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError {
    #[error(transparent)]
    Storage(#[from] postio_storage::Error),

    #[error(transparent)]
    Backend(#[from] BackendError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedOperation {
    pub rows: Vec<OperationId>,
    pub target: OperationTarget,
    pub op_type: &'static str,
    pub reason: String,
}

impl FailedOperation {
    /// The sentence the person sees: what did not happen, then why (#1487). The bare reason reads
    /// as a fact, not the fate of something they asked for.
    pub fn said(&self) -> String {
        let what = match self.op_type {
            "send" => "Not sent",
            "move" => "Not moved",
            "delete" => "Not deleted",
            "expunge" => "Not expunged",
            "purge" => "Not deleted for good",
            "append" => "Not uploaded",
            "set_flags" | "clear_flags" => "Flags not changed",
            "save_draft" => "Draft not saved",
            "discard_draft" => "Draft not discarded",
            "cross_account_copy" | "cross_account_remove" => "Not moved across accounts",
            other => return format!("{other} failed \u{2014} {}", self.reason),
        };
        format!("{what} \u{2014} {}", self.reason)
    }
}

/// Counted in queue rows, so the numbers add up to what was in the queue when the pass started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainReport {
    pub applied: usize,
    pub coalesced: usize,
    pub obsolete: usize,
    pub deferred: usize,
    pub failed: Vec<FailedOperation>,
    /// Rows that may or may not have happened (#674); kept apart from `failed` because the user is
    /// told differently. See `Outcome::Uncertain`.
    pub uncertain: Vec<FailedOperation>,
    pub needs_resync: Vec<MailboxId>,
    pub thread_arrivals: crate::threaded::ThreadArrivals,
}

impl DrainReport {
    pub fn settled(&self) -> usize {
        self.applied
            + self.coalesced
            + self.obsolete
            + self
                .failed
                .iter()
                .map(|failed| failed.rows.len())
                .sum::<usize>()
    }

    pub fn is_idle(&self) -> bool {
        self.settled() == 0 && self.deferred == 0
    }
}

#[derive(Debug)]
pub struct Drainer<'a> {
    backend: &'a dyn MailBackend,
    policy: RetryPolicy,
    smtp: Option<SmtpContext<'a>>,
    blobs: Option<&'a BlobStore>,
}

impl<'a> Drainer<'a> {
    /// `Operation::Send` fails outright until `with_smtp` gives it a transport.
    pub fn new(backend: &'a dyn MailBackend) -> Self {
        Self {
            backend,
            policy: RetryPolicy::default(),
            smtp: None,
            blobs: None,
        }
    }

    pub fn with_policy(backend: &'a dyn MailBackend, policy: RetryPolicy) -> Self {
        Self {
            backend,
            policy,
            smtp: None,
            blobs: None,
        }
    }

    pub fn with_smtp(mut self, smtp: SmtpContext<'a>) -> Self {
        self.blobs = Some(smtp.blobs);
        self.smtp = Some(smtp);
        self
    }

    pub fn with_blobs(mut self, blobs: &'a BlobStore) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// One pass; the caller decides when the next runs.
    pub async fn drain(
        &self,
        connection: &Connection,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<DrainReport> {
        // Before the early return: an account holding only orphaned sends would otherwise take that
        // return every pass and never heal.
        match postio_storage::repository::DraftRepository::new(connection)
            .fail_orphaned_sends(account)
            .await
        {
            Ok(0) => {}
            Ok(healed) => tracing::warn!(
                healed,
                "sends were queued with nothing left to carry them, and are \
                 now marked as not sent"
            ),
            Err(error) => tracing::warn!(%error, "could not reconcile orphaned sends"),
        }

        let queue = OperationQueueRepository::new(connection);
        let batch = queue.pending(account, now).await?;
        if batch.is_empty() {
            return Ok(DrainReport::default());
        }

        let plan = coalesce(&batch);
        tracing::debug!(
            pending = batch.len(),
            steps = plan.steps.len(),
            folded = plan.obsolete.len(),
            "draining the operation queue"
        );
        let mut report = DrainReport {
            coalesced: plan.obsolete.len(),
            ..DrainReport::default()
        };

        // Settled without a round trip; marked done, not deleted, since undo may still look for
        // them.
        for id in &plan.obsolete {
            queue.mark_done(*id, now).await?;
            queue
                .note(*id, "folded into an operation that undid it")
                .await?;
        }

        let capabilities = self.backend.capabilities().await?;
        let mut resync: BTreeSet<i64> = BTreeSet::new();
        let mut threaded = crate::threaded::ThreadArrivals::default();

        for step in &plan.steps {
            // One span per operation: op type and row only, never the payload (mailbox ids and blob
            // keys carry nothing of the message).
            let span = tracing::debug_span!(
                "operation",
                op = step.operation.op_type(),
                target = step.target.id()
            );
            let outcome = async {
                let outcome = self
                    .run(connection, step, &capabilities, &mut resync)
                    .await?;
                Ok::<_, crate::drain::SyncError>(match outcome {
                    Pending::Settled(outcome) => outcome,
                    Pending::Send(context) => {
                        self.send(
                            connection,
                            &context,
                            &capabilities,
                            &mut resync,
                            &mut threaded,
                        )
                        .await
                    }
                })
            };
            let outcome = tracing::Instrument::instrument(outcome, span.clone()).await?;
            let _entered = span.enter();
            tracing::debug!(outcome = ?outcome, "operation settled");
            drop(_entered);
            self.settle(connection, step, outcome, now, &mut report)
                .await?;
        }

        report.needs_resync = resync.into_iter().map(MailboxId::new).collect();
        report.thread_arrivals = threaded;
        Ok(report)
    }

    async fn run(
        &self,
        connection: &Connection,
        step: &Step,
        _capabilities: &Capabilities,
        resync: &mut BTreeSet<i64>,
    ) -> Result<Pending> {
        if let Some(outcome) = self.carry(connection, step).await? {
            return Ok(Pending::Settled(outcome));
        }
        Ok(match self.resolve(connection, step).await? {
            Resolved::Ready(context) => Pending::Send(context),
            Resolved::Obsolete { reason, mailbox } => {
                if let Some(mailbox) = mailbox {
                    resync.insert(mailbox.get());
                }
                Pending::Settled(Outcome::Obsolete { reason })
            }
            Resolved::Later(reason) => Pending::Settled(Outcome::Retry {
                reason,
                after: None,
            }),
            Resolved::Impossible(reason) => Pending::Settled(Outcome::Failed { reason }),
            Resolved::Uncertain(reason) => Pending::Settled(Outcome::Uncertain { reason }),
        })
    }

    async fn send(
        &self,
        connection: &Connection,
        context: &Context,
        capabilities: &Capabilities,
        resync: &mut BTreeSet<i64>,
        threaded: &mut crate::threaded::ThreadArrivals,
    ) -> Outcome {
        if let Operation::CrossAccountCopy { saga } = &context.operation {
            return crate::cross_account::copy(self.backend, self.blobs, connection, *saga).await;
        }
        if let Operation::CrossAccountRemove { saga } = &context.operation {
            return crate::cross_account::remove(self.backend, connection, *saga, &context.ids)
                .await;
        }

        let result = match &context.operation {
            Operation::SetFlags { flags } => self
                .store(context, FlagChange::Add(flags.clone()))
                .await
                .map(vanished_if_untouched),
            Operation::ClearFlags { flags } => self
                .store(context, FlagChange::Remove(flags.clone()))
                .await
                .map(vanished_if_untouched),
            Operation::Move { .. } | Operation::Delete { .. } => {
                let destination = context
                    .destination
                    .as_deref()
                    .expect("a move always resolves a destination");
                self.move_and_adopt(connection, context, destination)
                    .await
                    .map(|_mapping| {
                        // An empty mapping is not a vanished message: RFC 4315 §3 lets a server
                        // omit COPYUID, so the new UIDs are unknown (#903).
                        // Applied, and the destination is resynced, since that is where the answer
                        // is.
                        // A mapping does not spare the resync: `move_to` cleared the row's UID, and
                        // only a destination sync restores it (#1978).
                        let landed = match &context.operation {
                            Operation::Move { to, .. } => Some(*to),
                            Operation::Delete { trash, .. } => Some(*trash),
                            _ => None,
                        };
                        if let Some(landed) = landed {
                            resync.insert(landed.get());
                        }
                        Outcome::Applied
                    })
            }
            Operation::CrossAccountCopy { .. } | Operation::CrossAccountRemove { .. } => {
                unreachable!("handled above, before the match")
            }
            Operation::Expunge { .. } => self
                .backend
                .expunge(&context.path, None)
                .await
                .map(|_| Outcome::Applied),
            // Mark, then expunge only these: a backend without a targeted expunge leaves it marked,
            // the safe half.
            Operation::Purge { .. } => {
                let deleted: FlagSet = std::iter::once(Flag::Deleted).collect();
                match self.store(context, FlagChange::Add(deleted)).await {
                    Ok(_) => self
                        .backend
                        .expunge(&context.path, Some(&context.ids))
                        .await
                        .map(|_| Outcome::Applied),
                    Err(error) => Err(error),
                }
            }
            Operation::Append { .. } => {
                return Outcome::Failed {
                    reason: format!(
                        "`{}` needs the blob store, which the drainer does not have yet",
                        context.operation.op_type()
                    ),
                };
            }
            Operation::SaveDraft { .. } | Operation::DiscardDraft { .. } => {
                let job = context
                    .draft
                    .as_ref()
                    .expect("resolve() always attaches a DraftJob to a Ready draft context");
                return crate::drafts::run(connection, self.backend, capabilities, resync, job)
                    .await;
            }
            Operation::Send { .. } => {
                let job = context
                    .send
                    .as_ref()
                    .expect("resolve() always attaches a SendJob to a Ready Send context");
                let smtp = self
                    .smtp
                    .as_ref()
                    .expect("resolve() only returns Ready for Send when smtp is configured");
                return crate::send::send(connection, self.backend, smtp, resync, threaded, job)
                    .await;
            }
        };

        match result {
            Ok(outcome) => {
                if matches!(outcome, Outcome::Obsolete { .. }) {
                    resync.insert(context.mailbox.get());
                }
                outcome
            }
            Err(error) => {
                if error.requires_full_resync() {
                    resync.insert(context.mailbox.get());
                }
                Outcome::from_error(error)
            }
        }
    }

    /// Runs the server move, then gives the local row the identity from the answer's `COPYUID`
    /// (#1983).
    /// Best effort: a message without a Message-ID cannot be re-attached by a destination sync.
    async fn move_and_adopt(
        &self,
        connection: &Connection,
        context: &Context,
        destination: &str,
    ) -> std::result::Result<Vec<UidMapping>, BackendError> {
        let mapping = self
            .backend
            .move_messages(&context.path, &context.ids, destination)
            .await?;
        let landed = match &context.operation {
            Operation::Move { to, .. } => Some(*to),
            Operation::Delete { trash, .. } => Some(*trash),
            _ => None,
        };
        if let (Some(message), Some(landed), [landing]) =
            (context.message, landed, mapping.as_slice())
            && let Err(error) = MessageRepository::new(connection)
                .adopt_moved_identity(
                    message,
                    landed,
                    landing.destination,
                    landing.uid_validity,
                    &landing.destination_remote_id(),
                )
                .await
        {
            tracing::warn!(%error, "could not record where the moved message landed");
        }
        Ok(mapping)
    }

    async fn store(
        &self,
        context: &Context,
        change: FlagChange,
    ) -> std::result::Result<bool, BackendError> {
        let updates = self
            .backend
            .store_flags(&context.path, &context.ids, &change)
            .await?;
        Ok(updates.is_empty())
    }

    /// Fetches attachments a forward carries whose bytes were never downloaded (#1686); `None`
    /// means carry on.
    /// A separate stage because it is the one lookup that talks to the server.
    async fn carry(&self, connection: &Connection, step: &Step) -> Result<Option<Outcome>> {
        let draft = match (&step.operation, step.target) {
            (Operation::Send { draft }, _) => *draft,
            (Operation::SaveDraft { .. }, OperationTarget::Draft(draft)) => draft,
            _ => return Ok(None),
        };
        let Some(blobs) = self.blobs else {
            return Ok(None);
        };
        Ok(
            match crate::carry::fetch_missing(connection, self.backend, blobs, draft).await? {
                crate::carry::Carried::Ready => None,
                crate::carry::Carried::Later(reason) => Some(Outcome::Retry {
                    reason,
                    after: None,
                }),
                crate::carry::Carried::Impossible(reason) => match step.operation {
                    // A failed Drafts copy must not make a draft that is still being written say it
                    // failed; the send meets the same refusal.
                    Operation::SaveDraft { .. } => Some(Outcome::Retry {
                        reason,
                        after: None,
                    }),
                    _ => Some(Outcome::Failed { reason }),
                },
            },
        )
    }

    async fn resolve(&self, connection: &Connection, step: &Step) -> Result<Resolved> {
        if let Operation::Send { draft } = &step.operation {
            return Ok(
                match crate::send::resolve(connection, self.smtp.as_ref(), *draft).await? {
                    crate::send::ResolvedSend::Ready(job) => Resolved::Ready(Context {
                        operation: step.operation.clone(),
                        path: String::new(),
                        destination: None,
                        ids: Vec::new(),
                        mailbox: MailboxId::UNASSIGNED,
                        message: None,
                        send: Some(job),
                        draft: None,
                    }),
                    crate::send::ResolvedSend::Obsolete(reason) => Resolved::Obsolete {
                        reason,
                        mailbox: None,
                    },
                    crate::send::ResolvedSend::Impossible(reason) => Resolved::Impossible(reason),
                    crate::send::ResolvedSend::Uncertain(reason) => Resolved::Uncertain(reason),
                },
            );
        }
        if let Some(resolved) = self.resolve_draft(connection, step).await? {
            return Ok(resolved);
        }
        if matches!(
            step.operation,
            Operation::CrossAccountCopy { .. } | Operation::CrossAccountRemove { .. }
        ) {
            // A saga phase reads its state from the saga row at `send`, except the source
            // coordinate phase 3 removes by:
            // a snapshot taken before the local write hid the source row, needed because an inverse
            // saga's source has no identity (#940, #531, #289).
            let snapshot = match step.operation {
                Operation::CrossAccountRemove { .. } => OperationQueueRepository::new(connection)
                    .get(step.head())
                    .await?
                    .and_then(|row| row.source_remote_id),
                _ => None,
            };
            return Ok(Resolved::Ready(Context {
                operation: step.operation.clone(),
                path: String::new(),
                destination: None,
                ids: snapshot.into_iter().collect(),
                mailbox: MailboxId::UNASSIGNED,
                message: None,
                send: None,
                draft: None,
            }));
        }
        if matches!(step.operation, Operation::Append { .. }) {
            return Ok(Resolved::Ready(Context {
                operation: step.operation.clone(),
                path: String::new(),
                destination: None,
                ids: Vec::new(),
                mailbox: MailboxId::UNASSIGNED,
                message: None,
                send: None,
                draft: None,
            }));
        }

        let message = match step.target {
            OperationTarget::Message(id) => MessageRepository::new(connection).get(id).await?,
            _ => None,
        };

        // A flag change queued ahead of a move: the move nulled the live row's coordinates, so use
        // the move row's remembered position or the change is settled obsolete (#2076).
        let left = match (&step.operation, step.target, &message) {
            (
                Operation::SetFlags { .. } | Operation::ClearFlags { .. },
                OperationTarget::Message(id),
                Some(message),
            ) if message.server.remote_id.is_none() => {
                OperationQueueRepository::new(connection)
                    .leaving_after(id, step.head())
                    .await?
            }
            _ => None,
        };

        let (mailbox, destination) = match &step.operation {
            Operation::Move { from, to } => (*from, Some(*to)),
            Operation::Delete { from, trash } => (*from, Some(*trash)),
            Operation::Expunge { mailbox } | Operation::Purge { mailbox } => (*mailbox, None),
            Operation::SetFlags { .. } | Operation::ClearFlags { .. } => match &message {
                Some(message) => (
                    left.as_ref().map_or(message.mailbox_id, |(from, _)| *from),
                    None,
                ),
                None => {
                    return Ok(Resolved::Obsolete {
                        reason: "the message is no longer in the local store".to_owned(),
                        mailbox: None,
                    });
                }
            },
            Operation::Append { .. }
            | Operation::Send { .. }
            | Operation::SaveDraft { .. }
            | Operation::DiscardDraft { .. }
            | Operation::CrossAccountCopy { .. }
            | Operation::CrossAccountRemove { .. } => unreachable!("handled above"),
        };

        let ids = match step.target {
            OperationTarget::Message(_) => {
                let Some(message) = message.as_ref() else {
                    return Ok(Resolved::Obsolete {
                        reason: "the message is no longer in the local store".to_owned(),
                        mailbox: None,
                    });
                };
                // The live row cannot answer for a Move or Delete: its local half nulled the
                // coordinates when enqueuing, so the queue snapshot is the only record (#289).
                let snapshot = match &step.operation {
                    Operation::Move { .. } | Operation::Delete { .. } => {
                        OperationQueueRepository::new(connection)
                            .get(step.head())
                            .await?
                            .and_then(|row| row.source_remote_id)
                    }
                    _ => left.map(|(_, remote_id)| remote_id),
                };
                match snapshot.or(message.server.remote_id.clone()) {
                    Some(remote_id) => vec![remote_id],
                    None => {
                        return Ok(Resolved::Obsolete {
                            reason: "the message has never been uploaded, so the server has \
                                     nothing to change"
                                .to_owned(),
                            mailbox: None,
                        });
                    }
                }
            }
            _ => Vec::new(),
        };

        let mailboxes = MailboxRepository::new(connection);
        let Some(source) = mailboxes.get(mailbox).await? else {
            return Ok(Resolved::Impossible(format!(
                "mailbox {} is no longer in the local store",
                mailbox.get()
            )));
        };
        let destination = match destination {
            None => None,
            Some(id) => match mailboxes.get(id).await? {
                Some(mailbox) => Some(mailbox.path),
                None => {
                    return Ok(Resolved::Impossible(format!(
                        "the destination mailbox {} is no longer in the local store",
                        id.get()
                    )));
                }
            },
        };

        Ok(Resolved::Ready(Context {
            operation: step.operation.clone(),
            path: source.path,
            destination,
            ids,
            mailbox,
            message: match step.target {
                OperationTarget::Message(id) => Some(id),
                _ => None,
            },
            send: None,
            draft: None,
        }))
    }

    async fn resolve_draft(
        &self,
        connection: &Connection,
        step: &Step,
    ) -> Result<Option<Resolved>> {
        let resolved = match (&step.operation, step.target) {
            (Operation::SaveDraft { mailbox }, OperationTarget::Draft(draft)) => {
                crate::drafts::resolve_save(connection, self.blobs, draft, *mailbox).await?
            }
            (Operation::DiscardDraft { mailbox, remote_id }, _) => {
                crate::drafts::resolve_discard(connection, *mailbox, remote_id.clone()).await?
            }
            // Written by hand or a newer Postio; names nothing this build can act on.
            (Operation::SaveDraft { .. }, _) => crate::drafts::ResolvedDraft::Impossible(
                "a draft operation that does not name a draft".to_owned(),
            ),
            _ => return Ok(None),
        };

        Ok(Some(match resolved {
            crate::drafts::ResolvedDraft::Ready(job) => Resolved::Ready(Context {
                operation: step.operation.clone(),
                path: String::new(),
                destination: None,
                ids: Vec::new(),
                mailbox: MailboxId::UNASSIGNED,
                message: None,
                send: None,
                draft: Some(job),
            }),
            crate::drafts::ResolvedDraft::Obsolete(reason) => Resolved::Obsolete {
                reason,
                mailbox: None,
            },
            crate::drafts::ResolvedDraft::Later(reason) => Resolved::Later(reason),
            crate::drafts::ResolvedDraft::Impossible(reason) => Resolved::Impossible(reason),
        }))
    }

    async fn settle(
        &self,
        connection: &Connection,
        step: &Step,
        outcome: Outcome,
        now: DateTime<Utc>,
        report: &mut DrainReport,
    ) -> Result<()> {
        let queue = OperationQueueRepository::new(connection);
        let rows = step.rows.len();

        match outcome {
            Outcome::Applied => {
                for id in &step.rows {
                    queue.mark_done(*id, now).await?;
                }
                report.applied += rows;
            }
            Outcome::Obsolete { reason } => {
                // Settled, not failed; the reason is recorded for bug reports.
                for id in &step.rows {
                    queue.mark_done(*id, now).await?;
                    queue.note(*id, &reason).await?;
                }
                report.obsolete += rows;
            }
            Outcome::Retry { reason, after } => {
                let attempts = self.attempts(connection, step).await? + 1;
                if self.policy.is_exhausted(attempts) {
                    let reason = format!("{reason} (gave up after {attempts} attempts)");
                    self.fail(connection, &queue, step, &reason, now, report)
                        .await?;
                } else {
                    let retry_at = self.policy.next_attempt_at(now, attempts, after);
                    for id in &step.rows {
                        queue.defer(*id, retry_at, &reason).await?;
                    }
                    report.deferred += rows;
                }
            }
            Outcome::Failed { reason } => {
                self.fail(connection, &queue, step, &reason, now, report)
                    .await?;
            }
            Outcome::Uncertain { reason } => {
                // Settled and not retried: the payload may already be in an inbox and cannot be
                // recalled. `mark_done` because the unresolved thing is the message, which the
                // draft's state carries.
                for id in &step.rows {
                    queue.mark_done(*id, now).await?;
                    queue.note(*id, &reason).await?;
                }
                report.uncertain.push(FailedOperation {
                    rows: step.rows.clone(),
                    target: step.target,
                    op_type: step.operation.op_type(),
                    reason,
                });
            }
        }
        Ok(())
    }

    async fn fail(
        &self,
        connection: &Connection,
        queue: &OperationQueueRepository<'_>,
        step: &Step,
        reason: &str,
        now: DateTime<Utc>,
        report: &mut DrainReport,
    ) -> Result<()> {
        for id in &step.rows {
            queue.mark_failed(*id, now, reason).await?;
        }
        // A give-up must be written on the draft too, or it sits `Queued` in the Outbox for ever
        // once `prune_settled` removes the operation.
        // Best-effort and safe: only reached where a refusal was witnessed (`Uncertain` never comes
        // here), and a failed write leaves the draft as it was.
        if let postio_model::Operation::Send { draft } = step.operation
            && let Err(error) = postio_storage::repository::DraftRepository::new(connection)
                .set_state(draft, postio_model::DraftState::Failed)
                .await
        {
            tracing::warn!(%error, "a send gave up but the draft could not be marked");
        }
        report.failed.push(FailedOperation {
            rows: step.rows.clone(),
            target: step.target,
            op_type: step.operation.op_type(),
            reason: reason.to_owned(),
        });
        Ok(())
    }

    /// The most of any row's attempts, so a folded batch backs off on its worst member.
    async fn attempts(&self, connection: &Connection, step: &Step) -> Result<u32> {
        let queue = OperationQueueRepository::new(connection);
        let mut attempts = 0;
        for id in &step.rows {
            if let Some(row) = queue.get(*id).await? {
                attempts = attempts.max(row.attempts);
            }
        }
        Ok(attempts)
    }
}

#[derive(Debug)]
struct Context {
    operation: Operation,
    path: String,
    destination: Option<String>,
    ids: Vec<postio_model::RemoteId>,
    mailbox: MailboxId,
    message: Option<MessageId>,
    /// Resolved only for `Operation::Send`. Boxed: by far the largest field.
    send: Option<Box<crate::send::SendJob>>,
    draft: Option<Box<crate::drafts::DraftJob>>,
}

enum Pending {
    Send(Context),
    Settled(Outcome),
}

enum Resolved {
    Ready(Context),
    Obsolete {
        reason: String,
        mailbox: Option<MailboxId>,
    },
    /// Not sendable yet: something local is still being written. Deferred with the same backoff and
    /// attempt limit as a transient failure.
    Later(String),
    Impossible(String),
    Uncertain(String),
}

#[derive(Debug)]
pub(crate) enum Outcome {
    Applied,
    Obsolete {
        reason: String,
    },
    Retry {
        reason: String,
        after: Option<Duration>,
    },
    Failed {
        reason: String,
    },
    /// It may have happened and cannot be found out (ADR 0021 Decision 3, #674): an SMTP session
    /// dying mid-payload.
    /// Settled like `Failed` but carried separately, since `failed` means "did not happen".
    Uncertain {
        reason: String,
    },
}

impl Outcome {
    pub(crate) fn from_error(error: BackendError) -> Self {
        let reason = error.to_string();
        if error.requires_full_resync() {
            // The UID space was renumbered, so retrying would act on the wrong message; resync and
            // tell the user.
            return Self::Failed { reason };
        }
        if error.is_transient() {
            return Self::Retry {
                reason,
                after: error.retry_after(),
            };
        }
        Self::Failed { reason }
    }
}

fn vanished_if_untouched(untouched: bool) -> Outcome {
    if untouched {
        Outcome::Obsolete {
            reason: "the message is no longer in that mailbox on the server".to_owned(),
        }
    } else {
        Outcome::Applied
    }
}
