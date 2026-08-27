//! Contact groups: a named set of contacts, expanded to addresses at compose
//! time rather than referenced by a group address of their own (ADR 0007
//! Q3).

use postio_model::{AccountId, Contact, ContactGroup, ContactGroupId, ContactId};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params, params_from_iter};

use super::contacts::{CONTACT_COLUMNS, read_contact};
use super::{from_millis, to_millis};
use crate::error::{Error, Result};

/// Reads and writes [`ContactGroup`] rows and their membership.
#[derive(Debug)]
pub struct ContactGroupRepository<'a> {
    connection: &'a Connection,
}

const GROUP_COLUMNS: &str = "id, account_id, name, uid, created_at";

impl<'a> ContactGroupRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a group, assigning its id.
    pub fn create(&self, group: &mut ContactGroup) -> Result<ContactGroupId> {
        self.connection.execute(
            "INSERT INTO contact_groups (account_id, name, uid, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![
                group.account_id.map(AccountId::get),
                group.name,
                group.uid,
                to_millis(group.created_at),
            ],
        )?;
        let id = ContactGroupId::new(self.connection.last_insert_rowid());
        group.id = id;
        Ok(id)
    }

    /// One group.
    pub fn get(&self, id: ContactGroupId) -> Result<Option<ContactGroup>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {GROUP_COLUMNS} FROM contact_groups WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        Ok(rows.next()?.map(read_group).transpose()?)
    }

    /// Every group visible to `account_id` -- shared groups when `None`,
    /// exactly the same matching `ContactRepository::list` uses for
    /// contacts.
    pub fn list(&self, account_id: Option<AccountId>) -> Result<Vec<ContactGroup>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {GROUP_COLUMNS} FROM contact_groups WHERE {}
              ORDER BY name",
            account_filter(account_id)
        ))?;
        let arguments = account_argument(account_id);
        let rows = statement.query_map(params_from_iter(arguments), read_group)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Renames a group.
    pub fn set_name(&self, id: ContactGroupId, name: &str) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE contact_groups SET name = ?2 WHERE id = ?1",
            params![id.get(), name],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "contact_group",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Deletes a group, returning whether there was one.
    ///
    /// Cascades to `contact_group_members` (the foreign key says so); never
    /// to the contacts themselves -- a group is a way of naming people, not
    /// a place they live.
    pub fn delete(&self, id: ContactGroupId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM contact_groups WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    /// Adds a contact to a group. Adding one already a member is a no-op,
    /// not an error -- the membership either exists afterwards or it does
    /// not, and both calls asked for the same thing.
    pub fn add_member(&self, group_id: ContactGroupId, contact_id: ContactId) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
             VALUES (?1, ?2)",
            params![group_id.get(), contact_id.get()],
        )?;
        Ok(())
    }

    /// Removes a contact from a group. Removing one that was never a
    /// member is a no-op for the same reason adding twice is.
    pub fn remove_member(&self, group_id: ContactGroupId, contact_id: ContactId) -> Result<()> {
        self.connection.execute(
            "DELETE FROM contact_group_members WHERE group_id = ?1 AND contact_id = ?2",
            params![group_id.get(), contact_id.get()],
        )?;
        Ok(())
    }

    /// Every contact currently in a group, for expansion at compose time.
    ///
    /// Deliberately not filtered by `suppressed`: membership is an explicit
    /// choice the user made, and a contact suppressed from autocomplete
    /// afterwards is still someone they put in this group on purpose.
    pub fn members(&self, group_id: ContactGroupId) -> Result<Vec<Contact>> {
        let columns: Vec<String> = CONTACT_COLUMNS
            .split(", ")
            .map(|column| format!("c.{column}"))
            .collect();
        let mut statement = self.connection.prepare(&format!(
            "SELECT {} FROM contact_group_members m
              JOIN contacts c ON c.id = m.contact_id
              WHERE m.group_id = ?1
              ORDER BY c.id",
            columns.join(", ")
        ))?;
        let rows = statement.query_map([group_id.get()], read_contact)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

/// The account predicate, matching `ContactRepository`'s own -- see that
/// module's comment for why `account_id = ?` cannot stand in for `IS NULL`.
fn account_filter(account_id: Option<AccountId>) -> &'static str {
    match account_id {
        Some(_) => "account_id = ?1",
        None => "account_id IS NULL",
    }
}

fn account_argument(account_id: Option<AccountId>) -> Vec<Value> {
    account_id
        .map(|id| vec![Value::Integer(id.get())])
        .unwrap_or_default()
}

fn read_group(row: &Row<'_>) -> rusqlite::Result<ContactGroup> {
    Ok(ContactGroup {
        id: ContactGroupId::new(row.get(0)?),
        account_id: row.get::<_, Option<i64>>(1)?.map(AccountId::new),
        name: row.get(2)?,
        uid: row.get(3)?,
        created_at: from_millis(row.get(4)?),
    })
}
