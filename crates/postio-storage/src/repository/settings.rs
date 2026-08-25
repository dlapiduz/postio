//! Small persisted flags the app owns for itself.
//!
//! The `settings` table's own migration comment draws the line: the user's
//! *configuration* is TOML and belongs to `postio-config`; this is state the
//! app owns, and nobody edits it in `$EDITOR`. ADR 0012 Q6 is the first
//! caller — whether the first-run keyboard orientation has been shown.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use super::to_millis;
use crate::error::Result;

/// The JSON literal [`SettingsRepository::set_flag`] writes and
/// [`SettingsRepository::get_flag`] reads back.
const TRUE: &str = "true";

/// Reads and writes global (`account_id IS NULL`) rows of `settings`.
///
/// Per-account settings are not exposed here yet — nothing needs one. Add a
/// `get`/`set` pair taking an [`postio_model::AccountId`] the same shape
/// when something does; the table's partial unique indexes already support
/// both scopes.
#[derive(Debug)]
pub struct SettingsRepository<'a> {
    connection: &'a Connection,
}

impl<'a> SettingsRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// The raw JSON text under `key`, or `None` if it was never set.
    pub fn get_global(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1 AND account_id IS NULL",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Write `value` (already JSON) under `key`, replacing whatever was
    /// there — an upsert against `idx_settings_global_key`, the partial
    /// unique index that is exactly this scope.
    pub fn set_global(&self, key: &str, value: &str, now: DateTime<Utc>) -> Result<()> {
        self.connection.execute(
            "INSERT INTO settings (key, account_id, value, updated_at)
             VALUES (?1, NULL, ?2, ?3)
             ON CONFLICT (key) WHERE account_id IS NULL
             DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            params![key, value, to_millis(now)],
        )?;
        Ok(())
    }

    /// Whether `key` was set with [`set_flag`](Self::set_flag).
    ///
    /// A convenience for the common shape — a flag that is either "seen" or
    /// "not yet" — so a caller does not have to parse JSON for a boolean.
    /// Anything other than the exact literal [`set_flag`](Self::set_flag)
    /// writes reads as unset, rather than guessing at truthiness.
    pub fn get_flag(&self, key: &str) -> Result<bool> {
        Ok(self.get_global(key)?.as_deref() == Some(TRUE))
    }

    /// Set `key` to the JSON literal `true`.
    pub fn set_flag(&self, key: &str, now: DateTime<Utc>) -> Result<()> {
        self.set_global(key, TRUE, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::memory;

    #[test]
    fn an_unset_key_reads_as_no_value_and_an_unset_flag_as_false() {
        let database = memory();
        let connection = database.connection().unwrap();
        let settings = SettingsRepository::new(&connection);

        assert_eq!(settings.get_global("orientation_seen").unwrap(), None);
        assert!(!settings.get_flag("orientation_seen").unwrap());
    }

    #[test]
    fn a_flag_survives_being_set_and_reads_back_true() {
        let database = memory();
        let connection = database.connection().unwrap();
        let settings = SettingsRepository::new(&connection);

        settings.set_flag("orientation_seen", Utc::now()).unwrap();
        assert!(settings.get_flag("orientation_seen").unwrap());
    }

    #[test]
    fn setting_the_same_key_twice_replaces_rather_than_conflicts() {
        let database = memory();
        let connection = database.connection().unwrap();
        let settings = SettingsRepository::new(&connection);

        settings
            .set_global("a", "1", Utc::now())
            .expect("first write");
        settings
            .set_global("a", "2", Utc::now())
            .expect("second write must upsert, not fail as a duplicate");
        assert_eq!(settings.get_global("a").unwrap().as_deref(), Some("2"));
    }

    #[test]
    fn different_keys_do_not_collide() {
        let database = memory();
        let connection = database.connection().unwrap();
        let settings = SettingsRepository::new(&connection);

        settings.set_flag("a", Utc::now()).unwrap();
        assert!(settings.get_flag("a").unwrap());
        assert!(!settings.get_flag("b").unwrap());
    }
}
