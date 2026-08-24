//! Contacts, accumulated from the addresses Postio has seen.
//!
//! v1 has no address book integration (docs/PRODUCT.md): the recipient autocomplete
//! ranks from what has actually come through the mailbox. Every address on
//! every message becomes a sighting, and a correspondent's score is how often
//! and how recently they have been seen.
//!
//! Identity is the **normalized address** — the addr-spec, lowercased. Display
//! names change constantly (`Ada`, `Ada Norwood`, `ADA NORWOOD`, none at all)
//! and cannot be identity; the most recent one is kept for display, and a name
//! the *user* sets overrides it and is never overwritten by a later sighting.

use chrono::{DateTime, Utc};
use postio_model::{AccountId, Contact, ContactId, EmailAddress, Message};
use rusqlite::types::Value;
use rusqlite::{Connection, Row, params, params_from_iter};

use super::{from_millis, to_millis};
use crate::error::{Error, Result};

/// Reads and writes [`Contact`] rows.
#[derive(Debug)]
pub struct ContactRepository<'a> {
    connection: &'a Connection,
}

const CONTACT_COLUMNS: &str = "\
id, account_id, name, address, address_name, times_seen, last_seen_at";

impl<'a> ContactRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Records one sighting of `address`, creating the contact if it is new.
    ///
    /// `account_id` of `None` means a contact shared across accounts.
    /// `last_seen_at` only ever moves forward — a message that arrives late is
    /// still a sighting, but it does not make the correspondent look more
    /// recent than they are.
    pub fn record(
        &self,
        account_id: Option<AccountId>,
        address: &EmailAddress,
        at: DateTime<Utc>,
    ) -> Result<ContactId> {
        let transaction = super::Scope::open(self.connection)?;
        let id = record_in(&transaction, account_id, address, at)?;
        transaction.commit()?;
        Ok(id)
    }

    /// Records every address on a message, in one transaction.
    ///
    /// Returns how many distinct correspondents were seen. Appearing twice in
    /// one message — as a `To` and again as a `Cc`, or with two spellings — is
    /// one sighting: the score is "how many messages", not "how many headers".
    pub fn record_message(&self, message: &Message) -> Result<usize> {
        let at = message.best_date();
        let account_id = Some(message.account_id);

        let mut addresses: Vec<&EmailAddress> = Vec::new();
        for address in message
            .from
            .iter()
            .chain(&message.sender)
            .chain(&message.reply_to)
            .chain(message.all_recipients())
        {
            if address.address.trim().is_empty() {
                continue;
            }
            let normalized = address.normalized();
            if addresses.iter().any(|seen| seen.normalized() == normalized) {
                continue;
            }
            addresses.push(address);
        }

        let transaction = super::Scope::open(self.connection)?;
        for address in &addresses {
            record_in(&transaction, account_id, address, at)?;
        }
        transaction.commit()?;
        Ok(addresses.len())
    }

    /// One contact.
    pub fn get(&self, id: ContactId) -> Result<Option<Contact>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {CONTACT_COLUMNS} FROM contacts WHERE id = ?1"
        ))?;
        let mut rows = statement.query([id.get()])?;
        Ok(rows.next()?.map(read_contact).transpose()?)
    }

    /// The contact for an address, matched case-insensitively.
    pub fn by_address(
        &self,
        account_id: Option<AccountId>,
        address: &str,
    ) -> Result<Option<Contact>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {CONTACT_COLUMNS} FROM contacts
              WHERE {} AND address_normalized = ?{}",
            account_filter(account_id),
            first_free(account_id)
        ))?;
        let mut arguments = account_argument(account_id);
        arguments.push(Value::Text(address.to_lowercase()));
        let mut rows = statement.query(params_from_iter(arguments))?;
        Ok(rows.next()?.map(read_contact).transpose()?)
    }

    /// Every contact, most familiar first.
    pub fn list(&self, account_id: Option<AccountId>) -> Result<Vec<Contact>> {
        self.search(account_id, "", u32::MAX)
    }

    /// Autocomplete: contacts whose address or name starts with `prefix`.
    ///
    /// Ranked by how often the correspondent has been seen, then by how
    /// recently — a colleague written to daily outranks someone mailed once
    /// this morning, and between two equally familiar addresses the more recent
    /// one wins. An empty prefix returns the most familiar correspondents,
    /// which is what an empty recipient field should offer.
    ///
    /// The match is a prefix on the address, on the local part, and on any word
    /// of the display name: people type the name they remember, and they type
    /// surnames as often as first names.
    pub fn search(
        &self,
        account_id: Option<AccountId>,
        prefix: &str,
        limit: u32,
    ) -> Result<Vec<Contact>> {
        let prefix = prefix.trim().to_lowercase();
        let text = first_free(account_id);
        let limit_index = text + 1;
        let mut statement = self.connection.prepare(&format!(
            "SELECT {CONTACT_COLUMNS} FROM contacts
              WHERE {} AND (
                    ?{text} = ''
                 OR address_normalized LIKE ?{text} || '%'
                 OR lower(coalesce(name, '')) LIKE ?{text} || '%'
                 OR lower(coalesce(name, '')) LIKE '% ' || ?{text} || '%'
                 OR lower(coalesce(address_name, '')) LIKE ?{text} || '%'
                 OR lower(coalesce(address_name, '')) LIKE '% ' || ?{text} || '%'
              )
              ORDER BY times_seen DESC, last_seen_at DESC, id
              LIMIT ?{limit_index}",
            account_filter(account_id)
        ))?;

        let mut arguments = account_argument(account_id);
        arguments.push(Value::Text(prefix));
        arguments.push(Value::Integer(i64::from(limit)));
        let rows = statement.query_map(params_from_iter(arguments), read_contact)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Sets, or clears, the name the user chose for a contact.
    pub fn set_name(&self, id: ContactId, name: Option<&str>) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE contacts SET name = ?2 WHERE id = ?1",
            params![id.get(), name],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "contact",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Deletes a contact, returning whether there was one.
    pub fn delete(&self, id: ContactId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM contacts WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }
}

/// `record`, without a transaction of its own.
fn record_in(
    connection: &Connection,
    account_id: Option<AccountId>,
    address: &EmailAddress,
    at: DateTime<Utc>,
) -> Result<ContactId> {
    let normalized = address.normalized();

    let mut arguments = account_argument(account_id);
    arguments.push(Value::Text(normalized.clone()));
    let existing: Option<i64> = connection
        .query_row(
            &format!(
                "SELECT id FROM contacts WHERE {} AND address_normalized = ?{}",
                account_filter(account_id),
                first_free(account_id)
            ),
            params_from_iter(arguments),
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;

    match existing {
        Some(id) => {
            connection.execute(
                "UPDATE contacts
                    SET address = ?2, address_name = ?3, times_seen = times_seen + 1,
                        last_seen_at = max(coalesce(last_seen_at, ?4), ?4)
                  WHERE id = ?1",
                params![id, address.address, address.name, to_millis(at)],
            )?;
            Ok(ContactId::new(id))
        }
        None => {
            connection.execute(
                "INSERT INTO contacts (account_id, name, address, address_name,
                                       address_normalized, times_seen, last_seen_at)
                 VALUES (?1, NULL, ?2, ?3, ?4, 1, ?5)",
                params![
                    account_id.map(AccountId::get),
                    address.address,
                    address.name,
                    normalized,
                    to_millis(at),
                ],
            )?;
            Ok(ContactId::new(connection.last_insert_rowid()))
        }
    }
}

/// The account predicate, with `?2` bound to the account when there is one.
///
/// `account_id IS NULL` is not the same as `account_id = ?`: SQLite's `=` is
/// never true for NULL, so a shared contact has to be matched explicitly.
fn account_filter(account_id: Option<AccountId>) -> &'static str {
    match account_id {
        Some(_) => "account_id = ?1",
        None => "account_id IS NULL",
    }
}

/// The first parameter index left over once the account filter has taken its
/// own. Keeping the account at `?1` whenever it is bound is what stops the two
/// shapes of every query here from disagreeing about the numbering.
/// The leading parameter list: the account id, when there is one.
fn account_argument(account_id: Option<AccountId>) -> Vec<Value> {
    account_id
        .map(|id| vec![Value::Integer(id.get())])
        .unwrap_or_default()
}

fn first_free(account_id: Option<AccountId>) -> usize {
    match account_id {
        Some(_) => 2,
        None => 1,
    }
}

fn read_contact(row: &Row<'_>) -> rusqlite::Result<Contact> {
    Ok(Contact {
        id: ContactId::new(row.get(0)?),
        account_id: row.get::<_, Option<i64>>(1)?.map(AccountId::new),
        name: row.get(2)?,
        address: EmailAddress::new(row.get::<_, Option<String>>(4)?, row.get::<_, String>(3)?),
        times_seen: row.get(5)?,
        last_seen_at: row.get::<_, Option<i64>>(6)?.map(from_millis),
    })
}
