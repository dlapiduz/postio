//! Labels: an account's own set, and which messages carry them (#780). Adding one is incremental,
//! undoable and queued like `Flag`; rewriting the whole message row would race every other write to
//! it.

use postio_model::{AccountId, Label, LabelId, MessageId, ThreadId};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

#[derive(Debug)]
pub struct LabelRepository<'a> {
    connection: &'a Connection,
}

const LABEL_COLUMNS: &str = "id, account_id, name, color";

impl<'a> LabelRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// # Errors
    ///
    /// The name is unique per account, case-insensitively (`idx_labels_account_name`): a second
    /// `work` beside a `Work` is refused.
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

    pub async fn get(&self, id: LabelId) -> Result<Option<Label>> {
        sql::first(
            self.connection,
            &format!("SELECT {LABEL_COLUMNS} FROM labels WHERE id = ?1"),
            [id.get()],
            read_label,
        )
        .await
    }

    /// Scoped to the account: another account's labels cannot be carried by the message.
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

    /// `INSERT OR IGNORE`, so a retried drain running twice is harmless.
    pub async fn attach(&self, message: MessageId, label: LabelId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO message_labels (message_id, label_id) VALUES (?1, ?2)",
            bind![message.get(), label.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    /// `false` for a label that was not there, so an undo that runs twice does not report a
    /// removal.
    pub async fn detach(&self, message: MessageId, label: LabelId) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "DELETE FROM message_labels WHERE message_id = ?1 AND label_id = ?2",
            bind![message.get(), label.get()],
        )
        .await?;
        Ok(changed > 0)
    }

    pub async fn for_message(&self, message: MessageId) -> Result<Vec<LabelId>> {
        sql::all(
            self.connection,
            "SELECT label_id FROM message_labels WHERE message_id = ?1 ORDER BY label_id",
            [message.get()],
            |row| Ok(LabelId::new(row.col(0)?)),
        )
        .await
    }

    /// A conversation's labels are every label any of its messages carries, each once, in creation
    /// order. One statement for the page, never a statement per row.
    pub async fn for_threads(&self, threads: &[ThreadId]) -> Result<Vec<(ThreadId, Label)>> {
        if threads.is_empty() {
            return Ok(Vec::new());
        }
        let mut statement =
            sql::statement(self.connection, &Self::explain_for_threads(threads.len())).await?;
        sql::mapped(
            &mut statement,
            threads
                .iter()
                .map(|thread| thread.get())
                .collect::<Vec<_>>(),
            |row| Ok((ThreadId::new(row.col(0)?), read_label_at(row, 1)?)),
        )
        .await
    }

    pub fn explain_for_threads(threads: usize) -> String {
        let placeholders = (1..=threads)
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT DISTINCT m.thread_id, l.id, l.account_id, l.name, l.color
               FROM messages m
               JOIN message_labels ml ON ml.message_id = m.id
               JOIN labels l ON l.id = ml.label_id
              WHERE m.thread_id IN ({placeholders})
              ORDER BY m.thread_id, l.id"
        )
    }

    /// A conversation counts once however many messages carry the label; a message in no
    /// conversation counts as its own; unused labels are left out. One statement, via
    /// `idx_message_labels_label`.
    pub async fn counts(&self, account: AccountId) -> Result<Vec<(LabelId, u32)>> {
        sql::all(
            self.connection,
            Self::explain_counts(),
            [account.get()],
            |row| {
                let count: i64 = row.col(1)?;
                Ok((
                    LabelId::new(row.col(0)?),
                    u32::try_from(count).unwrap_or(u32::MAX),
                ))
            },
        )
        .await
    }

    pub fn explain_counts() -> &'static str {
        "SELECT ml.label_id, COUNT(DISTINCT COALESCE(m.thread_id, -m.id))
           FROM labels l
           JOIN message_labels ml ON ml.label_id = l.id
           JOIN messages m ON m.id = ml.message_id
          WHERE l.account_id = ?1
          GROUP BY ml.label_id"
    }

    /// `message_labels` cascades.
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

fn read_label_at(row: &Row, first: usize) -> Result<Label> {
    Ok(Label {
        id: LabelId::new(row.col(first)?),
        account_id: AccountId::new(row.col(first + 1)?),
        name: row.col(first + 2)?,
        color: row.col(first + 3)?,
    })
}

fn read_label(row: &Row) -> Result<Label> {
    read_label_at(row, 0)
}
