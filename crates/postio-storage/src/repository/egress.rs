//! The egress log (#151): what left this machine, auditable.
//!
//! See migration 0018 and `postio_model::egress` for the shape and the
//! rule — ids, counts and outcomes, never content.

use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
use postio_model::ids::AccountId;
use rusqlite::{Connection, params};

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
    pub fn record(&self, event: &EgressEvent) -> Result<()> {
        self.connection.execute(
            "INSERT INTO egress_log (at, subsystem, account_id, host, port, outcome)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event.at.timestamp_millis(),
                event.subsystem.as_str(),
                event.account.map(AccountId::get),
                event.host,
                event.port,
                event.outcome.as_str(),
            ],
        )?;
        Ok(())
    }

    /// The newest `limit` entries, newest first — what the settings surface
    /// lists for the user to audit.
    pub fn recent(&self, limit: u32) -> Result<Vec<EgressEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT at, subsystem, account_id, host, port, outcome
               FROM egress_log ORDER BY at DESC, id DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit], |row| {
            let at: i64 = row.get(0)?;
            let subsystem: String = row.get(1)?;
            let account: Option<i64> = row.get(2)?;
            let outcome: String = row.get(5)?;
            Ok(EgressEvent {
                at: chrono::DateTime::from_timestamp_millis(at).unwrap_or_default(),
                subsystem: EgressSubsystem::parse(&subsystem).unwrap_or(EgressSubsystem::Discovery),
                account: account.map(AccountId::new),
                host: row.get(3)?,
                port: row.get(4)?,
                outcome: EgressOutcome::parse(&outcome).unwrap_or(EgressOutcome::Failed),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// How many connections the log holds.
    ///
    /// The proof the documents promise runs on this: a default test suite
    /// that touched no network leaves it at zero.
    pub fn count(&self) -> Result<u64> {
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM egress_log", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use chrono::Utc;

    #[test]
    fn a_connection_round_trips_and_lists_newest_first() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let log = EgressLogRepository::new(&connection);
        assert_eq!(log.count().expect("count"), 0);

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
        log.record(&first).expect("record");
        log.record(&second).expect("record");

        assert_eq!(log.count().expect("count"), 2);
        let recent = log.recent(10).expect("recent");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].host, "imap.example.com");
        assert_eq!(recent[0].outcome, EgressOutcome::Connected);
        assert_eq!(recent[1].subsystem, EgressSubsystem::Discovery);
        assert_eq!(recent[1].account, None);
    }
}
