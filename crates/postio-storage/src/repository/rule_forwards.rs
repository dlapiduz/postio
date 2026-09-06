//! What a `forward:` rule has already sent (#1142, ADR 0008 Q5).
//!
//! Append-only, like the unsubscribe and egress logs: it is evidence of mail
//! that left the machine, and evidence that is edited is not evidence.
//!
//! Two questions are asked of it. The rate cap asks *how many has this rule
//! sent in the last hour* — the guard that stops a rule that has started
//! forwarding everything, without dropping any of the mail (ADR 0008 Q6). A
//! settings surface asks *what has this rule actually done*, which is the
//! only honest answer to "is my rule working".
//!
//! See migration 0012 for why `message_id` is not a foreign key.

use chrono::{DateTime, Utc};
use postio_model::ids::{AccountId, MessageId};
use rusqlite::{Connection, params};

use crate::error::Result;

/// Read and write the rule-forward log on one connection.
pub struct RuleForwardRepository<'a> {
    connection: &'a Connection,
}

impl<'a> RuleForwardRepository<'a> {
    /// Borrow `connection` for rule-forward reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Record that `rule` forwarded `message`.
    ///
    /// Written in the same transaction as the queued send it belongs to, so
    /// the cap counts sends that will actually go out: a row here without its
    /// `Operation::Send` would consume the rule's budget for mail nobody
    /// receives, and a send without this row would be invisible to the cap.
    pub fn record(
        &self,
        account: AccountId,
        rule: &str,
        message: MessageId,
        at: DateTime<Utc>,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO rule_forwards (account_id, rule, message_id, forwarded_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![account.get(), rule, message.get(), at.timestamp_millis()],
        )?;
        Ok(())
    }

    /// How many messages `rule` has forwarded since `since`.
    ///
    /// The rate cap's whole question. Counted rather than listed because that
    /// is what the cap needs and because the answer is asked once per matching
    /// message, on the backfill's path.
    pub fn count_since(&self, account: AccountId, rule: &str, since: DateTime<Utc>) -> Result<u32> {
        let count: i64 = self.connection.query_row(
            "SELECT count(*) FROM rule_forwards
              WHERE account_id = ?1 AND rule = ?2 AND forwarded_at >= ?3",
            params![account.get(), rule, since.timestamp_millis()],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }
}
