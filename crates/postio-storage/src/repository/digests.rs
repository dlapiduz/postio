//! Mail a `[[focus.digests]]` rule holds back from Focus's inbox, and the deliveries that bring it
//! back as one row when the rule comes due.
//! The rules are config; this is what they have done so far. What leaves the inbox is a message
//! with a hold whose delivery is not archived ([`super::focus_excludes`]).

use chrono::{DateTime, Utc};
use postio_model::ids::{DeliveryId, MessageId};

use super::to_millis;

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDelivery {
    pub id: DeliveryId,
    pub rule: String,
    pub due_at: DateTime<Utc>,
    pub delivered_at: DateTime<Utc>,
    pub count: u32,
    /// Statements and references as JSON, once written; the row's line comes from it.
    pub summary: Option<String>,
}

#[derive(Debug)]
pub struct DigestRepository<'a> {
    connection: &'a Connection,
}

impl<'a> DigestRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// A message is held once, by the first rule that matched it. Answers whether it was held.
    pub async fn hold(&self, message: MessageId, rule: &str, at: DateTime<Utc>) -> Result<bool> {
        let held = sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO digest_holds (message_id, rule, held_at) VALUES (?1, ?2, ?3)",
            bind![message.get(), rule, to_millis(at)],
        )
        .await?;
        Ok(held > 0)
    }

    pub async fn release(&self, message: MessageId) -> Result<bool> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE message_id = ?1",
            [message.get()],
        )
        .await?;
        Ok(released > 0)
    }

    /// Once its body shows a question or a to-do a message is not held; one already delivered stays
    /// in its digest.
    pub async fn release_waiting(&self, message: MessageId) -> Result<bool> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE message_id = ?1 AND delivery_id IS NULL",
            [message.get()],
        )
        .await?;
        Ok(released > 0)
    }

    /// As stopping the sender does: what a delivery already holds stays in it. Two statements
    /// however many it releases.
    pub async fn release_sender(&self, rule: &str, sender: &str) -> Result<Vec<MessageId>> {
        sql::in_scope(self.connection, |scope| async move {
            let released: Vec<MessageId> =
                sql::all(&scope, Self::explain_held_from(), [rule, sender], |row| {
                    Ok(MessageId::new(row.col(0)?))
                })
                .await?;
            if !released.is_empty() {
                sql::execute(&scope, Self::explain_release_sender(), [rule, sender]).await?;
            }
            Ok(released)
        })
        .await
    }

    pub fn explain_held_from() -> &'static str {
        "SELECT h.message_id FROM digest_holds h
          WHERE h.rule = ?1 AND h.delivery_id IS NULL
            AND EXISTS (SELECT 1 FROM recipients r JOIN addresses a ON a.id = r.address_id
                         WHERE r.message_id = h.message_id AND r.kind = 'from'
                           AND a.address_normalized = ?2)
          ORDER BY h.message_id"
    }

    pub fn explain_release_sender() -> &'static str {
        "DELETE FROM digest_holds
          WHERE rule = ?1 AND delivery_id IS NULL
            AND EXISTS (SELECT 1 FROM recipients r JOIN addresses a ON a.id = r.address_id
                         WHERE r.message_id = digest_holds.message_id AND r.kind = 'from'
                           AND a.address_normalized = ?2)"
    }

    pub async fn delivery_messages(&self, delivery: DeliveryId) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            Self::explain_delivery_messages(),
            [delivery.get()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    pub fn explain_delivery_messages() -> &'static str {
        "SELECT message_id FROM digest_holds WHERE delivery_id = ?1 ORDER BY message_id"
    }

    pub async fn reopen_delivery(&self, delivery: DeliveryId) -> Result<bool> {
        let reopened = sql::execute(
            self.connection,
            "UPDATE digest_deliveries SET archived_at = NULL
              WHERE id = ?1 AND archived_at IS NOT NULL",
            [delivery.get()],
        )
        .await?;
        Ok(reopened > 0)
    }

    pub async fn release_rule(&self, rule: &str) -> Result<usize> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE rule = ?1 AND delivery_id IS NULL",
            [rule],
        )
        .await?;
        Ok(released as usize)
    }

    /// `None` when nothing waits: the next delivery is the first due time after it, and with
    /// nothing waiting there is none. A seek on `idx_digest_holds_rule`.
    pub async fn waiting_since(&self, rule: &str) -> Result<Option<DateTime<Utc>>> {
        let since: Option<i64> = sql::first(
            self.connection,
            Self::explain_waiting_since(),
            [rule],
            |row| row.col(0),
        )
        .await?
        .flatten();
        Ok(since.map(super::from_millis))
    }

    pub fn explain_waiting_since() -> &'static str {
        "SELECT min(held_at) FROM digest_holds WHERE rule = ?1 AND delivery_id IS NULL"
    }

    /// `None`, creating nothing, when the rule holds nothing new: an empty digest is no row. Three
    /// statements in the caller's transaction, each over `idx_digest_holds_rule`.
    pub async fn deliver(
        &self,
        rule: &str,
        due_at: DateTime<Utc>,
        delivered_at: DateTime<Utc>,
    ) -> Result<Option<DeliveryId>> {
        sql::in_scope(self.connection, |transaction| async move {
            let waiting = sql::exists(
                &transaction,
                "SELECT 1 FROM digest_holds WHERE rule = ?1 AND delivery_id IS NULL",
                [rule],
            )
            .await?;
            if !waiting {
                return Ok(None);
            }
            sql::execute(
                &transaction,
                "INSERT INTO digest_deliveries (rule, due_at, delivered_at) VALUES (?1, ?2, ?3)",
                bind![rule, to_millis(due_at), to_millis(delivered_at)],
            )
            .await?;
            let delivery = DeliveryId::new(transaction.last_insert_rowid());
            sql::execute(
                &transaction,
                "UPDATE digest_holds SET delivery_id = ?2 WHERE rule = ?1 AND delivery_id IS NULL",
                bind![rule, delivery.get()],
            )
            .await?;
            Ok(Some(delivery))
        })
        .await
    }

    /// `false` is waiting for delivery, `true` delivered and not archived. One statement, each
    /// message by its key.
    pub async fn held(&self, messages: &[MessageId]) -> Result<Vec<(MessageId, String, bool)>> {
        if messages.is_empty() {
            return Ok(Vec::new());
        }
        sql::all(
            self.connection,
            &Self::explain_held(messages.len()),
            messages
                .iter()
                .map(|message| message.get())
                .collect::<Vec<_>>(),
            |row| {
                Ok((
                    MessageId::new(row.col(0)?),
                    row.col(1)?,
                    row.col::<i64>(2)? != 0,
                ))
            },
        )
        .await
    }

    pub fn explain_held(messages: usize) -> String {
        let placeholders = (1..=messages)
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "SELECT h.message_id, h.rule, h.delivery_id IS NOT NULL
               FROM digest_holds h
               LEFT JOIN digest_deliveries d ON d.id = h.delivery_id
              WHERE h.message_id IN ({placeholders}) AND d.archived_at IS NULL
              ORDER BY h.message_id"
        )
    }

    /// One statement for every rule, each count a seek on `idx_digest_holds_rule`.
    pub async fn waiting(&self, rules: &[&str]) -> Result<Vec<u32>> {
        if rules.is_empty() {
            return Ok(Vec::new());
        }
        let counts = sql::first(
            self.connection,
            &Self::explain_waiting(rules.len()),
            rules
                .iter()
                .map(|rule| (*rule).to_owned())
                .collect::<Vec<_>>(),
            |row| {
                (0..rules.len())
                    .map(|index| row.col::<i64>(index))
                    .collect::<Result<Vec<i64>>>()
            },
        )
        .await?
        .unwrap_or_default();
        Ok(counts
            .into_iter()
            .map(|count| u32::try_from(count.max(0)).unwrap_or(u32::MAX))
            .collect())
    }

    pub fn explain_waiting(rules: usize) -> String {
        let counts: Vec<String> = (1..=rules)
            .map(|n| {
                format!(
                    "(SELECT count(*) FROM digest_holds WHERE rule = ?{n} AND delivery_id IS NULL)"
                )
            })
            .collect();
        format!("SELECT {}", counts.join(", "))
    }

    /// One statement: a seek on `idx_digest_deliveries_open`, each count a seek on
    /// `idx_digest_holds_delivery`.
    pub async fn open_deliveries(&self) -> Result<Vec<OpenDelivery>> {
        sql::all(
            self.connection,
            Self::explain_open_deliveries(),
            (),
            |row| {
                Ok(OpenDelivery {
                    id: DeliveryId::new(row.col(0)?),
                    rule: row.col(1)?,
                    due_at: super::from_millis(row.col(2)?),
                    delivered_at: super::from_millis(row.col(3)?),
                    count: u32::try_from(row.col::<i64>(4)?).unwrap_or(u32::MAX),
                    summary: row.col(5)?,
                })
            },
        )
        .await
    }

    pub fn explain_open_deliveries() -> &'static str {
        "SELECT d.id, d.rule, d.due_at, d.delivered_at,
                (SELECT count(*) FROM digest_holds h WHERE h.delivery_id = d.id),
                d.summary
           FROM digest_deliveries d
          WHERE d.archived_at IS NULL
          ORDER BY d.due_at DESC, d.id DESC LIMIT 256"
    }

    /// One statement, a seek on `idx_digest_deliveries_open`.
    pub async fn unsummarised(&self, limit: u32) -> Result<Vec<DeliveryId>> {
        sql::all(
            self.connection,
            "SELECT id FROM digest_deliveries
              WHERE archived_at IS NULL AND summary IS NULL
              ORDER BY id DESC LIMIT ?1",
            [limit],
            |row| Ok(DeliveryId::new(row.col(0)?)),
        )
        .await
    }

    /// An empty summary is kept too: the model answered and nothing resolved, so it is not asked
    /// again.
    pub async fn set_summary(
        &self,
        delivery: DeliveryId,
        summary: &str,
        at: DateTime<Utc>,
    ) -> Result<bool> {
        let written = sql::execute(
            self.connection,
            "UPDATE digest_deliveries SET summary = ?2, summary_written_at = ?3 WHERE id = ?1",
            bind![delivery.get(), summary, to_millis(at)],
        )
        .await?;
        Ok(written > 0)
    }

    pub async fn summary(&self, delivery: DeliveryId) -> Result<Option<String>> {
        Ok(sql::first(
            self.connection,
            "SELECT summary FROM digest_deliveries WHERE id = ?1",
            [delivery.get()],
            |row| row.col::<Option<String>>(0),
        )
        .await?
        .flatten())
    }

    /// The digest row's line until summaries exist. One statement for all of them.
    pub async fn senders_of(
        &self,
        deliveries: &[DeliveryId],
    ) -> Result<Vec<(DeliveryId, postio_model::EmailAddress, u32)>> {
        if deliveries.is_empty() {
            return Ok(Vec::new());
        }
        sql::all(
            self.connection,
            &Self::explain_senders_of(deliveries.len()),
            deliveries
                .iter()
                .map(|delivery| delivery.get())
                .collect::<Vec<_>>(),
            |row| {
                Ok((
                    DeliveryId::new(row.col(0)?),
                    postio_model::EmailAddress::new(
                        row.col::<Option<String>>(2)?,
                        row.col::<String>(1)?,
                    ),
                    u32::try_from(row.col::<i64>(3)?).unwrap_or(u32::MAX),
                ))
            },
        )
        .await
    }

    pub fn explain_senders_of(deliveries: usize) -> String {
        format!(
            "SELECT h.delivery_id, a.address, max(r.name), count(DISTINCT h.message_id)
               FROM digest_holds h
               JOIN recipients r ON r.message_id = h.message_id AND r.kind = 'from'
               JOIN addresses a ON a.id = r.address_id
              WHERE h.delivery_id IN ({})
              GROUP BY h.delivery_id, a.id
              ORDER BY h.delivery_id, 4 DESC, a.address
              LIMIT 1024",
            super::messages::placeholders(deliveries, 1)
        )
    }

    /// Its row leaves the inbox, and its holds keep nothing out of it.
    pub async fn archive_delivery(&self, delivery: DeliveryId, at: DateTime<Utc>) -> Result<bool> {
        let archived = sql::execute(
            self.connection,
            "UPDATE digest_deliveries SET archived_at = ?2 WHERE id = ?1",
            bind![delivery.get(), to_millis(at)],
        )
        .await?;
        Ok(archived > 0)
    }
}
