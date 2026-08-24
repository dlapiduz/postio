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
    OperationTarget,
};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params, params_from_iter};

use super::{MessageSet, from_millis, require_persisted, to_millis, unknown_enum};
use crate::error::{Error, Result};

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
}

impl QueuedOperation {
    /// Whether undo can be offered for this row.
    pub fn is_undoable(&self) -> bool {
        self.inverse.is_some()
    }
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
/// # fn main() -> Result<(), postio_storage::Error> {
/// # let database = postio_storage::Database::open("postio.db")?;
/// # let mut connection = database.connection()?;
/// # let (account, message) = (AccountId::new(1), MessageId::new(1));
/// # let (inbox, archive) = (MailboxId::new(1), MailboxId::new(2));
/// let transaction = connection.transaction()?;
/// // ... move the message locally ...
/// OperationQueueRepository::new(&transaction).enqueue(
///     account,
///     OperationTarget::Message(message),
///     &Operation::Move { from: inbox, to: archive },
///     chrono::Utc::now(),
/// )?;
/// transaction.commit()?;
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
attempts, last_error, next_attempt_at, created_at, updated_at";

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
    pub fn enqueue(
        &self,
        account_id: AccountId,
        target: OperationTarget,
        operation: &Operation,
        at: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        let account_id = require_persisted(account_id.get(), "account")?;
        let scope = super::Scope::open(self.connection)?;

        let payload = encode(operation)?;
        let inverse = operation.inverse();
        let encoded_inverse = inverse.as_ref().map(encode).transpose()?;
        let mailbox_id = operation.mailbox().filter(|id| id.is_assigned());

        scope.execute(
            "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                          mailbox_id, payload, inverse, state, attempts,
                                          created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?9)",
            params![
                account_id,
                operation.op_type(),
                target.kind(),
                target.id(),
                mailbox_id.map(MailboxId::get),
                payload,
                encoded_inverse,
                OperationState::Pending.as_str(),
                to_millis(at),
            ],
        )?;
        let id = OperationId::new(scope.last_insert_rowid());

        refresh_pending_flag(&scope, target)?;
        scope.commit()?;

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
            next_attempt_at: None,
            created_at: at,
            updated_at: at,
        })
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
    pub fn enqueue_set(
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
        let scope = super::Scope::open(self.connection)?;

        // Read before writing: `first` is one past whatever the queue already
        // held, so the run this returns cannot swallow a row somebody else
        // wrote. Taking it from `last - changes + 1` instead would assume the
        // statement's rowids came out contiguous, which is true today and is
        // not something the schema promises.
        let highest: i64 = scope.query_row(
            "SELECT coalesce(max(id), 0) FROM operation_queue",
            [],
            |row| row.get(0),
        )?;

        let (predicate, mut arguments) = set.predicate(8);
        let sql = format!(
            "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                          mailbox_id, payload, inverse, state, attempts,
                                          created_at, updated_at)
             SELECT ?1, ?2, 'message', messages.id, ?3, ?4, ?5, ?6, 0, ?7, ?7
               FROM messages
              WHERE {predicate}"
        );
        let mut parameters = vec![
            Value::from(account_id),
            Value::from(operation.op_type().to_owned()),
            Value::from(mailbox_id.map(MailboxId::get)),
            Value::from(payload),
            Value::from(encoded_inverse),
            Value::from(OperationState::Pending.as_str().to_owned()),
            Value::from(to_millis(at)),
        ];
        parameters.extend(arguments.drain(..).map(Value::from));
        let written = scope.execute(&sql, params_from_iter(parameters))?;
        if written == 0 {
            scope.commit()?;
            return Ok(None);
        }
        let last = OperationId::new(scope.last_insert_rowid());

        // One statement rather than `refresh_pending_flag` per row: the flag
        // says "this message has something queued", and every row this wrote
        // is a message that now does.
        let (predicate, arguments) = set.predicate(1);
        scope.execute(
            &format!("UPDATE messages SET has_pending_operations = 1 WHERE {predicate}"),
            params_from_iter(arguments),
        )?;
        scope.commit()?;

        Ok(Some(OperationRange::new(
            OperationId::new(highest + 1),
            last,
        )))
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
    pub fn enqueue_many(
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
        let scope = super::Scope::open(self.connection)?;

        let sql = format!(
            "INSERT INTO operation_queue (account_id, op_type, target_kind, target_id,
                                          mailbox_id, payload, inverse, state, attempts,
                                          created_at, updated_at)
             SELECT ?1, ?2, 'message', messages.id, ?3, ?4, ?5, ?6, 0, ?7, ?7
               FROM messages
              WHERE messages.id IN ({})",
            super::messages::placeholders(ids.len(), 8)
        );
        let mut parameters = vec![
            Value::from(account_id),
            Value::from(operation.op_type().to_owned()),
            Value::from(mailbox_id.map(MailboxId::get)),
            Value::from(payload),
            Value::from(encoded_inverse),
            Value::from(OperationState::Pending.as_str().to_owned()),
            Value::from(to_millis(at)),
        ];
        parameters.extend(ids.iter().map(|id| Value::from(id.get())));
        scope.execute(&sql, params_from_iter(parameters))?;

        // One statement rather than `refresh_pending_flag` per row, same as
        // `enqueue_set`: every id this wrote a row for is a message that now
        // has something queued.
        let flag_sql = format!(
            "UPDATE messages SET has_pending_operations = 1 WHERE id IN ({})",
            super::messages::placeholders(ids.len(), 1)
        );
        scope.execute(&flag_sql, params_from_iter(ids.iter().map(|id| id.get())))?;
        scope.commit()?;

        Ok(())
    }

    /// Enqueues the inverse of a row that is already queued — this is undo.
    ///
    /// It goes onto the same queue, in the same state, behind everything
    /// already there. There is deliberately no separate undo path: whatever
    /// retry, backoff and conflict handling the drainer grows applies to undo
    /// for free.
    pub fn enqueue_inverse(
        &self,
        queued: &QueuedOperation,
        at: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        let inverse = queued.inverse.as_ref().ok_or(Error::NotUndoable {
            op_type: queued.operation.op_type(),
        })?;
        self.enqueue(queued.account_id, queued.target, inverse, at)
    }

    /// One row.
    pub fn get(&self, id: OperationId) -> Result<Option<QueuedOperation>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM operation_queue WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        rows.next()?.map(read_queued).transpose()
    }

    /// The account's operations that are due to be drained at `now`, in order.
    ///
    /// Ascending id, which is enqueue order, which is the order the user
    /// performed the actions in. A row whose backoff has not elapsed is
    /// skipped and keeps its place for the next pass.
    pub fn pending(
        &self,
        account_id: AccountId,
        now: DateTime<Utc>,
    ) -> Result<Vec<QueuedOperation>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM operation_queue
              WHERE account_id = ?1 AND state = ?2
                AND (next_attempt_at IS NULL OR next_attempt_at <= ?3)
              ORDER BY id"
        ))?;
        let rows = statement.query_and_then(
            params![
                account_id.get(),
                OperationState::Pending.as_str(),
                to_millis(now)
            ],
            read_queued,
        )?;
        rows.collect()
    }

    /// Whether anything unsettled is queued against `target`.
    pub fn has_pending(&self, target: OperationTarget) -> Result<bool> {
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM operation_queue
              WHERE target_kind = ?1 AND target_id = ?2 AND state IN ('pending', 'in_flight')",
            params![target.kind(), target.id()],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Records why a row ended up the way it did, without changing its state.
    ///
    /// For the outcome that is neither success nor failure: an operation the
    /// server had nothing to do about. The row is done, but *why* it was done
    /// without a round trip is the difference between an explicable bug report
    /// and a mystery.
    pub fn note(&self, id: OperationId, note: &str) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE operation_queue SET last_error = ?2 WHERE id = ?1",
            params![id.get(), note],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "operation",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Hands a row to the drainer.
    pub fn mark_in_flight(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::InFlight, at, None, false)
    }

    /// Records that the server applied a row.
    pub fn mark_done(&self, id: OperationId, at: DateTime<Utc>) -> Result<()> {
        self.set_state(id, OperationState::Done, at, None, false)
    }

    /// Gives up on a row. Only the user clears it from here.
    pub fn mark_failed(&self, id: OperationId, at: DateTime<Utc>, error: &str) -> Result<()> {
        self.set_state(id, OperationState::Failed, at, Some(error), true)
    }

    /// Puts a row back in the queue, not to be tried again before `retry_at`.
    ///
    /// The backoff schedule itself is the drainer's; this only records the
    /// decision.
    pub fn defer(&self, id: OperationId, retry_at: DateTime<Utc>, error: &str) -> Result<()> {
        let scope = super::Scope::open(self.connection)?;
        let changed = scope.execute(
            "UPDATE operation_queue
                SET state = ?2, attempts = attempts + 1, last_error = ?3,
                    next_attempt_at = ?4, updated_at = ?4
              WHERE id = ?1",
            params![
                id.get(),
                OperationState::Pending.as_str(),
                error,
                to_millis(retry_at),
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "operation",
                id: id.get(),
            });
        }
        scope.commit()?;
        Ok(())
    }

    /// Returns every in-flight row in an account to pending, and says how many.
    ///
    /// For start-up after a crash. A row left in flight is *not* known to have
    /// failed — the server may have applied it and died before the reply — so
    /// it is retried rather than dropped, and operations are written to be
    /// idempotent precisely so that retrying is safe.
    pub fn requeue_in_flight(&self, account_id: AccountId, at: DateTime<Utc>) -> Result<usize> {
        let changed = self.connection.execute(
            "UPDATE operation_queue
                SET state = ?2, next_attempt_at = NULL, updated_at = ?3
              WHERE account_id = ?1 AND state = ?4",
            params![
                account_id.get(),
                OperationState::Pending.as_str(),
                to_millis(at),
                OperationState::InFlight.as_str(),
            ],
        )?;
        Ok(changed)
    }

    /// Drops a row, returning whether there was one.
    ///
    /// This is how a send is cancelled: an operation that has not drained yet
    /// can simply be taken off the queue, which is a better undo than any
    /// compensating operation.
    pub fn delete(&self, id: OperationId) -> Result<bool> {
        let scope = super::Scope::open(self.connection)?;
        let target = self.get(id)?.map(|queued| queued.target);
        let deleted = scope.execute("DELETE FROM operation_queue WHERE id = ?1", [id.get()])?;
        if let Some(target) = target {
            refresh_pending_flag(&scope, target)?;
        }
        scope.commit()?;
        Ok(deleted > 0)
    }

    /// Removes settled rows older than `before`, returning how many went.
    ///
    /// Done rows are kept for a while so a late undo can still find them; this
    /// is the sweep that stops the table growing without bound.
    pub fn prune_settled(&self, account_id: AccountId, before: DateTime<Utc>) -> Result<usize> {
        let removed = self.connection.execute(
            "DELETE FROM operation_queue
              WHERE account_id = ?1 AND state IN ('done', 'failed') AND updated_at < ?2",
            params![account_id.get(), to_millis(before)],
        )?;
        Ok(removed)
    }

    fn set_state(
        &self,
        id: OperationId,
        state: OperationState,
        at: DateTime<Utc>,
        error: Option<&str>,
        count_attempt: bool,
    ) -> Result<()> {
        let scope = super::Scope::open(self.connection)?;
        let Some(queued) = OperationQueueRepository::new(&scope).get(id)? else {
            return Err(Error::NotFound {
                entity: "operation",
                id: id.get(),
            });
        };

        scope.execute(
            "UPDATE operation_queue
                SET state = ?2,
                    attempts = attempts + ?3,
                    last_error = coalesce(?4, last_error),
                    updated_at = ?5
              WHERE id = ?1",
            params![
                id.get(),
                state.as_str(),
                i64::from(count_attempt),
                error,
                to_millis(at),
            ],
        )?;
        refresh_pending_flag(&scope, queued.target)?;
        scope.commit()?;
        Ok(())
    }
}

/// Recomputes `messages.has_pending_operations` for whatever `target` covers.
///
/// The message list reads that column rather than joining the queue, which is
/// what keeps a page of rows at a fixed number of queries and inside the 16 ms
/// interaction budget. Only message-shaped targets have one; an operation on a
/// mailbox or the account itself has no per-message flag to keep true.
fn refresh_pending_flag(connection: &Connection, target: OperationTarget) -> Result<()> {
    let selector = match target {
        OperationTarget::Message(_) => "id = ?1",
        OperationTarget::Thread(_) => "thread_id = ?1",
        OperationTarget::Mailbox(_) | OperationTarget::Draft(_) | OperationTarget::Account(_) => {
            return Ok(());
        }
    };

    connection.execute(
        &format!(
            "UPDATE messages
                SET has_pending_operations = EXISTS (
                        SELECT 1 FROM operation_queue q
                         WHERE q.target_kind = ?2 AND q.target_id = ?1
                           AND q.state IN ('pending', 'in_flight'))
              WHERE {selector}"
        ),
        params![target.id(), target.kind()],
    )?;
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

fn read_queued(row: &Row<'_>) -> Result<QueuedOperation> {
    let target_kind: String = row.get(3)?;
    let target = OperationTarget::from_parts(&target_kind, row.get(4)?)
        .ok_or_else(|| unknown_enum("operation_queue.target_kind", target_kind))?;
    let state: String = row.get(8)?;
    let state = OperationState::from_name(&state)
        .ok_or_else(|| unknown_enum("operation_queue.state", state))?;

    let payload: String = row.get(6)?;
    let inverse: Option<String> = row.get(7)?;

    Ok(QueuedOperation {
        id: OperationId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        target,
        operation: decode("payload", &payload)?,
        inverse: inverse
            .as_deref()
            .map(|json| decode("inverse", json))
            .transpose()?,
        mailbox_id: row.get::<_, Option<i64>>(5)?.map(MailboxId::new),
        state,
        attempts: row.get::<_, i64>(9)? as u32,
        last_error: row.get(10)?,
        next_attempt_at: row.get::<_, Option<i64>>(11)?.map(from_millis),
        created_at: from_millis(row.get(12)?),
        updated_at: from_millis(row.get(13)?),
    })
}
