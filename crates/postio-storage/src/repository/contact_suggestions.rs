//! The four Suggestions switches; a switch with no row is at its default.

use postio_model::{ContactSuggestions, Suggestion};

use crate::error::Result;
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;

#[derive(Debug)]
pub struct ContactSuggestionRepository<'a> {
    connection: &'a Connection,
}

impl<'a> ContactSuggestionRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn get(&self) -> Result<ContactSuggestions> {
        let rows: Vec<(String, i64)> = sql::all_unbounded(
            self.connection,
            "SELECT name, enabled FROM contact_suggestions",
            (),
            |row| Ok((row.text(0)?, row.col(1)?)),
        )
        .await?;
        let mut switches = ContactSuggestions::default();
        for (name, enabled) in rows {
            if let Some(which) = Suggestion::from_name(&name) {
                switches = switches.with(which, enabled != 0);
            }
        }
        Ok(switches)
    }

    pub async fn set(&self, which: Suggestion, on: bool) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT INTO contact_suggestions (name, enabled) VALUES (?1, ?2)
             ON CONFLICT (name) DO UPDATE SET enabled = excluded.enabled",
            bind![which.name(), i64::from(on)],
        )
        .await?;
        Ok(())
    }
}
