//! Why a message was filed out of the inbox. [`FilterReason`] is the store's own copy of the
//! classifier's reasons so recording a decision needs no dependency on the crate that decides; the
//! `CHECK` is the backstop.
//! The reason travels with the message (#2076) as a keyword the server keeps;
//! [`FilterDecisionRepository::follow`] turns it into a decision on another machine. A backend with
//! no keywords (Gmail) drops it, and filtering stays local.

use chrono::{DateTime, Utc};
use postio_model::ids::MessageId;
use postio_model::{Flag, FlagSet};

use super::{from_millis, to_millis, unknown_enum};

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;
use turso::Row;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterReason {
    Spam,
    Promotion,
    Notification,
    Receipt,
    Shipping,
    Social,
}

impl FilterReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            FilterReason::Spam => "spam",
            FilterReason::Promotion => "promotion",
            FilterReason::Notification => "notification",
            FilterReason::Receipt => "receipt",
            FilterReason::Shipping => "shipping",
            FilterReason::Social => "social",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }

    /// What tells another machine why the message is in the Archive.
    pub fn keyword(self) -> Flag {
        Flag::Keyword(format!("$PostioFiltered-{}", self.as_str()))
    }

    pub fn in_flags(flags: &FlagSet) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| flags.contains(&reason.keyword()))
    }

    pub const ALL: [FilterReason; 6] = [
        FilterReason::Spam,
        FilterReason::Promotion,
        FilterReason::Notification,
        FilterReason::Receipt,
        FilterReason::Shipping,
        FilterReason::Social,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterLayer {
    Header,
    Senders,
    /// Its own verdict, `$Junk`, or the reason keyword another machine's filing put on the message.
    Server,
    Model,
}

impl FilterLayer {
    pub const fn as_str(self) -> &'static str {
        match self {
            FilterLayer::Header => "header",
            FilterLayer::Senders => "senders",
            FilterLayer::Server => "server",
            FilterLayer::Model => "model",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layer| layer.as_str() == name)
    }

    pub const ALL: [FilterLayer; 4] = [
        FilterLayer::Header,
        FilterLayer::Senders,
        FilterLayer::Server,
        FilterLayer::Model,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterDecision {
    pub message: MessageId,
    pub reason: FilterReason,
    pub source: Option<String>,
    pub layer: FilterLayer,
    pub decided_at: DateTime<Utc>,
}

/// A decision whose message went to the trash is hidden (the person deleted it from Filtered; C4
/// forbids deleting automatically, not deleting), and Undo shows it again. Two primary-key lookups
/// per decision.
const NOT_IN_TRASH: &str = "NOT EXISTS (SELECT 1 FROM messages m
                     JOIN mailboxes b ON b.id = m.mailbox_id
                    WHERE m.id = filter_decisions.message_id AND b.role = 'trash')";

#[derive(Debug)]
pub struct FilterDecisionRepository<'a> {
    connection: &'a Connection,
}

impl<'a> FilterDecisionRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// A message has one decision: recording another replaces it, as the latest word.
    pub async fn record(&self, decision: &FilterDecision) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO filter_decisions (message_id, reason, source, layer, decided_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (message_id) DO UPDATE
                SET reason = excluded.reason, source = excluded.source,
                    layer = excluded.layer, decided_at = excluded.decided_at,
                    restored_at = NULL",
            vec![
                turso::Value::Integer(decision.message.get()),
                turso::Value::Text(decision.reason.as_str().to_owned()),
                decision
                    .source
                    .clone()
                    .map_or(turso::Value::Null, turso::Value::Text),
                turso::Value::Text(decision.layer.as_str().to_owned()),
                turso::Value::Integer(to_millis(decision.decided_at)),
            ],
        )
        .await?;
        Ok(())
    }

    /// A decision the person took back by restoring the message is not one.
    pub async fn get(&self, message: MessageId) -> Result<Option<FilterDecision>> {
        sql::first(
            self.connection,
            "SELECT message_id, reason, source, layer, decided_at
               FROM filter_decisions WHERE message_id = ?1 AND restored_at IS NULL",
            [message.get()],
            read_decision,
        )
        .await
    }

    /// Only where this store has none (its own decision or restore wins) and only in the Archive: a
    /// keyword seen while the message is still in an inbox is another machine's filing on its way,
    /// and the Archive copy gets it. Answers whether a decision was recorded.
    pub async fn follow(&self, message: MessageId, flags: &FlagSet) -> Result<bool> {
        let Some(reason) = FilterReason::in_flags(flags) else {
            return Ok(false);
        };
        let recorded = sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO filter_decisions
                    (message_id, reason, source, layer, decided_at)
             SELECT m.id, ?2, NULL, ?3, m.received_at
               FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
              WHERE m.id = ?1 AND b.role = 'archive'",
            vec![
                turso::Value::Integer(message.get()),
                turso::Value::Text(reason.as_str().to_owned()),
                turso::Value::Text(FilterLayer::Server.as_str().to_owned()),
            ],
        )
        .await?;
        Ok(recorded > 0)
    }

    /// With `None`, makes the decision stand again, as undo does. The decision is kept either way:
    /// a restore is the person's word on it.
    pub async fn restore(&self, message: MessageId, at: Option<DateTime<Utc>>) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE filter_decisions SET restored_at = ?2
              WHERE message_id = ?1 AND (restored_at IS NULL) = (?2 IS NOT NULL)",
            vec![
                turso::Value::Integer(message.get()),
                at.map(to_millis)
                    .map_or(turso::Value::Null, turso::Value::Integer),
            ],
        )
        .await?;
        Ok(changed > 0)
    }

    /// How many of `sender`'s messages the person restored and has not taken back: whether a
    /// restore still stands behind the `[focus.filter] never` entry. One statement, driven from the
    /// address.
    pub async fn restored_from(&self, sender: &str) -> Result<u32> {
        let count = sql::scalar(self.connection, Self::explain_restored_from(), [sender]).await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    pub fn explain_restored_from() -> &'static str {
        "SELECT count(DISTINCT d.message_id)
           FROM addresses a
           JOIN recipients r ON r.address_id = a.id AND r.kind = 'from'
           JOIN filter_decisions d ON d.message_id = r.message_id
          WHERE a.address_normalized = ?1 AND d.restored_at IS NOT NULL"
    }

    pub const EXPLAIN_COUNT_SINCE: &'static str =
        "SELECT COUNT(*) FROM filter_decisions WHERE decided_at >= ?1";

    pub async fn count_since(&self, since: DateTime<Utc>) -> Result<u32> {
        let count = sql::scalar(
            self.connection,
            Self::EXPLAIN_COUNT_SINCE,
            [to_millis(since)],
        )
        .await?;
        Ok(u32::try_from(count.max(0)).unwrap_or(u32::MAX))
    }

    /// One statement, one count per reason, each sought through `idx_filter_decisions_reason`.
    pub async fn tabs(&self) -> Result<Vec<(FilterReason, u32)>> {
        let counts: Vec<i64> = sql::first(self.connection, Self::explain_tabs(), (), |row| {
            (0..FilterReason::ALL.len())
                .map(|index| row.col(index))
                .collect::<Result<Vec<i64>>>()
        })
        .await?
        .unwrap_or_default();
        Ok(FilterReason::ALL
            .into_iter()
            .zip(counts.into_iter().chain(std::iter::repeat(0)))
            .map(|(reason, count)| (reason, u32::try_from(count.max(0)).unwrap_or(u32::MAX)))
            .collect())
    }

    pub fn explain_tabs() -> &'static str {
        static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        SQL.get_or_init(|| {
            let counts: Vec<String> = FilterReason::ALL
                .iter()
                .map(|reason| {
                    format!(
                        "(SELECT count(*) FROM filter_decisions
                           WHERE reason = '{}' AND restored_at IS NULL
                             AND {NOT_IN_TRASH})",
                        reason.as_str()
                    )
                })
                .collect();
            format!("SELECT {}", counts.join(",\n       "))
        })
    }

    pub async fn filtered(
        &self,
        reason: Option<FilterReason>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<FilterDecision>> {
        let sql = Self::explain_filtered(reason.is_some());
        let mut arguments = Vec::new();
        if let Some(reason) = reason {
            arguments.push(turso::Value::Text(reason.as_str().to_owned()));
        }
        arguments.push(turso::Value::Integer(i64::from(limit)));
        arguments.push(turso::Value::Integer(i64::from(offset)));
        sql::all(self.connection, &sql, arguments, read_decision).await
    }

    /// Standing decisions through `idx_filter_decisions_reason` or `idx_filter_decisions_standing`,
    /// in the index's own order.
    pub fn explain_filtered(one_reason: bool) -> String {
        let (reason, limit, offset) = if one_reason {
            ("reason = ?1 AND ", 2, 3)
        } else {
            ("", 1, 2)
        };
        format!(
            "SELECT message_id, reason, source, layer, decided_at
               FROM filter_decisions
              WHERE {reason}restored_at IS NULL
                AND {NOT_IN_TRASH}
              ORDER BY decided_at DESC
              LIMIT ?{limit} OFFSET ?{offset}"
        )
    }

    /// What taking back a sweep of the inbox does: the person never saw that decision stand.
    pub async fn delete(&self, message: MessageId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM filter_decisions WHERE message_id = ?1",
            [message.get()],
        )
        .await?;
        Ok(deleted > 0)
    }
}

fn read_decision(row: &Row) -> Result<FilterDecision> {
    let reason: String = row.col(1)?;
    let layer: String = row.col(3)?;
    Ok(FilterDecision {
        message: MessageId::new(row.col(0)?),
        reason: FilterReason::from_name(&reason)
            .ok_or_else(|| unknown_enum("filter_decisions.reason", reason))?,
        source: row.col(2)?,
        layer: FilterLayer::from_name(&layer)
            .ok_or_else(|| unknown_enum("filter_decisions.layer", layer))?,
        decided_at: from_millis(row.col(4)?),
    })
}
