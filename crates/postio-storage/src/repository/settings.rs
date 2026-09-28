//! Persisted application state: the `settings` table, keyed strings.
//!
//! Migration 0001 created the table — "pane widths, last selected mailbox,
//! and the like. The user's *configuration* is TOML and belongs to
//! `postio-config`; this is state the app owns" — and nothing ever read or
//! wrote it until #491 needed one fact to survive a restart: whether the
//! last session ended cleanly. This is deliberately the smallest accessor
//! that fact needs; grow it when the next setting arrives, not before.
//!
//! Focus's move picker keeps its Recent here too ([`MOVE_RECENT`]).
//!
//! Global scope only (`account_id IS NULL`): nothing yet wants a per-account
//! setting, and an unused parameter is a decision nobody made.

use chrono::Utc;
use postio_model::MailboxId;
use turso::params;

use crate::sql::{self, RowExt as _};
use crate::store::Connection;

use crate::error::Result;

/// Read and write app-owned settings on one connection.
pub struct SettingsRepository<'a> {
    connection: &'a Connection,
}

impl<'a> SettingsRepository<'a> {
    /// Borrow `connection` for settings reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The globally-scoped value under `key`, or `None` if it was never set.
    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        sql::first(
            self.connection,
            "SELECT value FROM settings WHERE key = ?1 AND account_id IS NULL",
            [key],
            |row| row.text(0),
        )
        .await
    }

    /// Set the globally-scoped `key` to `value`, replacing what was there.
    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        // Delete-then-insert rather than an upsert: the table has no unique
        // index for `ON CONFLICT` to target — 0001 left it unconstrained —
        // and two rows under one key would make `get` answer arbitrarily.
        sql::execute(
            self.connection,
            "DELETE FROM settings WHERE key = ?1 AND account_id IS NULL",
            [key],
        )
        .await?;
        sql::execute(
            self.connection,
            "INSERT INTO settings (key, account_id, value, updated_at)
                 VALUES (?1, NULL, ?2, ?3)",
            params![key, value, Utc::now().timestamp_millis()],
        )
        .await?;
        Ok(())
    }
}

/// The key Focus's move picker keeps its Recent under (data-model,
/// "Changes to existing tables").
pub const MOVE_RECENT: &str = "focus.move_recent";

/// How many destinations Recent keeps.
pub const MOVE_RECENT_KEPT: usize = 4;

impl SettingsRepository<'_> {
    /// Where mail was last moved, newest first: the move picker's Recent.
    /// Nothing when nothing has been moved, or when the value cannot be
    /// read, which costs the picker its shortcuts and nothing else.
    pub async fn move_recent(&self) -> Result<Vec<MailboxId>> {
        let value = self.get(MOVE_RECENT).await?;
        let ids: Vec<i64> = value
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        Ok(ids.into_iter().map(MailboxId::new).collect())
    }

    /// Put `mailbox` first in Recent, once, keeping the newest
    /// [`MOVE_RECENT_KEPT`].
    pub async fn note_move(&self, mailbox: MailboxId) -> Result<()> {
        let mut recent = self.move_recent().await?;
        recent.retain(|kept| *kept != mailbox);
        recent.insert(0, mailbox);
        recent.truncate(MOVE_RECENT_KEPT);
        let ids: Vec<i64> = recent.iter().map(|id| id.get()).collect();
        let value = serde_json::to_string(&ids).unwrap_or_else(|_| "[]".to_owned());
        self.set(MOVE_RECENT, &value).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[tokio::test]
    async fn recent_moves_are_newest_first_once_each_and_four_at_most() {
        let store = test_support::memory().await;
        let connection = store.connect().await.expect("connect");
        let settings = SettingsRepository::new(&connection);
        assert_eq!(settings.move_recent().await.expect("read"), []);
        for id in [1, 2, 3, 2, 4, 5] {
            settings.note_move(MailboxId::new(id)).await.expect("write");
        }
        assert_eq!(
            settings.move_recent().await.expect("read"),
            [5, 4, 2, 3].map(MailboxId::new),
            "the newest first, 2 once, 1 dropped"
        );
    }

    #[tokio::test]
    async fn a_setting_round_trips_and_replaces() {
        let store = test_support::memory().await;
        let connection = store.connect().await.expect("connect");
        let settings = SettingsRepository::new(&connection);

        assert_eq!(settings.get("session_state").await.expect("read"), None);
        settings.set("session_state", "open").await.expect("write");
        assert_eq!(
            settings.get("session_state").await.expect("read"),
            Some("open".to_string())
        );
        settings
            .set("session_state", "closed")
            .await
            .expect("replace");
        assert_eq!(
            settings.get("session_state").await.expect("read"),
            Some("closed".to_string()),
            "one key holds one value; setting replaces, never accumulates"
        );
    }
}
