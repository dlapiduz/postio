//! Focus's filter decisions (spec 007): why a message was filed out of the
//! inbox, from the fixed vocabulary FR-113 asks for.
//!
//! # One vocabulary, spelled three times on purpose
//!
//! The reasons are `postio_classify::ReasonKind`'s six, and its `as_str`
//! spells them as this store's `CHECK` does. [`FilterReason`] is the store's
//! own copy rather than a dependency on the classifier: nothing here may
//! need the crate that decides in order to record what was decided, and the
//! filing pass that turns one into the other (T122) is the place that sees
//! both and holds them equal. The `CHECK` is the backstop either way -- a
//! reason outside it is refused, never stored.

use chrono::{DateTime, Utc};
use postio_model::ids::MessageId;

use super::{from_millis, to_millis, unknown_enum};

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;
use turso::Row;

/// Why a message was filtered: the fixed vocabulary of FR-113.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterReason {
    /// Unsolicited mail.
    Spam,
    /// Marketing.
    Promotion,
    /// An automated notice.
    Notification,
    /// A receipt or an invoice.
    Receipt,
    /// Delivery and tracking.
    Shipping,
    /// A social network's mail.
    Social,
}

impl FilterReason {
    /// The reason as the store spells it, and as the Filtered row shows it.
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

    /// The reason a stored spelling names.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.as_str() == name)
    }

    /// Every reason, in the order the Filtered view's tabs list them.
    pub const ALL: [FilterReason; 6] = [
        FilterReason::Spam,
        FilterReason::Promotion,
        FilterReason::Notification,
        FilterReason::Receipt,
        FilterReason::Shipping,
        FilterReason::Social,
    ];
}

/// Which layer of the classifier decided (FR-113).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FilterLayer {
    /// The message's own list, bulk and automated headers.
    Header,
    /// The shipped automated-senders table.
    Senders,
    /// The server's own verdict, `$Junk`.
    Server,
    /// The person's own model (milestone 2).
    Model,
}

impl FilterLayer {
    /// The layer as the store spells it.
    pub const fn as_str(self) -> &'static str {
        match self {
            FilterLayer::Header => "header",
            FilterLayer::Senders => "senders",
            FilterLayer::Server => "server",
            FilterLayer::Model => "model",
        }
    }

    /// The layer a stored spelling names.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|layer| layer.as_str() == name)
    }

    /// Every layer.
    pub const ALL: [FilterLayer; 4] = [
        FilterLayer::Header,
        FilterLayer::Senders,
        FilterLayer::Server,
        FilterLayer::Model,
    ];
}

/// One message's filter decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterDecision {
    /// The message filed out of the inbox.
    pub message: MessageId,
    /// Why.
    pub reason: FilterReason,
    /// Who it came from, shown after the reason ("notification · Forge").
    pub source: Option<String>,
    /// Which layer decided.
    pub layer: FilterLayer,
    /// When: local midnight bounds "filtered today".
    pub decided_at: DateTime<Utc>,
}

/// Reads and writes [`FilterDecision`]s.
#[derive(Debug)]
pub struct FilterDecisionRepository<'a> {
    connection: &'a Connection,
}

impl<'a> FilterDecisionRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Records `decision`, in the filing pass's transaction or as undo of a
    /// restore puts it back. A message has one decision: recording another
    /// replaces it, since it is the latest word on why the message is not in
    /// the inbox.
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

    /// The decision on `message`, while it stands: what its Filtered row
    /// and the open message say about why it is there. A decision the
    /// person took back by restoring the message is not one.
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

    /// Marks the decision on `message` restored at `at`, as `R` does, or
    /// with `None` makes it stand again, as undo does; answers whether the
    /// message has a decision that changed. The decision is kept either
    /// way: a restore is the person's word on it (SC-012).
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

    /// How many of `sender`'s messages (an address, as
    /// `EmailAddress::normalized` spells it) the person restored and has
    /// not taken back: whether a restore still stands behind the sender's
    /// `[focus.filter] never` entry. One statement, driven from the
    /// sender's address.
    pub async fn restored_from(&self, sender: &str) -> Result<u32> {
        let count = sql::scalar(self.connection, Self::explain_restored_from(), [sender]).await?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// The SQL [`Self::restored_from`] runs.
    pub fn explain_restored_from() -> &'static str {
        "SELECT count(DISTINCT d.message_id)
           FROM addresses a
           JOIN recipients r ON r.address_id = a.id AND r.kind = 'from'
           JOIN filter_decisions d ON d.message_id = r.message_id
          WHERE a.address_normalized = ?1 AND d.restored_at IS NOT NULL"
    }

    /// The SQL [`Self::count_since`] runs, for `EXPLAIN QUERY PLAN`.
    pub const EXPLAIN_COUNT_SINCE: &'static str =
        "SELECT COUNT(*) FROM filter_decisions WHERE decided_at >= ?1";

    /// How many messages were filed away at or after `since`: "186
    /// filtered today", with `since` local midnight (spec 007 screen 16).
    pub async fn count_since(&self, since: DateTime<Utc>) -> Result<u32> {
        let count = sql::scalar(
            self.connection,
            Self::EXPLAIN_COUNT_SINCE,
            [to_millis(since)],
        )
        .await?;
        Ok(u32::try_from(count.max(0)).unwrap_or(u32::MAX))
    }

    /// Each reason with how many messages it keeps filtered now, in the
    /// order the Filtered view's tabs list them (screen 21).
    ///
    /// One statement, one count a reason, each sought through
    /// `idx_filter_decisions_reason` to the reason's standing decisions.
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

    /// The SQL [`Self::tabs`] runs.
    pub fn explain_tabs() -> &'static str {
        static SQL: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        SQL.get_or_init(|| {
            let counts: Vec<String> = FilterReason::ALL
                .iter()
                .map(|reason| {
                    format!(
                        "(SELECT count(*) FROM filter_decisions
                           WHERE reason = '{}' AND restored_at IS NULL)",
                        reason.as_str()
                    )
                })
                .collect();
            format!("SELECT {}", counts.join(",\n       "))
        })
    }

    /// The standing decisions, newest first -- of `reason` only, when one
    /// is given -- `limit` of them from `offset`: a page of Filtered.
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

    /// The SQL [`Self::filtered`] runs, for one reason or for all: the
    /// standing decisions through `idx_filter_decisions_reason` or
    /// `idx_filter_decisions_standing`, in the index's own order.
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
              ORDER BY decided_at DESC
              LIMIT ?{limit} OFFSET ?{offset}"
        )
    }

    /// Deletes the decision on `message`, and answers whether there was one:
    /// what taking back a sweep of the inbox does, since the person never
    /// saw that decision stand.
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
