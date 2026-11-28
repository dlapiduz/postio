//! The local-first mutation queue: every mutating action writes SQLite and enqueues here in the
//! same transaction, then repaints; the network happens elsewhere later. That makes undo a replay
//! and keeps the UI off the network.
//! The vocabulary belongs to [`postio_model::Operation`]; `op_type` carries no `CHECK` so adding an
//! operation needs no migration.

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedOperation {
    pub id: OperationId,
    pub account_id: AccountId,
    pub target: OperationTarget,
    pub operation: Operation,
    pub inverse: Option<Operation>,
    pub mailbox_id: Option<MailboxId>,
    pub state: OperationState,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Snapshotted for message-target rows: the local half of a Move or Delete nulls the live row's
    /// coordinates in the same transaction (#289). The backend's own identity (#543); `None` for
    /// non-message targets and never-uploaded messages, where the drainer falls back to the live
    /// row.
    pub source_remote_id: Option<RemoteId>,
}

/// Every method takes a borrowed [`Connection`] and its atomic scopes are savepoints, so an enqueue
/// nests inside the transaction performing the local write. Enqueue first, then write locally: a
/// Move or Delete nulls the server coordinates the enqueue snapshots (#289). Enqueueing outside the
/// local write is the bug this shape prevents.
#[derive(Debug)]
pub struct OperationQueueRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "\
id, account_id, op_type, target_kind, target_id, mailbox_id, payload, inverse, state,
attempts, last_error, next_attempt_at, created_at, updated_at, source_remote_id";

impl<'a> OperationQueueRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The inverse is stored now, not when undo is pressed: by then the mailbox may no longer be
    /// where the message was.
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

    /// Reuses the gate [`defer`](Self::defer) puts a backed-off retry behind
    /// ([`pending`](Self::pending) skips rows whose `next_attempt_at` is in the future), so it is
    /// restart-safe: the row is read from SQLite each drain pass. `attempts` stays `0` and
    /// `last_error` `None`, so a scheduled send does not read like a retry.
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

            // Read before the caller's local write nulls it, which is why enqueue precedes the move
            // in every Move and Delete path (#289). A non-message target, or a missing row,
            // snapshots nothing.
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

    /// One row per message, resolved now: a single "move everything in INBOX" row would be resolved
    /// by the drainer later, after mail that arrived since had landed in INBOX, and file it away
    /// unseen. One `INSERT ... SELECT` over the move's index; `postio-sync`'s coalescer folds a run
    /// of identical moves before any is sent.
    ///
    /// Returns `None` when the set named nothing.
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
            // Read before writing: `first` is one past what the queue held, so the run cannot
            // swallow another row; `last - changes + 1` would assume contiguous rowids, which the
            // schema does not promise.
            let highest = sql::scalar(
                &scope,
                "SELECT coalesce(max(id), 0) FROM operation_queue",
                (),
            )
            .await?;

            // `ORDER BY messages.id` is not decoration: without it the queue order is whatever the
            // planner walked, a property of the indexes. #638 widened the list indexes and the
            // order silently reversed. The queue drains in id order, so this decides the order
            // operations reach the server.
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

            // One statement, not `refresh_pending_flag` per row: every row written is a message
            // that now has something queued.
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

    /// The named twin of [`enqueue_set`](Self::enqueue_set) for a multi-select already resolved to
    /// ids: one `INSERT ... SELECT` and one flag `UPDATE` whatever the length, where a loop over
    /// [`enqueue`](Self::enqueue) is 500 savepoints and refreshes for 500 rows. Does nothing for an
    /// empty `ids`.
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

    /// For a flag change queued ahead of a move: the move's local half nulls the live row's
    /// coordinates, a flag row carries a snapshot but no mailbox, so the move still names both, and
    /// the queue drains in order.
    pub async fn leaving_after(
        &self,
        message: MessageId,
        after: OperationId,
    ) -> Result<Option<(MailboxId, RemoteId)>> {
        let found: Option<(i64, String)> = sql::first(
            self.connection,
            "SELECT mailbox_id, source_remote_id FROM operation_queue
              WHERE target_kind = 'message' AND target_id = ?1 AND id > ?2
                AND op_type IN ('move', 'delete')
                AND state IN ('pending', 'in_flight')
                AND mailbox_id IS NOT NULL AND source_remote_id IS NOT NULL
              ORDER BY id LIMIT 1",
            bind![message.get(), after.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        Ok(found.map(|(mailbox, remote_id)| (MailboxId::new(mailbox), RemoteId::new(remote_id))))
    }

    pub async fn get(&self, id: OperationId) -> Result<Option<QueuedOperation>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT {COLUMNS} FROM operation_queue WHERE id = ?1"),
        )
        .await?;
        crate::sql::first_of(&mut statement, [id.get()], read_queued).await
    }

    /// Ascending id is enqueue order. A row whose backoff has not elapsed is skipped and keeps its
    /// place.
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

    /// The earliest by id; in practice a target has one `pending`/`in_flight` row. Used by
    /// [`super::DraftRepository::cancel_send`].
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

    /// [`pending_for`](Self::pending_for) answers for work in progress; this for work that stopped.
    /// A surface reopening a failed draft has only a `DraftId`, and the reason lives on a queue row
    /// keyed by target (#1487). Read, not copied onto the draft, so two copies cannot disagree. The
    /// most recent failure wins.
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
        // Flattened: no failed row and a failed row with no reason are the same answer to "what
        // should I tell them".
        Ok(reason.flatten())
    }

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

    /// Asks by mailbox, not target, and counts `failed` alongside `pending` and `in_flight`: a
    /// failed row is a local move the server never heard of, and only the user clears it.
    ///
    /// For `resync`'s short-of-`EXISTS` path: a mailbox with fewer messages than `EXISTS` has
    /// either lost mail or carries an unsent local move, and without this re-enumerating would
    /// refetch the message the user just archived.
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

    /// For an operation the server had nothing to do about: why it was done without a round trip is
    /// the difference between an explicable bug report and a mystery.
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

    pub async fn mark_in_flight(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::InFlight, at, None, false)
            .await
    }

    pub async fn mark_done(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::Done, at, None, false)
            .await
    }

    pub async fn mark_failed(&self, id: OperationId, at: DateTime<Utc>, error: &str) -> Result<()> {
        self.set_state(id, OperationState::Failed, at, Some(error), true)
            .await
    }

    /// The backoff schedule is the drainer's; this records the decision.
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

    /// For start-up after a crash: a row left in flight is not known to have failed (the server may
    /// have applied it), so it is retried, and operations are idempotent so that is safe.
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

    /// How a send is cancelled: an operation that has not drained can be taken off the queue, a
    /// better undo than a compensating operation.
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

    /// Done rows are kept for a while so a late undo can find them.
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

/// The message list reads that column rather than joining the queue, keeping a page at a fixed
/// number of queries inside the 16 ms budget. Only message-shaped targets have one.
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
