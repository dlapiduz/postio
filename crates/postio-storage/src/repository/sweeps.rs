//! The read behind "delete what the list sent" (#1931): which of an
//! account's messages a list sent.
//!
//! It only *finds* messages. Archiving or deleting them is a command, so it
//! is queued, undoable and local-first like every other verb. It never
//! returns a Flagged message.

use postio_model::ids::{AccountId, MessageId};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// Who a list's mail is identified by: its `List-Id`, or, for a sender that
/// sets none, their exact address (never the domain, which would take a whole
/// provider with it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMail<'a> {
    /// `messages.list_id`, as stored.
    ListId(&'a str),
    /// The `From` address, compared lower-cased.
    Sender(&'a str),
}

/// One page of candidates, and whether the page cut the answer short.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The messages, newest first.
    pub ids: Vec<MessageId>,
    /// `true` when more matched than `limit`.
    pub more: bool,
}

/// Read candidates for bulk clean-ups on one connection.
pub struct SweepRepository<'a> {
    connection: &'a Connection,
}

/// Folders whose mail is already past archiving: deleting from Trash or Junk
/// is not what "delete what the list sent" means, and Sent and Drafts are the
/// user's own words.
const SPENT: &str = "('trash', 'junk', 'sent', 'drafts')";

impl<'a> SweepRepository<'a> {
    /// Borrow `connection`.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Up to `limit` of `account`'s messages from `from`, newest first, in
    /// no folder that is already spent and not Flagged. One statement.
    pub async fn list_mail(
        &self,
        account: AccountId,
        from: ListMail<'_>,
        limit: u32,
    ) -> Result<Found> {
        let (join, filter, value) = match from {
            ListMail::ListId(list) => (String::new(), "messages.list_id = ?2", list.to_owned()),
            ListMail::Sender(address) => (
                "JOIN recipients ON recipients.message_id = messages.id
                                AND recipients.kind = 'from'
                 JOIN addresses ON addresses.id = recipients.address_id"
                    .to_owned(),
                "addresses.address_normalized = ?2",
                address.to_lowercase(),
            ),
        };
        let query = format!(
            "SELECT messages.id FROM messages
               JOIN mailboxes ON mailboxes.id = messages.mailbox_id
               {join}
              WHERE messages.account_id = ?1 AND {filter}
                AND messages.flagged = 0 AND messages.deleted_locally = 0
                AND mailboxes.role NOT IN {SPENT}
              ORDER BY messages.sort_at DESC, messages.id DESC
              LIMIT ?3"
        );
        self.page(
            &query,
            bind![account.get(), value, i64::from(limit) + 1].to_vec(),
            limit,
        )
        .await
    }

    async fn page(&self, query: &str, arguments: Vec<turso::Value>, limit: u32) -> Result<Found> {
        let mut ids = sql::all(self.connection, query, arguments, |row| {
            Ok(MessageId::new(row.col(0)?))
        })
        .await?;
        let more = ids.len() > limit as usize;
        ids.truncate(limit as usize);
        Ok(Found { ids, more })
    }
}
