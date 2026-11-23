//! Each account's map from role to server folder (ADR 0035): what the user chose, keyed by path so
//! it survives the folder's row being retired and re-created. `mailboxes.role` is what discovery
//! resolved on the last pass. A path the server no longer lists stays as a dangling entry.
//! `for_account` returns the pair shape `RoleOverrides::from_pairs` takes.

use chrono::Utc;
use postio_model::{AccountId, MailboxRole};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

pub struct MailboxRoleRepository<'a> {
    connection: &'a Connection,
}

impl<'a> MailboxRoleRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn for_account(&self, account: AccountId) -> Result<Vec<(MailboxRole, String)>> {
        let rows: Vec<(String, String)> = sql::all(
            self.connection,
            "SELECT role, path FROM mailbox_roles WHERE account_id = ?1 ORDER BY role",
            [account.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let mut pairs = Vec::new();
        for (role, path) in rows {
            // The CHECK admits only roles `from_name` parses.
            if let Some(role) = MailboxRole::from_name(&role) {
                pairs.push((role, path));
            }
        }
        Ok(pairs)
    }

    pub async fn set(&self, account: AccountId, role: MailboxRole, path: &str) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO mailbox_roles (account_id, role, path, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, role) DO UPDATE
             SET path = excluded.path, updated_at = excluded.updated_at",
            bind![
                account.get(),
                role.as_str(),
                path,
                Utc::now().timestamp_millis()
            ],
        )
        .await?;
        Ok(())
    }

    /// Clearing a role that was never mapped is not an error.
    pub async fn clear(&self, account: AccountId, role: MailboxRole) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM mailbox_roles WHERE account_id = ?1 AND role = ?2",
            bind![account.get(), role.as_str()],
        )
        .await?;
        // A choice supersedes a refusal: the record of the server saying no would keep suppressing
        // an attempt.
        self.clear_refusal(account, role).await
    }

    pub async fn refusals(&self, account: AccountId) -> Result<Vec<(MailboxRole, String)>> {
        let rows: Vec<(String, String)> = sql::all(
            self.connection,
            "SELECT role, reason FROM mailbox_role_refusals
              WHERE account_id = ?1
              ORDER BY role",
            [account.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;

        let mut refusals = Vec::new();
        for (role, reason) in rows {
            if let Some(role) = MailboxRole::from_name(&role) {
                refusals.push((role, reason));
            }
        }
        Ok(refusals)
    }

    /// Replaces any earlier refusal: only the current answer and reason matter.
    pub async fn refuse(&self, account: AccountId, role: MailboxRole, reason: &str) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO mailbox_role_refusals (account_id, role, refused_at, reason)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, role) DO UPDATE
             SET refused_at = excluded.refused_at, reason = excluded.reason",
            bind![
                account.get(),
                role.as_str(),
                Utc::now().timestamp_millis(),
                reason
            ],
        )
        .await?;
        Ok(())
    }

    /// Called when the folder turns up or the role is mapped by hand. Clearing one never recorded
    /// is not an error.
    pub async fn clear_refusal(&self, account: AccountId, role: MailboxRole) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM mailbox_role_refusals WHERE account_id = ?1 AND role = ?2",
            bind![account.get(), role.as_str()],
        )
        .await?;
        Ok(())
    }
}
