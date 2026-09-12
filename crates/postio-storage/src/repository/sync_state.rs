//! Per-mailbox synchronization state: read and written as one unit.
//!
//! The table is the sync engine's own; nothing above it reads these columns.
//! [`MailboxRepository`](super::MailboxRepository) joins the server counters
//! onto a [`Mailbox`](postio_model::Mailbox) for display, but the sync engine
//! goes through here, because only this repository writes
//! `last_full_sync_at` — the column that says the local mailbox actually holds
//! the messages the counters claim.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Generation, MailboxId, MailboxStatus, ModSeq, ResyncPlan, SyncState, Uid,
};

use super::{from_millis, require_persisted, to_millis};

use crate::sql::{self, RowExt as _, bind};
use turso::Row;
use crate::store::Connection;
use crate::error::{Error, Result};

/// Reads and writes [`SyncState`] rows.
///
/// # Atomicity
///
/// Every method here takes a borrowed [`Connection`], and the engine's
/// `Transaction` derefs to one — so the sync engine builds this repository
/// *inside* the transaction that writes the messages, and the state and the
/// messages it describes commit or roll back together:
///
/// ```no_run
/// # use postio_model::{Generation, MailboxStatus, MailboxId};
/// # use postio_storage::repository::SyncStateRepository;
/// # fn main() -> Result<(), postio_storage::Error> {
/// # use postio_storage::key::{Purpose, StoreKey};
/// # let key = StoreKey::generate().derive(Purpose::Database);
/// # let database = postio_storage::Database::open("postio.db", &key)?;
/// # let mut connection = database.connection()?;
/// # let mailbox = MailboxId::new(1);
/// # let status = MailboxStatus::new(Generation::new(1));
/// let transaction = connection.transaction()?;
/// // ... write the fetched messages ...
/// SyncStateRepository::new(&transaction).observe(mailbox, &status, chrono::Utc::now())?;
/// SyncStateRepository::new(&transaction).complete_full_sync(mailbox, chrono::Utc::now())?;
/// transaction.commit()?;
/// # Ok(())
/// # }
/// ```
///
/// Getting that ordering wrong is the one bug this table exists to prevent: a
/// `HIGHESTMODSEQ` committed ahead of the messages it covers tells the next
/// incremental resync to skip exactly the messages that were lost.
#[derive(Debug)]
pub struct SyncStateRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "\
mailbox_id, account_id, uid_validity, uid_next, highest_mod_seq, last_full_sync_at, last_seen_at";

impl<'a> SyncStateRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// One mailbox's state, or `None` when there is no such mailbox.
    ///
    /// A mailbox always has a row — [`MailboxRepository::create`] writes one,
    /// `NULL`-filled, so "never synced" is a readable state rather than an
    /// absence — so `None` here means the mailbox itself is gone.
    ///
    /// [`MailboxRepository::create`]: super::MailboxRepository::create
    pub async fn get(&self, mailbox_id: MailboxId) -> Result<Option<SyncState>> {
        sql::first(
            self.connection,
            &format!(
            "SELECT {COLUMNS} FROM sync_state WHERE mailbox_id = ?1"
        ),
            [mailbox_id.get()],
            read_state,
        )
        .await}

    /// One mailbox's state, failing when the mailbox is not there.
    pub async fn require(&self, mailbox_id: MailboxId) -> Result<SyncState> {
        self.get(mailbox_id).await?.ok_or(Error::NotFound {
            entity: "mailbox",
            id: mailbox_id.get(),
        })
    }

    /// Every mailbox's state in an account, in mailbox-id order.
    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<SyncState>> {
        sql::all(
            self.connection,
            &format!(
            "SELECT {COLUMNS} FROM sync_state WHERE account_id = ?1 ORDER BY mailbox_id"
        ),
            [account_id.get()],
            read_state,
        )
        .await}

    /// Writes a whole state back, creating the row if it is somehow missing.
    ///
    /// Every column at once, deliberately: a partial update is how the counters
    /// drift apart from each other.
    pub async fn save(&self, state: &SyncState) -> Result<()> {
        let mailbox_id = require_persisted(state.mailbox_id.get(), "mailbox")?;
        let account_id = require_persisted(state.account_id.get(), "account")?;

        self.connection.execute(
            "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, uid_next,
                                     highest_mod_seq, last_full_sync_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (mailbox_id) DO UPDATE
                SET account_id = excluded.account_id,
                    uid_validity = excluded.uid_validity,
                    uid_next = excluded.uid_next,
                    highest_mod_seq = excluded.highest_mod_seq,
                    last_full_sync_at = excluded.last_full_sync_at,
                    last_seen_at = excluded.last_seen_at",
            bind![
                mailbox_id,
                account_id,
                state.generation.map(|value| i64::from(value.get())),
                state.uid_next.map(|value| i64::from(value.get())),
                state.highest_mod_seq.map(|value| value.get() as i64),
                state.last_full_sync_at.map(to_millis),
                state.last_seen_at.map(to_millis),
            ],
        ).await?;
        Ok(())
    }

    /// Records what the server reported, returning the state as it now stands.
    ///
    /// A `UIDVALIDITY` change drops the counters that belonged to the old UID
    /// space, including the completed-sync marker — see [`SyncState::observe`].
    pub async fn observe(
        &self,
        mailbox_id: MailboxId,
        status: &MailboxStatus,
        at: DateTime<Utc>,
    ) -> Result<SyncState> {
        self.mutate(mailbox_id, |state| state.observe(status, at))
            .await
    }

    /// Marks a full synchronization of this mailbox as complete.
    ///
    /// Call it *after* the messages are written and in the same transaction:
    /// this is the flag that says the local mailbox is whole.
    pub async fn complete_full_sync(
        &self,
        mailbox_id: MailboxId,
        at: DateTime<Utc>,
    ) -> Result<SyncState> {
        self.mutate(mailbox_id, |state| state.complete_full_sync(at))
            .await
    }

    /// Returns this mailbox back to never-synced.
    ///
    /// For the caller that has just discarded the mailbox's messages — after a
    /// `UIDVALIDITY` reset, or when the user asks for a rebuild. It does not
    /// delete anything itself: dropping the state and dropping the rows it
    /// describes belong in one transaction, and the caller owns that.
    pub async fn reset(&self, mailbox_id: MailboxId) -> Result<SyncState> {
        let account_id = self.require(mailbox_id).await?.account_id;
        let state = SyncState::never_synced(mailbox_id, account_id);
        self.save(&state).await?;
        Ok(state)
    }

    /// What the sync engine should do about this mailbox.
    ///
    /// A thin read plus [`SyncState::plan`]; the decision itself is pure and
    /// lives in the model.
    pub async fn plan(&self, mailbox_id: MailboxId, status: &MailboxStatus) -> Result<ResyncPlan> {
        Ok(self.require(mailbox_id).await?.plan(status))
    }

    async fn mutate(
        &self,
        mailbox_id: MailboxId,
        change: impl FnOnce(&mut SyncState),
    ) -> Result<SyncState> {
        // Read-modify-write, because the transitions are model logic and there
        // is no sensible way to spell "drop the MODSEQ if UIDVALIDITY moved" in
        // one UPDATE. The enclosing transaction — the caller's, or the implicit
        // one around a bare statement — is what makes it atomic.
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
    })
}
