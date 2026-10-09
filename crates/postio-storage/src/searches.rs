//! What the search bar remembers (spec 010): the queries a person ran, and
//! how far they have seen each saved search's results.
//!
//! Two small tables, no foreign keys. `recent_searches` is the last
//! [`RECENT_KEPT`] distinct queries, written when a search is committed
//! (the results view opens, a hit is opened, a saved search is run) and
//! never per keystroke. `saved_search_seen` holds, per saved search key, the
//! `received_at` its badge counts from.

use chrono::{DateTime, TimeZone, Utc};

use crate::error::Result;
use crate::sql::{self, RowExt as _};
use crate::store::Connection;

/// How many distinct queries Recent keeps.
pub const RECENT_KEPT: usize = 20;

/// A query a person ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentSearch {
    /// Exactly as run.
    pub query: String,
    /// When it last ran.
    pub last_run_at: DateTime<Utc>,
    /// Conversations it matched, as the footer said.
    pub hits: u64,
}

/// Recent searches and saved-search progress on one connection.
pub struct SearchRepository<'a> {
    connection: &'a Connection,
}

fn from_millis(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().unwrap_or_default()
}

impl<'a> SearchRepository<'a> {
    /// Borrow `connection` for these reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The remembered queries, newest first, at most [`RECENT_KEPT`].
    pub async fn recent(&self) -> Result<Vec<RecentSearch>> {
        sql::all(
            self.connection,
            "SELECT query, last_run_at, hits FROM recent_searches
              ORDER BY last_run_at DESC, query LIMIT ?1",
            sql::bind![RECENT_KEPT],
            |row| {
                Ok(RecentSearch {
                    query: row.text(0)?,
                    last_run_at: from_millis(row.int(1)?),
                    hits: u64::try_from(row.int(2)?).unwrap_or(0),
                })
            },
        )
        .await
    }

    /// Record that `query` ran at `at` and matched `hits` conversations:
    /// one row per query, then everything past the newest [`RECENT_KEPT`]
    /// is deleted.
    pub async fn remember(&self, query: &str, hits: u64, at: DateTime<Utc>) -> Result<()> {
        sql::in_scope(self.connection, |_| async {
            sql::execute(
                self.connection,
                "INSERT INTO recent_searches (query, last_run_at, hits) VALUES (?1, ?2, ?3)
                 ON CONFLICT (query) DO UPDATE
                    SET last_run_at = excluded.last_run_at, hits = excluded.hits",
                sql::bind![query, at.timestamp_millis(), hits],
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

    /// Forget `query`; whether it was remembered.
    pub async fn forget(&self, query: &str) -> Result<bool> {
        Ok(sql::execute(
            self.connection,
            "DELETE FROM recent_searches WHERE query = ?1",
            sql::bind![query],
        )
        .await?
            > 0)
    }

    /// The newest `received_at` the person has seen saved search `key`'s
    /// results up to, or `None` before they first looked.
    pub async fn seen_up_to(&self, key: &str) -> Result<Option<DateTime<Utc>>> {
        sql::first(
            self.connection,
            "SELECT seen_up_to FROM saved_search_seen WHERE key = ?1",
            sql::bind![key],
            |row| Ok(from_millis(row.int(0)?)),
        )
        .await
    }

    /// Record that `key`'s results have been seen up to `up_to`.
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

    /// Delete the progress rows of saved searches no longer in `keys`
    /// (deleted from `config.toml`); how many went.
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
