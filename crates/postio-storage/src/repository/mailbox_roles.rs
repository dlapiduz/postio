//! Each account's own map from role to server folder: the `mailbox_roles`
//! table (ADR 0035).
//!
//! What the user chose, and nothing else. Which row currently *wears* a role
//! is `mailboxes.role`, written by discovery, and the two are deliberately
//! separate: this table is a statement about the server ("Sent Messages is
//! where sent mail goes"), keyed by path so it survives the folder's row being
//! retired and re-created, while `mailboxes.role` is what that statement
//! resolved to on the last pass. A path the server no longer lists stays here
//! as a dangling entry -- settings shows it, nothing drops it.
//!
//! The pair shape `for_account` returns is exactly what
//! `RoleOverrides::from_pairs` takes, so the sync side needs no new type.

use chrono::Utc;
use postio_model::{AccountId, MailboxRole};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// Read and write an account's role map on one connection.
pub struct MailboxRoleRepository<'a> {
    connection: &'a Connection,
}

impl<'a> MailboxRoleRepository<'a> {
    /// Borrow `connection` for role-map reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Every role the account has mapped, with the path it is mapped to,
    /// ordered by role so the answer is the same on every call.
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
            // A role the CHECK admits is one `from_name` parses; anything else
            // would be a schema change nobody made here.
            if let Some(role) = MailboxRole::from_name(&role) {
                pairs.push((role, path));
            }
        }
        Ok(pairs)
    }

    /// Map `role` to the folder at `path` for this account, replacing any
    /// earlier choice for the role.
    pub async fn set(&self, account: AccountId, role: MailboxRole, path: &str) -> Result<()> {
        self.connection
            .execute(
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

    /// Forget the account's choice for `role`, so it resolves automatically
    /// again. Clearing a role that was never mapped is not an error.
    pub async fn clear(&self, account: AccountId, role: MailboxRole) -> Result<()> {
        self.connection
            .execute(
                "DELETE FROM mailbox_roles WHERE account_id = ?1 AND role = ?2",
                bind![account.get(), role.as_str()],
            )
            .await?;
        // A choice supersedes a refusal: the user has answered the question
        // another way, so the record of the server saying no is stale and
        // must not keep suppressing an attempt.
        self.clear_refusal(account, role).await
    }

    /// Every role this account's server has refused to create a folder for,
    /// with the server's own words (spec 003, FR-031).
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
            // Same rule as `for_account` directly above: a role the CHECK
            // admits is one `from_name` parses, so anything else would be a
            // schema change nobody made here.
            if let Some(role) = MailboxRole::from_name(&role) {
                refusals.push((role, reason));
            }
        }
        Ok(refusals)
    }

    /// Record that this account's server refused to create a folder for
    /// `role`, so the next discovery pass does not ask again.
    ///
    /// Replaces any earlier refusal for the role: what matters is the current
    /// answer and the current reason, not how many times it has been given.
    pub async fn refuse(&self, account: AccountId, role: MailboxRole, reason: &str) -> Result<()> {
        self.connection
            .execute(
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

    /// Forget a refusal, so discovery may try again.
    ///
    /// Called when the folder turns up or the role is mapped by hand — both
    /// mean the question has been answered by something other than another
    /// attempt. Clearing one that was never recorded is not an error.
    pub async fn clear_refusal(&self, account: AccountId, role: MailboxRole) -> Result<()> {
        self.connection
            .execute(
                "DELETE FROM mailbox_role_refusals WHERE account_id = ?1 AND role = ?2",
                bind![account.get(), role.as_str()],
            )
            .await?;
        Ok(())
    }
}
