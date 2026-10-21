//! Which mail is old enough to empty from Junk and Trash (#1932).
//!
//! Finds candidates only; the deleting is [`crate::actions::purge`]. One page
//! per call, oldest first, so a pass over a mailbox of a hundred thousand
//! reads a few hundred ids at a time and never the mailbox.

use chrono::{DateTime, Utc};
use postio_model::MailboxRole;
use postio_model::ids::{AccountId, MailboxId, MessageId};

use super::to_millis;
use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// One page of expired mail, grouped as the purge needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expired {
    /// `(mailbox, message)`, oldest first.
    pub messages: Vec<(MailboxId, MessageId)>,
    /// More matched than the page held.
    pub more: bool,
}

/// Read expired mail on one connection.
pub struct ExpiryRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ExpiryRepository<'a> {
    /// Borrow `connection`.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Up to `limit` of `account`'s messages in folders of `role` received
    /// before `before`, oldest first. Never a Flagged message, and none
    /// already hidden by a delete in flight. One statement.
    pub async fn expired_in(
        &self,
        account: AccountId,
        role: MailboxRole,
        before: DateTime<Utc>,
        limit: u32,
    ) -> Result<Expired> {
        let mut messages = sql::all(
            self.connection,
            "SELECT messages.mailbox_id, messages.id FROM messages
               JOIN mailboxes ON mailboxes.id = messages.mailbox_id
              WHERE messages.account_id = ?1 AND mailboxes.role = ?2
                AND messages.received_at < ?3
                AND messages.flagged = 0 AND messages.deleted_locally = 0
              ORDER BY messages.received_at ASC, messages.id ASC
              LIMIT ?4",
            bind![
                account.get(),
                role.as_str(),
                to_millis(before),
                i64::from(limit) + 1
            ]
            .to_vec(),
            |row| Ok((MailboxId::new(row.col(0)?), MessageId::new(row.col(1)?))),
        )
        .await?;
        let more = messages.len() > limit as usize;
        messages.truncate(limit as usize);
        Ok(Expired { messages, more })
    }
}
