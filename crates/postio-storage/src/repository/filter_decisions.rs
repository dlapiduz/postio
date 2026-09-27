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
                    layer = excluded.layer, decided_at = excluded.decided_at",
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

    /// The decision on `message`: what its Filtered row and the open
    /// message say about why it is there.
    pub async fn get(&self, message: MessageId) -> Result<Option<FilterDecision>> {
        sql::first(
            self.connection,
            "SELECT message_id, reason, source, layer, decided_at
               FROM filter_decisions WHERE message_id = ?1",
            [message.get()],
            read_decision,
        )
        .await
    }

    /// Deletes the decision on `message`, as restoring it to the inbox
    /// (`R`) does, and answers whether there was one.
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
