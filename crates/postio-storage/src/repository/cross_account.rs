//! The cross-account move saga's own state (#188, ADR 0005 Q9).
//!
//! See migration 0020 for the table and the phase vocabulary. What this
//! repository adds over plain rows is the **transition rule**: a phase can
//! only move forward along the saga, and in particular nothing can reach
//! `done` except from `confirmed` — the ordering that makes the only
//! possible failure a duplicate, never a loss.

use chrono::Utc;
use postio_model::ids::{AccountId, CrossAccountMoveId, MailboxId, MessageId, RemoteId};
use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{Error, Result};

/// Where a saga is in its life. See migration 0020 for what each means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovePhase {
    /// The copy may be (re-)attempted; nothing is deleted.
    Copying,
    /// The append ran but arrival could not be proven: stop and ask.
    Unconfirmed,
    /// The target has the message; the source's remove may run.
    Confirmed,
    /// The source copy is gone; the move is complete.
    Done,
    /// Ended without deleting anything; the source copy is intact.
    Aborted,
}

impl MovePhase {
    /// The stored spelling, constrained by the schema's `CHECK`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copying => "copying",
            Self::Unconfirmed => "unconfirmed",
            Self::Confirmed => "confirmed",
            Self::Done => "done",
            Self::Aborted => "aborted",
        }
    }

    /// The inverse of [`as_str`](Self::as_str).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "copying" => Some(Self::Copying),
            "unconfirmed" => Some(Self::Unconfirmed),
            "confirmed" => Some(Self::Confirmed),
            "done" => Some(Self::Done),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }

    /// Whether the saga may move from `self` to `next`.
    ///
    /// Forward only, and `done` is reachable **only** from `confirmed`:
    /// however the drainers race or replay, a phase walk that deletes the
    /// source cannot happen before a phase walk that proved the copy.
    pub fn allows(self, next: MovePhase) -> bool {
        matches!(
            (self, next),
            (Self::Copying, Self::Unconfirmed)
                | (Self::Copying, Self::Confirmed)
                | (Self::Copying, Self::Aborted)
                | (Self::Unconfirmed, Self::Confirmed)
                | (Self::Unconfirmed, Self::Aborted)
                | (Self::Confirmed, Self::Done)
                | (Self::Confirmed, Self::Aborted)
        )
    }
}

/// One saga, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct CrossAccountMove {
    /// The saga's id, referenced by both queues' operations.
    pub id: CrossAccountMoveId,
    /// The source row, until phase 3 removes it.
    pub source_message: Option<MessageId>,
    /// The account the message leaves.
    pub source_account: Option<AccountId>,
    /// The mailbox it leaves.
    pub source_mailbox: Option<MailboxId>,
    /// The account it arrives in.
    pub target_account: Option<AccountId>,
    /// The mailbox it arrives in.
    pub target_mailbox: Option<MailboxId>,
    /// The provisional local row shown in the target immediately.
    pub target_message: Option<MessageId>,
    /// The raw bytes to append, content-addressed.
    pub raw_blob_id: Option<String>,
    /// The Message-ID: idempotency key, and the no-UIDPLUS confirmation.
    pub rfc_message_id: Option<String>,
    /// Where the saga is.
    pub phase: MovePhase,
    /// Where the target server filed it, once proven.
    pub confirmed_remote_id: Option<RemoteId>,
}

/// What a new saga needs to exist. Everything else starts empty.
#[derive(Debug, Clone)]
pub struct NewCrossAccountMove {
    /// The source row.
    pub source_message: MessageId,
    /// The account the message leaves.
    pub source_account: AccountId,
    /// The mailbox it leaves.
    pub source_mailbox: MailboxId,
    /// The account it arrives in.
    pub target_account: AccountId,
    /// The mailbox it arrives in.
    pub target_mailbox: MailboxId,
    /// The provisional local row in the target.
    pub target_message: Option<MessageId>,
    /// The raw bytes to append.
    pub raw_blob_id: Option<String>,
    /// The Message-ID, when the message has one.
    pub rfc_message_id: Option<String>,
}

/// Read and write sagas on one connection.
pub struct CrossAccountMoveRepository<'a> {
    connection: &'a Connection,
}

impl<'a> CrossAccountMoveRepository<'a> {
    /// Borrow `connection`.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Start a saga, in `copying`. The row is on disk before either queue
    /// runs anything — resumability is this insert.
    pub fn create(&self, saga: &NewCrossAccountMove) -> Result<CrossAccountMoveId> {
        let now = Utc::now().timestamp_millis();
        self.connection.execute(
            "INSERT INTO cross_account_moves
                 (source_message_id, source_account_id, source_mailbox_id,
                  target_account_id, target_mailbox_id, target_message_id,
                  raw_blob_id, rfc_message_id, phase, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'copying', ?9, ?9)",
            params![
                saga.source_message.get(),
                saga.source_account.get(),
                saga.source_mailbox.get(),
                saga.target_account.get(),
                saga.target_mailbox.get(),
                saga.target_message.map(MessageId::get),
                saga.raw_blob_id,
                saga.rfc_message_id,
                now,
            ],
        )?;
        Ok(CrossAccountMoveId::new(self.connection.last_insert_rowid()))
    }

    /// One saga, or `None`.
    pub fn get(&self, id: CrossAccountMoveId) -> Result<Option<CrossAccountMove>> {
        self.connection
            .query_row(
                "SELECT id, source_message_id, source_account_id, source_mailbox_id,
                        target_account_id, target_mailbox_id, target_message_id,
                        raw_blob_id, rfc_message_id, phase, confirmed_remote_id
                   FROM cross_account_moves WHERE id = ?1",
                [id.get()],
                |row| {
                    let phase: String = row.get(9)?;
                    Ok(CrossAccountMove {
                        id: CrossAccountMoveId::new(row.get(0)?),
                        source_message: row.get::<_, Option<i64>>(1)?.map(MessageId::new),
                        source_account: row.get::<_, Option<i64>>(2)?.map(AccountId::new),
                        source_mailbox: row.get::<_, Option<i64>>(3)?.map(MailboxId::new),
                        target_account: row.get::<_, Option<i64>>(4)?.map(AccountId::new),
                        target_mailbox: row.get::<_, Option<i64>>(5)?.map(MailboxId::new),
                        target_message: row.get::<_, Option<i64>>(6)?.map(MessageId::new),
                        raw_blob_id: row.get(7)?,
                        rfc_message_id: row.get(8)?,
                        phase: MovePhase::parse(&phase).unwrap_or(MovePhase::Aborted),
                        confirmed_remote_id: row.get::<_, Option<String>>(10)?.map(RemoteId::new),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }
    /// Every saga in one of `phases` whose *source* is among `sources`.
    ///
    /// Phases are the caller's because the two callers want different sets.
    /// The forward path asks about sagas it can still walk out of; undo has
    /// to see `done` as well, since a move that finished is exactly the one
    /// a user is most likely to take back and is not "open" by any
    /// definition the forward path needed (#531).
    pub fn for_sources(
        &self,
        sources: &[MessageId],
        phases: &[MovePhase],
    ) -> Result<Vec<CrossAccountMove>> {
        if sources.is_empty() || phases.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: std::collections::BTreeSet<i64> = sources.iter().map(|id| id.get()).collect();
        // Built rather than bound: `phase` is a closed set of five literals
        // this crate owns, so there is no user text anywhere near this.
        let list = phases
            .iter()
            .map(|phase| format!("'{}'", phase.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = self.connection.prepare(&format!(
            "SELECT id FROM cross_account_moves
              WHERE phase IN ({list})
              ORDER BY id"
        ))?;
        let ids: Vec<i64> = statement
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        let mut found = Vec::new();
        for id in ids {
            let Some(saga) = self.get(CrossAccountMoveId::new(id))? else {
                continue;
            };
            if saga
                .source_message
                .is_some_and(|source| wanted.contains(&source.get()))
            {
                found.push(saga);
            }
        }
        Ok(found)
    }

    /// Move the saga to `next`, enforcing [`MovePhase::allows`].
    ///
    /// Refusal is an error, not a no-op: a drainer asking for an illegal
    /// transition has misread the saga, and silently ignoring it would let
    /// the walk continue on a wrong belief.
    pub fn transition(&self, id: CrossAccountMoveId, next: MovePhase) -> Result<()> {
        let Some(current) = self.get(id)? else {
            return Err(Error::NotFound {
                entity: "cross-account move",
                id: id.get(),
            });
        };
        if !current.phase.allows(next) {
            return Err(Error::ForbiddenTransition {
                what: "cross-account move phase",
                reason: format!(
                    "{} cannot become {} — the saga only moves forward, and \
                     nothing reaches done except through confirmed",
                    current.phase.as_str(),
                    next.as_str()
                ),
            });
        }
        self.connection.execute(
            "UPDATE cross_account_moves SET phase = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.get(), next.as_str(), Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }

    /// Record the proof of arrival and move to `confirmed`.
    ///
    /// `remote_id` is `Some` from APPENDUID, `None` when a Message-ID
    /// search proved presence without naming where.
    pub fn confirm(&self, id: CrossAccountMoveId, remote_id: Option<&RemoteId>) -> Result<()> {
        self.transition(id, MovePhase::Confirmed)?;
        let Some(remote_id) = remote_id else {
            return Ok(());
        };
        self.connection.execute(
            "UPDATE cross_account_moves SET confirmed_remote_id = ?2 WHERE id = ?1",
            params![id.get(), remote_id.as_str()],
        )?;

        // And onto the row the user is looking at (ADR 0026, #531).
        //
        // The confirmation is a whole identity, not half of one: `APPENDUID`
        // carries the destination mailbox's `UIDVALIDITY` as well as the
        // assigned UID, and the no-UIDPLUS fallback searches a mailbox whose
        // live generation `ensure_selected` has just observed. So there is no
        // half-identified row to be afraid of here — the fear that kept this
        // write out is answered in the ADR.
        //
        // Two things go wrong without it. The target account's next sync
        // matches fetched mail to existing rows by
        // `find_by_remote_id(mailbox_id, remote_id)`, so a provisional copy
        // carrying nothing gets no match and becomes a **second** row for the
        // same message. And an inverse saga (#531) has no coordinate for the
        // copy it must remove — which is the failure that reaches no server
        // and reports success.
        self.connection.execute(
            "UPDATE messages SET remote_id = ?2
              WHERE id = (SELECT target_message_id FROM cross_account_moves WHERE id = ?1)",
            params![id.get(), remote_id.as_str()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    fn a_saga(connection: &Connection) -> CrossAccountMoveId {
        let (account, inbox) = test_support::account_with_inbox(connection);
        let mut second = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        crate::repository::AccountRepository::new(connection)
            .create(&mut second)
            .expect("second account");
        let target = test_support::mailbox(connection, &second, "INBOX");
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        let message_id = crate::repository::MessageRepository::new(connection)
            .create(&mut message)
            .expect("a message");
        CrossAccountMoveRepository::new(connection)
            .create(&NewCrossAccountMove {
                source_message: message_id,
                source_account: account.id,
                source_mailbox: inbox,
                target_account: second.id,
                target_mailbox: target.id,
                target_message: None,
                raw_blob_id: None,
                rfc_message_id: Some("<pair@example.com>".to_string()),
            })
            .expect("a saga")
    }

    #[test]
    fn the_phase_walk_is_forward_only_and_done_needs_confirmed() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection);

        // The transition that would lose mail: deleting the source while
        // the copy is unproven. Refused however it is asked for.
        assert!(
            sagas.transition(id, MovePhase::Done).is_err(),
            "copying -> done skips the proof, and the proof is the point"
        );
        sagas
            .transition(id, MovePhase::Unconfirmed)
            .expect("copying -> unconfirmed: the append ran, arrival unproven");
        assert!(
            sagas.transition(id, MovePhase::Done).is_err(),
            "unconfirmed -> done is exactly the guess the ADR forbids"
        );
        sagas
            .confirm(id, Some(&RemoteId::new("1:4242")))
            .expect("unconfirmed -> confirmed, with the identity recorded");
        let saga = sagas.get(id).expect("read").expect("the saga");
        assert_eq!(saga.phase, MovePhase::Confirmed);
        assert_eq!(saga.confirmed_remote_id, Some(RemoteId::new("1:4242")));

        sagas
            .transition(id, MovePhase::Done)
            .expect("confirmed -> done is the one legal ending that deletes");
        assert!(
            sagas.transition(id, MovePhase::Copying).is_err(),
            "done is terminal; a saga never runs backwards"
        );
    }

    #[test]
    fn an_aborted_saga_is_terminal_and_deletes_nothing() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection);

        sagas
            .transition(id, MovePhase::Aborted)
            .expect("a saga may abort from copying");
        assert!(
            sagas.transition(id, MovePhase::Confirmed).is_err(),
            "aborted is terminal"
        );
        let saga = sagas.get(id).expect("read").expect("the saga");
        assert!(
            saga.source_message.is_some(),
            "aborting touched no rows: the source copy is intact (Q13)"
        );
    }

    #[test]
    fn removing_the_target_account_leaves_the_saga_naming_nobody() {
        // Q13: the CASCADE that removes an account must not silently vanish
        // a half-finished saga — SET NULL leaves the row to be aborted by
        // whoever reads it next, with the source intact.
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection);
        let target = sagas
            .get(id)
            .expect("read")
            .expect("the saga")
            .target_account
            .expect("a target");

        crate::repository::AccountRepository::new(&connection)
            .delete(target)
            .expect("remove the target account");

        let saga = sagas.get(id).expect("read").expect("the saga survives");
        assert_eq!(
            saga.target_account, None,
            "the target is gone, not the saga"
        );
        assert!(
            saga.source_message.is_some(),
            "and the source copy is exactly where it was"
        );
    }
}
