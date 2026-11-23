//! Drafts: the composer's durable buffer, autosaved on every keystroke. The body is inline because
//! a content-addressed store would accumulate a blob per keystroke; it moves to the blob store when
//! sent.
//! [`DraftRepository::save`] inserts the first time and updates after, so callers need not know
//! which.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, AttachmentId, BlobId, Disposition, Draft, DraftId, DraftKind,
    DraftState, EmailAddress, IdentityId, LabelId, MailboxRole, MessageBody, MessageId, ModSeq,
    Operation, OperationTarget, RemoteId, RfcMessageId, ServerIdentifiers, ThreadId, Uid,
    UidValidity,
};

/// The wire pair is what the uid-range pull still enumerates by (ADR 0018 Q3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCopyLocation {
    pub remote_id: RemoteId,
    pub uid: Uid,
    pub uid_validity: UidValidity,
}

use super::{OperationQueueRepository, QueuedOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSendOutcome {
    Cancelled,
    NotQueued,
    AlreadyInFlight,
}
use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

#[derive(Debug)]
pub struct DraftRepository<'a> {
    connection: &'a Connection,
}

const DRAFT_COLUMNS: &str = "\
id, account_id, identity_id, kind, in_reply_to_message_id, thread_id, subject, body_text,
body_html, rich, state, uid, uid_validity, mod_seq, remote_id, created_at, updated_at,
rfc_message_id, forwarded_message_id, body_markdown, label_ids, calendar_reply,
remind_at";

impl<'a> DraftRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Idempotent: the draft's id and its attachments' ids do not change between saves.
    ///
    /// `uid`, `uid_validity`, `mod_seq` and `remote_id` are cleared only by
    /// [`set_server_copy`](Self::set_server_copy), never by a save that does not know them: the
    /// composer autosaves a stale value while the drainer records where the server copy landed, and
    /// a wipe would make the next upload add a second copy.
    pub async fn save(&self, draft: &mut Draft) -> Result<DraftId> {
        sql::in_scope(self.connection, |transaction| async move {
            if draft.id.is_assigned() {
                let changed = sql::execute(
                    &transaction,
                    "UPDATE drafts
                        SET account_id = ?2, identity_id = ?3, kind = ?4,
                            in_reply_to_message_id = ?5, thread_id = ?6, subject = ?7,
                            body_text = ?8, body_html = ?9, rich = ?10, state = ?11,
                            uid = coalesce(?12, uid),
                            uid_validity = coalesce(?13, uid_validity),
                            mod_seq = coalesce(?14, mod_seq),
                            remote_id = coalesce(?15, remote_id),
                            updated_at = ?16,
                            rfc_message_id = ?17,
                            forwarded_message_id = ?18,
                            body_markdown = ?19,
                            label_ids = ?20,
                            calendar_reply = ?21,
                            remind_at = ?22
                      WHERE id = ?1",
                    bind![
                        draft.id.get(),
                        draft.account_id.get(),
                        optional_identity(draft.identity_id),
                        draft.kind.as_str(),
                        optional_message(draft.in_reply_to),
                        optional_thread(draft.thread_id),
                        draft.subject,
                        draft.body.text,
                        draft.body.html,
                        draft.rich,
                        draft.state.as_str(),
                        draft.server.uid.map(|uid| i64::from(uid.get())),
                        draft
                            .server
                            .uid_validity
                            .map(|validity| i64::from(validity.get())),
                        draft.server.mod_seq.map(|seq| seq.get() as i64),
                        draft
                            .server
                            .remote_id
                            .as_ref()
                            .map(|id| id.as_str().to_owned()),
                        to_millis(draft.updated_at),
                        reservation_for(draft),
                        optional_message(draft.forwarded_from),
                        draft.body_markdown,
                        label_ids(draft),
                        draft.calendar_reply,
                        draft.remind_at.map(to_millis),
                    ],
                )
                .await?;
                if changed == 0 {
                    return Err(Error::NotFound {
                        entity: "draft",
                        id: draft.id.get(),
                    });
                }
            } else {
                let account_id = require_persisted(draft.account_id.get(), "account")?;
                sql::execute(
                    &transaction,
                    "INSERT INTO drafts (account_id, identity_id, kind, in_reply_to_message_id,
                                         thread_id, subject, body_text, body_html, rich, state,
                                         uid, uid_validity, mod_seq, remote_id, created_at,
                                         updated_at, rfc_message_id, forwarded_message_id,
                                         body_markdown, label_ids, calendar_reply, remind_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
                    bind![
                        account_id,
                        optional_identity(draft.identity_id),
                        draft.kind.as_str(),
                        optional_message(draft.in_reply_to),
                        optional_thread(draft.thread_id),
                        draft.subject,
                        draft.body.text,
                        draft.body.html,
                        draft.rich,
                        draft.state.as_str(),
                        draft.server.uid.map(|uid| i64::from(uid.get())),
                        draft
                            .server
                            .uid_validity
                            .map(|validity| i64::from(validity.get())),
                        draft.server.mod_seq.map(|seq| seq.get() as i64),
                        draft
                            .server
                            .remote_id
                            .as_ref()
                            .map(|id| id.as_str().to_owned()),
                        to_millis(draft.created_at),
                        to_millis(draft.updated_at),
                        reservation_for(draft),
                        optional_message(draft.forwarded_from),
                        draft.body_markdown,
                        label_ids(draft),
                        draft.calendar_reply,
                        draft.remind_at.map(to_millis),
                    ],
                )
                .await?;
                draft.id = DraftId::new(transaction.last_insert_rowid());
            }

            write_recipients(&transaction, draft).await?;
            write_attachments(&transaction, draft).await?;
            list_row(&transaction, draft).await?;

            Ok(draft.id)
        })
        .await
    }

    /// A narrow write, not [`save`](Self::save): the drainer fetching a forward's attachment bytes
    /// (#1686) does not hold the composer's draft, and rewriting the row from a stale copy would
    /// overwrite its saves.
    ///
    /// `false` when `attachment` is not one of `draft`'s.
    pub async fn set_attachment_blob(
        &self,
        draft: DraftId,
        attachment: AttachmentId,
        blob: &BlobId,
    ) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE attachments SET blob_id = ?3 WHERE draft_id = ?1 AND id = ?2",
            bind![draft.get(), attachment.get(), blob.as_str()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// [`save`](Self::save) plus the enqueue in one write: a local write without its queue row
    /// never reaches the server, and the reverse tells it about something the user never saw.
    ///
    /// Returns `None`, having still saved, when the account has no Drafts mailbox yet; the next
    /// save after it appears files it.
    pub async fn save_and_sync(
        &self,
        draft: &mut Draft,
        at: DateTime<Utc>,
    ) -> Result<Option<QueuedOperation>> {
        sql::in_scope(self.connection, |scope| async move {
            DraftRepository::new(&scope).save(draft).await?;
            let queued = match super::MailboxRepository::new(&scope)
                .by_role(draft.account_id, MailboxRole::Drafts)
                .await?
            {
                Some(mailbox) => Some(
                    OperationQueueRepository::new(&scope)
                        .enqueue(
                            draft.account_id,
                            OperationTarget::Draft(draft.id),
                            &Operation::SaveDraft {
                                mailbox: mailbox.id,
                            },
                            at,
                        )
                        .await?,
                ),
                None => None,
            };

            Ok(queued)
        })
        .await
    }

    /// Local-first: the draft goes to [`DraftState::Queued`] and the [`Operation::Send`] row is
    /// written now, so the composer closes without waiting on SMTP.
    ///
    /// Saves rather than marks: autosave is debounced, so a quickly sent draft may have no
    /// [`DraftId`], and the operation names the draft by id (a send carries no bytes).
    ///
    /// The local row stays: `postio-sync::send` treats a `Send` whose draft is gone as obsolete, so
    /// deleting it would drop the message. The drainer deletes it after acceptance and filing in
    /// Sent.
    ///
    /// Always returns a queue row: a send names no mailbox.
    pub async fn queue_send(
        &self,
        draft: &mut Draft,
        at: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        sql::in_scope(self.connection, |scope| async move {
            reserve_message_id(&scope, draft).await?;
            draft.state = DraftState::Queued;
            draft.updated_at = at;
            DraftRepository::new(&scope).save(draft).await?;
            let queued = OperationQueueRepository::new(&scope)
                .enqueue(
                    draft.account_id,
                    OperationTarget::Draft(draft.id),
                    &Operation::Send { draft: draft.id },
                    at,
                )
                .await?;

            Ok(queued)
        })
        .await
    }

    /// As [`queue_send`](Self::queue_send), but [`OperationQueueRepository::pending`] withholds the
    /// row until `send_at`, stored on the row so it survives a restart.
    pub async fn queue_send_at(
        &self,
        draft: &mut Draft,
        at: DateTime<Utc>,
        send_at: DateTime<Utc>,
    ) -> Result<QueuedOperation> {
        sql::in_scope(self.connection, |scope| async move {
            reserve_message_id(&scope, draft).await?;
            draft.state = DraftState::Queued;
            draft.updated_at = at;
            DraftRepository::new(&scope).save(draft).await?;
            let queued = OperationQueueRepository::new(&scope)
                .enqueue_not_before(
                    draft.account_id,
                    OperationTarget::Draft(draft.id),
                    &Operation::Send { draft: draft.id },
                    at,
                    send_at,
                )
                .await?;

            // The chosen time only: a backoff `next_attempt_at` is the retry clock, and showing it
            // would misstate intent.
            sql::execute(
                &scope,
                "UPDATE messages SET send_at = ?2
                  WHERE id IN (SELECT message_id FROM drafts
                                WHERE id = ?1 AND message_id IS NOT NULL)",
                bind![draft.id.get(), to_millis(send_at)],
            )
            .await?;
            Ok(queued)
        })
        .await
    }

    /// Heals a `Queued` draft whose operation gave up (marked `failed`, then pruned): it would sit
    /// in the Outbox for ever.
    ///
    /// `Queued` only, and that is the safety argument: the drainer commits `Sending` before SMTP
    /// (ADR 0021), so a `Queued` draft was never submitted. A `Sending` draft with no operation is
    /// uncertain and belongs to `postio_sync::send::resolve`; touching it could report failed a
    /// message that is in somebody's inbox.
    ///
    /// Returns how many were healed. Idempotent, because it runs on every drain pass.
    pub async fn fail_orphaned_sends(&self, account: AccountId) -> Result<usize> {
        // Read first: this runs every drain pass and the answer is almost always zero, and an
        // `UPDATE` would take the writer each time to find nothing.
        let orphans = sql::scalar(
            self.connection,
            "SELECT count(*) FROM drafts
              WHERE account_id = ?1
                AND state = 'queued'
                AND NOT EXISTS (
                      SELECT 1 FROM operation_queue q
                       WHERE q.target_kind = 'draft'
                         AND q.target_id = drafts.id
                         AND q.op_type = 'send'
                         AND q.state IN ('pending', 'in_flight'))",
            [account.get()],
        )
        .await?;
        if orphans == 0 {
            return Ok(0);
        }

        sql::in_scope(self.connection, |transaction| async move {
            let healed = sql::execute(
                &transaction,
                "UPDATE drafts
                        SET state = 'failed'
                      WHERE account_id = ?1
                        AND state = 'queued'
                        AND NOT EXISTS (
                              SELECT 1 FROM operation_queue q
                               WHERE q.target_kind = 'draft'
                                 AND q.target_id = drafts.id
                                 AND q.op_type = 'send'
                                 AND q.state IN ('pending', 'in_flight'))",
                [account.get()],
            )
            .await?;
            // The mirror row in Drafts (#166) carries the same state and the lists read it, so the
            // heal must update it too.
            sql::execute(
                &transaction,
                "UPDATE messages
                        SET send_state = 'failed'
                      WHERE account_id = ?1
                        AND send_state = 'queued'
                        AND id IN (SELECT message_id FROM drafts
                                    WHERE drafts.state = 'failed'
                                      AND drafts.message_id IS NOT NULL)",
                [account.get()],
            )
            .await?;
            Ok(healed as usize)
        })
        .await
    }

    /// Cancels a pending send and hands the draft back for editing (#433). [`Operation::Send`] has
    /// no [`Operation::inverse`]; instead [`OperationQueueRepository::delete`] takes a
    /// not-yet-draining row off the queue, and local-first means it is still in SQLite until the
    /// drainer marks it.
    ///
    /// Returns [`CancelSendOutcome::AlreadyInFlight`] without touching anything once the drainer
    /// has: a submission may be on the wire, and an editable draft would risk a second, different
    /// message.
    pub async fn cancel_send(&self, id: DraftId, at: DateTime<Utc>) -> Result<CancelSendOutcome> {
        sql::in_scope(self.connection, |scope| async move {
            let drafts = DraftRepository::new(&scope);

            let Some(mut draft) = drafts.get(id).await? else {
                return Ok(CancelSendOutcome::NotQueued);
            };
            // `Sending` and `Sent` are "too late", not "nothing to cancel": the drainer commits
            // `Sending` just before SMTP (ADR 0021), and `NotQueued` would let a caller report a
            // message recalled while on the wire.
            if matches!(draft.state, DraftState::Sending | DraftState::Sent) {
                return Ok(CancelSendOutcome::AlreadyInFlight);
            }
            if draft.state != DraftState::Queued {
                return Ok(CancelSendOutcome::NotQueued);
            }

            let queue = OperationQueueRepository::new(&scope);
            let target = OperationTarget::Draft(id);
            let Some(pending) = queue.pending_for(target).await? else {
                // Settled between the caller's read and this one: too late.
                return Ok(CancelSendOutcome::AlreadyInFlight);
            };
            if pending.state != postio_model::OperationState::Pending {
                return Ok(CancelSendOutcome::AlreadyInFlight);
            }

            queue.delete(pending.id).await?;
            draft.state = DraftState::Editing;
            draft.updated_at = at;
            drafts.save(&mut draft).await?;

            Ok(CancelSendOutcome::Cancelled)
        })
        .await
    }

    /// The local row goes now (local-first), so the queued [`Operation::DiscardDraft`] carries the
    /// `UID` and generation rather than naming the draft.
    ///
    /// Returns `None` when nothing needed queueing: already gone, never uploaded, or no Drafts
    /// mailbox.
    pub async fn discard(&self, id: DraftId, at: DateTime<Utc>) -> Result<Option<QueuedOperation>> {
        sql::in_scope(self.connection, |scope| async move {
            let drafts = DraftRepository::new(&scope);

            let Some(draft) = drafts.get(id).await? else {
                // A retried discard, or one racing a send that cleared the row: expected.
                return Ok(None);
            };

            let queued = match server_copy(&draft) {
                Some(remote_id) => {
                    match super::MailboxRepository::new(&scope)
                        .by_role(draft.account_id, MailboxRole::Drafts)
                        .await?
                    {
                        Some(mailbox) => Some(
                            OperationQueueRepository::new(&scope)
                                .enqueue(
                                    draft.account_id,
                                    OperationTarget::Draft(id),
                                    &Operation::DiscardDraft {
                                        mailbox: mailbox.id,
                                        remote_id,
                                    },
                                    at,
                                )
                                .await?,
                        ),
                        None => None,
                    }
                }
                None => None,
            };

            drafts.delete(id).await?;
            Ok(queued)
        })
        .await
    }

    pub async fn get(&self, id: DraftId) -> Result<Option<Draft>> {
        let mut statement = sql::statement(
            self.connection,
            &format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"),
        )
        .await?;
        let found = crate::sql::first_of(&mut statement, [id.get()], read_draft).await?;
        let Some(mut draft) = found else {
            return Ok(None);
        };

        self.fill(&mut draft).await?;
        Ok(Some(draft))
    }

    /// The `To` recipients in order, for all of them in one statement: what a Drafts or Outbox row
    /// names. A message that is no draft has no entry.
    pub async fn recipients_of(
        &self,
        messages: &[MessageId],
    ) -> Result<std::collections::HashMap<MessageId, Vec<EmailAddress>>> {
        let mut found: std::collections::HashMap<MessageId, Vec<EmailAddress>> =
            std::collections::HashMap::new();
        for chunk in messages.chunks(500) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let mut statement = sql::statement(
                self.connection,
                &format!(
                    "SELECT drafts.message_id, r.name, a.address FROM recipients r
                       JOIN drafts ON drafts.id = r.draft_id
                       JOIN addresses a ON a.id = r.address_id
                      WHERE r.kind = 'to' AND drafts.message_id IN ({placeholders})
                      ORDER BY drafts.message_id, r.position, r.id"
                ),
            )
            .await?;
            let params: Vec<i64> = chunk.iter().map(|id| id.get()).collect();
            let rows = sql::mapped(&mut statement, params, |row| {
                Ok((
                    MessageId::new(row.col::<i64>(0)?),
                    EmailAddress::new(row.col::<Option<String>>(1)?, row.col::<String>(2)?),
                ))
            })
            .await?;
            for (message, address) in rows {
                found.entry(message).or_default().push(address);
            }
        }
        Ok(found)
    }

    /// The reverse of the link [`save`](Self::save) writes: a Drafts row has to lead back to the
    /// composer's buffer, not the reader (#166). `None` for a draft written by another client.
    pub async fn by_message(&self, message: MessageId) -> Result<Option<Draft>> {
        let mut drafts = self.query("WHERE message_id = ?1", [message.get()]).await?;
        Ok(drafts.pop())
    }

    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Draft>> {
        self.query(
            "WHERE account_id = ?1 ORDER BY updated_at DESC, id DESC",
            [account_id.get()],
        )
        .await
    }

    /// Oldest first: the sender drains in that order.
    pub async fn by_state(&self, state: DraftState) -> Result<Vec<Draft>> {
        let mut drafts: Vec<Draft> = sql::all(
            self.connection,
            &format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE state = ?1 ORDER BY updated_at, id"),
            [state.as_str()],
            read_draft,
        )
        .await?;
        for draft in drafts.iter_mut() {
            self.fill(draft).await?;
        }
        Ok(drafts)
    }

    /// Narrower than [`save`](Self::save): it runs when a queued [`Operation::SaveDraft`] drains,
    /// possibly while the user is still typing, and writing the whole row back would undo what they
    /// added. See [`ServerCopyLocation`].
    pub async fn set_server_copy(
        &self,
        id: DraftId,
        copy: Option<&ServerCopyLocation>,
    ) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE drafts SET remote_id = ?2, uid = ?3, uid_validity = ?4 WHERE id = ?1",
            bind![
                id.get(),
                copy.map(|copy| copy.remote_id.as_str().to_owned()),
                copy.map(|copy| i64::from(copy.uid.get())),
                copy.map(|copy| i64::from(copy.uid_validity.get())),
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "draft",
                id: id.get(),
            });
        }
        sql::in_scope(self.connection, |scope| async move {
            // The stray row goes first: `messages` is unique on (mailbox, UIDVALIDITY, UID), so
            // attaching the UID while it exists violates the constraint. It is a duplicate a sync
            // pass made before this ran (#51), which `upsert_batch`'s skip cannot reach.
            // Scoped to the account's Drafts mailbox: identities are per-mailbox for IMAP.
            sql::execute(
                &scope,
                "DELETE FROM messages
                   WHERE remote_id = ?2
                     AND id IS NOT (SELECT message_id FROM drafts WHERE id = ?1)
                     AND mailbox_id IN (SELECT mailboxes.id FROM mailboxes
                                          JOIN drafts ON drafts.account_id = mailboxes.account_id
                                         WHERE drafts.id = ?1 AND mailboxes.role = 'drafts')",
                bind![
                    id.get(),
                    copy.map(|copy| copy.remote_id.as_str().to_owned())
                ],
            )
            .await?;
            // The row the folder already shows becomes the row that names the server copy (#166).
            sql::execute(
                &scope,
                "UPDATE messages
                    SET remote_id = ?2, uid = ?3, uid_validity = ?4
                  WHERE id IN (SELECT message_id FROM drafts
                                WHERE id = ?1 AND message_id IS NOT NULL)",
                bind![
                    id.get(),
                    copy.map(|copy| copy.remote_id.as_str().to_owned()),
                    copy.map(|copy| i64::from(copy.uid.get())),
                    copy.map(|copy| i64::from(copy.uid_validity.get())),
                ],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Returning it to `Editing` gives the reserved `Message-ID` back, as [`save`](Self::save)
    /// does, so no row is both editable and named by a message a previous attempt may have
    /// delivered. See [`reservation_for`].
    pub async fn set_state(&self, id: DraftId, state: DraftState) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            let changed = sql::execute(
                &transaction,
                "UPDATE drafts
                    SET state = ?2,
                        rfc_message_id = CASE WHEN ?2 = 'editing'
                                              THEN NULL ELSE rfc_message_id END
                  WHERE id = ?1",
                bind![id.get(), state.as_str()],
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "draft",
                    id: id.get(),
                });
            }
            // Same transaction, so the list row never disagrees with its draft; this moves a
            // message between the Outbox and Drafts.
            sql::execute(
                &transaction,
                "UPDATE messages
                    SET send_state = ?2,
                        -- Cleared unless it is still merely waiting: once the
                        -- drainer has it, or it has failed, the time somebody
                        -- chose is history rather than a plan.
                        send_at = CASE WHEN ?2 = 'queued' THEN send_at ELSE NULL END
                  WHERE id IN (SELECT message_id FROM drafts
                                WHERE id = ?1 AND message_id IS NOT NULL)",
                bind![id.get(), state.as_str()],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Resolves an `Unconfirmed` send (#674): the copy arrived in a sync, so the draft and that row
    /// are one mail. Separate from [`set_state`](Self::set_state): a draft can be `Sent` with no
    /// synced copy yet.
    pub async fn set_synced_message(&self, id: DraftId, message: MessageId) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE drafts SET message_id = ?2 WHERE id = ?1",
            bind![id.get(), message.get()],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "draft",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Its row in the Drafts folder goes with it: this is the single exit for discard and send, so
    /// a sent draft is not listed as unsent.
    pub async fn delete(&self, id: DraftId) -> Result<bool> {
        sql::in_scope(self.connection, |scope| async move {
            sql::execute(
                &scope,
                "DELETE FROM messages
                  WHERE id IN (SELECT message_id FROM drafts
                                WHERE id = ?1 AND message_id IS NOT NULL)",
                [id.get()],
            )
            .await?;
            let deleted =
                sql::execute(&scope, "DELETE FROM drafts WHERE id = ?1", [id.get()]).await?;
            Ok(deleted > 0)
        })
        .await
    }

    async fn query(&self, filter: &str, parameters: impl turso::IntoParams) -> Result<Vec<Draft>> {
        let mut drafts: Vec<Draft> = sql::all(
            self.connection,
            &format!("SELECT {DRAFT_COLUMNS} FROM drafts {filter}"),
            parameters,
            read_draft,
        )
        .await?;
        for draft in drafts.iter_mut() {
            self.fill(draft).await?;
        }
        Ok(drafts)
    }

    async fn fill(&self, draft: &mut Draft) -> Result<()> {
        let mut statement = sql::statement(
            self.connection,
            "SELECT r.kind, r.name, a.address FROM recipients r
               JOIN addresses a ON a.id = r.address_id
              WHERE r.draft_id = ?1 ORDER BY r.kind, r.position, r.id",
        )
        .await?;
        let rows = sql::mapped(&mut statement, [draft.id.get()], |row| {
            Ok((
                row.col::<String>(0)?,
                EmailAddress::new(row.col::<Option<String>>(1)?, row.col::<String>(2)?),
            ))
        })
        .await?;
        for (kind, address) in rows {
            match kind.as_str() {
                "to" => draft.to.push(address),
                "cc" => draft.cc.push(address),
                "bcc" => draft.bcc.push(address),
                other => return Err(unknown_enum("recipients.kind", other)),
            }
        }

        let mut statement = sql::statement(
            self.connection,
            "SELECT id, filename, mime_type, size, content_id, disposition, disposition_raw,
                    part_id, blob_id, part_headers
               FROM attachments WHERE draft_id = ?1 ORDER BY position, id",
        )
        .await?;
        let rows = sql::mapped(&mut statement, [draft.id.get()], |row| {
            let disposition: String = row.col(5)?;
            let raw: Option<String> = row.col(6)?;
            Ok(Attachment {
                id: AttachmentId::new(row.col(0)?),
                // A draft's attachment has no message yet; the model spells
                // that UNASSIGNED and the schema spells it NULL.
                message_id: MessageId::UNASSIGNED,
                filename: row.col(1)?,
                mime_type: row.col(2)?,
                size: row.col::<i64>(3)? as u64,
                content_id: row.col(4)?,
                disposition: Disposition::from_parts(&disposition, raw.as_deref())
                    .ok_or_else(|| unknown_enum("attachments.disposition", disposition))?,
                part_id: row.col(7)?,
                blob_id: row.col::<Option<String>>(8)?.map(BlobId::new),
                part_headers: row.col(9)?,
            })
        })
        .await?;
        draft.attachments = rows;
        Ok(())
    }
}

/// A `UID` is an identity only with the generation it was observed under; half of the pair means a
/// hand-written or pre-append row, and acting on it would be guessing.
fn server_copy(draft: &Draft) -> Option<RemoteId> {
    draft.server.remote_id.clone()
}

/// Written in the draft's transaction because the list is a query over `messages`: a draft with no
/// row is absent from the folder and the badge (#166), and waiting for a sync copy would list it
/// only after a network exchange, which local-first forbids.
///
/// `\Draft` and `\Seen`; `received_at` is the draft's `updated_at`; no `uid` until
/// [`DraftRepository::set_server_copy`] attaches one.
///
/// Does nothing when the account has no Drafts mailbox yet; the next save after it appears files
/// it.
async fn list_row(connection: &Connection, draft: &Draft) -> Result<()> {
    let Some(mailbox) = super::MailboxRepository::new(connection)
        .by_role(draft.account_id, MailboxRole::Drafts)
        .await?
    else {
        return Ok(());
    };
    let existing: Option<i64> = sql::first(
        connection,
        "SELECT message_id FROM drafts WHERE id = ?1",
        [draft.id.get()],
        |row| row.col(0),
    )
    .await?
    .flatten();

    let mut message = postio_model::Message::new(draft.account_id, mailbox.id, draft.updated_at);
    message.subject = (!draft.subject.trim().is_empty()).then(|| draft.subject.clone());
    message.preview = preview(draft.body.text.as_deref());
    message.to = draft.to.clone();
    message.cc = draft.cc.clone();
    message.bcc = draft.bcc.clone();
    message.from = sender(connection, draft).await?.into_iter().collect();
    message.flags = [postio_model::Flag::Draft, postio_model::Flag::Seen]
        .into_iter()
        .collect();
    message.attachments = draft.attachments.clone();

    let messages = super::MessageRepository::new(connection);
    let id = match existing {
        // `update` rewrites the children, so a recipient removed in the composer disappears.
        Some(id) => {
            message.id = MessageId::new(id);
            messages.update(&mut message).await?;
            message.id
        }
        None => {
            let id = messages.create(&mut message).await?;
            sql::execute(
                connection,
                "UPDATE drafts SET message_id = ?2 WHERE id = ?1",
                bind![draft.id.get(), id.get()],
            )
            .await?;
            id
        }
    };
    // Written here, not on `Message`: send state is a fact about a draft, and every other message
    // would carry `None`.
    set_send_state(connection, id, draft.state).await?;
    Ok(())
}

/// The only writer of `messages.send_state`; it runs in its caller's transaction, so the column
/// never disagrees with `drafts.state`.
async fn set_send_state(
    connection: &Connection,
    message: MessageId,
    state: DraftState,
) -> Result<()> {
    sql::execute(
        connection,
        "UPDATE messages SET send_state = ?2 WHERE id = ?1",
        bind![message.get(), state.as_str()],
    )
    .await?;
    Ok(())
}

/// One indexed point read per save.
async fn sender(connection: &Connection, draft: &Draft) -> Result<Option<EmailAddress>> {
    let found: Option<(Option<String>, String)> = match draft.identity_id {
        Some(identity) => {
            sql::first(
                connection,
                "SELECT display_name, address FROM identities WHERE id = ?1",
                [identity.get()],
                |row| Ok((row.col(0)?, row.col(1)?)),
            )
            .await?
        }
        None => None,
    };
    let found = match found {
        Some(found) => Some(found),
        None => {
            sql::first(
                connection,
                "SELECT display_name, address FROM accounts WHERE id = ?1",
                [draft.account_id.get()],
                |row| Ok((row.col(0)?, row.col(1)?)),
            )
            .await?
        }
    };
    Ok(found.map(|(name, address)| EmailAddress::new(name, address)))
}

/// Called inside `queue_send`'s transaction before the save, so the reservation and the
/// `Operation::Send` row land together or not at all: a send whose id was lost would be a retry
/// nothing downstream recognises (ADR 0021).
///
/// Idempotent: re-queuing is the same attempt series, and only a return to `Editing` gives the
/// reservation back ([`reservation_for`]).
async fn reserve_message_id(connection: &Connection, draft: &mut Draft) -> Result<()> {
    if draft.rfc_message_id.is_some() {
        return Ok(());
    }
    let from = sender(connection, draft).await?;
    let domain = from.as_ref().and_then(EmailAddress::domain);
    draft.rfc_message_id = Some(postio_model::outgoing::reserve_message_id(domain));
    Ok(())
}

/// Same shape as `postio-model`'s MIME reader gives a received message, so draft and message rows
/// read alike.
fn preview(text: Option<&str>) -> Option<String> {
    let flattened = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return None;
    }
    Some(flattened.chars().take(200).collect())
}

/// Delete and reinsert: a recipient row has no identity anything points at.
async fn write_recipients(connection: &Connection, draft: &Draft) -> Result<()> {
    sql::execute(
        connection,
        "DELETE FROM recipients WHERE draft_id = ?1",
        [draft.id.get()],
    )
    .await?;
    for (kind, addresses) in [("to", &draft.to), ("cc", &draft.cc), ("bcc", &draft.bcc)] {
        for (position, address) in addresses.iter().enumerate() {
            sql::execute(
                connection,
                "INSERT INTO recipients (draft_id, kind, position, name, address_id)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                bind![
                    draft.id.get(),
                    kind,
                    position as i64,
                    address.name,
                    crate::repository::messages::address_id(connection, address).await?,
                ],
            )
            .await?;
        }
    }
    Ok(())
}

/// Unlike recipients, an attachment row is referenced (the composer holds its id), so rewriting per
/// keystroke would change the id of an untouched file.
async fn write_attachments(connection: &Connection, draft: &mut Draft) -> Result<()> {
    let keep: Vec<i64> = draft
        .attachments
        .iter()
        .filter(|attachment| attachment.id.is_assigned())
        .map(|attachment| attachment.id.get())
        .collect();

    let placeholders = super::messages::placeholders(keep.len(), 2);
    let mut arguments: Vec<i64> = Vec::with_capacity(keep.len() + 1);
    arguments.push(draft.id.get());
    arguments.extend(&keep);
    sql::execute(
        connection,
        &format!("DELETE FROM attachments WHERE draft_id = ?1 AND id NOT IN ({placeholders})"),
        arguments,
    )
    .await?;

    for (position, attachment) in draft.attachments.iter_mut().enumerate() {
        if attachment.id.is_assigned() {
            sql::execute(
                connection,
                "UPDATE attachments
                    SET position = ?2, filename = ?3, mime_type = ?4, size = ?5,
                        content_id = ?6, disposition = ?7, disposition_raw = ?8,
                        part_id = ?9, blob_id = ?10, part_headers = ?11
                  WHERE id = ?1",
                bind![
                    attachment.id.get(),
                    position as i64,
                    attachment.filename,
                    attachment.mime_type,
                    attachment.size as i64,
                    attachment.content_id,
                    attachment.disposition.as_str(),
                    attachment.disposition.raw(),
                    attachment.part_id,
                    attachment.blob_id.as_ref().map(BlobId::as_str),
                    attachment.part_headers,
                ],
            )
            .await?;
        } else {
            sql::execute(
                connection,
                "INSERT INTO attachments (draft_id, position, filename, mime_type, size,
                                          content_id, disposition, disposition_raw, part_id,
                                          blob_id, part_headers)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                bind![
                    draft.id.get(),
                    position as i64,
                    attachment.filename,
                    attachment.mime_type,
                    attachment.size as i64,
                    attachment.content_id,
                    attachment.disposition.as_str(),
                    attachment.disposition.raw(),
                    attachment.part_id,
                    attachment.blob_id.as_ref().map(BlobId::as_str),
                    attachment.part_headers,
                ],
            )
            .await?;
            attachment.id = AttachmentId::new(connection.last_insert_rowid());
        }
    }
    Ok(())
}

fn label_ids(draft: &Draft) -> String {
    draft
        .labels
        .iter()
        .map(|label| label.get().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_draft(row: &Row) -> Result<Draft> {
    let kind: String = row.col(3)?;
    // `rich` sits after `body_html` in `DRAFT_COLUMNS` (#1271), so later columns shift by one.
    let state: String = row.col(10)?;

    Ok(Draft {
        id: DraftId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        identity_id: row.col::<Option<i64>>(2)?.map(IdentityId::new),
        kind: DraftKind::from_name(&kind).ok_or_else(|| unknown_enum("drafts.kind", kind))?,
        in_reply_to: row.col::<Option<i64>>(4)?.map(MessageId::new),
        forwarded_from: row.col::<Option<i64>>(18)?.map(MessageId::new),
        thread_id: row.col::<Option<i64>>(5)?.map(ThreadId::new),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: row.col(6)?,
        body: MessageBody {
            text: row.col(7)?,
            html: row.col(8)?,
        },
        rich: row.col(9)?,
        attachments: Vec::new(),
        state: DraftState::from_name(&state).ok_or_else(|| unknown_enum("drafts.state", state))?,
        server: ServerIdentifiers {
            content_identity: None,
            uid: row.col::<Option<i64>>(11)?.map(|uid| Uid::new(uid as u32)),
            uid_validity: row
                .col::<Option<i64>>(12)?
                .map(|validity| UidValidity::new(validity as u32)),
            mod_seq: row
                .col::<Option<i64>>(13)?
                .map(|seq| ModSeq::new(seq as u64)),
            remote_id: row.col::<Option<String>>(14)?.map(RemoteId::new),
        },
        rfc_message_id: row.col::<Option<String>>(17)?.map(RfcMessageId::new),
        body_markdown: row.col(19)?,
        labels: row
            .col::<String>(20)?
            .split_whitespace()
            .filter_map(|id| id.parse().ok())
            .map(LabelId::new)
            .collect(),
        calendar_reply: row.col(21)?,
        remind_at: row.col::<Option<i64>>(22)?.map(from_millis),
        created_at: from_millis(row.col(15)?),
        updated_at: from_millis(row.col(16)?),
    })
}

/// Derived from state, not trusted from the struct (ADR 0021): a row saying "editable" while naming
/// a message a previous attempt may have delivered cannot be written by any path.
fn reservation_for(draft: &Draft) -> Option<String> {
    if draft.state == DraftState::Editing {
        return None;
    }
    draft
        .rfc_message_id
        .as_ref()
        .map(|id| id.as_str().to_owned())
}

fn optional_identity(id: Option<IdentityId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(IdentityId::get)
}

fn optional_message(id: Option<MessageId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(MessageId::get)
}

fn optional_thread(id: Option<ThreadId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(ThreadId::get)
}
