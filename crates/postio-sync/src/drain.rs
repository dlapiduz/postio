//! Draining the mutation queue against a server that may have moved.
//!
//! # The problem this solves
//!
//! Every mutating action in Postio happened locally already — the row is
//! written, the list repainted, the user moved on. What is left in the queue is
//! *intent*, and by the time it reaches the server the world may not be the one
//! it was formed in: another client archived the message, a filter moved it,
//! the mailbox was renumbered. Reconciling that is where a mail client either
//! stays trustworthy or quietly loses mail.
//!
//! # The four answers
//!
//! Every step ends in exactly one of these, and every queue row is settled by
//! one of them. Nothing is ever dropped on the floor.
//!
//! * **Applied.** The server did it.
//! * **Obsolete.** The message the operation names is not where it was, so
//!   there is nothing to send. Settled rather than failed — but the local row
//!   now disagrees with the server, so the mailbox goes into
//!   [`DrainReport::needs_resync`].
//! * **Deferred.** A transient failure. It comes back later with an exponential
//!   backoff ([`RetryPolicy`]).
//! * **Failed.** Everything else, and anything out of attempts. Recorded on the
//!   row with its reason and returned in [`DrainReport::failed`] so the user is
//!   told. Failure is loud on purpose: an operation that vanished silently is a
//!   message the user believes they filed and cannot find.
//!
//! # Detecting a vanished message
//!
//! IMAP does not report one. `STORE` against a UID that is no longer in the
//! mailbox succeeds and says nothing, which is indistinguishable from success
//! unless you look at what came back — so that is what [`Drainer`] looks at: a
//! store over a non-empty UID set that updates no message means the message is
//! gone. A move is checked the same way, but only when the server speaks
//! UIDPLUS; without it an empty mapping is the ordinary answer and carries no
//! information, so the move is treated as applied and the next resync
//! reconciles.
//!
//! # Sending
//!
//! [`Operation::Send`] is a step like any other, except that once the SMTP
//! transaction is accepted, it can never be retried or reported failed again
//! -- the recipient's server already has the message, and a "failure" from
//! here on would mean sending it twice. See [`crate::send`] for the whole
//! path: resolving the draft, building the message, the transport, and
//! filing the local Sent copy.
//!
//! # Drafts
//!
//! [`Operation::SaveDraft`] and [`Operation::DiscardDraft`] keep the account's
//! Drafts mailbox in step with the composer. They are steps like any other,
//! except that neither names a message: one is resolved against the draft row,
//! and the other carries its own `UID` because by then the row is gone. See
//! [`crate::drafts`].
//!
//! # Not here yet
//!
//! [`Operation::Append`] needs the blob store, which the drainer does not
//! have unless it is draining a [`Operation::Send`] (which brings its own).
//! It is its own bead, and is *failed* with a clear reason rather than
//! skipped, so one cannot sit in a queue forever looking pending.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, Utc};
use postio_imap::backend::{BackendError, Capabilities, Capability, FlagChange, MailBackend};
use postio_model::{AccountId, MailboxId, Operation, OperationId, OperationTarget};
use postio_storage::BlobStore;
use postio_storage::repository::{MailboxRepository, MessageRepository, OperationQueueRepository};
use rusqlite::Connection;

use crate::coalesce::{Step, coalesce};
use crate::retry::RetryPolicy;
use crate::send::SmtpContext;

/// This module's result type.
pub type Result<T> = std::result::Result<T, SyncError>;

/// A failure that stops the whole pass rather than one operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError {
    /// The local database could not be read or written.
    #[error(transparent)]
    Storage(#[from] postio_storage::Error),

    /// Talking to the server failed in a way no single operation owns — a lost
    /// session, a refused password.
    #[error(transparent)]
    Backend(#[from] BackendError),
}

/// An operation the server will not accept, on its way to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedOperation {
    /// The queue rows that failed together.
    pub rows: Vec<OperationId>,
    /// What it was acting on.
    pub target: OperationTarget,
    /// Which operation it was, for the message shown to the user.
    pub op_type: &'static str,
    /// Why the server would not do it.
    pub reason: String,
}

/// What one drain pass did.
///
/// Counted in *queue rows* rather than steps, so the numbers add up to what was
/// in the queue when the pass started.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DrainReport {
    /// Rows the server accepted.
    pub applied: usize,
    /// Rows that needed no round trip because they cancelled each other out.
    pub coalesced: usize,
    /// Rows whose message was not where the operation expected it.
    pub obsolete: usize,
    /// Rows waiting for a retry.
    pub deferred: usize,
    /// Rows given up on, with the reason for each. Never silently empty.
    pub failed: Vec<FailedOperation>,
    /// Rows that may or may not have happened, with the reason for each.
    ///
    /// Its own field rather than a flavour of `failed` (#674): these are the
    /// operations nobody can settle either way, and the difference decides
    /// what the user is told. See [`Outcome::Uncertain`].
    pub uncertain: Vec<FailedOperation>,
    /// Mailboxes whose local state is now known to disagree with the server.
    pub needs_resync: Vec<MailboxId>,
}

impl DrainReport {
    /// How many rows the pass settled altogether.
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

    /// Whether the queue had nothing due.
    pub fn is_idle(&self) -> bool {
        self.settled() == 0 && self.deferred == 0
    }
}

/// Replays the mutation queue against a server.
///
/// Holds no state of its own: one is built per pass, over a backend and a
/// connection the caller owns.
#[derive(Debug)]
pub struct Drainer<'a> {
    backend: &'a dyn MailBackend,
    policy: RetryPolicy,
    smtp: Option<SmtpContext<'a>>,
    /// Where a draft's attachments are read from. Set by [`Drainer::with_smtp`]
    /// as well, since sending needs the same store — a drainer that can send
    /// can always save a draft.
    blobs: Option<&'a BlobStore>,
}

impl<'a> Drainer<'a> {
    /// Drains against `backend` with the default retry policy.
    ///
    /// `Operation::Send` fails outright until [`Drainer::with_smtp`] gives it
    /// a transport — a queue with nothing to send through is not a bug.
    pub fn new(backend: &'a dyn MailBackend) -> Self {
        Self {
            backend,
            policy: RetryPolicy::default(),
            smtp: None,
            blobs: None,
        }
    }

    /// Drains with a different retry policy.
    pub fn with_policy(backend: &'a dyn MailBackend, policy: RetryPolicy) -> Self {
        Self {
            backend,
            policy,
            smtp: None,
            blobs: None,
        }
    }

    /// Gives this drainer what [`Operation::Send`] needs: an SMTP transport,
    /// the account's credentials, and the blob store attachments and the
    /// sent copy read and write through.
    pub fn with_smtp(mut self, smtp: SmtpContext<'a>) -> Self {
        self.blobs = Some(smtp.blobs);
        self.smtp = Some(smtp);
        self
    }

    /// Gives this drainer the blob store a draft's attachments are read from.
    ///
    /// [`Drainer::with_smtp`] already does this, so this is for a drainer that
    /// keeps drafts in step with the server without being able to send —
    /// which is every drainer built for a test that has nothing to send.
    pub fn with_blobs(mut self, blobs: &'a BlobStore) -> Self {
        self.blobs = Some(blobs);
        self
    }

    /// Sends everything due for `account`, in order, and settles every row.
    ///
    /// One pass. The caller decides when the next one runs — after a reconnect,
    /// after the shortest deferred backoff elapses, or when the user does
    /// something new.
    pub async fn drain(
        &self,
        connection: &Connection,
        account: AccountId,
        now: DateTime<Utc>,
    ) -> Result<DrainReport> {
        let queue = OperationQueueRepository::new(connection);
        let batch = queue.pending(account, now)?;
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

        // Rows that cancelled each other out are settled without a round trip.
        // Marked done rather than deleted: the local write they accompanied did
        // happen, and undo may still want to find them.
        for id in &plan.obsolete {
            queue.mark_done(*id, now)?;
            queue.note(*id, "folded into an operation that undid it")?;
        }

        let capabilities = self.backend.capabilities().await?;
        let mut resync: BTreeSet<i64> = BTreeSet::new();

        for step in &plan.steps {
            // One span per operation: the op type and the row it acts on, so a
            // failure three retries deep can be traced back to the action the
            // user took. The payload is never a field — a `Move` names two
            // mailbox ids, an `Append` names a blob key, and neither carries
            // anything of the message itself.
            let span = tracing::debug_span!(
                "operation",
                op = step.operation.op_type(),
                target = step.target.id()
            );
            let outcome = async {
                let outcome = self.run(connection, step, &capabilities, &mut resync)?;
                Ok::<_, crate::drain::SyncError>(match outcome {
                    Pending::Settled(outcome) => outcome,
                    Pending::Send(context) => {
                        self.send(connection, &context, &capabilities, &mut resync)
                            .await
                    }
                })
            };
            let outcome = tracing::Instrument::instrument(outcome, span.clone()).await?;
            let _entered = span.enter();
            tracing::debug!(outcome = ?outcome, "operation settled");
            drop(_entered);
            self.settle(connection, step, outcome, now, &mut report)?;
        }

        report.needs_resync = resync.into_iter().map(MailboxId::new).collect();
        Ok(report)
    }

    /// Looks a step up locally: either it is ready to send, or it is already
    /// decided.
    fn run(
        &self,
        connection: &Connection,
        step: &Step,
        _capabilities: &Capabilities,
        resync: &mut BTreeSet<i64>,
    ) -> Result<Pending> {
        Ok(match self.resolve(connection, step)? {
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

    /// Issues the backend call for a resolved step.
    async fn send(
        &self,
        connection: &Connection,
        context: &Context,
        capabilities: &Capabilities,
        resync: &mut BTreeSet<i64>,
    ) -> Outcome {
        if let Operation::CrossAccountCopy { saga } = &context.operation {
            return crate::cross_account::copy(self.backend, self.blobs, connection, *saga).await;
        }
        if let Operation::CrossAccountRemove { saga } = &context.operation {
            return crate::cross_account::remove(self.backend, connection, *saga).await;
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
                self.backend
                    .move_messages(&context.path, &context.ids, destination)
                    .await
                    .map(|mapping| {
                        // Without UIDPLUS an empty mapping is the ordinary
                        // answer and says nothing about whether anything moved,
                        // so it must not be read as a vanished message.
                        if capabilities.contains(Capability::UidPlus) {
                            vanished_if_untouched(mapping.is_empty())
                        } else {
                            Outcome::Applied
                        }
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
                return crate::send::send(connection, self.backend, smtp, resync, job).await;
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

    /// Looks up everything the backend call needs, or says why it cannot run.
    fn resolve(&self, connection: &Connection, step: &Step) -> Result<Resolved> {
        if let Operation::Send { draft } = &step.operation {
            return Ok(
                match crate::send::resolve(connection, self.smtp.as_ref(), *draft)? {
                    crate::send::ResolvedSend::Ready(job) => Resolved::Ready(Context {
                        operation: step.operation.clone(),
                        path: String::new(),
                        destination: None,
                        ids: Vec::new(),
                        mailbox: MailboxId::UNASSIGNED,
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
        if let Some(resolved) = self.resolve_draft(connection, step)? {
            return Ok(resolved);
        }
        if matches!(
            step.operation,
            Operation::CrossAccountCopy { .. } | Operation::CrossAccountRemove { .. }
        ) {
            // Everything a saga phase needs lives on the saga row, which
            // `send` reads fresh — a Context resolved here could be a
            // restart old by the time it runs.
            return Ok(Resolved::Ready(Context {
                operation: step.operation.clone(),
                path: String::new(),
                destination: None,
                ids: Vec::new(),
                mailbox: MailboxId::UNASSIGNED,
                send: None,
                draft: None,
            }));
        }
        if matches!(step.operation, Operation::Append { .. }) {
            // Resolved as ready so `send` can report it uniformly; there is no
            // mailbox or UID to look up.
            return Ok(Resolved::Ready(Context {
                operation: step.operation.clone(),
                path: String::new(),
                destination: None,
                ids: Vec::new(),
                mailbox: MailboxId::UNASSIGNED,
                send: None,
                draft: None,
            }));
        }

        // The message is read once: it carries both the mailbox a flag change
        // applies to and the UID every message operation needs.
        let message = match step.target {
            OperationTarget::Message(id) => MessageRepository::new(connection).get(id)?,
            _ => None,
        };

        let (mailbox, destination) = match &step.operation {
            Operation::Move { from, to } => (*from, Some(*to)),
            Operation::Delete { from, trash } => (*from, Some(*trash)),
            Operation::Expunge { mailbox } => (*mailbox, None),
            Operation::SetFlags { .. } | Operation::ClearFlags { .. } => match &message {
                Some(message) => (message.mailbox_id, None),
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
                // For a Move or Delete the live row cannot answer: its local
                // half nulled the coordinates in the same transaction that
                // enqueued this, so the queue row's snapshot is the only
                // thing that still names the server position. The earliest
                // contributing row was enqueued before any nulling. Rows
                // from before the snapshot existed carry none and fall back
                // to the live row, which is the old behavior. #289.
                let snapshot = match &step.operation {
                    Operation::Move { .. } | Operation::Delete { .. } => {
                        OperationQueueRepository::new(connection)
                            .get(step.head())?
                            .and_then(|row| row.source_remote_id)
                    }
                    _ => None,
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
            // A mailbox-wide operation names no messages.
            _ => Vec::new(),
        };

        let mailboxes = MailboxRepository::new(connection);
        let Some(source) = mailboxes.get(mailbox)? else {
            return Ok(Resolved::Impossible(format!(
                "mailbox {} is no longer in the local store",
                mailbox.get()
            )));
        };
        let destination = match destination {
            None => None,
            Some(id) => match mailboxes.get(id)? {
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
            send: None,
            draft: None,
        }))
    }

    /// Resolves the two operations that keep the Drafts mailbox in step, or
    /// `None` when this step is not one of them.
    ///
    /// Split out rather than folded into [`Drainer::resolve`]'s match because
    /// neither names a message: a draft has no row in `messages` and, in the
    /// discard case, no row anywhere at all by the time this runs.
    fn resolve_draft(&self, connection: &Connection, step: &Step) -> Result<Option<Resolved>> {
        let resolved = match (&step.operation, step.target) {
            (Operation::SaveDraft { mailbox }, OperationTarget::Draft(draft)) => {
                crate::drafts::resolve_save(connection, self.blobs, draft, *mailbox)?
            }
            (Operation::DiscardDraft { mailbox, remote_id }, _) => {
                crate::drafts::resolve_discard(connection, *mailbox, remote_id.clone())?
            }
            // A draft operation whose target is not a draft is a row written
            // by hand or by a newer Postio; it names nothing this build can
            // act on.
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

    /// Writes an outcome back onto every row behind a step.
    fn settle(
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
                    queue.mark_done(*id, now)?;
                }
                report.applied += rows;
            }
            Outcome::Obsolete { reason } => {
                // Settled, not failed: there was nothing for the server to do.
                // The reason is recorded so it stays explicable in a bug report.
                for id in &step.rows {
                    queue.mark_done(*id, now)?;
                    queue.note(*id, &reason)?;
                }
                report.obsolete += rows;
            }
            Outcome::Retry { reason, after } => {
                let attempts = self.attempts(connection, step)? + 1;
                if self.policy.is_exhausted(attempts) {
                    let reason = format!("{reason} (gave up after {attempts} attempts)");
                    self.fail(&queue, step, &reason, now, report)?;
                } else {
                    let retry_at = self.policy.next_attempt_at(now, attempts, after);
                    for id in &step.rows {
                        queue.defer(*id, retry_at, &reason)?;
                    }
                    report.deferred += rows;
                }
            }
            Outcome::Failed { reason } => {
                self.fail(&queue, step, &reason, now, report)?;
            }
            Outcome::Uncertain { reason } => {
                // Settled, and deliberately not retried: the payload may
                // already be in somebody's inbox, and a duplicate cannot be
                // recalled. `mark_done` rather than `mark_failed` because the
                // queue's work here is over either way -- what is unresolved
                // is the *message*, which the draft's own state carries.
                for id in &step.rows {
                    queue.mark_done(*id, now)?;
                    queue.note(*id, &reason)?;
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

    fn fail(
        &self,
        queue: &OperationQueueRepository<'_>,
        step: &Step,
        reason: &str,
        now: DateTime<Utc>,
        report: &mut DrainReport,
    ) -> Result<()> {
        for id in &step.rows {
            queue.mark_failed(*id, now, reason)?;
        }
        report.failed.push(FailedOperation {
            rows: step.rows.clone(),
            target: step.target,
            op_type: step.operation.op_type(),
            reason: reason.to_owned(),
        });
        Ok(())
    }

    /// How many attempts the rows behind a step have had.
    ///
    /// The most of any of them, so a folded batch backs off on its worst member
    /// rather than its luckiest.
    fn attempts(&self, connection: &Connection, step: &Step) -> Result<u32> {
        let queue = OperationQueueRepository::new(connection);
        let mut attempts = 0;
        for id in &step.rows {
            if let Some(row) = queue.get(*id)? {
                attempts = attempts.max(row.attempts);
            }
        }
        Ok(attempts)
    }
}

/// Everything one backend call needs.
#[derive(Debug)]
struct Context {
    operation: Operation,
    path: String,
    destination: Option<String>,
    ids: Vec<postio_model::RemoteId>,
    mailbox: MailboxId,
    /// Resolved only for [`Operation::Send`]: everything local storage could
    /// answer, so [`Drainer::send`] has nothing left to look up. Boxed: it is
    /// by far the largest field here and every other operation leaves it
    /// `None`.
    send: Option<Box<crate::send::SendJob>>,
    /// Resolved only for the two operations that keep the Drafts mailbox in
    /// step, and boxed for the same reason as `send`: by the time the backend
    /// call is made there is nothing left to look up.
    draft: Option<Box<crate::drafts::DraftJob>>,
}

/// Whether a step still has to go to the server.
enum Pending {
    Send(Context),
    Settled(Outcome),
}

/// The result of looking a step up in the local store.
enum Resolved {
    Ready(Context),
    /// Nothing to send; the local store said so before the server was asked.
    Obsolete {
        reason: String,
        /// The mailbox to resynchronize, when one is implicated.
        mailbox: Option<MailboxId>,
    },
    /// Not sendable *yet*, and no server was asked: something local is still
    /// being written. Deferred like a transient failure, so it comes back with
    /// the same backoff and the same attempt limit rather than inventing a
    /// second waiting mechanism.
    Later(String),
    /// Cannot be sent and never will be.
    Impossible(String),
    /// It may already have happened, and there is no way to find out — see
    /// [`Outcome::Uncertain`].
    Uncertain(String),
}

/// What happened to a step.
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
    /// It may have happened, and there is no way to find out from here.
    ///
    /// ADR 0021 Decision 3, #674: an SMTP session that dies after the
    /// payload has begun going out leaves delivery and failure
    /// indistinguishable. Settled like `Failed` — the row is done, the
    /// reason recorded, nothing retried — but carried separately, because
    /// `failed` means *did not happen* and the runtime turns that straight
    /// into an error the user reads. A send that may have gone must not
    /// travel as one.
    Uncertain {
        reason: String,
    },
}

impl Outcome {
    pub(crate) fn from_error(error: BackendError) -> Self {
        let reason = error.to_string();
        if error.requires_full_resync() {
            // The UID space was renumbered, so the UID this operation carries
            // names a different message — or none at all. Retrying it as it
            // stands would act on the wrong message, which is worse than not
            // acting; the mailbox is resynchronized and the user is told.
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

/// A store or move that touched nothing means the message is not there.
fn vanished_if_untouched(untouched: bool) -> Outcome {
    if untouched {
        Outcome::Obsolete {
            reason: "the message is no longer in that mailbox on the server".to_owned(),
        }
    } else {
        Outcome::Applied
    }
}
