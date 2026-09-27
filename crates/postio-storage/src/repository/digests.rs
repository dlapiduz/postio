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
use crate::sql::{self, bind};
use crate::store::Connection;

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
