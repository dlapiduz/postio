//! Each account's own map from role to server folder: the `mailbox_roles`
//! table (ADR 0025).
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
use rusqlite::{Connection, params};

use crate::error::Result;

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
    pub fn for_account(&self, account: AccountId) -> Result<Vec<(MailboxRole, String)>> {
        let mut statement = self
            .connection
            .prepare("SELECT role, path FROM mailbox_roles WHERE account_id = ?1 ORDER BY role")?;
        let rows = statement.query_map([account.get()], |row| {
            let role: String = row.get(0)?;
            let path: String = row.get(1)?;
            Ok((role, path))
        })?;
        let mut pairs = Vec::new();
        for row in rows {
            let (role, path) = row?;
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
    pub fn set(&self, account: AccountId, role: MailboxRole, path: &str) -> Result<()> {
        self.connection.execute(
            "INSERT INTO mailbox_roles (account_id, role, path, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (account_id, role) DO UPDATE
             SET path = excluded.path, updated_at = excluded.updated_at",
            params![
                account.get(),
                role.as_str(),
                path,
                Utc::now().timestamp_millis()
            ],
        )?;
        Ok(())
    }

    /// Forget the account's choice for `role`, so it resolves automatically
    /// again. Clearing a role that was never mapped is not an error.
    pub fn clear(&self, account: AccountId, role: MailboxRole) -> Result<()> {
        self.connection.execute(
            "DELETE FROM mailbox_roles WHERE account_id = ?1 AND role = ?2",
            params![account.get(), role.as_str()],
        )?;
        Ok(())
    }
}
