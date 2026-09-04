//! The one-click-unsubscribe activation log (#971).
//!
//! See `postio_model::unsubscribe` and migration 0008 for the shape and
//! the reason it exists: CLAUDE.md's privacy section requires activation
//! to be deliberate, and this is the record of it, the same append-only
//! shape `EgressLogRepository` already uses for the same kind of claim.

use postio_model::UnsubscribeActivation;
use postio_model::ids::{AccountId, UnsubscribeActivationId};
use rusqlite::{Connection, Row, params};

use super::{from_millis, to_millis};
use crate::error::Result;

/// Read and write the unsubscribe-activation log on one connection.
pub struct UnsubscribeRepository<'a> {
    connection: &'a Connection,
}

const COLUMNS: &str = "id, account_id, list_identifier, activated_at";

impl<'a> UnsubscribeRepository<'a> {
    /// Borrow `connection` for unsubscribe-log reads and writes.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Appends one activation, assigning its id.
    pub fn record(
        &self,
        activation: &mut UnsubscribeActivation,
    ) -> Result<UnsubscribeActivationId> {
        self.connection.execute(
            "INSERT INTO unsubscribe_activations (account_id, list_identifier, activated_at)
             VALUES (?1, ?2, ?3)",
            params![
                activation.account_id.get(),
                activation.list_identifier,
                to_millis(activation.activated_at),
            ],
        )?;
        let id = UnsubscribeActivationId::new(self.connection.last_insert_rowid());
        activation.id = id;
        Ok(id)
    }

    /// Every activation for `account_id`, newest first — what the privacy
    /// settings pane lists.
    pub fn for_account(&self, account_id: AccountId) -> Result<Vec<UnsubscribeActivation>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM unsubscribe_activations
              WHERE account_id = ?1
              ORDER BY activated_at DESC, id DESC"
        ))?;
        let rows = statement.query_map([account_id.get()], read_activation)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

fn read_activation(row: &Row<'_>) -> rusqlite::Result<UnsubscribeActivation> {
    Ok(UnsubscribeActivation {
        id: UnsubscribeActivationId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        list_identifier: row.get(2)?,
        activated_at: from_millis(row.get(3)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use chrono::Utc;

    #[test]
    fn an_activation_round_trips_and_lists_newest_first() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let account = test_support::account(&connection).id;
        let log = UnsubscribeRepository::new(&connection);

        assert_eq!(log.for_account(account).expect("empty list"), vec![]);

        let mut first = UnsubscribeActivation::new(
            account,
            "list-id.old-newsletter.example.com",
            Utc::now() - chrono::Duration::minutes(2),
        );
        let mut second =
            UnsubscribeActivation::new(account, "new-newsletter.example.com", Utc::now());
        log.record(&mut first).expect("record");
        log.record(&mut second).expect("record");

        assert!(first.id.is_assigned());
        assert_ne!(first.id, second.id);

        let listed = log.for_account(account).expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].list_identifier, "new-newsletter.example.com");
        assert_eq!(
            listed[1].list_identifier,
            "list-id.old-newsletter.example.com"
        );
    }

    #[test]
    fn activations_are_scoped_to_their_own_account() {
        let database = test_support::memory();
        let connection = database.connection().expect("checkout");
        let mine_account = test_support::account(&connection).id;
        let mut theirs_owner = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        crate::repository::AccountRepository::new(&connection)
            .create(&mut theirs_owner)
            .expect("second account");
        let log = UnsubscribeRepository::new(&connection);

        let mut mine = UnsubscribeActivation::new(mine_account, "mine.example.com", Utc::now());
        let mut theirs =
            UnsubscribeActivation::new(theirs_owner.id, "theirs.example.com", Utc::now());
        log.record(&mut mine).expect("record");
        log.record(&mut theirs).expect("record");

        let listed = log.for_account(mine_account).expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].list_identifier, "mine.example.com");
    }
}
