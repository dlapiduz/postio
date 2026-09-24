//! Contact groups: a named set of people, expanded to their preferred
//! addresses at compose time rather than referenced by a group address of
//! their own (specs/005-contacts FR-041). Groups are shared across accounts,
//! as people are.

use postio_model::{Contact, ContactGroup, ContactGroupId, ContactId};

use super::contacts::{ContactRepository, PERSON_COLUMNS_C, read_person};
use super::{from_millis, to_millis};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Reads and writes [`ContactGroup`] rows and their membership.
#[derive(Debug)]
pub struct ContactGroupRepository<'a> {
    connection: &'a Connection,
}

const GROUP_COLUMNS: &str = "id, name, uid, created_at";

impl<'a> ContactGroupRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a group, assigning its id.
    pub async fn create(&self, group: &mut ContactGroup) -> Result<ContactGroupId> {
        refuse_taken(self.connection, &group.name, None).await?;
        sql::execute(
            self.connection,
            "INSERT INTO contact_groups (name, uid, created_at) VALUES (?1, ?2, ?3)",
            bind![group.name, group.uid, to_millis(group.created_at)],
        )
        .await?;
        let id = ContactGroupId::new(self.connection.last_insert_rowid());
        group.id = id;
        Ok(id)
    }

    /// One group.
    pub async fn get(&self, id: ContactGroupId) -> Result<Option<ContactGroup>> {
        sql::first(
            self.connection,
            &format!("SELECT {GROUP_COLUMNS} FROM contact_groups WHERE id = ?1"),
            [id.get()],
            read_group,
        )
        .await
    }

    /// Every group, by name.
    pub async fn list(&self) -> Result<Vec<ContactGroup>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {GROUP_COLUMNS} FROM contact_groups
                  ORDER BY name COLLATE NOCASE LIMIT 10000"
            ),
            (),
            read_group,
        )
        .await
    }

    /// Renames a group.
    pub async fn set_name(&self, id: ContactGroupId, name: &str) -> Result<()> {
        refuse_taken(self.connection, name, Some(id)).await?;
        let changed = sql::execute(
            self.connection,
            "UPDATE contact_groups SET name = ?2 WHERE id = ?1",
            bind![id.get(), name],
        )
        .await?;
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
    pub async fn delete(&self, id: ContactGroupId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM contact_groups WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    /// Deletes a group and hands back what [`restore`](Self::restore) needs.
    pub async fn remove(
        &self,
        id: ContactGroupId,
    ) -> Result<Option<(ContactGroup, Vec<ContactId>)>> {
        sql::in_scope(self.connection, |transaction| async move {
            let groups = ContactGroupRepository::new(&transaction);
            let Some(group) = groups.get(id).await? else {
                return Ok(None);
            };
            // Every member, the deleted ones too: restoring the group has to
            // give a restored person their place back as well.
            let members = member_ids(&transaction, id).await?;
            groups.delete(id).await?;
            Ok(Some((group, members)))
        })
        .await
    }

    /// Puts a deleted group back, same id, same members.
    pub async fn restore(&self, group: &ContactGroup, members: &[ContactId]) -> Result<()> {
        let group = group.clone();
        let members = members.to_vec();
        sql::in_scope(self.connection, |transaction| async move {
            refuse_taken(&transaction, &group.name, Some(group.id)).await?;
            sql::execute(
                &transaction,
                "INSERT INTO contact_groups (id, name, uid, created_at) VALUES (?1, ?2, ?3, ?4)",
                bind![
                    group.id.get(),
                    group.name,
                    group.uid,
                    to_millis(group.created_at)
                ],
            )
            .await?;
            ContactGroupRepository::new(&transaction)
                .add_members(group.id, &members)
                .await?;
            Ok(())
        })
        .await
    }

    /// Adds people, returning the ones who were not members already.
    pub async fn add_members(
        &self,
        group: ContactGroupId,
        people: &[ContactId],
    ) -> Result<Vec<ContactId>> {
        let mut added = Vec::new();
        for person in people {
            let changed = sql::execute(
                self.connection,
                "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
                 VALUES (?1, ?2)",
                bind![group.get(), person.get()],
            )
            .await?;
            if changed > 0 {
                added.push(*person);
            }
        }
        Ok(added)
    }

    /// Removes people, returning the ones who were members.
    pub async fn remove_members(
        &self,
        group: ContactGroupId,
        people: &[ContactId],
    ) -> Result<Vec<ContactId>> {
        let mut removed = Vec::new();
        for person in people {
            let changed = sql::execute(
                self.connection,
                "DELETE FROM contact_group_members WHERE group_id = ?1 AND contact_id = ?2",
                bind![group.get(), person.get()],
            )
            .await?;
            if changed > 0 {
                removed.push(*person);
            }
        }
        Ok(removed)
    }

    /// What picking the group fills in: each live member's preferred
    /// address, under their name (FR-041).
    pub async fn expand(&self, group: ContactGroupId) -> Result<Vec<postio_model::EmailAddress>> {
        Ok(self
            .members(group)
            .await?
            .iter()
            .filter_map(|person| {
                let preferred = person.preferred_address()?;
                let name = [person.name.as_deref(), person.seen_name.as_deref()]
                    .into_iter()
                    .flatten()
                    .map(str::trim)
                    .find(|name| !name.is_empty());
                Some(postio_model::EmailAddress::new(
                    name,
                    preferred.address.address.clone(),
                ))
            })
            .collect())
    }

    /// Adds a contact to a group. Adding one already a member is a no-op,
    /// not an error -- the membership either exists afterwards or it does
    /// not, and both calls asked for the same thing.
    pub async fn add_member(&self, group_id: ContactGroupId, contact_id: ContactId) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
             VALUES (?1, ?2)",
            bind![group_id.get(), contact_id.get()],
        )
        .await?;
        Ok(())
    }

    /// Removes a contact from a group. Removing one that was never a
    /// member is a no-op for the same reason adding twice is.
    pub async fn remove_member(
        &self,
        group_id: ContactGroupId,
        contact_id: ContactId,
    ) -> Result<()> {
        sql::execute(
            self.connection,
            "DELETE FROM contact_group_members WHERE group_id = ?1 AND contact_id = ?2",
            bind![group_id.get(), contact_id.get()],
        )
        .await?;
        Ok(())
    }

    /// The live people in a group, with their addresses — what a group
    /// expands to at compose time.
    ///
    /// A deleted member keeps its membership, so restoring them returns it
    /// (FR-023a), but is offered nowhere, and a group picked in the composer
    /// is no exception.
    pub async fn members(&self, group_id: ContactGroupId) -> Result<Vec<Contact>> {
        self.members_where(group_id, "AND c.state = 'live'").await
    }

    async fn members_where(&self, group_id: ContactGroupId, filter: &str) -> Result<Vec<Contact>> {
        let people = sql::all(
            self.connection,
            &format!(
                "SELECT {PERSON_COLUMNS_C} FROM contact_group_members m
                   JOIN contacts c ON c.id = m.contact_id
                  WHERE m.group_id = ?1 {filter}
                  ORDER BY c.sort_key, c.id LIMIT 10000"
            ),
            [group_id.get()],
            read_person,
        )
        .await?;
        ContactRepository::new(self.connection)
            .with_addresses(people)
            .await
    }
}

/// Every member of `group`, whatever their state.
async fn member_ids(connection: &Connection, group: ContactGroupId) -> Result<Vec<ContactId>> {
    sql::all(
        connection,
        "SELECT contact_id FROM contact_group_members WHERE group_id = ?1
          ORDER BY contact_id LIMIT 10000",
        [group.get()],
        |row| Ok(ContactId::new(row.col(0)?)),
    )
    .await
}

/// Refuses `name` when another group has it in any case, naming that group
/// -- two groups a person cannot tell apart by name are one too many.
async fn refuse_taken(
    connection: &Connection,
    name: &str,
    except: Option<ContactGroupId>,
) -> Result<()> {
    let taken: Option<String> = sql::first(
        connection,
        "SELECT name FROM contact_groups WHERE name = ?1 COLLATE NOCASE AND id <> ?2",
        bind![name.trim(), except.map_or(0, |id| id.get())],
        |row| row.col(0),
    )
    .await?;
    match taken {
        Some(existing) => Err(Error::ForbiddenTransition {
            what: "contact_group",
            reason: format!("there is already a group called {existing}"),
        }),
        None => Ok(()),
    }
}

fn read_group(row: &Row) -> Result<ContactGroup> {
    Ok(ContactGroup {
        id: ContactGroupId::new(row.col(0)?),
        name: row.col(1)?,
        uid: row.col(2)?,
        created_at: from_millis(row.col(3)?),
    })
}
