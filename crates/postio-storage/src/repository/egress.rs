//! The egress log (#151): what left this machine, auditable.
//!
//! See migration 0018 and `postio_model::egress` for the shape and the
//! rule — ids, counts and outcomes, never content.

use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
use postio_model::ids::AccountId;


use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use crate::error::Result;

/// Read and write the egress log on one connection.
pub struct EgressLogRepository<'a> {
    connection: &'a Connection,
}

impl<'a> EgressLogRepository<'a> {
    /// Borrow `connection` for egress reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Append one connection attempt.
    pub async fn record(&self, event: &EgressEvent) -> Result<()> {
        self.connection.execute(
            "INSERT INTO egress_log (at, subsystem, account_id, host, port, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            bind![
                event.at.timestamp_millis(),
                event.subsystem.as_str(),
                event.account.map(AccountId::get),
                event.host,
                event.port,
                event.outcome.as_str(),
            ],
        ).await?;
        Ok(())
    }

    /// The newest `limit` entries, newest first — what the settings surface
    /// lists for the user to audit.
    pub async fn recent(&self, limit: u32) -> Result<Vec<EgressEvent>> {
        sql::all(
            self.connection,
            "SELECT at, subsystem, account_id, host, port, outcome
               FROM egress_log ORDER BY at DESC, id DESC LIMIT ?1",
            [limit],
            |row| {
            let at: i64 = row.col(0)?;
            let subsystem: String = row.col(1)?;
            let account: Option<i64> = row.col(2)?;
            let outcome: String = row.col(5)?;
            Ok(EgressEvent {
                at: chrono::DateTime::from_timestamp_millis(at).unwrap_or_default(),
                subsystem: EgressSubsystem::parse(&subsystem).unwrap_or(EgressSubsystem::Discovery),
                account: account.map(AccountId::new),
                host: row.col(3)?,
                port: row.col(4)?,
                outcome: EgressOutcome::parse(&outcome).unwrap_or(EgressOutcome::Failed),
            })
        },
        )
        .await}

    /// How many connections the log holds.
    ///
    /// The proof the documents promise runs on this: a default test suite
    /// that touched no network leaves it at zero.
    pub async fn count(&self) -> Result<u64> {
        let count: i64 =
            sql::one(
                self.connection,
                "SELECT count(*) FROM egress_log",
                (),
                |row| row.col(0),
            )
            .await?;
        Ok(count as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use chrono::Utc;

    #[tokio::test]
    async fn a_connection_round_trips_and_lists_newest_first() {
        let database = test_support::memory().await;
        let connection = database.connect().await.expect("checkout");
        let log = EgressLogRepository::new(&connection);
        assert_eq!(log.count().await.expect("count"), 0);

        let first = EgressEvent {
            at: Utc::now() - chrono::Duration::minutes(2),
            subsystem: EgressSubsystem::Discovery,
            account: None,
            host: "autoconfig.example.com".to_string(),
            port: 443,
            outcome: EgressOutcome::Failed,
        };
        let second = EgressEvent {
            at: Utc::now(),
            subsystem: EgressSubsystem::Imap,
            account: None,
            host: "imap.example.com".to_string(),
            port: 993,
            outcome: EgressOutcome::Connected,
        };
        log.record(&first).await.expect("record");
        log.record(&second).await.expect("record");

        assert_eq!(log.count().await.expect("count"), 2);
        let recent = log.recent(10).await.expect("recent");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].host, "imap.example.com");
        assert_eq!(recent[0].outcome, EgressOutcome::Connected);
        assert_eq!(recent[1].subsystem, EgressSubsystem::Discovery);
        assert_eq!(recent[1].account, None);
    }
}
