//! Persisted app state: the `settings` table, keyed strings, global scope only (`account_id IS
//! NULL`). The user's configuration is TOML in `postio-config`; this is state the app owns.
//! Deliberately the smallest accessor needed; grow it with the next setting.
//! Focus's move picker keeps its Recent here ([`MOVE_RECENT`]).

use chrono::Utc;
use postio_model::MailboxId;
use turso::params;

use crate::sql::{self, RowExt as _};
use crate::store::Connection;

use crate::error::Result;

pub struct SettingsRepository<'a> {
    connection: &'a Connection,
}

impl<'a> SettingsRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>> {
        sql::first(
            self.connection,
            "SELECT value FROM settings WHERE key = ?1 AND account_id IS NULL",
            [key],
            |row| row.text(0),
        )
        .await
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<()> {
        // Delete-then-insert: the table has no unique index for `ON CONFLICT` to target (0001 left
        // it unconstrained), and two rows under one key would make `get` answer arbitrarily.
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

pub const MOVE_RECENT: &str = "focus.move_recent";

pub const MOVE_RECENT_KEPT: usize = 4;

impl SettingsRepository<'_> {
    /// Nothing when the value cannot be read, which costs the picker its shortcuts and nothing
    /// else.
    pub async fn move_recent(&self) -> Result<Vec<MailboxId>> {
        let value = self.get(MOVE_RECENT).await?;
        let ids: Vec<i64> = value
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        Ok(ids.into_iter().map(MailboxId::new).collect())
    }

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
