//! What Focus has classified (spec 007, data-model.md "`focus_classified`").
//!
//! Focus classifies mail twice: as it is filed, from its headers, and once
//! its body is here, for invitations and the needs-action detector. Mail it
//! was not running for -- filed by another app, or here before Focus first
//! opened -- is caught up on when Focus starts, in the background, newest
//! first (FR-134, FR-141). A record says a message has had a stage at a
//! classifier version; the catch-up reads what has none at the current one,
//! so a newer classifier makes every message due again.

use chrono::{DateTime, Utc};
use postio_model::{MailboxId, MessageId};

use super::messages::placeholders;
use super::to_millis;

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// Which of Focus's two moments a record is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FocusStage {
    /// As the message was filed: filtering and holding.
    Filing,
    /// Once its body was here: invitations, questions and to-dos.
    Body,
}

impl FocusStage {
    /// The stage as the store spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            FocusStage::Filing => "filing",
            FocusStage::Body => "body",
        }
    }
}

/// Reads and writes Focus's classification records.
#[derive(Debug)]
pub struct FocusClassifiedRepository<'a> {
    connection: &'a Connection,
}

impl<'a> FocusClassifiedRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Up to `limit` messages in `inbox`, filed since `since`, whose body is
    /// here and that the body stage has not classified at `version`, newest
    /// first: the body stage's catch-up, a batch at a time (FR-141).
    ///
    /// One statement: a walk of the inbox's own list index, newest first,
    /// that stops at `since` or at the batch, each row's record a lookup by
    /// its key.
    pub async fn pending_bodies(
        &self,
        inbox: MailboxId,
        since: DateTime<Utc>,
        version: u32,
        limit: u32,
    ) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            Self::explain_pending_bodies(),
            bind![
                inbox.get(),
                to_millis(since),
                i64::from(version),
                i64::from(limit)
            ],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::pending_bodies`] runs.
    pub fn explain_pending_bodies() -> &'static str {
        "SELECT m.id FROM messages m
          WHERE m.mailbox_id = ?1 AND m.sort_at >= ?2 AND m.deleted_locally = 0
            AND m.body_state IN ('partial', 'full')
            AND NOT EXISTS (SELECT 1 FROM focus_classified c
                             WHERE c.message_id = m.id AND c.stage = 'body'
                               AND c.version = ?3)
          ORDER BY m.sort_at DESC, m.id DESC
          LIMIT ?4"
    }

    /// Up to `limit` messages filed in `inbox` after `after` -- the newest
    /// message Focus had accounted for when it last ran -- that have not been
    /// filed at `version`, newest first: mail another app filed while Focus
    /// was closed, which Focus sorts when it opens (FR-134, US9 scenario 8).
    ///
    /// One statement: message ids only grow, so the walk is of the rows
    /// newer than the mark, newest first, and never of the mail before it.
    pub async fn pending_filings(
        &self,
        inbox: MailboxId,
        after: MessageId,
        version: u32,
        limit: u32,
    ) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            Self::explain_pending_filings(),
            bind![
                inbox.get(),
                after.get(),
                i64::from(version),
                i64::from(limit)
            ],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::pending_filings`] runs.
    pub fn explain_pending_filings() -> &'static str {
        "SELECT m.id FROM messages m
          WHERE m.id > ?2 AND m.mailbox_id = ?1 AND m.deleted_locally = 0
            AND NOT EXISTS (SELECT 1 FROM focus_classified c
                             WHERE c.message_id = m.id AND c.stage = 'filing'
                               AND c.version = ?3)
          ORDER BY m.id DESC
          LIMIT ?4"
    }

    /// Which of `messages` -- bodies that just landed, in any folder -- are
    /// the body stage's to classify: the mail its catch-up would take, in an
    /// inbox, filed since `since`, body here, and not yet classified at
    /// `version`. Newest first, in one statement.
    pub async fn bodies_to_classify(
        &self,
        messages: &[MessageId],
        since: DateTime<Utc>,
        version: u32,
    ) -> Result<Vec<MessageId>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        let sql = format!(
            "SELECT m.id FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
              WHERE m.id IN ({}) AND b.role = 'inbox'
                AND m.sort_at >= ?1 AND m.deleted_locally = 0
                AND m.body_state IN ('partial', 'full')
                AND NOT EXISTS (SELECT 1 FROM focus_classified c
                                 WHERE c.message_id = m.id AND c.stage = 'body'
                                   AND c.version = ?2)
              ORDER BY m.sort_at DESC, m.id DESC",
            placeholders(messages.len(), 3)
        );
        let mut arguments = vec![
            turso::Value::Integer(to_millis(since)),
            turso::Value::Integer(i64::from(version)),
        ];
        arguments.extend(
            messages
                .iter()
                .map(|message| turso::Value::Integer(message.get())),
        );
        sql::all(self.connection, &sql, arguments, |row| {
            Ok(MessageId::new(row.col(0)?))
        })
        .await
    }

    /// Records that `messages` have had `stage` at `version`, replacing
    /// whatever was recorded for them at that stage before.
    pub async fn record(
        &self,
        messages: &[MessageId],
        stage: FocusStage,
        version: u32,
    ) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let sql = format!(
            "INSERT INTO focus_classified (message_id, stage, version)
             SELECT id, ?1, ?2 FROM messages WHERE id IN ({})
             ON CONFLICT (message_id, stage) DO UPDATE SET version = excluded.version",
            placeholders(messages.len(), 3)
        );
        let mut arguments = vec![
            turso::Value::Text(stage.as_str().to_owned()),
            turso::Value::Integer(i64::from(version)),
        ];
        arguments.extend(
            messages
                .iter()
                .map(|message| turso::Value::Integer(message.get())),
        );
        sql::execute(self.connection, &sql, arguments).await?;
        Ok(())
    }
}
