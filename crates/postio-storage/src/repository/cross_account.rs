//! The cross-account move saga's state (#188, ADR 0005 Q9; migration 0020). A phase moves only
//! forward and `done` is reachable only from `confirmed`, so the only possible failure is a
//! duplicate, never a loss.

use chrono::Utc;
use postio_model::ids::{AccountId, CrossAccountMoveId, MailboxId, MessageId, RemoteId};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovePhase {
    Copying,
    Unconfirmed,
    Confirmed,
    Done,
    Aborted,
}

impl MovePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copying => "copying",
            Self::Unconfirmed => "unconfirmed",
            Self::Confirmed => "confirmed",
            Self::Done => "done",
            Self::Aborted => "aborted",
        }
    }

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

    /// Forward only, and `done` only from `confirmed`: however drainers race or replay, deleting
    /// the source cannot precede proving the copy.
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

#[derive(Debug, Clone, PartialEq)]
pub struct CrossAccountMove {
    pub id: CrossAccountMoveId,
    pub source_message: Option<MessageId>,
    pub source_account: Option<AccountId>,
    pub source_mailbox: Option<MailboxId>,
    pub target_account: Option<AccountId>,
    pub target_mailbox: Option<MailboxId>,
    pub target_message: Option<MessageId>,
    pub raw_blob_id: Option<String>,
    pub rfc_message_id: Option<String>,
    pub phase: MovePhase,
    pub confirmed_remote_id: Option<RemoteId>,
}

#[derive(Debug, Clone)]
pub struct NewCrossAccountMove {
    pub source_message: MessageId,
    pub source_account: AccountId,
    pub source_mailbox: MailboxId,
    pub target_account: AccountId,
    pub target_mailbox: MailboxId,
    pub target_message: Option<MessageId>,
    pub raw_blob_id: Option<String>,
    pub rfc_message_id: Option<String>,
}

pub struct CrossAccountMoveRepository<'a> {
    connection: &'a Connection,
}

impl<'a> CrossAccountMoveRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The row is on disk before either queue runs anything: resumability is this insert.
    pub async fn create(&self, saga: &NewCrossAccountMove) -> Result<CrossAccountMoveId> {
        let now = Utc::now().timestamp_millis();
        sql::execute(
            self.connection,
            "INSERT INTO cross_account_moves
                 (source_message_id, source_account_id, source_mailbox_id,
                  target_account_id, target_mailbox_id, target_message_id,
                  raw_blob_id, rfc_message_id, phase, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'copying', ?9, ?9)",
            bind![
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
        )
        .await?;
        Ok(CrossAccountMoveId::new(self.connection.last_insert_rowid()))
    }

    pub async fn get(&self, id: CrossAccountMoveId) -> Result<Option<CrossAccountMove>> {
        sql::first(
            self.connection,
            "SELECT id, source_message_id, source_account_id, source_mailbox_id,
                        target_account_id, target_mailbox_id, target_message_id,
                        raw_blob_id, rfc_message_id, phase, confirmed_remote_id
                   FROM cross_account_moves WHERE id = ?1",
            [id.get()],
            |row| {
                let phase: String = row.col(9)?;
                Ok(CrossAccountMove {
                    id: CrossAccountMoveId::new(row.col(0)?),
                    source_message: row.col::<Option<i64>>(1)?.map(MessageId::new),
                    source_account: row.col::<Option<i64>>(2)?.map(AccountId::new),
                    source_mailbox: row.col::<Option<i64>>(3)?.map(MailboxId::new),
                    target_account: row.col::<Option<i64>>(4)?.map(AccountId::new),
                    target_mailbox: row.col::<Option<i64>>(5)?.map(MailboxId::new),
                    target_message: row.col::<Option<i64>>(6)?.map(MessageId::new),
                    raw_blob_id: row.col(7)?,
                    rfc_message_id: row.col(8)?,
                    phase: MovePhase::parse(&phase).unwrap_or(MovePhase::Aborted),
                    confirmed_remote_id: row.col::<Option<String>>(10)?.map(RemoteId::new),
                })
            },
        )
        .await
    }
    /// Phases are the caller's: undo must see `done` too, since a finished move is the one most
    /// likely taken back (#531).
    pub async fn for_sources(
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
        let mut statement = sql::statement(
            self.connection,
            &format!(
                "SELECT id FROM cross_account_moves
              WHERE phase IN ({list})
              ORDER BY id"
            ),
        )
        .await?;
        let ids: Vec<i64> = sql::mapped(&mut statement, (), |row| row.col(0)).await?;
        let mut found = Vec::new();
        for id in ids {
            let Some(saga) = self.get(CrossAccountMoveId::new(id)).await? else {
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

    /// Refusal is an error, not a no-op: a drainer asking for an illegal transition has misread the
    /// saga.
    pub async fn transition(&self, id: CrossAccountMoveId, next: MovePhase) -> Result<()> {
        let Some(current) = self.get(id).await? else {
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
        sql::execute(
            self.connection,
            "UPDATE cross_account_moves SET phase = ?2, updated_at = ?3 WHERE id = ?1",
            bind![id.get(), next.as_str(), Utc::now().timestamp_millis()],
        )
        .await?;
        Ok(())
    }

    /// `remote_id` is `Some` from APPENDUID, `None` when a Message-ID search proved presence
    /// without naming where.
    pub async fn confirm(
        &self,
        id: CrossAccountMoveId,
        remote_id: Option<&RemoteId>,
    ) -> Result<()> {
        self.transition(id, MovePhase::Confirmed).await?;
        let Some(remote_id) = remote_id else {
            return Ok(());
        };
        sql::execute(
            self.connection,
            "UPDATE cross_account_moves SET confirmed_remote_id = ?2 WHERE id = ?1",
            bind![id.get(), remote_id.as_str()],
        )
        .await?;

        // Also onto the row the user is looking at (ADR 0026, #531); the confirmation is a whole
        // identity, since `APPENDUID` carries UIDVALIDITY too.
        // Without it the target's next sync finds no row by `find_by_remote_id` and creates a
        // second one, and an inverse saga has no coordinate for the copy it must remove.
        sql::execute(
            self.connection,
            "UPDATE messages SET remote_id = ?2
              WHERE id = (SELECT target_message_id FROM cross_account_moves WHERE id = ?1)",
            bind![id.get(), remote_id.as_str()],
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    async fn a_saga(connection: &Connection) -> CrossAccountMoveId {
        let (account, inbox) = test_support::account_with_inbox(connection).await;
        let mut second = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        crate::repository::AccountRepository::new(connection)
            .create(&mut second)
            .await
            .expect("second account");
        let target = test_support::mailbox(connection, &second, "INBOX").await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        let message_id = crate::repository::MessageRepository::new(connection)
            .create(&mut message)
            .await
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
            .await
            .expect("a saga")
    }

    #[tokio::test]
    async fn the_phase_walk_is_forward_only_and_done_needs_confirmed() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection).await;

        assert!(
            sagas.transition(id, MovePhase::Done).await.is_err(),
            "copying -> done skips the proof, and the proof is the point"
        );
        sagas
            .transition(id, MovePhase::Unconfirmed)
            .await
            .expect("copying -> unconfirmed: the append ran, arrival unproven");
        assert!(
            sagas.transition(id, MovePhase::Done).await.is_err(),
            "unconfirmed -> done is exactly the guess the ADR forbids"
        );
        sagas
            .confirm(id, Some(&RemoteId::new("1:4242")))
            .await
            .expect("unconfirmed -> confirmed, with the identity recorded");
        let saga = sagas.get(id).await.expect("read").expect("the saga");
        assert_eq!(saga.phase, MovePhase::Confirmed);
        assert_eq!(saga.confirmed_remote_id, Some(RemoteId::new("1:4242")));

        sagas
            .transition(id, MovePhase::Done)
            .await
            .expect("confirmed -> done is the one legal ending that deletes");
        assert!(
            sagas.transition(id, MovePhase::Copying).await.is_err(),
            "done is terminal; a saga never runs backwards"
        );
    }

    #[tokio::test]
    async fn an_aborted_saga_is_terminal_and_deletes_nothing() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection).await;

        sagas
            .transition(id, MovePhase::Aborted)
            .await
            .expect("a saga may abort from copying");
        assert!(
            sagas.transition(id, MovePhase::Confirmed).await.is_err(),
            "aborted is terminal"
        );
        let saga = sagas.get(id).await.expect("read").expect("the saga");
        assert!(
            saga.source_message.is_some(),
            "aborting touched no rows: the source copy is intact (Q13)"
        );
    }

    #[tokio::test]
    async fn removing_the_target_account_leaves_the_saga_naming_nobody() {
        // The CASCADE that removes an account must not vanish a half-finished saga: SET NULL leaves
        // the row to be aborted with the source intact.
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let sagas = CrossAccountMoveRepository::new(&connection);
        let id = a_saga(&connection).await;
        let target = sagas
            .get(id)
            .await
            .expect("read")
            .expect("the saga")
            .target_account
            .expect("a target");

        crate::repository::AccountRepository::new(&connection)
            .delete(target)
            .await
            .expect("remove the target account");

        let saga = sagas
            .get(id)
            .await
            .expect("read")
            .expect("the saga survives");
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
