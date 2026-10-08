//! Who the person has written to (spec 007, data-model.md
//! "`correspondents`"): one row per address they sent mail to, in To, Cc or
//! Bcc, and how many times.
//!
//! # What reads it
//!
//! Focus's filter never files mail from someone the person wrote to (the
//! "wrote to" guard, FR-111), and recipient completion ranks by how often
//! (FR-052). Both are one lookup by address here, rather than a walk of
//! Sent's recipients.
//!
//! # What writes it
//!
//! A message counts once, when it is first filed in Sent: at local send,
//! before the network (`postio_sync::send`), and when a Sent folder syncs a
//! message it did not have (`postio_sync`'s passes, from the rows the
//! upsert inserted, so a sent copy a resync adopts by its Message-ID is not
//! counted twice). A send that fails takes its one back. The sender's own
//! addresses -- the account's and every identity's -- are never counted:
//! writing to yourself is not writing to someone.

use chrono::{DateTime, Utc};
use postio_model::{AccountId, EmailAddress, MessageId};

use super::from_millis;
use super::messages::placeholders;

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;

/// One address the person has sent mail to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    /// How many messages the person sent with it in To, Cc or Bcc.
    pub sent_count: u32,
    /// When the last of them was sent.
    pub last_sent_at: Option<DateTime<Utc>>,
}

/// Reads and writes [`Correspondent`]s.
#[derive(Debug)]
pub struct CorrespondentRepository<'a> {
    connection: &'a Connection,
}

impl<'a> CorrespondentRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Counts `messages`, sent from `account`, once each: one more for every
    /// address each went to in To, Cc or Bcc, an address named twice in one
    /// message counted once, and `last_sent_at` moved to the latest.
    ///
    /// Two statements whatever the messages hold: the sender's own
    /// addresses, read once, and one write from the messages' recipients.
    pub async fn record_sent(&self, account: AccountId, messages: &[MessageId]) -> Result<()> {
        if messages.is_empty() {
            return Ok(());
        }
        let own = self.own_addresses(account).await?;
        let sql = format!(
            "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
             SELECT r.address_id, count(DISTINCT r.message_id), max(m.received_at)
               FROM recipients r
               JOIN messages m ON m.id = r.message_id
               JOIN addresses a ON a.id = r.address_id
              WHERE r.message_id IN ({messages}) AND r.kind IN ('to', 'cc', 'bcc')
                AND a.address_normalized NOT IN ({own})
              GROUP BY r.address_id
             ON CONFLICT (address_id) DO UPDATE
                SET sent_count = sent_count + excluded.sent_count,
                    last_sent_at = max(coalesce(last_sent_at, 0), excluded.last_sent_at)",
            messages = placeholders(messages.len(), 1),
            own = placeholders(own.len(), messages.len() + 1),
        );
        sql::execute(self.connection, &sql, arguments(messages, &own)).await?;
        Ok(())
    }

    /// Takes back what [`Self::record_sent`] counted for `message`, whose
    /// send failed: nothing was delivered, so nobody was written to. The
    /// latest-sent time is left as it was; a count of zero is what says
    /// nobody was.
    pub async fn unrecord_sent(&self, account: AccountId, message: MessageId) -> Result<()> {
        let own = self.own_addresses(account).await?;
        let sql = format!(
            "UPDATE correspondents SET sent_count = max(sent_count - 1, 0)
              WHERE address_id IN (
                    SELECT r.address_id FROM recipients r
                      JOIN addresses a ON a.id = r.address_id
                     WHERE r.message_id = ?1 AND r.kind IN ('to', 'cc', 'bcc')
                       AND a.address_normalized NOT IN ({own}))",
            own = placeholders(own.len(), 2),
        );
        sql::execute(self.connection, &sql, arguments(&[message], &own)).await?;
        Ok(())
    }

    /// What is known of `address`, matched as addresses are, in any case.
    pub async fn get(&self, address: &EmailAddress) -> Result<Option<Correspondent>> {
        sql::first(
            self.connection,
            &self.explain_get(),
            [address.normalized()],
            |row| {
                Ok(Correspondent {
                    sent_count: row.col::<i64>(0)?.max(0) as u32,
                    last_sent_at: row.col::<Option<i64>>(1)?.map(from_millis),
                })
            },
        )
        .await
    }

    /// Whether the person has written to `address`: the filter's guard
    /// (FR-111), one lookup.
    pub async fn wrote_to(&self, address: &EmailAddress) -> Result<bool> {
        Ok(self
            .get(address)
            .await?
            .is_some_and(|correspondent| correspondent.sent_count > 0))
    }

    /// Which of `addresses` the person has written to, normalised: the
    /// filter's "wrote to" guard for every sender of one message at once
    /// (FR-111), one statement whatever their number.
    pub async fn written_to(&self, addresses: &[EmailAddress]) -> Result<Vec<String>> {
        if addresses.is_empty() {
            return Ok(Vec::new());
        }
        let normalized: Vec<String> = addresses.iter().map(EmailAddress::normalized).collect();
        sql::all(
            self.connection,
            &Self::explain_written_to(normalized.len()),
            normalized
                .into_iter()
                .map(turso::Value::Text)
                .collect::<Vec<_>>(),
            |row| row.col(0),
        )
        .await
    }

    /// The SQL [`Self::written_to`] runs for `count` addresses: a seek on
    /// each address, then the key.
    pub fn explain_written_to(count: usize) -> String {
        format!(
            "SELECT a.address_normalized
               FROM addresses a JOIN correspondents c ON c.address_id = a.id
              WHERE a.address_normalized IN ({}) AND c.sent_count > 0",
            placeholders(count.max(1), 1)
        )
    }

    /// Every address the person has written to, normalised, with how many
    /// messages each: recipient completion's "wrote N times" for a whole
    /// directory at once (spec 007 FR-052, T076). One statement, over the
    /// correspondents -- the people the person writes to, a few hundred --
    /// never over the contacts.
    pub async fn sent_counts(&self) -> Result<std::collections::HashMap<String, u32>> {
        let rows: Vec<(String, i64)> =
            sql::all(self.connection, &Self::explain_sent_counts(), (), |row| {
                Ok((row.col(0)?, row.col(1)?))
            })
            .await?;
        Ok(rows
            .into_iter()
            .map(|(address, sent)| (address, sent.max(0) as u32))
            .collect())
    }

    /// The SQL [`Self::sent_counts`] runs.
    pub fn explain_sent_counts() -> String {
        "SELECT a.address_normalized, c.sent_count
           FROM correspondents c JOIN addresses a ON a.id = c.address_id
          WHERE c.sent_count > 0"
            .to_owned()
    }

    /// The SQL [`Self::get`] runs: a seek on the address, then the key.
    pub fn explain_get(&self) -> String {
        "SELECT c.sent_count, c.last_sent_at
           FROM addresses a JOIN correspondents c ON c.address_id = a.id
          WHERE a.address_normalized = ?1"
            .to_owned()
    }

    /// Every address `account` sends as, normalised: its own and its
    /// identities'.
    async fn own_addresses(&self, account: AccountId) -> Result<Vec<String>> {
        let addresses: Vec<String> = sql::all(
            self.connection,
            "SELECT address FROM accounts WHERE id = ?1
             UNION SELECT address FROM identities WHERE account_id = ?1",
            [account.get()],
            |row| row.col(0),
        )
        .await?;
        let mut own: Vec<String> = addresses
            .iter()
            .map(|address| EmailAddress::new(None::<String>, address.as_str()).normalized())
            .collect();
        own.sort();
        own.dedup();
        // `NOT IN ()` does not parse; an address no row can hold stands in.
        if own.is_empty() {
            own.push(String::new());
        }
        Ok(own)
    }
}

/// The messages' ids, then the sender's own addresses, as parameters.
fn arguments(messages: &[MessageId], own: &[String]) -> Vec<turso::Value> {
    messages
        .iter()
        .map(|message| turso::Value::Integer(message.get()))
        .chain(
            own.iter()
                .map(|address| turso::Value::Text(address.clone())),
        )
        .collect()
}
