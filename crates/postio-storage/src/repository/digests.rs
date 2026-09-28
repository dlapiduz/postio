//! Focus's digests (spec 007, research R13): mail a `[[focus.digests]]`
//! rule holds back from Focus's inbox, and the deliveries that bring it
//! back as one row when the rule comes due.
//!
//! The rules are the person's, in `config.toml`; this is only what they
//! have done so far, which a resync loses like a snooze. Holding is decided
//! by the filing pass (T133), delivering by the due timer (T135), and
//! archiving a delivery by the digest window (T137). What leaves Focus's
//! inbox is a message with a hold whose delivery is not yet archived, and
//! that predicate is [`super::focus_excludes`].

use chrono::{DateTime, Utc};
use postio_model::ids::{DeliveryId, MessageId};

use super::to_millis;

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

/// A delivery not yet archived: a digest row of Focus's inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDelivery {
    /// Its row.
    pub id: DeliveryId,
    /// The rule's name, as `[[focus.digests]]` had it.
    pub rule: String,
    /// When it came due.
    pub due_at: DateTime<Utc>,
    /// When Focus delivered it: later, if Focus was closed at the due time.
    pub delivered_at: DateTime<Utc>,
    /// How many messages it holds.
    pub count: u32,
    /// Its summary's statements and references, as JSON, once one is
    /// written (spec 007 T154): the row's line comes from it.
    pub summary: Option<String>,
}

/// Reads and writes digest holds and deliveries.
#[derive(Debug)]
pub struct DigestRepository<'a> {
    connection: &'a Connection,
}

impl<'a> DigestRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Holds `message` for `rule` from `at`, and answers whether it did: a
    /// message is held once, by the first rule that matched it (contracts
    /// config.md, `[[focus.digests]]`).
    pub async fn hold(&self, message: MessageId, rule: &str, at: DateTime<Utc>) -> Result<bool> {
        let held = sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO digest_holds (message_id, rule, held_at) VALUES (?1, ?2, ?3)",
            bind![message.get(), rule, to_millis(at)],
        )
        .await?;
        Ok(held > 0)
    }

    /// Releases `message` -- its sender was stopped (`D`) -- and answers
    /// whether it was held. It rejoins Focus's inbox.
    pub async fn release(&self, message: MessageId) -> Result<bool> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE message_id = ?1",
            [message.get()],
        )
        .await?;
        Ok(released > 0)
    }

    /// Releases `message` if it is still waiting for its digest, and answers
    /// whether it was: once its body shows a question or a to-do, a message
    /// is not held (FR-122). One already delivered stays in its digest,
    /// where the person can see it.
    pub async fn release_waiting(&self, message: MessageId) -> Result<bool> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE message_id = ?1 AND delivery_id IS NULL",
            [message.get()],
        )
        .await?;
        Ok(released > 0)
    }

    /// Releases what `rule` holds of `sender`'s mail (an address, as
    /// `EmailAddress::normalized` spells it) and has not delivered, as
    /// stopping the sender does (`D`, FR-125), and answers which messages
    /// rejoined the inbox. What a delivery already holds stays in it, where
    /// the person can see it. Two statements, however many it releases.
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

    /// The SQL [`Self::release_sender`] reads what it releases with.
    pub fn explain_held_from() -> &'static str {
        "SELECT h.message_id FROM digest_holds h
          WHERE h.rule = ?1 AND h.delivery_id IS NULL
            AND EXISTS (SELECT 1 FROM recipients r JOIN addresses a ON a.id = r.address_id
                         WHERE r.message_id = h.message_id AND r.kind = 'from'
                           AND a.address_normalized = ?2)
          ORDER BY h.message_id"
    }

    /// The SQL [`Self::release_sender`] releases with: the same holds.
    pub fn explain_release_sender() -> &'static str {
        "DELETE FROM digest_holds
          WHERE rule = ?1 AND delivery_id IS NULL
            AND EXISTS (SELECT 1 FROM recipients r JOIN addresses a ON a.id = r.address_id
                         WHERE r.message_id = digest_holds.message_id AND r.kind = 'from'
                           AND a.address_normalized = ?2)"
    }

    /// The messages `delivery` holds, oldest first: what archiving the whole
    /// digest (`⇧A`) archives. One statement.
    pub async fn delivery_messages(&self, delivery: DeliveryId) -> Result<Vec<MessageId>> {
        sql::all(
            self.connection,
            Self::explain_delivery_messages(),
            [delivery.get()],
            |row| Ok(MessageId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::delivery_messages`] runs.
    pub fn explain_delivery_messages() -> &'static str {
        "SELECT message_id FROM digest_holds WHERE delivery_id = ?1 ORDER BY message_id"
    }

    /// Takes back the archiving of `delivery`, as undo does, and answers
    /// whether it was archived: its row is back among the inbox's.
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

    /// Releases everything `rule` holds and has not delivered, as removing
    /// the rule does (FR-126), and answers how many.
    pub async fn release_rule(&self, rule: &str) -> Result<usize> {
        let released = sql::execute(
            self.connection,
            "DELETE FROM digest_holds WHERE rule = ?1 AND delivery_id IS NULL",
            [rule],
        )
        .await?;
        Ok(released as usize)
    }

    /// When `rule`'s oldest hold that no delivery has taken was made, or
    /// `None` when nothing waits: its next delivery is the first due time
    /// after that (T135), and with nothing waiting there is none (US10
    /// scenario 6). One statement, a seek on `idx_digest_holds_rule`.
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

    /// The SQL [`Self::waiting_since`] runs.
    pub fn explain_waiting_since() -> &'static str {
        "SELECT min(held_at) FROM digest_holds WHERE rule = ?1 AND delivery_id IS NULL"
    }

    /// Delivers everything `rule` holds and has not yet delivered, as one
    /// delivery that came due at `due_at` and was made at `delivered_at`, and
    /// answers it -- or `None`, creating nothing, when the rule holds
    /// nothing new: an empty digest is no row (US10 scenario 6).
    ///
    /// Three statements in the caller's transaction, whatever the rule
    /// holds: whether it holds anything, the delivery, and the holds joining
    /// it, each over `idx_digest_holds_rule`.
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

    /// How many messages each of `rules` holds now, waiting for its next
    /// delivery, in their order: the `g d` list's "holds N".
    ///
    /// One statement for every rule, each count a seek on
    /// `idx_digest_holds_rule`.
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

    /// The SQL [`Self::waiting`] runs over `rules` rules.
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

    /// Every delivery not yet archived, newest first, each with how many
    /// messages it holds: the digest rows of Focus's inbox. One statement,
    /// a seek on `idx_digest_deliveries_open` and each count a seek on
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

    /// The SQL [`Self::open_deliveries`] runs.
    pub fn explain_open_deliveries() -> &'static str {
        "SELECT d.id, d.rule, d.due_at, d.delivered_at,
                (SELECT count(*) FROM digest_holds h WHERE h.delivery_id = d.id),
                d.summary
           FROM digest_deliveries d
          WHERE d.archived_at IS NULL
          ORDER BY d.due_at DESC, d.id DESC LIMIT 256"
    }

    /// The open deliveries no summary has been written for, newest first, at
    /// most `limit`: what the summariser takes next (spec 007 T154). One
    /// statement, a seek on `idx_digest_deliveries_open`.
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

    /// Keep `summary` -- statements and references, as JSON -- with
    /// `delivery`, written at `at`. One statement. An empty summary is
    /// kept too: the model answered and nothing resolved, and it is not
    /// asked again.
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

    /// `delivery`'s summary, as JSON, when one is written. One statement.
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

    /// Who sent the mail each of `deliveries` holds, each sender once with
    /// how many messages they sent, most first: the digest row's line until
    /// summaries exist (FR-124). One statement for all of them.
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

    /// The SQL [`Self::senders_of`] runs for `deliveries` deliveries.
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

    /// Archives `delivery` at `at`, as `⇧A` does, and answers whether there
    /// was one: its row leaves the inbox, and its holds keep nothing out of
    /// it any more.
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
