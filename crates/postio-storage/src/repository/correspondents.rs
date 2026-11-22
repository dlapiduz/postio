//! One row per address the person sent mail to (To, Cc, Bcc) and how many times, so the filter's
//! "wrote to" guard and recipient completion are one lookup, not a walk of Sent.
//! A message counts once, when first filed in Sent (a Sent sync counts only upserted rows, so a
//! resync adoption is not double counted); a failed send takes its one back. The sender's own
//! addresses are never counted.

use chrono::{DateTime, Utc};
use postio_model::{AccountId, EmailAddress, MessageId};

use super::from_millis;
use super::messages::placeholders;

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    pub sent_count: u32,
    pub last_sent_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct CorrespondentRepository<'a> {
    connection: &'a Connection,
}

impl<'a> CorrespondentRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// An address named twice in one message counts once. Two statements: the sender's own
    /// addresses, then one write from the recipients.
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

    /// A failed send delivered nothing, so nobody was written to. `last_sent_at` is left as it was;
    /// a count of zero says nobody was.
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

    pub async fn wrote_to(&self, address: &EmailAddress) -> Result<bool> {
        Ok(self
            .get(address)
            .await?
            .is_some_and(|correspondent| correspondent.sent_count > 0))
    }

    /// One statement whatever the number of addresses.
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

    pub fn explain_written_to(count: usize) -> String {
        format!(
            "SELECT a.address_normalized
               FROM addresses a JOIN correspondents c ON c.address_id = a.id
              WHERE a.address_normalized IN ({}) AND c.sent_count > 0",
            placeholders(count.max(1), 1)
        )
    }

    /// One statement over the correspondents (a few hundred), never over the contacts.
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

    pub fn explain_sent_counts() -> String {
        "SELECT a.address_normalized, c.sent_count
           FROM correspondents c JOIN addresses a ON a.id = c.address_id
          WHERE c.sent_count > 0"
            .to_owned()
    }

    pub fn explain_get(&self) -> String {
        "SELECT c.sent_count, c.last_sent_at
           FROM addresses a JOIN correspondents c ON c.address_id = a.id
          WHERE a.address_normalized = ?1"
            .to_owned()
    }

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
