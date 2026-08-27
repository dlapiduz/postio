//! Persisted application state: the `settings` table, keyed strings.
//!
//! Migration 0001 created the table — "pane widths, last selected mailbox,
//! and the like. The user's *configuration* is TOML and belongs to
//! `postio-config`; this is state the app owns" — and nothing ever read or
//! wrote it until #491 needed one fact to survive a restart: whether the
//! last session ended cleanly. This is deliberately the smallest accessor
//! that fact needs; grow it when the next setting arrives, not before.
//!
//! Global scope only (`account_id IS NULL`): nothing yet wants a per-account
//! setting, and an unused parameter is a decision nobody made.

use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

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
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1 AND account_id IS NULL",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    /// Set the globally-scoped `key` to `value`, replacing what was there.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        // Delete-then-insert rather than an upsert: the table has no unique
        // index for `ON CONFLICT` to target — 0001 left it unconstrained —
        // and two rows under one key would make `get` answer arbitrarily.
        self.connection.execute(
            "DELETE FROM settings WHERE key = ?1 AND account_id IS NULL",
            [key],
        )?;
        self.connection.execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES (?1, NULL, ?2, ?3)",
            params![key, value, Utc::now().timestamp_millis()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;

    #[test]
    fn a_setting_round_trips_and_replaces() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let settings = SettingsRepository::new(&connection);

        assert_eq!(settings.get("session_state").expect("read"), None);
        settings.set("session_state", "open").expect("write");
        assert_eq!(
            settings.get("session_state").expect("read"),
            Some("open".to_string())
        );
        settings.set("session_state", "closed").expect("replace");
        assert_eq!(
            settings.get("session_state").expect("read"),
            Some("closed".to_string()),
            "one key holds one value; setting replaces, never accumulates"
        );
    }
}
