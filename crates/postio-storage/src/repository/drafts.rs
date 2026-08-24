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
    OperationTarget, ServerIdentifiers, ThreadId, Uid, UidValidity,
};
use rusqlite::{Connection, Row, params};

use super::{OperationQueueRepository, QueuedOperation};
use super::{from_millis, require_persisted, to_millis, unknown_enum};
use crate::error::{Error, Result};

/// Reads and writes [`Draft`] rows.
#[derive(Debug)]
pub struct DraftRepository<'a> {
    connection: &'a Connection,
}

const DRAFT_COLUMNS: &str = "\
id, account_id, identity_id, kind, in_reply_to_message_id, thread_id, subject, body_text,
body_html, state, uid, uid_validity, mod_seq, remote_id, created_at, updated_at";

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
    pub fn save(&self, draft: &mut Draft) -> Result<DraftId> {
        let transaction = super::Scope::open(self.connection)?;

        if draft.id.is_assigned() {
            let changed = transaction.execute(
                "UPDATE drafts
                    SET account_id = ?2, identity_id = ?3, kind = ?4,
                        in_reply_to_message_id = ?5, thread_id = ?6, subject = ?7,
                        body_text = ?8, body_html = ?9, state = ?10,
                        uid = coalesce(?11, uid),
                        uid_validity = coalesce(?12, uid_validity),
                        mod_seq = coalesce(?13, mod_seq),
                        remote_id = coalesce(?14, remote_id),
                        updated_at = ?15
                  WHERE id = ?1",
                params![
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
                    draft.server.remote_id,
                    to_millis(draft.updated_at),
                ],
            )?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "draft",
                    id: draft.id.get(),
                });
            }
        } else {
            let account_id = require_persisted(draft.account_id.get(), "account")?;
            transaction.execute(
                "INSERT INTO drafts (account_id, identity_id, kind, in_reply_to_message_id,
                                     thread_id, subject, body_text, body_html, state, uid,
                                     uid_validity, mod_seq, remote_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
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
                    draft.server.remote_id,
                    to_millis(draft.created_at),
                    to_millis(draft.updated_at),
                ],
            )?;
            draft.id = DraftId::new(transaction.last_insert_rowid());
        }

        write_recipients(&transaction, draft)?;
        write_attachments(&transaction, draft)?;

        transaction.commit()?;
        Ok(draft.id)
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
    pub fn save_and_sync(
        &self,
        draft: &mut Draft,
        at: DateTime<Utc>,
    ) -> Result<Option<QueuedOperation>> {
        let scope = super::Scope::open(self.connection)?;

        DraftRepository::new(&scope).save(draft)?;
        let queued = match super::MailboxRepository::new(&scope)
            .by_role(draft.account_id, MailboxRole::Drafts)?
        {
            Some(mailbox) => Some(OperationQueueRepository::new(&scope).enqueue(
                draft.account_id,
                OperationTarget::Draft(draft.id),
                &Operation::SaveDraft {
                    mailbox: mailbox.id,
                },
                at,
            )?),
            None => None,
        };

        scope.commit()?;
        Ok(queued)
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
    pub fn discard(&self, id: DraftId, at: DateTime<Utc>) -> Result<Option<QueuedOperation>> {
        let scope = super::Scope::open(self.connection)?;
        let drafts = DraftRepository::new(&scope);

        let Some(draft) = drafts.get(id)? else {
            // A retried discard, or one racing a send that already cleared the
            // row. Both are the expected case rather than a failure.
            scope.commit()?;
            return Ok(None);
        };

        let queued = match server_copy(&draft) {
            Some((uid, uid_validity)) => {
                match super::MailboxRepository::new(&scope)
                    .by_role(draft.account_id, MailboxRole::Drafts)?
                {
                    Some(mailbox) => Some(OperationQueueRepository::new(&scope).enqueue(
                        draft.account_id,
                        OperationTarget::Draft(id),
                        &Operation::DiscardDraft {
                            mailbox: mailbox.id,
                            uid,
                            uid_validity,
                        },
                        at,
                    )?),
                    None => None,
                }
            }
            // Never uploaded, so there is nothing on the server to remove and
            // no round trip worth spending to say so.
            None => None,
        };

        drafts.delete(id)?;
        scope.commit()?;
        Ok(queued)
    }

    /// One draft, with its recipients and attachments.
    pub fn get(&self, id: DraftId) -> Result<Option<Draft>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT {DRAFT_COLUMNS} FROM drafts WHERE id = ?1"))?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let mut draft = read_draft(row)?;
        drop(rows);
        drop(statement);

        self.fill(&mut draft)?;
        Ok(Some(draft))
    }

    /// An account's drafts, most recently edited first.
    pub fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Draft>> {
        self.query(
            "WHERE account_id = ?1 ORDER BY updated_at DESC, id DESC",
            [account_id.get()],
        )
    }

    /// Every draft in one life-cycle state, oldest first.
    ///
    /// Oldest first because this is how the sender drains its queue, and a
    /// queue that serves the newest first is a stack.
    pub fn by_state(&self, state: DraftState) -> Result<Vec<Draft>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {DRAFT_COLUMNS} FROM drafts WHERE state = ?1 ORDER BY updated_at, id"
        ))?;
        let rows = statement.query_map([state.as_str()], read_draft)?;
        let mut drafts: Vec<Draft> = rows.collect::<Result<_, _>>()?;
        drop(statement);
        for draft in &mut drafts {
            self.fill(draft)?;
        }
        Ok(drafts)
    }

    /// The drafts belonging to a thread, so the composer can appear inline.
    pub fn in_thread(&self, thread_id: ThreadId) -> Result<Vec<Draft>> {
        self.query(
            "WHERE thread_id = ?1 ORDER BY updated_at DESC, id DESC",
            [thread_id.get()],
        )
    }

    /// Records where the draft's server copy landed, or that it has none.
    ///
    /// Narrower than [`save`](Self::save) on purpose: this runs when a queued
    /// [`Operation::SaveDraft`] drains, which is minutes after the text it
    /// uploaded was typed and quite possibly while the user is still typing.
    /// Writing the whole row back from what the drainer read would undo
    /// whatever they have added since.
    pub fn set_server_copy(
        &self,
        id: DraftId,
        uid: Option<Uid>,
        uid_validity: Option<UidValidity>,
    ) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE drafts SET uid = ?2, uid_validity = ?3 WHERE id = ?1",
            params![
                id.get(),
                uid.map(|uid| i64::from(uid.get())),
                uid_validity.map(|validity| i64::from(validity.get())),
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "draft",
                id: id.get(),
            });
        }
        // Claiming the copy is also what disowns any message row for it. A
        // sync pass that fetched the appended draft before this ran left an
        // ordinary row behind, and `upsert_batch`'s skip cannot reach it —
        // that only declines to *create* one, and every later pass would find
        // this row and keep it current for ever. See #51.
        //
        // Scoped to the account's Drafts mailbox because UIDs are per-mailbox:
        // the message that happens to be number 7 in the inbox is mail.
        self.connection.execute(
            "DELETE FROM messages
               WHERE uid = ?2 AND uid_validity = ?3
                 AND mailbox_id IN (SELECT mailboxes.id FROM mailboxes
                                      JOIN drafts ON drafts.account_id = mailboxes.account_id
                                     WHERE drafts.id = ?1 AND mailboxes.role = 'drafts')",
            params![
                id.get(),
                uid.map(|uid| i64::from(uid.get())),
                uid_validity.map(|validity| i64::from(validity.get())),
            ],
        )?;
        Ok(())
    }

    /// Moves a draft through its life cycle.
    pub fn set_state(&self, id: DraftId, state: DraftState) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE drafts SET state = ?2 WHERE id = ?1",
            params![id.get(), state.as_str()],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "draft",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Deletes a draft and everything on it, returning whether there was one.
    pub fn delete(&self, id: DraftId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM drafts WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    fn query<P: rusqlite::Params>(&self, filter: &str, parameters: P) -> Result<Vec<Draft>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT {DRAFT_COLUMNS} FROM drafts {filter}"))?;
        let rows = statement.query_map(parameters, read_draft)?;
        let mut drafts: Vec<Draft> = rows.collect::<Result<_, _>>()?;
        drop(statement);
        for draft in &mut drafts {
            self.fill(draft)?;
        }
        Ok(drafts)
    }

    fn fill(&self, draft: &mut Draft) -> Result<()> {
        let mut statement = self.connection.prepare(
            "SELECT kind, name, address FROM recipients
              WHERE draft_id = ?1 ORDER BY kind, position, id",
        )?;
        let rows = statement.query_map([draft.id.get()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                EmailAddress::new(row.get::<_, Option<String>>(1)?, row.get::<_, String>(2)?),
            ))
        })?;
        for row in rows {
            let (kind, address) = row?;
            match kind.as_str() {
                "to" => draft.to.push(address),
                "cc" => draft.cc.push(address),
                "bcc" => draft.bcc.push(address),
                other => return Err(unknown_enum("recipients.kind", other)),
            }
        }
        drop(statement);

        let mut statement = self.connection.prepare(
            "SELECT id, filename, mime_type, size, content_id, disposition, disposition_raw,
                    part_id, blob_id
               FROM attachments WHERE draft_id = ?1 ORDER BY position, id",
        )?;
        let rows = statement.query_map([draft.id.get()], |row| {
            let disposition: String = row.get(5)?;
            let raw: Option<String> = row.get(6)?;
            Ok(Attachment {
                id: AttachmentId::new(row.get(0)?),
                // A draft's attachment has no message yet; the model spells
                // that UNASSIGNED and the schema spells it NULL.
                message_id: MessageId::UNASSIGNED,
                filename: row.get(1)?,
                mime_type: row.get(2)?,
                size: row.get::<_, i64>(3)? as u64,
                content_id: row.get(4)?,
                disposition: Disposition::from_parts(&disposition, raw.as_deref()).ok_or_else(
                    || {
                        rusqlite::Error::FromSqlConversionFailure(
                            5,
                            rusqlite::types::Type::Text,
                            Box::new(unknown_enum("attachments.disposition", disposition)),
                        )
                    },
                )?,
                part_id: row.get(7)?,
                blob_id: row.get::<_, Option<String>>(8)?.map(BlobId::new),
            })
        })?;
        draft.attachments = rows.collect::<Result<_, _>>()?;
        Ok(())
    }
}

/// The server copy of a draft, when it has one.
///
/// Both halves or neither: a `UID` is only an identity together with the
/// generation it was observed under, and both arrive together from the append
/// that created the copy. Half of the pair means the row predates that append
/// or was written by hand, and acting on it would be guessing.
fn server_copy(draft: &Draft) -> Option<(Uid, UidValidity)> {
    Some((draft.server.uid?, draft.server.uid_validity?))
}

/// Rewrites a draft's recipient rows.
///
/// Deleting and reinserting is right here in a way it would not be for a
/// message: a recipient row carries no identity of its own that anything else
/// points at, and the composer's list is small and changes on every keystroke.
fn write_recipients(connection: &Connection, draft: &Draft) -> Result<()> {
    connection.execute(
        "DELETE FROM recipients WHERE draft_id = ?1",
        [draft.id.get()],
    )?;
    for (kind, addresses) in [("to", &draft.to), ("cc", &draft.cc), ("bcc", &draft.bcc)] {
        for (position, address) in addresses.iter().enumerate() {
            connection.execute(
                "INSERT INTO recipients (draft_id, kind, position, name, address,
                                         address_normalized)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    draft.id.get(),
                    kind,
                    position as i64,
                    address.name,
                    address.address,
                    address.normalized(),
                ],
            )?;
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
fn write_attachments(connection: &Connection, draft: &mut Draft) -> Result<()> {
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
    connection.execute(
        &format!("DELETE FROM attachments WHERE draft_id = ?1 AND id NOT IN ({placeholders})"),
        rusqlite::params_from_iter(arguments),
    )?;

    for (position, attachment) in draft.attachments.iter_mut().enumerate() {
        if attachment.id.is_assigned() {
            connection.execute(
                "UPDATE attachments
                    SET position = ?2, filename = ?3, mime_type = ?4, size = ?5,
                        content_id = ?6, disposition = ?7, disposition_raw = ?8,
                        part_id = ?9, blob_id = ?10
                  WHERE id = ?1",
                params![
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
                ],
            )?;
        } else {
            connection.execute(
                "INSERT INTO attachments (draft_id, position, filename, mime_type, size,
                                          content_id, disposition, disposition_raw, part_id,
                                          blob_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
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
                ],
            )?;
            attachment.id = AttachmentId::new(connection.last_insert_rowid());
        }
    }
    Ok(())
}

fn read_draft(row: &Row<'_>) -> rusqlite::Result<Draft> {
    let kind: String = row.get(3)?;
    let state: String = row.get(9)?;

    Ok(Draft {
        id: DraftId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        identity_id: row.get::<_, Option<i64>>(2)?.map(IdentityId::new),
        kind: DraftKind::from_name(&kind).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(unknown_enum("drafts.kind", kind)),
            )
        })?,
        in_reply_to: row.get::<_, Option<i64>>(4)?.map(MessageId::new),
        thread_id: row.get::<_, Option<i64>>(5)?.map(ThreadId::new),
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: row.get(6)?,
        body: MessageBody {
            text: row.get(7)?,
            html: row.get(8)?,
        },
        attachments: Vec::new(),
        state: DraftState::from_name(&state).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                9,
                rusqlite::types::Type::Text,
                Box::new(unknown_enum("drafts.state", state)),
            )
        })?,
        server: ServerIdentifiers {
            uid: row
                .get::<_, Option<i64>>(10)?
                .map(|uid| Uid::new(uid as u32)),
            uid_validity: row
                .get::<_, Option<i64>>(11)?
                .map(|validity| UidValidity::new(validity as u32)),
            mod_seq: row
                .get::<_, Option<i64>>(12)?
                .map(|seq| ModSeq::new(seq as u64)),
            remote_id: row.get(13)?,
        },
        created_at: from_millis(row.get(14)?),
        updated_at: from_millis(row.get(15)?),
    })
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
