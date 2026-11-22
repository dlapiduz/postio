//! Recent searches (written when a search is committed, never per keystroke) and how far each
//! saved search's results have been seen.

use chrono::{DateTime, TimeZone, Utc};

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;

pub const RECENT_KEPT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSearch {
    pub query: String,
    pub last_run_at: DateTime<Utc>,
    /// The search stopped at the walk's cap, so `hits` is a floor.
    pub hits: u64,
    pub capped: bool,
}

pub struct SearchRepository<'a> {
    connection: &'a Connection,
}

fn from_millis(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

impl<'a> SearchRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn recent(&self) -> Result<Vec<RecentSearch>> {
        sql::all(
            self.connection,
            "SELECT query, last_run_at, hits, capped FROM recent_searches
              ORDER BY last_run_at DESC, query LIMIT ?1",
            sql::bind![RECENT_KEPT],
            |row| {
                Ok(RecentSearch {
                    query: row.text(0)?,
                    last_run_at: from_millis(row.int(1)?),
                    hits: u64::try_from(row.int(2)?).unwrap_or(0),
                    capped: row.int(3)? != 0,
                })
            },
        )
        .await
    }

    /// One row per query; everything past the newest `RECENT_KEPT` is deleted.
    pub async fn remember(
        &self,
        query: &str,
        hits: u64,
        capped: bool,
        at: DateTime<Utc>,
    ) -> Result<()> {
        sql::in_scope(self.connection, |_| async {
            sql::execute(
                self.connection,
                "INSERT INTO recent_searches (query, last_run_at, hits, capped)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (query) DO UPDATE
                    SET last_run_at = excluded.last_run_at, hits = excluded.hits,
                        capped = excluded.capped",
                sql::bind![query, at.timestamp_millis(), hits, capped],
            )
            .await?;
            sql::execute(
                self.connection,
                "DELETE FROM recent_searches WHERE query NOT IN
                    (SELECT query FROM recent_searches
                      ORDER BY last_run_at DESC, query LIMIT ?1)",
                sql::bind![RECENT_KEPT],
            )
            .await?;
            Ok::<_, crate::Error>(())
        })
        .await
    }

    pub async fn forget(&self, query: &str) -> Result<bool> {
        Ok(sql::execute(
            self.connection,
            "DELETE FROM recent_searches WHERE query = ?1",
            sql::bind![query],
        )
        .await?
            > 0)
    }

    /// `None` before the person first looked.
    pub async fn seen_up_to(&self, key: &str) -> Result<Option<DateTime<Utc>>> {
        sql::first(
            self.connection,
            "SELECT seen_up_to FROM saved_search_seen WHERE key = ?1",
            sql::bind![key],
            |row| Ok(from_millis(row.int(0)?)),
        )
        .await
    }

    pub async fn mark_seen(&self, key: &str, up_to: DateTime<Utc>) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO saved_search_seen (key, seen_up_to) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET seen_up_to = excluded.seen_up_to",
            sql::bind![key, up_to.timestamp_millis()],
        )
        .await?;
        Ok(())
    }

    /// Deletes progress for saved searches no longer in `keys`; returns how many.
    pub async fn forget_seen_except(&self, keys: &[&str]) -> Result<u64> {
        let placeholders = (1..=keys.len())
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let statement = format!("DELETE FROM saved_search_seen WHERE key NOT IN ({placeholders})");
        let params: Vec<turso::Value> = keys.iter().map(sql::Bind::bind).collect();
        sql::execute(self.connection, &statement, params).await
    }
}
