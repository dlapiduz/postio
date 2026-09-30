//! The local-first mutation queue.
//!
//! Every mutating action in Postio writes SQLite and enqueues here in the same
//! transaction, then repaints; the network happens later and somewhere else.
//! That is what makes the app work offline, what makes undo a replay rather
//! than a second implementation, and what keeps the UI off the network — the
//! three invariants `CLAUDE.md` states for mutations.
//!
//! The vocabulary — which operations exist and what each one's inverse is —
//! belongs to [`postio_model::Operation`]. This module only stores it, which is
//! why `op_type` carries no `CHECK` constraint: adding an operation must not
//! need a migration.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, MailboxId, MessageId, Operation, OperationId, OperationRange, OperationState,
    OperationTarget, RemoteId,
};

use super::{MessageSet, from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// One row of the mutation queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedOperation {
    /// Local id. Also the drain order — see [`OperationId`].
    pub id: OperationId,
    /// The account whose server this will be replayed against.
    pub account_id: AccountId,
    /// What it acts on.
    pub target: OperationTarget,
    /// What to do.
    pub operation: Operation,
    /// What undoes it, decided at enqueue time. `None` when nothing does.
    pub inverse: Option<Operation>,
    /// The mailbox the drainer has to select, when the operation names one.
    pub mailbox_id: Option<MailboxId>,
    /// Where it is in its life cycle.
    pub state: OperationState,
    /// How many times it has been tried.
    pub attempts: u32,
    /// What went wrong on the last attempt.
    pub last_error: Option<String>,
    /// Backoff: the drainer skips this row until then.
    pub next_attempt_at: Option<DateTime<Utc>>,
    /// When the user performed the action.
    pub created_at: DateTime<Utc>,
    /// When the row last changed.
    pub updated_at: DateTime<Utc>,
    /// Where the message sat on the server when this was enqueued.
    ///
    /// Snapshotted for message-target rows, because the local half of a Move
    /// or Delete nulls the live row's coordinates in the same transaction —
    /// by drain time this row is the only thing that remembers them (#289).
    /// The backend's own identity (#543); `None` for rows written before the
    /// snapshot existed, for non-message targets, and for messages that had
    /// never been uploaded; the drainer falls back to the live row.
    pub source_remote_id: Option<RemoteId>,
}

/// Reads and writes the mutation queue.
///
/// # Enqueue is part of the caller's write
///
/// Every method takes a borrowed [`Connection`], and the repository's own
/// atomic scopes are savepoints, so an enqueue nests inside the transaction
/// that performs the local write:
///
/// ```no_run
/// # use postio_model::{AccountId, MailboxId, MessageId, Operation, OperationTarget};
/// # use postio_storage::repository::OperationQueueRepository;
/// # async fn demo() -> Result<(), postio_storage::Error> {
/// # use postio_storage::key::{Purpose, StoreKey};
/// # let key = StoreKey::generate().derive(Purpose::Database);
/// # let database = postio_storage::Store::open("postio.db", &key).await?;
/// # let connection = database.connect().await?;
/// # let (account, message) = (AccountId::new(1), MessageId::new(1));
/// # let (inbox, archive) = (MailboxId::new(1), MailboxId::new(2));
/// postio_storage::transaction(&connection, |transaction| async move {
///     // Enqueue FIRST, then perform the local write: a Move or Delete nulls
///     // the row's server coordinates, and the enqueue is what snapshots them
///     // while they are still there (#289).
///     OperationQueueRepository::new(&transaction)
///         .enqueue(
///             account,
///             OperationTarget::Message(message),
///             &Operation::Move { from: inbox, to: archive },
///             chrono::Utc::now(),
///         )
///         .await?;
///     // ... perform the local write ...
///     Ok::<_, postio_storage::Error>(())
/// })
/// .await?;
/// # Ok(())
/// # }
/// ```
///
/// Enqueueing outside the local write is the bug this shape exists to prevent:
/// a queue row without its local write tells the server about something the
/// user never saw happen, and a local write without its row silently never
/// reaches the server.
#[derive(Debug)]
pub struct OperationQueueRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "\
id, account_id, op_type, target_kind, target_id, mailbox_id, payload, inverse, state,
attempts, last_error, next_attempt_at, created_at, updated_at, source_remote_id";

impl<'a> OperationQueueRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Appends an operation to the account's queue.
    ///
    /// The inverse is computed and stored now rather than when undo is pressed:
    /// by then the mailbox the message came from may no longer be where it is,
    /// and reconstructing "where was this before" from the current state is
    /// exactly the guesswork undo must not do.
    pub async fn enqueue(
        &self,
        account_id: AccountId,
        target: OperationTarget,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        self.enqueue_inner(account_id, target, operation, at, None)
            .await
    }

    /// Appends an operation the drainer must not touch before `not_before` —
    /// a scheduled send, or anything else the user asked to happen later.
    ///
    /// Reuses the same gate [`defer`](Self::defer) puts a backed-off retry
    /// behind ([`pending`](Self::pending) skips any row whose
    /// `next_attempt_at` is still in the future), which is what makes this
    /// restart-safe for free: the row is read from SQLite on every drain
    /// pass, not timed from an in-memory clock that a restart would lose.
    /// Unlike a deferred row this one has made no attempt and failed
    /// nothing, so `attempts` stays `0` and `last_error` stays `None` — a
    /// scheduled send must not read like a retry in a bug report.
    pub async fn enqueue_not_before(
        &self,
        account_id: AccountId,
        target: OperationTarget,
        operation: &Operation,
        at: DateTime<Utc>,
        not_before: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        self.enqueue_inner(account_id, target, operation, at, Some(not_before))
            .await
    }

    async fn enqueue_inner(
        &self,
        account_id: AccountId,
        target: OperationTarget,
        operation: &Operation,
        at: DateTime<Utc>,
        next_attempt_at: Option<DateTime<Utc>>,
    ) -> Result<QueuedOperation> {
        let account_id = require_persisted(account_id.get(), "account")?;
        sql::in_scope(self.connection, |scope| async move {
            let payload = encode(operation)?;
            let inverse = operation.inverse();
            let encoded_inverse = inverse.as_ref().map(encode).transpose()?;
            let mailbox_id = operation.mailbox().filter(|id| id.is_assigned());

            // The snapshot has to be read before the caller's local write nulls
            // it — which is why enqueue comes before the move in every Move and
            // Delete path (#289). A target that is not a message, or a row that
            // does not exist, snapshots nothing.
            let source_remote_id: Option<String> = match target {
                OperationTarget::Message(id) => sql::first(
                    &scope,
                    "SELECT remote_id FROM messages WHERE id = ?1",
                    [id.get()],
                    |row| row.col(0),
                )
                .await
                .unwrap_or(None),
                _ => None,
            };

            sql::execute(
                &scope,
                "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                              mailbox_id, payload, inverse, state, attempts,
                                              next_attempt_at, created_at, updated_at,
                                              source_remote_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, ?10, ?11)",
                bind![
                    account_id,
                    operation.op_type(),
                    target.kind(),
                    target.id(),
                    mailbox_id.map(MailboxId::get),
                    payload,
                    encoded_inverse,
                    OperationState::Pending.as_str(),
                    next_attempt_at.map(to_millis),
                    to_millis(at),
                    source_remote_id,
                ],
            )
            .await?;
            let id = OperationId::new(scope.last_insert_rowid());

            refresh_pending_flag(&scope, target).await?;

            Ok(QueuedOperation {
                id,
                account_id: AccountId::new(account_id),
                target,
                operation: operation.clone(),
                inverse,
                mailbox_id,
                state: OperationState::Pending,
                attempts: 0,
                last_error: None,
                next_attempt_at,
                created_at: at,
                updated_at: at,
                source_remote_id: source_remote_id.map(RemoteId::new),
            })
        })
        .await
    }

    /// Enqueues `operation` once per message a [`MessageSet`] names, in one
    /// statement, and returns the run of rows it wrote.
    ///
    /// # Why one row per message rather than one row for the mailbox
    ///
    /// The queue is a promise about *specific* messages. A single row saying
    /// "move everything in INBOX to Archive" would be resolved by the drainer
    /// later — by which time mail that arrived after the user acted would be
    /// sitting in INBOX too, and the drainer would file it away. That is not a
    /// slower version of the right answer; it is the user losing mail they
    /// never saw. So the set is resolved *now*, against the mailbox as it was
    /// when the key was pressed, and each row carries the message whose UID the
    /// drainer will need.
    ///
    /// What must not happen is one statement per row, and does not: this is a
    /// single `INSERT ... SELECT` over the same index the move uses. The rows
    /// it writes cost the server nothing extra either — `postio-sync`'s
    /// coalescer folds a run of identical moves before any of it is sent.
    ///
    /// Returns `None` when the set named nothing, which is not an error: a
    /// whole-mailbox action over an empty mailbox has simply nothing to do.
    pub async fn enqueue_set(
        &self,
        account_id: AccountId,
        set: &MessageSet,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<Option<OperationRange>> {
        let account_id = require_persisted(account_id.get(), "account")?;
        let payload = encode(operation)?;
        let encoded_inverse = operation.inverse().as_ref().map(encode).transpose()?;
        let mailbox_id = operation.mailbox().filter(|id| id.is_assigned());
        sql::in_scope(self.connection, |scope| async move {
            // Read before writing: `first` is one past whatever the queue already
            // held, so the run this returns cannot swallow a row somebody else
            // wrote. Taking it from `last - changes + 1` instead would assume the
            // statement's rowids came out contiguous, which is true today and is
            // not something the schema promises.
            let highest = sql::scalar(
                &scope,
                "SELECT coalesce(max(id), 0) FROM operation_queue",
                (),
            )
            .await?;

            // `ORDER BY messages.id` is not decoration: without it the queue's
            // own order is whatever order the planner happened to walk the rows
            // in, and that is a property of the *indexes*, not of this statement.
            // #638 widened the list indexes and the order silently reversed --
            // the planner could suddenly answer the predicate from
            // `idx_messages_list`, which is keyed `received_at DESC, id DESC`,
            // where before it had scanned in rowid order. The queue drains in id
            // order, so this decides the order operations reach the server.
            let (predicate, arguments) = set.predicate(8);
            let sql = format!(
                "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                              mailbox_id, payload, inverse, state, attempts,
                                              created_at, updated_at, source_remote_id)
                 SELECT ?1, ?2, 'message', messages.id, ?3, ?4, ?5, ?6, 0, ?7, ?7,
                        messages.remote_id
                   FROM messages
                  WHERE {predicate}
                  ORDER BY messages.id"
            );
            let mut parameters = vec![
                turso::Value::from(account_id),
                turso::Value::from(operation.op_type().to_owned()),
                turso::Value::from(mailbox_id.map(MailboxId::get)),
                turso::Value::from(payload),
                turso::Value::from(encoded_inverse),
                turso::Value::from(OperationState::Pending.as_str().to_owned()),
                turso::Value::from(to_millis(at)),
            ];
            parameters.extend(arguments.into_iter().map(turso::Value::from));
            let written = sql::execute(&scope, &sql, parameters).await?;
            if written == 0 {
                return Ok(None);
            }
            let last = OperationId::new(scope.last_insert_rowid());

            // One statement rather than `refresh_pending_flag` per row: the flag
            // says "this message has something queued", and every row this wrote
            // is a message that now does.
            let (predicate, arguments) = set.predicate(1);
            sql::execute(
                &scope,
                &format!("UPDATE messages SET has_pending_operations = 1 WHERE {predicate}"),
                arguments,
            )
            .await?;

            Ok(Some(OperationRange::new(
                OperationId::new(highest + 1),
                last,
            )))
        })
        .await
    }

    /// Enqueues `operation` once for each of `ids`, in one statement.
    ///
    /// The named twin of [`enqueue_set`](Self::enqueue_set): a multi-select
    /// has no mailbox predicate to enqueue against — the caller already
    /// resolved it to a list of ids — so this takes that list directly rather
    /// than building a throwaway [`super::MessageSet`] around it. What both
    /// share is the shape that matters: one `INSERT ... SELECT` and one flag
    /// `UPDATE`, whatever the length of `ids`, rather than a loop calling
    /// [`enqueue`](Self::enqueue) once per message. A 500-row multi-select
    /// through that loop is 500 savepoints and 500
    /// `has_pending_operations` refreshes for the same net effect.
    ///
    /// Does nothing, successfully, for an empty `ids` — a verb with an empty
    /// selection has already been rejected further up before this is ever
    /// called.
    pub async fn enqueue_many(
        &self,
        account_id: AccountId,
        ids: &[MessageId],
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let account_id = require_persisted(account_id.get(), "account")?;
        let payload = encode(operation)?;
        let encoded_inverse = operation.inverse().as_ref().map(encode).transpose()?;
        let mailbox_id = operation.mailbox().filter(|id| id.is_assigned());
        sql::in_scope(self.connection, |scope| async move {
            let sql = format!(
                "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                              mailbox_id, payload, inverse, state, attempts,
                                              created_at, updated_at, source_remote_id)
                 SELECT ?1, ?2, 'message', messages.id, ?3, ?4, ?5, ?6, 0, ?7, ?7,
                        messages.remote_id
                   FROM messages
                  WHERE messages.id IN ({})",
                super::messages::placeholders(ids.len(), 8)
            );
            let mut parameters = vec![
                turso::Value::from(account_id),
                turso::Value::from(operation.op_type().to_owned()),
                turso::Value::from(mailbox_id.map(MailboxId::get)),
                turso::Value::from(payload),
                turso::Value::from(encoded_inverse),
                turso::Value::from(OperationState::Pending.as_str().to_owned()),
                turso::Value::from(to_millis(at)),
            ];
            parameters.extend(
                ids.iter()
                    .map(|id| turso::Value::from(id.get()))
                    .collect::<Vec<_>>(),
            );
            sql::execute(&scope, &sql, parameters).await?;

            // One statement rather than `refresh_pending_flag` per row, same as
            // `enqueue_set`: every id this wrote a row for is a message that now
            // has something queued.
            let flag_sql = format!(
                "UPDATE messages SET has_pending_operations = 1 WHERE id IN ({})",
                super::messages::placeholders(ids.len(), 1)
            );
            sql::execute(
                &scope,
                &flag_sql,
                ids.iter().map(|id| id.get()).collect::<Vec<_>>(),
            )
            .await?;

            Ok(())
        })
        .await
    }

    /// One row.
    pub async fn get(&self, id: OperationId) -> Result<Option<QueuedOperation>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT {COLUMNS} FROM operation_queue WHERE id = ?1"),
        )
        .await?;
        crate::sql::first_of(&mut statement, [id.get()], read_queued).await
    }

    /// The account's operations that are due to be drained at `now`, in order.
    ///
    /// Ascending id, which is enqueue order, which is the order the user
    /// performed the actions in. A row whose backoff has not elapsed is
    /// skipped and keeps its place for the next pass.
    pub async fn pending(
        &self,
        account_id: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<QueuedOperation>> {
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT {COLUMNS} FROM operation_queue
              WHERE account_id = ?1 AND state = ?2
                AND (next_attempt_at IS NULL OR next_attempt_at <= ?3)
              ORDER BY id"
            ),
        )
        .await?;
        sql::mapped(
            &mut statement,
            bind![
                account_id.get(),
                OperationState::Pending.as_str(),
                to_millis(now)
            ],
            read_queued,
        )
        .await
    }

    /// The unsettled row against `target`, if there is one.
    ///
    /// A target has at most one row in `pending`/`in_flight` at a time in
    /// practice — the caller that would enqueue a second one guards against
    /// it — so the earliest by id (there is only ever the one) is what this
    /// returns. Used by [`super::DraftRepository::cancel_send`] to find the
    /// `Send` a queued draft is waiting on.
    pub async fn pending_for(&self, target: OperationTarget) -> Result<Option<QueuedOperation>> {
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT {COLUMNS} FROM operation_queue
              WHERE target_kind = ?1 AND target_id = ?2 AND state IN ('pending', 'in_flight')
              ORDER BY id
              LIMIT 1"
            ),
        )
        .await?;
        crate::sql::first_of(
            &mut statement,
            bind![target.kind(), target.id()],
            read_queued,
        )
        .await
    }

    /// Why the last attempt against `target` gave up, if one did.
    ///
    /// [`pending_for`](Self::pending_for) answers for work still in progress;
    /// this answers for work that stopped. A surface reopening a failed draft
    /// has a `DraftId` and nothing else, and the reason lives on a queue row
    /// keyed by target — without a query from one to the other the reason is
    /// durable and unreachable, which is exactly what it was (#1487).
    ///
    /// Read rather than copied onto the draft: one source of truth, and a
    /// second copy is one that can come to disagree with the first about why
    /// a send failed.
    ///
    /// The most recent, since a target may have failed more than once and the
    /// current reason is the one worth saying.
    pub async fn last_failure_for(&self, target: OperationTarget) -> Result<Option<String>> {
        let reason: Option<Option<String>> = sql::first(
            self.connection,
            "SELECT last_error FROM operation_queue
                  WHERE target_kind = ?1 AND target_id = ?2 AND state = 'failed'
                  ORDER BY id DESC
                  LIMIT 1",
            bind![target.kind(), target.id()],
            |row| row.col(0),
        )
        .await?;
        // Flattened: no failed row and a failed row that recorded no reason
        // are the same answer to "what should I tell them", and a caller that
        // had to tell them apart would have nothing to do with the
        // difference.
        Ok(reason.flatten())
    }

    /// Whether anything unsettled is queued against `target`.
    pub async fn has_pending(&self, target: OperationTarget) -> Result<bool> {
        let count: i64 = sql::one(
            self.connection,
            "SELECT count(*) FROM operation_queue
              WHERE target_kind = ?1 AND target_id = ?2 AND state IN ('pending', 'in_flight')",
            bind![target.kind(), target.id()],
            |row| row.col(0),
        )
        .await?;
        Ok(count > 0)
    }

    /// Whether anything queued against `mailbox` is still unreconciled.
    ///
    /// Wider than [`has_pending`](Self::has_pending) in two ways, and both are
    /// deliberate. It asks by *mailbox* rather than by target, because the
    /// question behind it is about the mailbox as a whole: does its local
    /// contents already reflect a mutation the server has not been told about?
    /// And it counts `failed` alongside `pending` and `in_flight`, because a
    /// failed row is precisely that — the local move stands, the server never
    /// heard, and only the user can clear it.
    ///
    /// What it is for: a mailbox holding fewer messages than the server's
    /// `EXISTS` has either lost mail or is carrying a local move the server
    /// has not caught up with, and the row counts alone cannot tell those
    /// apart. This can. See `resync`'s short-of-`EXISTS` path — without it,
    /// re-enumerating would refetch the very message the user just archived
    /// and put it back in the mailbox they took it out of.
    pub async fn has_unsettled_in(&self, mailbox: MailboxId) -> Result<bool> {
        let count: i64 = sql::one(
            self.connection,
            "SELECT count(*) FROM operation_queue
              WHERE mailbox_id = ?1
                AND state IN ('pending', 'in_flight', 'failed')",
            bind![mailbox.get()],
            |row| row.col(0),
        )
        .await?;
        Ok(count > 0)
    }

    /// Records why a row ended up the way it did, without changing its state.
    ///
    /// For the outcome that is neither success nor failure: an operation the
    /// server had nothing to do about. The row is done, but *why* it was done
    /// without a round trip is the difference between an explicable bug report
    /// and a mystery.
    pub async fn note(&self, id: OperationId, note: &str) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE operation_queue SET last_error = ?2 WHERE id = ?1",
            bind![id.get(), note],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "operation",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Hands a row to the drainer.
    pub async fn mark_in_flight(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::InFlight, at, None, false)
            .await
    }

    /// Records that the server applied a row.
    pub async fn mark_done(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::Done, at, None, false)
            .await
    }

    /// Gives up on a row. Only the user clears it from here.
    pub async fn mark_failed(&self, id: OperationId, at: DateTime<Utc>, error: &str) -> Result<()> {
        self.set_state(id, OperationState::Failed, at, Some(error), true)
            .await
    }

    /// Puts a row back in the queue, not to be tried again before `retry_at`.
    ///
    /// The backoff schedule itself is the drainer's; this only records the
    /// decision.
    pub async fn defer(&self, id: OperationId, retry_at: DateTime<Utc>, error: &str) -> Result<()> {
        sql::in_scope(self.connection, |scope| async move {
            let changed = sql::execute(
                &scope,
                "UPDATE operation_queue
                    SET state = ?2, attempts = attempts + 1, last_error = ?3,
                        next_attempt_at = ?4, updated_at = ?4
                  WHERE id = ?1",
                bind![
                    id.get(),
                    OperationState::Pending.as_str(),
                    error,
                    to_millis(retry_at),
                ],
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "operation",
                    id: id.get(),
                });
            }
            Ok(())
        })
        .await
    }

    /// Returns every in-flight row in an account to pending, and says how many.
    ///
    /// For start-up after a crash. A row left in flight is *not* known to have
    /// failed — the server may have applied it and died before the reply — so
    /// it is retried rather than dropped, and operations are written to be
    /// idempotent precisely so that retrying is safe.
    pub async fn requeue_in_flight(
        &self,
        account_id: AccountId,
        at: DateTime<Utc>,
    ) -> Result<usize> {
        let changed = sql::execute(
            self.connection,
            "UPDATE operation_queue
                SET state = ?2, next_attempt_at = NULL, updated_at = ?3
              WHERE account_id = ?1 AND state = ?4",
            bind![
                account_id.get(),
                OperationState::Pending.as_str(),
                to_millis(at),
                OperationState::InFlight.as_str(),
            ],
        )
        .await?;
        Ok(changed as usize)
    }

    /// Drops a row, returning whether there was one.
    ///
    /// This is how a send is cancelled: an operation that has not drained yet
    /// can simply be taken off the queue, which is a better undo than any
    /// compensating operation.
    pub async fn delete(&self, id: OperationId) -> Result<bool> {
        sql::in_scope(self.connection, |scope| async move {
            let target = self.get(id).await?.map(|queued| queued.target);
            let deleted = sql::execute(
                &scope,
                "DELETE FROM operation_queue WHERE id = ?1",
                [id.get()],
            )
            .await?;
            if let Some(target) = target {
                refresh_pending_flag(&scope, target).await?;
            }
            Ok(deleted > 0)
        })
        .await
    }

    /// Removes settled rows older than `before`, returning how many went.
    ///
    /// Done rows are kept for a while so a late undo can still find them; this
    /// is the sweep that stops the table growing without bound.
    pub async fn prune_settled(
        &self,
        account_id: AccountId,
        before: DateTime<Utc>,
    ) -> Result<usize> {
        let removed = sql::execute(
            self.connection,
            "DELETE FROM operation_queue
              WHERE account_id = ?1 AND state IN ('done', 'failed') AND updated_at < ?2",
            bind![account_id.get(), to_millis(before)],
        )
        .await?;
        Ok(removed as usize)
    }

    async fn set_state(
        &self,
        id: OperationId,
        state: OperationState,
        at: DateTime<Utc>,
        error: Option<&str>,
        count_attempt: bool,
    ) -> Result<()> {
        sql::in_scope(self.connection, |scope| async move {
            let Some(queued) = OperationQueueRepository::new(&scope).get(id).await? else {
                return Err(Error::NotFound {
                    entity: "operation",
                    id: id.get(),
                });
            };

            sql::execute(
                &scope,
                "UPDATE operation_queue
                    SET state = ?2,
                        attempts = attempts + ?3,
                        last_error = coalesce(?4, last_error),
                        updated_at = ?5
                  WHERE id = ?1",
                bind![
                    id.get(),
                    state.as_str(),
                    i64::from(count_attempt),
                    error,
                    to_millis(at),
                ],
            )
            .await?;
            refresh_pending_flag(&scope, queued.target).await?;
            Ok(())
        })
        .await
    }
}

/// Recomputes `messages.has_pending_operations` for whatever `target` covers.
///
/// The message list reads that column rather than joining the queue, which is
/// what keeps a page of rows at a fixed number of queries and inside the 16 ms
/// interaction budget. Only message-shaped targets have one; an operation on a
/// mailbox or the account itself has no per-message flag to keep true.
async fn refresh_pending_flag(connection: &Connection, target: OperationTarget) -> Result<()> {
    let selector = match target {
        OperationTarget::Message(_) => "id = ?1",
        OperationTarget::Thread(_) => "thread_id = ?1",
        OperationTarget::Mailbox(_) | OperationTarget::Draft(_) | OperationTarget::Account(_) => {
            return Ok(());
        }
    };

    sql::execute(
        connection,
        &format!(
            "UPDATE messages
                SET has_pending_operations = EXISTS (
                        SELECT 1 FROM operation_queue q
                         WHERE q.target_kind = ?2 AND q.target_id = ?1
                           AND q.state IN ('pending', 'in_flight'))
              WHERE {selector}"
        ),
        bind![target.id(), target.kind()],
    )
    .await?;
    Ok(())
}

fn encode(operation: &Operation) -> Result<String> {
    serde_json::to_string(operation).map_err(|source| Error::CorruptPayload {
        column: "payload",
        source,
    })
}

fn decode(column: &'static str, json: &str) -> Result<Operation> {
    serde_json::from_str(json).map_err(|source| Error::CorruptPayload { column, source })
}

fn read_queued(row: &Row) -> Result<QueuedOperation> {
    let target_kind: String = row.col(3)?;
    let target = OperationTarget::from_parts(&target_kind, row.col(4)?)
        .ok_or_else(|| unknown_enum("operation_queue.target_kind", target_kind))?;
    let state: String = row.col(8)?;
    let state = OperationState::from_name(&state)
        .ok_or_else(|| unknown_enum("operation_queue.state", state))?;

    let payload: String = row.col(6)?;
    let inverse: Option<String> = row.col(7)?;

    Ok(QueuedOperation {
        id: OperationId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        target,
        operation: decode("payload", &payload)?,
        inverse: inverse
            .as_deref()
            .map(|json| decode("inverse", json))
            .transpose()?,
        mailbox_id: row.col::<Option<i64>>(5)?.map(MailboxId::new),
        state,
        attempts: row.col::<i64>(9)? as u32,
        last_error: row.col(10)?,
        next_attempt_at: row.col::<Option<i64>>(11)?.map(from_millis),
        created_at: from_millis(row.col(12)?),
        updated_at: from_millis(row.col(13)?),
        source_remote_id: row.col::<Option<String>>(14)?.map(RemoteId::new),
    })
}
