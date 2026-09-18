//! Drafts: the composer's durable buffer.
//!
//! # Why the body is stored inline
//!
//! Everywhere else in Postio the bytes of a message live in the blob store and
//! SQLite holds the key. A draft is the exception, and the schema says so: it
//! is the composer's live buffer, autosaved on every keystroke, and a
//! content-addressed store would accumulate one immutable blob per keystroke.
//! It moves to the blob store when it becomes a sent message.
//!
//! # Autosave
//!
//! [`DraftRepository::save`] is the only write the composer needs: it inserts
//! the first time and updates every time after, so the caller does not have to
//! know which. Calling it fifty times in a row leaves one row, the recipients
//! it was last given, and the attachment ids the user's attachments already
//! had.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, AttachmentId, BlobId, Disposition, Draft, DraftId, DraftKind,
    DraftState, EmailAddress, IdentityId, MailboxRole, MessageBody, MessageId, ModSeq, Operation,
    OperationTarget, RemoteId, RfcMessageId, ServerIdentifiers, ThreadId, Uid, UidValidity,
};

/// Where an appended draft landed, as [`DraftRepository::set_server_copy`]
/// records it: the backend-neutral identity the engine addresses (#543),
/// plus the wire pair the uid-range pull machinery still enumerates by
/// (ADR 0018 Q3 keeps that IMAP-shaped until the native delta seam).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerCopyLocation {
    /// The backend's own identity for the copy.
    pub remote_id: RemoteId,
    /// The wire uid it landed at.
    pub uid: Uid,
    /// The generation `uid` was observed under.
    pub uid_validity: UidValidity,
}

use super::{OperationQueueRepository, QueuedOperation};

/// What [`DraftRepository::cancel_send`] found when it was asked to take a
/// draft's send back off the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelSendOutcome {
    /// The queued `Send` was removed and the draft is `Editing` again.
    Cancelled,
    /// There was nothing queued to cancel: the draft is not [`DraftState::Queued`],
    /// or it no longer exists.
    NotQueued,
    /// The send is no longer safely cancellable — the drainer has already
    /// started it, or it settled between the caller's read and this call.
    AlreadyInFlight,
}
use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Reads and writes [`Draft`] rows.
#[derive(Debug)]
pub struct DraftRepository<'a> {
    connection: &'a Connection,
}

const DRAFT_COLUMNS: &str = "\
id, account_id, identity_id, kind, in_reply_to_message_id, thread_id, subject, body_text,
body_html, state, uid, uid_validity, mod_seq, remote_id, created_at, updated_at,
rfc_message_id";

impl<'a> DraftRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Writes a draft, inserting it the first time and updating it thereafter.
    ///
    /// This is what autosave calls. It is idempotent: the draft's id, and the
    /// ids of the attachments already on it, do not change from one save to the
    /// next, so nothing the composer is holding goes stale.
    ///
    /// # The server identifiers are not the composer's to write
    ///
    /// `uid`, `uid_validity`, `mod_seq` and `remote_id` are only ever cleared
    /// by [`set_server_copy`](Self::set_server_copy), never by a save that
    /// simply does not know them. The composer holds a [`Draft`] for as long as
    /// the window is open and autosaves the same value repeatedly; the drainer
    /// writes where the server copy landed *while it is holding it*. Without
    /// this, every autosave after an upload would wipe the id of the copy on
    /// the server, and the next upload would add a second one instead of
    /// replacing the first.
    pub async fn save(&self, draft: &mut Draft) -> Result<DraftId> {
        sql::in_scope(self.connection, |transaction| async move {
            if draft.id.is_assigned() {
                let changed = transaction
                    .execute(
                        "UPDATE drafts
                        SET account_id = ?2, identity_id = ?3, kind = ?4,
                            in_reply_to_message_id = ?5, thread_id = ?6, subject = ?7,
                            body_text = ?8, body_html = ?9, state = ?10,
                            uid = coalesce(?11, uid),
                            uid_validity = coalesce(?12, uid_validity),
                            mod_seq = coalesce(?13, mod_seq),
                            remote_id = coalesce(?14, remote_id),
                            updated_at = ?15,
                            rfc_message_id = ?16
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
                transaction
                    .execute(
                        "INSERT INTO drafts (account_id, identity_id, kind, in_reply_to_message_id,
                                         thread_id, subject, body_text, body_html, state, uid,
                                         uid_validity, mod_seq, remote_id, created_at, updated_at,
                                         rfc_message_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                             ?16)",
                        bind![
                            account_id,
                            optional_identity(draft.identity_id),
                            draft.kind.as_str(),
                            optional_message(draft.in_reply_to),
                            optional_thread(draft.thread_id),
                            draft.subject,
                            draft.body.text,
                            draft.body.html,
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

    /// Saves a draft and queues its server copy to be brought up to date.
    ///
    /// This is [`save`](Self::save) plus the enqueue, in one write, which is
    /// the local-first rule stated in [`OperationQueueRepository`]: a local
    /// write without its queue row never reaches the server, and a queue row
    /// without its local write tells the server about something the user never
    /// saw happen.
    ///
    /// Returns `None` — having still saved the draft — when the account has no
    /// Drafts mailbox yet, which is the ordinary state of an account that has
    /// not finished its first sync. The draft is durable here regardless, and
    /// the next save after the folder turns up files it.
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

    /// Hands a draft to the operation queue to be sent, in one write.
    ///
    /// The sibling of [`save_and_sync`](Self::save_and_sync) for the other
    /// verb, and local-first in the same way: the draft goes to
    /// [`DraftState::Queued`] and the [`Operation::Send`] row is written now,
    /// so the composer can close the moment the user presses the key and the
    /// UI never waits on SMTP. `postio-sync::send` picks the row up whenever
    /// there is a connection.
    ///
    /// # Why the draft is saved rather than merely marked
    ///
    /// Autosave is debounced, so a message typed and sent inside the quiet
    /// period has never been written and has no [`DraftId`] — and the queued
    /// operation names the draft by id, because a send carries no bytes for
    /// the same reason [`Operation::SaveDraft`] does not: they are built when
    /// it drains, from whatever the row says then. So this saves first and
    /// enqueues against the id that save assigned.
    ///
    /// # Why the local row stays
    ///
    /// Every other terminal verb on a draft deletes it. This one must not:
    /// `postio-sync::send` resolves a `Send` whose draft is gone as *obsolete*
    /// and drops it, so deleting the row here would throw the message away
    /// instead of sending it. The row is deleted by the drainer, after the
    /// submission server has accepted it and the Sent copy is filed.
    ///
    /// Unlike `save_and_sync` this always returns a queue row: a send names no
    /// mailbox, so there is no folder that might not exist yet to stop it.
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

    /// [`queue_send`](Self::queue_send), except the drainer must not touch
    /// the row before `send_at` — a scheduled send.
    ///
    /// The draft goes to `Queued` immediately, same as an immediate send:
    /// the composer has let go of it either way, and there is nothing left
    /// to edit while a send — due now or due later — is sitting in the
    /// queue. What differs is only when [`OperationQueueRepository::pending`]
    /// starts offering the row to the drainer, which is stored on the row
    /// itself and so survives a restart with no timer to lose.
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

            // The chosen time, on the row the list draws. Only here: a
            // `next_attempt_at` set by backoff is the retry clock, not a plan
            // anybody made, and showing it as one would be a lie about intent.
            scope
                .execute(
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

    /// Mark every send that has nothing left to carry it as `Failed`.
    ///
    /// A draft is `Queued` because `queue_send` wrote the state and the
    /// `Send` operation in one transaction, so the two cannot come apart at
    /// the moment of queueing. They come apart afterwards: an operation that
    /// gives up is marked `failed`, and `prune_settled` removes settled rows
    /// in time. A draft left `Queued` behind that is in the Outbox for ever,
    /// under a row that says it is on its way, with nothing coming.
    ///
    /// **`Queued` only, and that is the safety argument.** The drainer commits
    /// `Sending` before it hands anything to SMTP (ADR 0021), so a draft still
    /// at `Queued` was never submitted and "it did not go" is a fact rather
    /// than a guess. A `Sending` draft with no operation is the *uncertain*
    /// case -- nobody can say whether the server took it -- and
    /// `postio_sync::send::resolve` promotes that one to `Unconfirmed`. This
    /// must never touch it, or it would tell a user a message failed that may
    /// be in somebody's inbox.
    ///
    /// Returns how many were healed. Idempotent, because it runs on every
    /// drain pass.
    pub async fn fail_orphaned_sends(&self, account: AccountId) -> Result<usize> {
        // Read first, and write only when there is something to write. This
        // runs on every drain pass, and the answer is almost always zero: an
        // `UPDATE` issued regardless would take the writer each time to
        // decide it had nothing to do.
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
            let healed = transaction
                .execute(
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
            // The mirror row #166 keeps in Drafts carries the same state, and
            // the lists read *it* rather than the draft -- so a heal that
            // stopped at the `drafts` table would leave the Outbox still
            // drawing the row.
            transaction
                .execute(
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

    /// Cancels a draft's pending send and hands it back for editing (#433).
    ///
    /// A queued draft's row stayed in the Drafts folder the whole time it sat
    /// in the queue, and opening it from there reopened the composer on live
    /// state that could be edited — while the drainer might build the
    /// outgoing bytes from that same row at any moment. Whether an edit made
    /// it into the message the server received depended on nothing but
    /// timing.
    ///
    /// This is what makes it safe to open again: [`Operation::Send`] has no
    /// [`Operation::inverse`] — there is nothing to *replay* against a
    /// server that has not seen it — but [`OperationQueueRepository::delete`]
    /// documents the real answer for a row that has not drained yet, taking
    /// it off the queue outright. That is exactly this call, and it costs
    /// nothing because nothing has left the machine: `Composer::send`'s
    /// local-first write means the operation is still sitting in SQLite,
    /// never in flight to SMTP, right up until the drainer marks it so.
    ///
    /// Returns [`CancelSendOutcome::AlreadyInFlight`] rather than touching
    /// anything once the drainer has: at that point a submission may already
    /// be on the wire, and reopening the draft as editable would risk a
    /// second, different message going out behind it.
    pub async fn cancel_send(&self, id: DraftId, at: DateTime<Utc>) -> Result<CancelSendOutcome> {
        sql::in_scope(self.connection, |scope| async move {
            let drafts = DraftRepository::new(&scope);

            let Some(mut draft) = drafts.get(id).await? else {
                return Ok(CancelSendOutcome::NotQueued);
            };
            // `Sending` and `Sent` are not "nothing to cancel" — they are "too
            // late", and the two are opposite facts. `postio-sync::send` commits
            // `Sending` immediately before it opens the SMTP transaction (ADR
            // 0021), so a draft in either state may already have reached the
            // recipient's server, and answering `NotQueued` here would let a
            // caller report a message recalled while it was on the wire.
            if matches!(draft.state, DraftState::Sending | DraftState::Sent) {
                return Ok(CancelSendOutcome::AlreadyInFlight);
            }
            if draft.state != DraftState::Queued {
                return Ok(CancelSendOutcome::NotQueued);
            }

            let queue = OperationQueueRepository::new(&scope);
            let target = OperationTarget::Draft(id);
            let Some(pending) = queue.pending_for(target).await? else {
                // The row says `Queued` but the operation is already gone --
                // settled between the caller's read and this one. Too late to
                // pretend otherwise.
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

    /// Deletes a draft and queues the removal of its server copy.
    ///
    /// The local row goes now: discarding a draft is local-first like every
    /// other mutation, and the composer must not wait for a server to agree.
    /// That is why the queued [`Operation::DiscardDraft`] carries the `UID` and
    /// its generation rather than naming the draft — by the time it drains
    /// there is no row left to read them from.
    ///
    /// Returns `None` when nothing needed queueing: the draft was already gone,
    /// it never reached the server, or the account has no Drafts mailbox.
    pub async fn discard(&self, id: DraftId, at: DateTime<Utc>) -> Result<Option<QueuedOperation>> {
        sql::in_scope(self.connection, |scope| async move {
            let drafts = DraftRepository::new(&scope);

            let Some(draft) = drafts.get(id).await? else {
                // A retried discard, or one racing a send that already cleared the
                // row. Both are the expected case rather than a failure.
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
                // Never uploaded, so there is nothing on the server to remove and
                // no round trip worth spending to say so.
                None => None,
            };

            drafts.delete(id).await?;
            Ok(queued)
        })
        .await
    }

    /// One draft, with its recipients and attachments.
    pub async fn get(&self, id: DraftId) -> Result<Option<Draft>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"))
            .await?;
        let found = crate::sql::first_of(&mut statement, [id.get()], read_draft).await?;
        let Some(mut draft) = found else {
            return Ok(None);
        };

        self.fill(&mut draft).await?;
        Ok(Some(draft))
    }

    /// The draft a message row in the Drafts folder is listing, if it is
    /// listing one.
    ///
    /// The reverse of the link [`save`](Self::save) writes. The message list
    /// hands activation a `MessageId`, and a draft's row has to lead back to
    /// the buffer the composer edits — opening the reader on it instead is the
    /// dead end #166 is about.
    ///
    /// `None` for a draft written by another client: it has a row in the
    /// folder and no local buffer behind it.
    pub async fn by_message(&self, message: MessageId) -> Result<Option<Draft>> {
        let mut drafts = self.query("WHERE message_id = ?1", [message.get()]).await?;
        Ok(drafts.pop())
    }

    /// An account's drafts, most recently edited first.
    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Draft>> {
        self.query(
            "WHERE account_id = ?1 ORDER BY updated_at DESC, id DESC",
            [account_id.get()],
        )
        .await
    }

    /// Every draft in one life-cycle state, oldest first.
    ///
    /// Oldest first because this is how the sender drains its queue, and a
    /// queue that serves the newest first is a stack.
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

    /// The drafts belonging to a thread, so the composer can appear inline.
    pub async fn in_thread(&self, thread_id: ThreadId) -> Result<Vec<Draft>> {
        self.query(
            "WHERE thread_id = ?1 ORDER BY updated_at DESC, id DESC",
            [thread_id.get()],
        )
        .await
    }

    /// Records where the draft's server copy landed, or that it has none.
    ///
    /// See [`ServerCopyLocation`] for what "where" means since #543.
    ///
    /// Narrower than [`save`](Self::save) on purpose: this runs when a queued
    /// [`Operation::SaveDraft`] drains, which is minutes after the text it
    /// uploaded was typed and quite possibly while the user is still typing.
    /// Writing the whole row back from what the drainer read would undo
    /// whatever they have added since.
    pub async fn set_server_copy(
        &self,
        id: DraftId,
        copy: Option<&ServerCopyLocation>,
    ) -> Result<()> {
        let changed = self
            .connection
            .execute(
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
            // The stray row goes first, and it has to: it is a row for this very
            // copy, and `messages` is unique on (mailbox, UIDVALIDITY, UID), so
            // attaching the UID below while it still exists is a constraint
            // violation rather than a duplicate.
            //
            // It is a duplicate a sync pass made before this ran, and
            // `upsert_batch`'s skip cannot reach it — that only declines to create
            // one, and every later pass would find this row and keep it current
            // for ever. See #51.
            //
            // Scoped to the account's Drafts mailbox because identities are
            // per-mailbox for IMAP: the message that happens to hold the same
            // number in the inbox is mail.
            scope
                .execute(
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
            // The row the folder is already showing becomes the row that names the
            // server copy. It is the same message: this draft, listed since the
            // moment it was first saved (#166), now with somewhere on the server
            // to point at.
            scope
                .execute(
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

    /// Moves a draft through its life cycle.
    ///
    /// Returning it to `Editing` gives the reserved `Message-ID` back, the
    /// same way [`save`](Self::save) does — the invariant is that no row can
    /// be both editable and still named by a message a previous attempt may
    /// have delivered, and it holds on every write path or it is not an
    /// invariant. See [`reservation_for`] for why.
    pub async fn set_state(&self, id: DraftId, state: DraftState) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            let changed = transaction
                .execute(
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
            // In the same transaction, so the row the list draws cannot be seen
            // disagreeing with the draft it stands for. This is the path the
            // drainer takes -- Queued, Sending, Failed, Unconfirmed -- and it is
            // what moves a message between the Outbox and Drafts.
            transaction
                .execute(
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

    /// Point a draft at the `messages` row it turned out to be.
    ///
    /// What resolves an `Unconfirmed` send (#674): the copy arrived in a
    /// sync, so the draft is the same mail as that row and every surface
    /// that opens one should reach the other. Separate from
    /// [`set_state`](Self::set_state) because the two facts are learnt in
    /// the same breath but are not the same fact — a draft can be `Sent`
    /// with no synced copy yet, and the link is what makes "show me it"
    /// work once there is one.
    pub async fn set_synced_message(&self, id: DraftId, message: MessageId) -> Result<()> {
        let changed = self
            .connection
            .execute(
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

    /// Deletes a draft and everything on it, returning whether there was one.
    ///
    /// Its row in the Drafts folder goes with it. This is the single exit both
    /// discard and send go through — `postio-sync::send` finishes here — so it
    /// is the one place that has to remember, and a draft that has been sent
    /// must not go on being listed as unsent.
    pub async fn delete(&self, id: DraftId) -> Result<bool> {
        sql::in_scope(self.connection, |scope| async move {
            scope
                .execute(
                    "DELETE FROM messages
                  WHERE id IN (SELECT message_id FROM drafts
                                WHERE id = ?1 AND message_id IS NOT NULL)",
                    [id.get()],
                )
                .await?;
            let deleted = scope
                .execute("DELETE FROM drafts WHERE id = ?1", [id.get()])
                .await?;
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
        let mut statement = self
            .connection
            .prepare(
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

        let mut statement = self
            .connection
            .prepare(
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

/// The server copy of a draft, when it has one.
///
/// Both halves or neither: a `UID` is only an identity together with the
/// generation it was observed under, and both arrive together from the append
/// that created the copy. Half of the pair means the row predates that append
/// or was written by hand, and acting on it would be guessing.
fn server_copy(draft: &Draft) -> Option<RemoteId> {
    draft.server.remote_id.clone()
}

/// Keeps the draft's row in the Drafts folder in step with the draft.
///
/// # Why the list row is written here and not brought back by sync
///
/// The message list is a windowed query over `messages`, so a draft that is
/// only a `drafts` row cannot appear in the folder the sidebar sends people to
/// — and the badge, which reads the mailbox's cached count of message rows,
/// says 0 while the composer holds a draft. #166.
///
/// The other way round — keeping the copy a sync pass brings back, which #51
/// skips, and routing its activation to the composer — is cheaper and wrong: a
/// draft has no server copy until an append has round-tripped, so the folder
/// would list your draft only *after* a network exchange. docs/PRODUCT.md §18
/// and the local-first rule both forbid exactly that. This row is written in
/// the same transaction as the draft, offline and always.
///
/// # What the row says
///
/// `\Draft` and `\Seen`: the list already draws a draft mark and says "Draft"
/// in the accessible label off `MessageListRow::draft`, and unread is a thing
/// mail that arrived is. `received_at` is the draft's `updated_at`, so the
/// folder orders by when it was last touched, which is the only ordering a
/// draft has. There is no `uid` until [`DraftRepository::set_server_copy`]
/// attaches one to *this* row.
///
/// Does nothing when the account has no Drafts mailbox yet — the ordinary
/// state of an account that has not finished its first sync. The draft is
/// durable regardless; it simply has nowhere to be listed, and the next save
/// after the folder turns up files it.
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
        // `update` rewrites the children, which is what makes a recipient
        // removed in the composer disappear from the row.
        Some(id) => {
            message.id = MessageId::new(id);
            messages.update(&mut message).await?;
            message.id
        }
        None => {
            let id = messages.create(&mut message).await?;
            connection
                .execute(
                    "UPDATE drafts SET message_id = ?2 WHERE id = ?1",
                    bind![draft.id.get(), id.get()],
                )
                .await?;
            id
        }
    };
    // The send state, denormalised onto the row the list draws (spec 003).
    // `Message` has no field for it: it is a fact about a draft, and every
    // other message has none, so it is written here rather than travelling
    // through a type that would carry `None` for the whole mailbox.
    set_send_state(connection, id, draft.state).await?;
    Ok(())
}

/// Write a draft's state onto the `messages` row standing for it.
///
/// The only writer of `messages.send_state`, and it runs in whatever
/// transaction its caller opened — which is how the column cannot be seen
/// disagreeing with `drafts.state`.
async fn set_send_state(
    connection: &Connection,
    message: MessageId,
    state: DraftState,
) -> Result<()> {
    connection
        .execute(
            "UPDATE messages SET send_state = ?2 WHERE id = ?1",
            bind![message.get(), state.as_str()],
        )
        .await?;
    Ok(())
}

/// Who the draft will be from: the identity it picked, or the account's own
/// address when it has not picked one.
///
/// Read rather than carried on [`Draft`], which holds an `identity_id` and not
/// an address. One indexed point read per save, which is the same order as the
/// recipient rewrite beside it.
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

/// Mints the `Message-ID` this send attempt series will go out under, unless
/// one is already reserved (ADR 0021).
///
/// Called from inside `queue_send`'s transaction, before the save that stores
/// it, so the reservation and the `Operation::Send` row are written together
/// or not at all — a queued send whose id was lost on the way would be a
/// retry nothing downstream could recognise as one, which is the whole
/// failure this exists to prevent.
///
/// Idempotent: a draft that already carries a reservation keeps it. Re-queuing
/// is the same attempt series, and only a return to `Editing` gives the
/// reservation back — see [`reservation_for`].
async fn reserve_message_id(connection: &Connection, draft: &mut Draft) -> Result<()> {
    if draft.rfc_message_id.is_some() {
        return Ok(());
    }
    let from = sender(connection, draft).await?;
    let domain = from.as_ref().and_then(EmailAddress::domain);
    draft.rfc_message_id = Some(postio_model::outgoing::reserve_message_id(domain));
    Ok(())
}

/// The snippet the list draws under the subject.
///
/// The same shape `postio-model`'s MIME reader produces for a received
/// message, so a draft's row and a message's row read alike.
fn preview(text: Option<&str>) -> Option<String> {
    let flattened = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.is_empty() {
        return None;
    }
    Some(flattened.chars().take(200).collect())
}

/// Rewrites a draft's recipient rows.
///
/// Deleting and reinserting is right here in a way it would not be for a
/// message: a recipient row carries no identity of its own that anything else
/// points at, and the composer's list is small and changes on every keystroke.
async fn write_recipients(connection: &Connection, draft: &Draft) -> Result<()> {
    connection
        .execute(
            "DELETE FROM recipients WHERE draft_id = ?1",
            [draft.id.get()],
        )
        .await?;
    for (kind, addresses) in [("to", &draft.to), ("cc", &draft.cc), ("bcc", &draft.bcc)] {
        for (position, address) in addresses.iter().enumerate() {
            connection
                .execute(
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

/// Inserts the attachments that are new and removes the ones that are gone.
///
/// Unlike recipients, an attachment row is *referenced*: the composer holds its
/// id, and the bytes it points at are in the blob store. Rewriting them on
/// every keystroke would hand the composer a new id for a file the user has not
/// touched.
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
    connection
        .execute(
            &format!("DELETE FROM attachments WHERE draft_id = ?1 AND id NOT IN ({placeholders})"),
            arguments,
        )
        .await?;

    for (position, attachment) in draft.attachments.iter_mut().enumerate() {
        if attachment.id.is_assigned() {
            connection
                .execute(
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
            connection
                .execute(
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

fn read_draft(row: &Row) -> Result<Draft> {
    let kind: String = row.col(3)?;
    let state: String = row.col(9)?;

    Ok(Draft {
        id: DraftId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        identity_id: row.col::<Option<i64>>(2)?.map(IdentityId::new),
        kind: DraftKind::from_name(&kind).ok_or_else(|| unknown_enum("drafts.kind", kind))?,
        in_reply_to: row.col::<Option<i64>>(4)?.map(MessageId::new),
        thread_id: row.col::<Option<i64>>(5)?.map(ThreadId::new),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: row.col(6)?,
        body: MessageBody {
            text: row.col(7)?,
            html: row.col(8)?,
        },
        attachments: Vec::new(),
        state: DraftState::from_name(&state).ok_or_else(|| unknown_enum("drafts.state", state))?,
        server: ServerIdentifiers {
            uid: row.col::<Option<i64>>(10)?.map(|uid| Uid::new(uid as u32)),
            uid_validity: row
                .col::<Option<i64>>(11)?
                .map(|validity| UidValidity::new(validity as u32)),
            mod_seq: row
                .col::<Option<i64>>(12)?
                .map(|seq| ModSeq::new(seq as u64)),
            remote_id: row.col::<Option<String>>(13)?.map(RemoteId::new),
        },
        rfc_message_id: row.col::<Option<String>>(16)?.map(RfcMessageId::new),
        created_at: from_millis(row.col(14)?),
        updated_at: from_millis(row.col(15)?),
    })
}

/// The `rfc_message_id` column's value for `draft`, **derived from its state
/// rather than trusted from the struct**.
///
/// ADR 0021 says the reservation is given back when a draft becomes editable
/// again, and a caller that sets `state` without also clearing the id would
/// otherwise write a row that contradicts itself — one that says "editable"
/// and still names the message a previous attempt may already have delivered.
/// Deriving it here means that row cannot be written by any path, rather than
/// by every path remembering.
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
