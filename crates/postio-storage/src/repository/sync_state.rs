//! Per-mailbox sync state, read and written as one unit. Only this repository writes
//! `last_full_sync_at`, the column saying the local mailbox holds the messages the counters claim.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Generation, MailboxId, MailboxStatus, ModSeq, ResyncPlan, SyncState, Uid,
};

use super::{from_millis, require_persisted, to_millis};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Built inside the transaction that writes the messages, so the state and the messages it
/// describes commit or roll back together. Getting that ordering wrong is the one bug this table
/// exists to prevent: a `HIGHESTMODSEQ` committed ahead of its messages tells the next incremental
/// resync to skip exactly the messages that were lost.
#[derive(Debug)]
pub struct SyncStateRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "\
mailbox_id, account_id, uid_validity, uid_next, highest_mod_seq, last_full_sync_at, last_seen_at, \
passes_since_flag_sweep";

impl<'a> SyncStateRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// A mailbox always has a row ([`MailboxRepository::create`] writes it `NULL`-filled, so "never
    /// synced" is readable), so `None` means the mailbox is gone.
    ///
    /// [`MailboxRepository::create`]: super::MailboxRepository::create
    pub async fn get(&self, mailbox_id: MailboxId) -> Result<Option<SyncState>> {
        sql::first(
            self.connection,
            &format!("SELECT {COLUMNS} FROM sync_state WHERE mailbox_id = ?1"),
            [mailbox_id.get()],
            read_state,
        )
        .await
    }

    pub async fn require(&self, mailbox_id: MailboxId) -> Result<SyncState> {
        self.get(mailbox_id).await?.ok_or(Error::NotFound {
            entity: "mailbox",
            id: mailbox_id.get(),
        })
    }

    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<SyncState>> {
        sql::all(
            self.connection,
            &format!("SELECT {COLUMNS} FROM sync_state WHERE account_id = ?1 ORDER BY mailbox_id"),
            [account_id.get()],
            read_state,
        )
        .await
    }

    /// Every column at once: a partial update is how the counters drift apart.
    pub async fn save(&self, state: &SyncState) -> Result<()> {
        let mailbox_id = require_persisted(state.mailbox_id.get(), "mailbox")?;
        let account_id = require_persisted(state.account_id.get(), "account")?;

        sql::execute(
            self.connection,
            "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, uid_next,
                                     highest_mod_seq, last_full_sync_at, last_seen_at,
                                     passes_since_flag_sweep)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT (mailbox_id) DO UPDATE
                SET account_id = excluded.account_id,
                    uid_validity = excluded.uid_validity,
                    uid_next = excluded.uid_next,
                    highest_mod_seq = excluded.highest_mod_seq,
                    last_full_sync_at = excluded.last_full_sync_at,
                    last_seen_at = excluded.last_seen_at,
                    passes_since_flag_sweep = excluded.passes_since_flag_sweep",
            bind![
                mailbox_id,
                account_id,
                state.generation.map(|value| i64::from(value.get())),
                state.uid_next.map(|value| i64::from(value.get())),
                state.highest_mod_seq.map(|value| value.get() as i64),
                state.last_full_sync_at.map(to_millis),
                state.last_seen_at.map(to_millis),
                i64::from(state.passes_since_flag_sweep),
            ],
        )
        .await?;
        Ok(())
    }

    /// A `UIDVALIDITY` change drops the counters of the old UID space, including the completed-sync
    /// marker (see [`SyncState::observe`]).
    pub async fn observe(
        &self,
        mailbox_id: MailboxId,
        status: &MailboxStatus,
        at: DateTime<Utc>,
    ) -> Result<SyncState> {
        self.mutate(mailbox_id, |state| state.observe(status, at))
            .await
    }

    /// Call after the messages are written, in the same transaction: this flag says the local
    /// mailbox is whole.
    pub async fn complete_full_sync(
        &self,
        mailbox_id: MailboxId,
        at: DateTime<Utc>,
    ) -> Result<SyncState> {
        self.mutate(mailbox_id, |state| state.complete_full_sync(at))
            .await
    }

    /// Called after the pass's flags were reconciled and [`observe`](Self::observe) took the
    /// server's `HIGHESTMODSEQ`.
    pub async fn record_flag_pass(&self, mailbox_id: MailboxId, swept: bool) -> Result<SyncState> {
        self.mutate(mailbox_id, |state| state.record_flag_pass(swept))
            .await
    }

    /// Deletes nothing itself: dropping the state and the rows it describes belong in one
    /// transaction the caller owns.
    pub async fn reset(&self, mailbox_id: MailboxId) -> Result<SyncState> {
        let account_id = self.require(mailbox_id).await?.account_id;
        let state = SyncState::never_synced(mailbox_id, account_id);
        self.save(&state).await?;
        Ok(state)
    }

    /// The decision is pure and lives in the model ([`SyncState::plan`]).
    pub async fn plan(&self, mailbox_id: MailboxId, status: &MailboxStatus) -> Result<ResyncPlan> {
        Ok(self.require(mailbox_id).await?.plan(status))
    }

    async fn mutate(
        &self,
        mailbox_id: MailboxId,
        change: impl FnOnce(&mut SyncState),
    ) -> Result<SyncState> {
        // Read-modify-write: the transitions are model logic with no single-UPDATE spelling. The
        // enclosing transaction makes it atomic.
        sql::in_scope(self.connection, |transaction| async move {
            let repository = SyncStateRepository::new(&transaction);
            let mut state = repository.require(mailbox_id).await?;
            change(&mut state);
            repository.save(&state).await?;
            Ok(state)
        })
        .await
    }
}

fn read_state(row: &Row) -> Result<SyncState> {
    Ok(SyncState {
        mailbox_id: MailboxId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        generation: row
            .col::<Option<i64>>(2)?
            .map(|value| Generation::new(value as u32)),
        uid_next: row
            .col::<Option<i64>>(3)?
            .map(|value| Uid::new(value as u32)),
        highest_mod_seq: row
            .col::<Option<i64>>(4)?
            .map(|value| ModSeq::new(value as u64)),
        last_full_sync_at: row.col::<Option<i64>>(5)?.map(from_millis),
        last_seen_at: row.col::<Option<i64>>(6)?.map(from_millis),
        passes_since_flag_sweep: row.col::<i64>(7)? as u32,
    })
}
