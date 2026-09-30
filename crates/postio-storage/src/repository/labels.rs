//! Labels: an account's own set, and which messages carry them (#780).
//!
//! The schema has held `labels` and `message_labels` since migration 0001,
//! and [`MessageRepository`](super::MessageRepository) writes a message's
//! whole label set when it inserts or updates the row. What was missing is
//! everything a *command* needs: listing an account's labels so a picker can
//! offer them, creating one, and moving a single label on or off a message
//! that already exists.
//!
//! That last shape is why this is not simply "read the message, push a label,
//! write it back". `AddLabel` is incremental, undoable and queued, the way
//! `Flag` is; rewriting the whole row to add one label would race every other
//! write to that message and would undo whatever landed in between.

use postio_model::{AccountId, Label, LabelId, MessageId};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Reads and writes [`Label`] rows and their attachment to messages.
#[derive(Debug)]
pub struct LabelRepository<'a> {
    connection: &'a Connection,
}

const LABEL_COLUMNS: &str = "id, account_id, name, color";

impl<'a> LabelRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a label, assigning its id.
    ///
    /// # Errors
    ///
    /// The name is unique per account and compared case-insensitively
    /// (`idx_labels_account_name`), so a second `work` beside a `Work` is
    /// refused here rather than becoming two rows a person would read as one.
    pub async fn create(&self, label: &mut Label) -> Result<LabelId> {
        sql::execute(
            self.connection,
            "INSERT INTO labels (account_id, name, color) VALUES (?1, ?2, ?3)",
            bind![label.account_id.get(), label.name, label.color],
        )
        .await?;
        let id = LabelId::new(self.connection.last_insert_rowid());
        label.id = id;
        Ok(id)
    }

    /// One label.
    pub async fn get(&self, id: LabelId) -> Result<Option<Label>> {
        sql::first(
            self.connection,
            &format!("SELECT {LABEL_COLUMNS} FROM labels WHERE id = ?1"),
            [id.get()],
            read_label,
        )
        .await
    }

    /// Every label `account_id` owns, by name.
    ///
    /// Scoped to the account because a picker that offered another account's
    /// labels would be offering something the message cannot carry. Ordered
    /// by name so the list a person scans does not move between openings.
    pub async fn list(&self, account_id: AccountId) -> Result<Vec<Label>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {LABEL_COLUMNS} FROM labels WHERE account_id = ?1
              ORDER BY name COLLATE NOCASE"
            ),
            [account_id.get()],
            read_label,
        )
        .await
    }

    /// Puts `label` on `message`. Answers whether that changed anything.
    ///
    /// `INSERT OR IGNORE` against a table keyed on the pair, so running twice
    /// is harmless — which a queued command has to be, because a drain that
    /// is retried after an uncertain failure runs it again.
    pub async fn attach(&self, message: MessageId, label: LabelId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            bind![message.get(), label.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// Takes `label` off `message`. Answers whether that changed anything.
    ///
    /// `false` for a label that was not there, so an undo that runs twice
    /// does not report having removed something.
    pub async fn detach(&self, message: MessageId, label: LabelId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "DELETE FROM message_labels WHERE message_id = ?1 AND label_id = ?2",
            bind![message.get(), label.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// The labels on `message`, in the order the message row reports them.
    pub async fn for_message(&self, message: MessageId) -> Result<Vec<LabelId>> {
        sql::all(
            self.connection,
            "SELECT label_id FROM message_labels WHERE message_id = ?1 ORDER BY label_id",
            [message.get()],
            |row| Ok(LabelId::new(row.col(0)?)),
        )
        .await
    }

    /// Removes a label entirely. Answers whether there was one.
    ///
    /// `message_labels` cascades, so this takes it off every message carrying
    /// it rather than leaving rows pointing at a label that is gone.
    pub async fn delete(&self, id: LabelId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "DELETE FROM labels WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(changed > 0)
    }
}

fn read_label(row: &Row) -> Result<Label> {
    Ok(Label {
        id: LabelId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        name: row.col(2)?,
        color: row.col(3)?,
    })
}
