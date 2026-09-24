//! Contacts: people, the addresses they own, and what the mail says about them.
//!
//! A contact is a **person** who owns one or more addresses
//! (specs/005-contacts). Three places hold it, each for one reason:
//!
//! - `addresses.contact_id` says who owns an address. An address has at most
//!   one owner because it is a column, not a rule to keep in step (R1).
//! - `contact_sightings` is the mail's evidence, per `(address, account)`:
//!   how often, how recently, the display name last seen, and how many times
//!   the user wrote to it. It never moves when people are joined or split —
//!   history travels with the address (R2).
//! - `contacts` is the person: the name the user set, organisation, note,
//!   provenance, a `live | deleted | merged` state, and aggregates over its
//!   addresses' sightings so the list can order people without adding them up.
//!
//! **An address is suppressed exactly when its owner is not `live`** (R4).
//! Deleting a person keeps it, with its addresses, so a later message from one
//! of them counts toward someone hidden — which is what stops that message
//! bringing them back — and restoring returns them whole.
//!
//! Every address seen in mail starts as a person of its own. Nothing here ever
//! joins two addresses without being asked to.

use std::collections::{BTreeMap, BTreeSet};

use chrono::Utc;
use postio_model::{
    AddressId, Contact, ContactAddress, ContactDetail, ContactId, ContactListRow, ContactSource,
    AddressMove, ContactState, ContactView, EmailAddress, JoinReceipt, Message, PersonFields,
};

use super::{from_millis, to_millis};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// Reads and writes people and the addresses they own.
#[derive(Debug)]
pub struct ContactRepository<'a> {
    connection: &'a Connection,
}

/// The person's columns, in the order [`read_person`] reads them.
pub(super) const PERSON_COLUMNS: &str = "id, name, organization, note, source, state, \
     preferred_address, seen_name, times_seen, last_seen_at, written";

/// [`PERSON_COLUMNS`], qualified by `c.` for a join.
pub(super) const PERSON_COLUMNS_C: &str = "c.id, c.name, c.organization, c.note, c.source, \
     c.state, c.preferred_address, c.seen_name, c.times_seen, c.last_seen_at, c.written";

/// How many columns [`PERSON_COLUMNS`] is, so a joined row can find what
/// follows it.
const PERSON_WIDTH: usize = 11;

/// The completion order: the address book above the mail, then recency, then
/// frequency (the address-book decision's Q6, inherited by specs/005-contacts).
///
/// Spelled exactly as `idx_contacts_rank` declares it, because the planner
/// only reads an expression index through the identical expression.
pub(super) const RANK_ORDER: &str = "CASE WHEN source = 'mail' THEN 1 ELSE 0 END, \
     last_seen_at DESC, times_seen DESC, id";

/// One owned address, summed over the accounts it was seen through, in the
/// order [`read_address`] reads it. Needs `a` (addresses) and a `LEFT JOIN`ed
/// `s` (contact_sightings), grouped by `a.id`.
const ADDRESS_COLUMNS: &str = "a.id, a.address,
       (SELECT s2.last_name FROM contact_sightings s2
         WHERE s2.address_id = a.id AND s2.last_name IS NOT NULL
         ORDER BY s2.last_seen_at DESC LIMIT 1),
       coalesce(sum(s.times_seen), 0), max(s.last_seen_at), coalesce(sum(s.times_written), 0)";

/// One address on a message, and whether the user wrote to it.
struct Seen<'m> {
    address: &'m EmailAddress,
    written: bool,
}

impl<'a> ContactRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Records every correspondent on a message, in one transaction.
    ///
    /// `own` is the account's own addresses — its primary address and every
    /// identity. They are never recorded: the user is not their own contact.
    /// They are what decides "written to", though: when the message's `From`
    /// is one of them, every `To`/`Cc`/`Bcc` address was written to by the
    /// user (research R3), whatever folder the message sits in.
    ///
    /// Returns how many distinct correspondents were recorded. Appearing twice
    /// in one message — as a `To` and again as a `Cc`, or with two spellings —
    /// is one sighting: the count is "how many messages", not "how many
    /// headers". Callers record a message once, on first insert, which is what
    /// keeps a re-enumeration from double counting.
    pub async fn record_message(&self, message: &Message, own: &[EmailAddress]) -> Result<usize> {
        let at = to_millis(message.best_date());
        let account = message.account_id.get();
        let is_own = |address: &EmailAddress| own.iter().any(|mine| mine.same_address(address));
        let from_user = message.from.iter().any(is_own);
        let written: BTreeSet<String> = if from_user {
            message
                .all_recipients()
                .map(EmailAddress::normalized)
                .collect()
        } else {
            BTreeSet::new()
        };

        let mut seen: Vec<Seen<'_>> = Vec::new();
        for address in message
            .from
            .iter()
            .chain(&message.sender)
            .chain(&message.reply_to)
            .chain(message.all_recipients())
        {
            if address.address.trim().is_empty() || is_own(address) {
                continue;
            }
            let normalized = address.normalized();
            if seen.iter().any(|s| s.address.normalized() == normalized) {
                continue;
            }
            seen.push(Seen {
                address,
                written: written.contains(&normalized),
            });
        }

        sql::in_scope(self.connection, |transaction| async move {
            for one in &seen {
                record_in(&transaction, account, one, at).await?;
            }
            Ok(seen.len())
        })
        .await
    }

    /// One person, with every address they own.
    pub async fn get(&self, id: ContactId) -> Result<Option<Contact>> {
        let person = sql::first(
            self.connection,
            &format!("SELECT {PERSON_COLUMNS} FROM contacts WHERE id = ?1"),
            [id.get()],
            read_person,
        )
        .await?;
        match person {
            Some(person) => Ok(self.with_addresses(vec![person]).await?.pop()),
            None => Ok(None),
        }
    }

    /// The person who owns an address, matched case-insensitively — whatever
    /// their state.
    pub async fn by_address(&self, address: &str) -> Result<Option<Contact>> {
        let person = sql::first(
            self.connection,
            &format!(
                "SELECT {PERSON_COLUMNS_C} FROM addresses a
                   JOIN contacts c ON c.id = a.contact_id
                  WHERE a.address_normalized = ?1"
            ),
            [address.to_lowercase()],
            read_person,
        )
        .await?;
        match person {
            Some(person) => Ok(self.with_addresses(vec![person]).await?.pop()),
            None => Ok(None),
        }
    }

    /// Recipient completion: live people matching `prefix`, best first, each
    /// with their addresses, preferred first.
    ///
    /// **Banded, then ranked within a band — never across.** A person the
    /// user made or imported outranks every person known only from mail,
    /// however often a mailing-list robot has written: frequency is not
    /// evidence that the user wants to write back. Within a band, the person
    /// seen most recently leads, and frequency settles a tie (#424, #476).
    ///
    /// The match is a prefix on any word of the person's name, organisation or
    /// the name the mail gave them, and on each address — whole, local part,
    /// or domain — through `contact_terms` (research R5). People are shared
    /// across accounts, so completion in one account offers someone known
    /// through another.
    ///
    /// This runs on the UI thread, so its cost is the 16 ms budget: two
    /// statements, the people then their addresses, and neither a full scan.
    pub async fn complete(&self, prefix: &str, limit: u32) -> Result<Vec<Contact>> {
        let prefix = prefix.trim().to_lowercase();
        let people = if prefix.is_empty() {
            sql::all(
                self.connection,
                &format!(
                    "SELECT {PERSON_COLUMNS} FROM contacts WHERE state = 'live'
                      ORDER BY {RANK_ORDER} LIMIT ?1"
                ),
                [i64::from(limit)],
                read_person,
            )
            .await?
        } else {
            let upper = term_upper_bound(&prefix);
            sql::all(
                self.connection,
                &format!(
                    "SELECT {PERSON_COLUMNS} FROM contacts
                      WHERE state = 'live'
                        AND id IN (SELECT contact_id FROM contact_terms
                                    WHERE term >= ?1 AND term < ?2)
                      ORDER BY {RANK_ORDER} LIMIT ?3"
                ),
                bind![prefix, upper, i64::from(limit)],
                read_person,
            )
            .await?
        };
        self.with_addresses(people).await
    }

    /// Every live person, most familiar first, with their addresses — what
    /// the `@` finder holds to search in memory.
    ///
    /// One statement, `limit` bounding the addresses it returns: the finder
    /// scores in memory for the fuzzy matching it does, and the cap is what
    /// keeps that memory bounded however large the address book grows.
    pub async fn people(&self, limit: u32) -> Result<Vec<Contact>> {
        let rows = sql::all(
            self.connection,
            &format!(
                "SELECT {PERSON_COLUMNS_C}, {ADDRESS_COLUMNS}
                   FROM contacts c
                   JOIN addresses a ON a.contact_id = c.id
                   LEFT JOIN contact_sightings s ON s.address_id = a.id
                  WHERE c.state = 'live'
                  GROUP BY a.id
                  ORDER BY CASE WHEN c.source = 'mail' THEN 1 ELSE 0 END,
                           c.last_seen_at DESC, c.times_seen DESC, c.id, a.id
                  LIMIT ?1"
            ),
            [i64::from(limit)],
            |row| Ok((read_person(row)?, read_address(row, PERSON_WIDTH)?)),
        )
        .await?;

        let mut people: Vec<Contact> = Vec::new();
        for (person, address) in rows {
            match people.last_mut() {
                Some(last) if last.id == person.id => last.addresses.push(address),
                _ => {
                    let mut person = person;
                    person.addresses.push(address);
                    people.push(person);
                }
            }
        }
        for person in &mut people {
            put_preferred_first(person);
        }
        Ok(people)
    }

    /// Creates a person the user made, owning `addresses`, whether or not any
    /// mail involves them (specs/005-contacts FR-020).
    ///
    /// An address nobody owns, or one a *deleted* person owns, becomes this
    /// person's — the latter keeps its sighting history and stops being
    /// suppressed, because the user has said they want it (FR-024). An address
    /// a live person owns is refused with [`Error::AddressOwned`]: taking it is
    /// a decision the user makes, not a side effect (FR-015).
    pub async fn create(
        &self,
        name: Option<&str>,
        addresses: &[EmailAddress],
    ) -> Result<ContactId> {
        let name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned);
        let now = to_millis(Utc::now());
        sql::in_scope(self.connection, |transaction| async move {
            let mut claimed: Vec<(i64, Option<i64>)> = Vec::new();
            for address in addresses {
                let (id, owner) = address_and_owner(&transaction, address).await?;
                if let Some((owner, state)) = &owner
                    && state == "live"
                {
                    return Err(Error::AddressOwned {
                        address: id,
                        owner: *owner,
                    });
                }
                claimed.push((id, owner.map(|(id, _)| id)));
            }
            let Some(&(first, _)) = claimed.first() else {
                return Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "a person owns at least one address".into(),
                });
            };
            sql::execute(
                &transaction,
                "INSERT INTO contacts (name, source, state, preferred_address, sort_key,
                                       name_key, listed, created_at, updated_at)
                 VALUES (?1, 'user', 'live', ?2, '', '', 1, ?3, ?3)",
                bind![name, first, now],
            )
            .await?;
            let person = transaction.last_insert_rowid();
            let mut previous: BTreeSet<i64> = BTreeSet::new();
            for (address, owner) in &claimed {
                sql::execute(
                    &transaction,
                    "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                    bind![*address, person],
                )
                .await?;
                previous.extend(*owner);
            }
            refresh(&transaction, person).await?;
            for owner in previous {
                settle_after_losing_addresses(&transaction, owner, Some(person)).await?;
            }
            Ok(ContactId::new(person))
        })
        .await
    }

    /// Fills in each person's addresses, preferred first. One statement
    /// whatever the number of people.
    pub(super) async fn with_addresses(&self, mut people: Vec<Contact>) -> Result<Vec<Contact>> {
        if people.is_empty() {
            return Ok(people);
        }
        let placeholders = (1..=people.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let ids: Vec<turso::Value> = people
            .iter()
            .map(|p| turso::Value::Integer(p.id.get()))
            .collect();
        let rows = sql::all_unbounded(
            self.connection,
            &format!(
                "SELECT a.contact_id, {ADDRESS_COLUMNS}
                   FROM addresses a
                   LEFT JOIN contact_sightings s ON s.address_id = a.id
                  WHERE a.contact_id IN ({placeholders})
                  GROUP BY a.id
                  ORDER BY a.id"
            ),
            ids,
            |row| Ok((row.col::<i64>(0)?, read_address(row, 1)?)),
        )
        .await?;
        let mut by_person: BTreeMap<i64, Vec<ContactAddress>> = BTreeMap::new();
        for (person, address) in rows {
            by_person.entry(person).or_default().push(address);
        }
        for person in &mut people {
            person.addresses = by_person.remove(&person.id.get()).unwrap_or_default();
            put_preferred_first(person);
        }
        Ok(people)
    }
}

/// Where the next page of a Contacts view starts: just past this row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCursor {
    sort_key: String,
    id: ContactId,
}

impl ContactCursor {
    /// The cursor just past `row`.
    pub fn after(row: &ContactListRow) -> Self {
        Self {
            sort_key: row.sort_key.clone(),
            id: row.id,
        }
    }
}

impl ContactRepository<'_> {
    /// One page of a Contacts view, by name, starting just past `after`.
    ///
    /// Keyset, never `OFFSET`: the next page seeks past the last row's
    /// `(sort_key, id)`, and the cursor carries a bare `sort_key >= ?` in
    /// front of the tie-break because this engine seeks on that and only
    /// filters on the row value (docs/notes, 2026-09-12). One statement a
    /// page however deep, rows bounded by `limit`.
    pub async fn page(
        &self,
        view: ContactView,
        after: Option<&ContactCursor>,
        limit: u32,
    ) -> Result<Vec<ContactListRow>> {
        self.page_from(view, after, 0, limit).await
    }

    /// [`page`](Self::page), starting `skip` rows past `after`.
    ///
    /// For a jump into the list: the caller seeks to the nearest page
    /// boundary it has already read -- the message list's marks, in
    /// `postio-runtime` -- and skips only what is left, so sequential
    /// scrolling skips nothing and a jump skips less than a page's worth.
    pub async fn page_from(
        &self,
        view: ContactView,
        after: Option<&ContactCursor>,
        skip: u32,
        limit: u32,
    ) -> Result<Vec<ContactListRow>> {
        match after {
            None => {
                sql::all(
                    self.connection,
                    &first_page_sql(view),
                    bind![i64::from(limit), i64::from(skip)],
                    read_list_row,
                )
                .await
            }
            Some(cursor) => {
                sql::all(
                    self.connection,
                    &next_page_sql(view),
                    bind![
                        cursor.sort_key,
                        cursor.id.get(),
                        i64::from(limit),
                        i64::from(skip)
                    ],
                    read_list_row,
                )
                .await
            }
        }
    }

    /// What the detail view shows about one person: every address with its
    /// evidence, the groups they are in, and how many distinct messages
    /// involve any of their addresses (FR-006) -- a message to two of their
    /// addresses is one message, which is why this is a `DISTINCT` over
    /// recipients rather than a sum of per-address counts.
    ///
    /// Four statements whatever the person: them, their addresses, their
    /// groups, the count.
    pub async fn detail(&self, id: ContactId) -> Result<Option<ContactDetail>> {
        let Some(person) = self.get(id).await? else {
            return Ok(None);
        };
        let groups: Vec<String> = sql::all(
            self.connection,
            "SELECT g.name FROM contact_group_members m
               JOIN contact_groups g ON g.id = m.group_id
              WHERE m.contact_id = ?1
              ORDER BY g.name COLLATE NOCASE LIMIT 1000",
            [id.get()],
            |row| row.col(0),
        )
        .await?;
        let messages = sql::scalar(
            self.connection,
            "SELECT count(DISTINCT r.message_id) FROM recipients r
               JOIN addresses a ON a.id = r.address_id
              WHERE a.contact_id = ?1 AND r.message_id IS NOT NULL",
            [id.get()],
        )
        .await?;
        Ok(Some(ContactDetail {
            person,
            groups,
            messages: u64::try_from(messages).unwrap_or(0),
        }))
    }

    /// How many people a view holds, for the list's length.
    pub async fn count(&self, view: ContactView) -> Result<u64> {
        let count = sql::scalar(
            self.connection,
            &format!(
                "SELECT count(*) FROM contacts c WHERE {}",
                view_predicate(view)
            ),
            (),
        )
        .await?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    /// The people in a view matching every word of `text`, by name, at most
    /// `cap` of them (specs/005-contacts FR-004, research R5).
    ///
    /// Each word is a prefix range over `contact_terms` -- a name, the
    /// organisation, the name the mail gave, any part of any address -- never
    /// a `LIKE` over everyone. The cap is what bounds the rows a keystroke
    /// materialises; typing more is how a person narrows past it.
    pub async fn filtered(
        &self,
        view: ContactView,
        text: &str,
        cap: u32,
    ) -> Result<Vec<ContactListRow>> {
        let words: Vec<String> = words(text).collect();
        if words.is_empty() {
            return self.page(view, None, cap).await;
        }
        let mut arguments: Vec<turso::Value> = Vec::new();
        let mut clauses = Vec::new();
        for word in &words {
            let low = arguments.len() + 1;
            clauses.push(format!(
                "c.id IN (SELECT contact_id FROM contact_terms \
                  WHERE term >= ?{low} AND term < ?{})",
                low + 1
            ));
            arguments.push(turso::Value::Text(word.clone()));
            arguments.push(turso::Value::Text(term_upper_bound(word)));
        }
        let limit = arguments.len() + 1;
        arguments.push(turso::Value::Integer(i64::from(cap)));
        sql::all(
            self.connection,
            &format!(
                "SELECT {LIST_COLUMNS} FROM contacts c
                   LEFT JOIN addresses pa ON pa.id = c.preferred_address
                  WHERE {} AND {}
                  ORDER BY c.sort_key, c.id LIMIT ?{limit}",
                view_predicate(view),
                clauses.join(" AND ")
            ),
            arguments,
            read_list_row,
        )
        .await
    }
}

/// A list row's columns, in the order [`read_list_row`] reads them. Needs `c`
/// (contacts) and a `LEFT JOIN`ed `pa` (the preferred address).
const LIST_COLUMNS: &str = "c.id,
       coalesce(nullif(trim(c.name), ''), nullif(trim(c.seen_name), ''), pa.address, ''),
       pa.address,
       (SELECT count(*) FROM addresses x WHERE x.contact_id = c.id),
       c.last_seen_at, c.source, c.state, c.sort_key";

/// Which people a view holds. Each is a prefix of one of the list indexes, so
/// a page is a seek: `idx_contacts_list_default` for the default view,
/// `idx_contacts_list` for the other two.
fn view_predicate(view: ContactView) -> &'static str {
    match view {
        ContactView::Written => "c.state = 'live' AND c.listed = 1",
        ContactView::Everyone => "c.state = 'live'",
        ContactView::Deleted => "c.state = 'deleted'",
    }
}

fn first_page_sql(view: ContactView) -> String {
    format!(
        "SELECT {LIST_COLUMNS} FROM contacts c
           LEFT JOIN addresses pa ON pa.id = c.preferred_address
          WHERE {}
          ORDER BY c.sort_key, c.id LIMIT ?1 OFFSET ?2",
        view_predicate(view)
    )
}

fn next_page_sql(view: ContactView) -> String {
    format!(
        "SELECT {LIST_COLUMNS} FROM contacts c
           LEFT JOIN addresses pa ON pa.id = c.preferred_address
          WHERE {} AND c.sort_key >= ?1 AND (c.sort_key > ?1 OR c.id > ?2)
          ORDER BY c.sort_key, c.id LIMIT ?3 OFFSET ?4",
        view_predicate(view)
    )
}

/// Every statement the list issues, named -- for the budget that asks the
/// planner about them.
#[cfg(feature = "test-support")]
pub(crate) fn list_statements() -> Vec<(&'static str, String)> {
    vec![
        ("written first", first_page_sql(ContactView::Written)),
        ("written after", next_page_sql(ContactView::Written)),
        ("everyone first", first_page_sql(ContactView::Everyone)),
        ("everyone after", next_page_sql(ContactView::Everyone)),
        ("deleted first", first_page_sql(ContactView::Deleted)),
        ("deleted after", next_page_sql(ContactView::Deleted)),
    ]
}

fn read_list_row(row: &Row) -> Result<ContactListRow> {
    let source: String = row.col(5)?;
    let state: String = row.col(6)?;
    Ok(ContactListRow {
        id: ContactId::new(row.col(0)?),
        name: row.col(1)?,
        preferred: row.col(2)?,
        address_count: row.col(3)?,
        last_seen_at: row.col::<Option<i64>>(4)?.map(from_millis),
        source: ContactSource::from_name(&source).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.source",
            value: source.clone(),
        })?,
        state: ContactState::from_name(&state).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.state",
            value: state.clone(),
        })?,
        sort_key: row.col(7)?,
    })
}

/// Takes `address` away from whichever person owns it, because it has become
/// one of the user's own (an account's or an identity's). The user is never
/// their own contact (spec edge case): the record path skips own addresses as
/// it sees them, and this is the other half, for an address that was
/// somebody's before it was the user's. Doing it at write time is what lets
/// every list read go without a per-row exclusion.
pub(crate) async fn release_own_address(
    connection: &Connection,
    address: &EmailAddress,
) -> Result<()> {
    let owner: Option<Option<i64>> = sql::first(
        connection,
        "SELECT contact_id FROM addresses WHERE address_normalized = ?1",
        bind![address.normalized()],
        |row| row.col(0),
    )
    .await?;
    if let Some(Some(owner)) = owner {
        sql::execute(
            connection,
            "UPDATE addresses SET contact_id = NULL WHERE address_normalized = ?1",
            bind![address.normalized()],
        )
        .await?;
        settle_after_losing_addresses(connection, owner, None).await?;
    }
    Ok(())
}

impl ContactRepository<'_> {
    /// Joins `others` into `into`: one person afterwards, owning every
    /// address, keeping every sighting, every group and every note
    /// (specs/005-contacts FR-012, FR-013).
    ///
    /// `name` is the joined person's name, which the user picked or typed --
    /// every join ends with one, and it counts as set by the user from here
    /// on. `organization`, when given, is the one the user chose between
    /// conflicting ones; otherwise the survivor's stands, or the first absorbed
    /// person's if it had none. The absorbed become `merged` rather than
    /// deleted, which is what lets [`unjoin`](Self::unjoin) put them back
    /// exactly from the [`JoinReceipt`] this returns.
    pub async fn join(
        &self,
        into: ContactId,
        others: &[ContactId],
        name: &str,
        organization: Option<&str>,
    ) -> Result<JoinReceipt> {
        let name = name.trim().to_owned();
        let organization = organization
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(str::to_owned);
        let others: Vec<ContactId> = others.iter().copied().filter(|o| *o != into).collect();
        sql::in_scope(self.connection, |transaction| async move {
            let survivor = fields(&transaction, into).await?;
            let into_groups = groups_of(&transaction, into).await?;
            let mut restore = Vec::new();
            let mut added: Vec<postio_model::ContactGroupId> = Vec::new();
            let mut notes: Vec<String> = survivor.note.iter().cloned().collect();
            let mut fallback_organization = survivor.organization.clone();
            for other in &others {
                let absorbed = fields(&transaction, *other).await?;
                let owned: Vec<i64> = sql::all(
                    &transaction,
                    "SELECT id FROM addresses WHERE contact_id = ?1 ORDER BY id LIMIT 10000",
                    [other.get()],
                    |row| row.col(0),
                )
                .await?;
                restore.push((*other, owned.iter().map(|a| AddressId::new(*a)).collect()));
                sql::execute(
                    &transaction,
                    "UPDATE addresses SET contact_id = ?2 WHERE contact_id = ?1",
                    bind![other.get(), into.get()],
                )
                .await?;
                for group in groups_of(&transaction, *other).await? {
                    if !into_groups.contains(&group) && !added.contains(&group) {
                        sql::execute(
                            &transaction,
                            "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
                             VALUES (?1, ?2)",
                            bind![group.get(), into.get()],
                        )
                        .await?;
                        added.push(group);
                    }
                }
                notes.extend(absorbed.note.into_iter().filter(|n| !n.trim().is_empty()));
                if fallback_organization.is_none() {
                    fallback_organization = absorbed.organization;
                }
                sql::execute(
                    &transaction,
                    "UPDATE contacts SET state = 'merged', merged_into = ?2 WHERE id = ?1",
                    bind![other.get(), into.get()],
                )
                .await?;
                sql::execute(
                    &transaction,
                    "DELETE FROM contact_terms WHERE contact_id = ?1",
                    [other.get()],
                )
                .await?;
            }
            let note = (!notes.is_empty()).then(|| notes.join("\n\n"));
            sql::execute(
                &transaction,
                "UPDATE contacts
                    SET name = ?2, organization = ?3, note = ?4, source = 'user',
                        updated_at = ?5
                  WHERE id = ?1",
                bind![
                    into.get(),
                    name,
                    organization.or(fallback_organization),
                    note,
                    to_millis(Utc::now())
                ],
            )
            .await?;
            refresh(&transaction, into.get()).await?;
            Ok(JoinReceipt {
                into,
                restore,
                prior: survivor,
                added_memberships: added,
            })
        })
        .await
    }

    /// Takes a join apart: every absorbed person back, with the addresses
    /// that were theirs, and the survivor's fields and groups as they were.
    pub async fn unjoin(&self, receipt: &JoinReceipt) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            for (person, owned) in &receipt.restore {
                for address in owned {
                    sql::execute(
                        &transaction,
                        "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                        bind![address.get(), person.get()],
                    )
                    .await?;
                }
                sql::execute(
                    &transaction,
                    "UPDATE contacts SET state = 'live', merged_into = NULL WHERE id = ?1",
                    [person.get()],
                )
                .await?;
                refresh(&transaction, person.get()).await?;
            }
            for group in &receipt.added_memberships {
                sql::execute(
                    &transaction,
                    "DELETE FROM contact_group_members WHERE group_id = ?1 AND contact_id = ?2",
                    bind![group.get(), receipt.into.get()],
                )
                .await?;
            }
            let prior = &receipt.prior;
            sql::execute(
                &transaction,
                "UPDATE contacts
                    SET name = ?2, organization = ?3, note = ?4, source = ?5,
                        preferred_address = ?6
                  WHERE id = ?1",
                bind![
                    receipt.into.get(),
                    prior.name,
                    prior.organization,
                    prior.note,
                    prior.source.as_str(),
                    prior.preferred.get()
                ],
            )
            .await?;
            refresh(&transaction, receipt.into.get()).await?;
            Ok(())
        })
        .await
    }

    /// Takes `address` away from its person and makes it a person of its own,
    /// carrying its own sighting history (FR-014). Refused for a person's last
    /// address -- deleting is how a person goes.
    pub async fn detach_address(&self, address: AddressId) -> Result<ContactId> {
        sql::in_scope(self.connection, |transaction| async move {
            let owner: Option<Option<i64>> = sql::first(
                &transaction,
                "SELECT contact_id FROM addresses WHERE id = ?1",
                [address.get()],
                |row| row.col(0),
            )
            .await?;
            let Some(Some(owner)) = owner else {
                return Err(Error::NotFound {
                    entity: "address",
                    id: address.get(),
                });
            };
            let owned = sql::scalar(
                &transaction,
                "SELECT count(*) FROM addresses WHERE contact_id = ?1",
                [owner],
            )
            .await?;
            if owned <= 1 {
                return Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "a person keeps at least one address; delete them instead".into(),
                });
            }
            let seen = sql::scalar(
                &transaction,
                "SELECT coalesce(sum(times_seen), 0) FROM contact_sightings WHERE address_id = ?1",
                [address.get()],
            )
            .await?;
            // Known from mail if the mail knows it; otherwise it was typed in.
            let source = if seen > 0 { "mail" } else { "user" };
            let now = to_millis(Utc::now());
            sql::execute(
                &transaction,
                "INSERT INTO contacts (source, state, preferred_address, sort_key, name_key,
                                       created_at, updated_at)
                 VALUES (?1, 'live', ?2, '', '', ?3, ?3)",
                bind![source, address.get(), now],
            )
            .await?;
            let person = transaction.last_insert_rowid();
            sql::execute(
                &transaction,
                "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                bind![address.get(), person],
            )
            .await?;
            refresh(&transaction, person).await?;
            refresh(&transaction, owner).await?;
            Ok(ContactId::new(person))
        })
        .await
    }

    /// Gives `person` an address they typed. One nobody owns, or one a deleted
    /// person owns, becomes theirs; one a live person owns is refused with
    /// [`Error::AddressOwned`], naming who, so the caller can offer to move it
    /// (FR-015).
    pub async fn add_address(&self, person: ContactId, address: &EmailAddress) -> Result<AddressId> {
        sql::in_scope(self.connection, |transaction| async move {
            let (id, owner) = address_and_owner(&transaction, address).await?;
            match &owner {
                Some((owner, _)) if *owner == person.get() => return Ok(AddressId::new(id)),
                Some((owner, state)) if state == "live" => {
                    return Err(Error::AddressOwned {
                        address: id,
                        owner: *owner,
                    });
                }
                _ => {}
            }
            sql::execute(
                &transaction,
                "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                bind![id, person.get()],
            )
            .await?;
            promote(&transaction, person).await?;
            refresh(&transaction, person.get()).await?;
            if let Some((previous, _)) = owner {
                settle_after_losing_addresses(&transaction, previous, Some(person.get())).await?;
            }
            Ok(AddressId::new(id))
        })
        .await
    }

    /// Moves `address` to `person`, whoever had it -- the "move it" answer to
    /// [`Error::AddressOwned`], and what undoes a detach or an add. Moving an
    /// address off a deleted person lifts its suppression (FR-024).
    ///
    /// `revive`, given by an undo, is the state `person` had before an earlier
    /// move left them with nothing: they come back in it. The returned
    /// [`AddressMove`] is what the same undo needs for this move.
    pub async fn move_address(
        &self,
        address: AddressId,
        person: ContactId,
        revive: Option<ContactState>,
    ) -> Result<AddressMove> {
        sql::in_scope(self.connection, |transaction| async move {
            let previous = owner_of(&transaction, address).await?;
            if previous == Some(person.get()) && revive.is_none() {
                return Ok(AddressMove {
                    previous: previous.map(ContactId::new),
                    emptied: None,
                });
            }
            sql::execute(
                &transaction,
                "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                bind![address.get(), person.get()],
            )
            .await?;
            if let Some(state) = revive {
                sql::execute(
                    &transaction,
                    "UPDATE contacts SET state = ?2, merged_into = NULL WHERE id = ?1",
                    bind![person.get(), state.as_str()],
                )
                .await?;
            }
            refresh(&transaction, person.get()).await?;
            let emptied = match previous {
                Some(previous) if previous != person.get() => {
                    settle_after_losing_addresses(&transaction, previous, Some(person.get()))
                        .await?
                }
                _ => None,
            };
            Ok(AddressMove {
                previous: previous.map(ContactId::new),
                emptied,
            })
        })
        .await
    }

    /// The names the user gave the live owners of `addresses`, keyed by
    /// normalised address -- what the reader substitutes for a header's
    /// display name (FR-032). An address nobody live owns, or whose owner the
    /// user never named, is absent: its mail says what it said.
    pub async fn user_names(
        &self,
        addresses: &[String],
    ) -> Result<std::collections::HashMap<String, String>> {
        if addresses.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        let placeholders = (1..=addresses.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let wanted: Vec<turso::Value> = addresses
            .iter()
            .map(|a| turso::Value::Text(a.to_lowercase()))
            .collect();
        let rows = sql::all_unbounded(
            self.connection,
            &format!(
                "SELECT a.address_normalized, c.name
                   FROM addresses a
                   JOIN contacts c ON c.id = a.contact_id
                  WHERE a.address_normalized IN ({placeholders})
                    AND c.state = 'live' AND c.name IS NOT NULL"
            ),
            wanted,
            |row| Ok((row.col::<String>(0)?, row.col::<String>(1)?)),
        )
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Who owns `address`, or `None` for nobody.
    pub async fn owner_of_address(&self, address: AddressId) -> Result<Option<ContactId>> {
        Ok(owner_of(self.connection, address)
            .await?
            .map(ContactId::new))
    }

    /// Lets go of `address`: it belongs to nobody afterwards, as a typed
    /// address did before anyone added it. What undoes adding one nobody
    /// owned.
    pub async fn release_address(&self, address: AddressId) -> Result<AddressMove> {
        sql::in_scope(self.connection, |transaction| async move {
            let previous = owner_of(&transaction, address).await?;
            sql::execute(
                &transaction,
                "UPDATE addresses SET contact_id = NULL WHERE id = ?1",
                [address.get()],
            )
            .await?;
            let emptied = match previous {
                Some(previous) => settle_after_losing_addresses(&transaction, previous, None).await?,
                None => None,
            };
            Ok(AddressMove {
                previous: previous.map(ContactId::new),
                emptied,
            })
        })
        .await
    }

    /// Makes `address` the one completion offers first and "write to" uses
    /// (FR-016). Returns the preferred address it replaced. Refused for an
    /// address that is not the person's own.
    pub async fn set_preferred(&self, person: ContactId, address: AddressId) -> Result<AddressId> {
        sql::in_scope(self.connection, |transaction| async move {
            let owns = sql::exists(
                &transaction,
                "SELECT 1 FROM addresses WHERE id = ?1 AND contact_id = ?2",
                bind![address.get(), person.get()],
            )
            .await?;
            if !owns {
                return Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "a person's preferred address is one of their own".into(),
                });
            }
            let previous = fields(&transaction, person).await?.preferred;
            sql::execute(
                &transaction,
                "UPDATE contacts SET preferred_address = ?2 WHERE id = ?1",
                bind![person.get(), address.get()],
            )
            .await?;
            refresh(&transaction, person.get()).await?;
            Ok(previous)
        })
        .await
    }
}

/// Who owned `address`, or `None` for nobody; an address the store does not
/// have is an error.
async fn owner_of(connection: &Connection, address: AddressId) -> Result<Option<i64>> {
    let found: Option<Option<i64>> = sql::first(
        connection,
        "SELECT contact_id FROM addresses WHERE id = ?1",
        [address.get()],
        |row| row.col(0),
    )
    .await?;
    found.ok_or(Error::NotFound {
        entity: "address",
        id: address.get(),
    })
}

/// A person's editable fields, for a receipt or an inverse.
async fn fields(connection: &Connection, person: ContactId) -> Result<PersonFields> {
    type Row = (Option<String>, Option<String>, Option<String>, String, Option<i64>);
    let row: Option<Row> = sql::first(
        connection,
        "SELECT name, organization, note, source, preferred_address FROM contacts WHERE id = ?1",
        [person.get()],
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?, row.col(3)?, row.col(4)?)),
    )
    .await?;
    let Some((name, organization, note, source, preferred)) = row else {
        return Err(Error::NotFound {
            entity: "contact",
            id: person.get(),
        });
    };
    Ok(PersonFields {
        name,
        organization,
        note,
        source: ContactSource::from_name(&source).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.source",
            value: source.clone(),
        })?,
        preferred: AddressId::new(preferred.unwrap_or(0)),
    })
}

/// The groups a person is in.
async fn groups_of(
    connection: &Connection,
    person: ContactId,
) -> Result<Vec<postio_model::ContactGroupId>> {
    sql::all(
        connection,
        "SELECT group_id FROM contact_group_members WHERE contact_id = ?1 LIMIT 10000",
        [person.get()],
        |row| Ok(postio_model::ContactGroupId::new(row.col(0)?)),
    )
    .await
}

/// A deliberate edit promotes a person known only from mail to one the user
/// made, in place (FR-022).
async fn promote(connection: &Connection, person: ContactId) -> Result<()> {
    sql::execute(
        connection,
        "UPDATE contacts SET source = 'user' WHERE id = ?1 AND source = 'mail'",
        [person.get()],
    )
    .await?;
    Ok(())
}

/// Records one correspondent, inside the message's transaction.
///
/// Three statements when the address already has an owner and its name has
/// not changed, which is almost every call on a mailbox of any age: find the
/// address and its owner, count the sighting, add it to the owner's
/// aggregates. A first sighting creates the person — two more — and a new
/// display name adds its words to the filter index. Every statement is a
/// literal so it is compiled once per connection (#728).
async fn record_in(connection: &Connection, account: i64, seen: &Seen<'_>, at: i64) -> Result<()> {
    type Found = (
        i64,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<i64>,
    );
    let address = seen.address;
    let normalized = address.normalized();
    let found: Option<Found> = sql::first(
        connection,
        "SELECT a.id, a.contact_id, c.name, c.seen_name, c.last_seen_at
           FROM addresses a LEFT JOIN contacts c ON c.id = a.contact_id
          WHERE a.address_normalized = ?1",
        bind![normalized],
        |row| {
            Ok((
                row.col(0)?,
                row.col(1)?,
                row.col(2)?,
                row.col(3)?,
                row.col(4)?,
            ))
        },
    )
    .await?;
    let (address_id, owner, user_name, seen_name, last_seen) = match found {
        Some(found) => found,
        // Sync writes a message's recipients before recording it, so the
        // address row is already there; a caller recording a message it never
        // stored gets one made here.
        None => (
            super::messages::address_id(connection, address).await?,
            None,
            None,
            None,
            None,
        ),
    };
    let header_name = address
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let written = i64::from(seen.written);

    sql::execute(
        connection,
        "INSERT INTO contact_sightings
             (address_id, account_id, times_seen, last_seen_at, last_name, times_written)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)
         ON CONFLICT (address_id, account_id) DO UPDATE SET
             times_seen = times_seen + 1,
             last_name = CASE WHEN excluded.last_seen_at >= coalesce(last_seen_at, 0)
                              THEN coalesce(excluded.last_name, last_name)
                              ELSE last_name END,
             last_seen_at = max(coalesce(last_seen_at, excluded.last_seen_at),
                                excluded.last_seen_at),
             times_written = times_written + excluded.times_written",
        bind![address_id, account, at, header_name, written],
    )
    .await?;

    match owner {
        None => {
            let displayed = header_name.unwrap_or(&normalized);
            sql::execute(
                connection,
                "INSERT INTO contacts (source, state, preferred_address, seen_name, sort_key,
                                       name_key, times_seen, last_seen_at, written, listed,
                                       created_at, updated_at)
                 VALUES ('mail', 'live', ?1, ?2, ?3, ?4, 1, ?5, ?6, ?6 > 0, ?5, ?5)",
                bind![
                    address_id,
                    header_name,
                    sort_key(displayed),
                    name_key(displayed),
                    at,
                    written,
                ],
            )
            .await?;
            let person = connection.last_insert_rowid();
            sql::execute(
                connection,
                "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
                bind![address_id, person],
            )
            .await?;
            let mut terms = address_terms(address);
            terms.extend(header_name.into_iter().flat_map(words));
            insert_terms(connection, person, &terms).await?;
        }
        Some(person) => {
            let is_newest = last_seen.is_none_or(|last| at >= last);
            let renamed = header_name
                .filter(|name| is_newest && seen_name.as_deref().map(str::trim) != Some(*name));
            match renamed {
                None => {
                    sql::execute(
                        connection,
                        "UPDATE contacts
                            SET times_seen = times_seen + 1, written = written + ?2,
                                listed = max(listed, ?2 > 0),
                                last_seen_at = max(coalesce(last_seen_at, ?3), ?3)
                          WHERE id = ?1",
                        bind![person, written, at],
                    )
                    .await?;
                }
                Some(name) => {
                    // The person's displayed name only follows the mail while
                    // the user has not set one of their own.
                    let displayed = user_name
                        .as_deref()
                        .map(str::trim)
                        .filter(|n| !n.is_empty())
                        .unwrap_or(name);
                    sql::execute(
                        connection,
                        "UPDATE contacts
                            SET times_seen = times_seen + 1, written = written + ?2,
                                listed = max(listed, ?2 > 0),
                                last_seen_at = max(coalesce(last_seen_at, ?3), ?3),
                                seen_name = ?4, sort_key = ?5, name_key = ?6
                          WHERE id = ?1",
                        bind![
                            person,
                            written,
                            at,
                            name,
                            sort_key(displayed),
                            name_key(displayed)
                        ],
                    )
                    .await?;
                    insert_terms(connection, person, &words(name).collect()).await?;
                }
            }
        }
    }
    Ok(())
}

/// An address's row id and, if someone owns it, the owner's id and state.
/// Creates the address row when the store has never seen it.
pub(super) async fn address_and_owner(
    connection: &Connection,
    address: &EmailAddress,
) -> Result<(i64, Option<(i64, String)>)> {
    let found: Option<(i64, Option<i64>, Option<String>)> = sql::first(
        connection,
        "SELECT a.id, a.contact_id, c.state
           FROM addresses a LEFT JOIN contacts c ON c.id = a.contact_id
          WHERE a.address_normalized = ?1",
        bind![address.normalized()],
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
    )
    .await?;
    match found {
        Some((id, Some(owner), Some(state))) => Ok((id, Some((owner, state)))),
        Some((id, _, _)) => Ok((id, None)),
        None => Ok((
            super::messages::address_id(connection, address).await?,
            None,
        )),
    }
}

/// Recomputes everything about a person that is derived from their addresses:
/// the aggregates, the name the mail last gave them, the sort and suggestion
/// keys, a preferred address that is one of their own, and their filter
/// terms. Used after any structural edit — create, join, detach, move — which
/// are rare, so clarity wins over statement count here.
pub(super) async fn refresh(connection: &Connection, person: i64) -> Result<()> {
    sql::execute(
        connection,
        "UPDATE contacts SET
             times_seen = (SELECT coalesce(sum(s.times_seen), 0) FROM contact_sightings s
                             JOIN addresses a ON a.id = s.address_id WHERE a.contact_id = ?1),
             last_seen_at = (SELECT max(s.last_seen_at) FROM contact_sightings s
                               JOIN addresses a ON a.id = s.address_id WHERE a.contact_id = ?1),
             written = (SELECT coalesce(sum(s.times_written), 0) FROM contact_sightings s
                          JOIN addresses a ON a.id = s.address_id WHERE a.contact_id = ?1),
             seen_name = (SELECT s.last_name FROM contact_sightings s
                            JOIN addresses a ON a.id = s.address_id
                           WHERE a.contact_id = ?1 AND s.last_name IS NOT NULL
                           ORDER BY s.last_seen_at DESC LIMIT 1),
             preferred_address = CASE
                 WHEN preferred_address IN (SELECT id FROM addresses WHERE contact_id = ?1)
                 THEN preferred_address
                 ELSE (SELECT min(id) FROM addresses WHERE contact_id = ?1) END
         WHERE id = ?1",
        [person],
    )
    .await?;

    type Facts = (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let facts: Option<Facts> = sql::first(
        connection,
        "SELECT c.name, c.organization, c.seen_name, a.address
           FROM contacts c LEFT JOIN addresses a ON a.id = c.preferred_address
          WHERE c.id = ?1",
        [person],
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?, row.col(3)?)),
    )
    .await?;
    let Some((name, organization, seen_name, preferred)) = facts else {
        return Err(Error::NotFound {
            entity: "contact",
            id: person,
        });
    };
    let displayed = [name.as_deref(), seen_name.as_deref(), preferred.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|n| !n.is_empty())
        .unwrap_or("")
        .to_owned();
    sql::execute(
        connection,
        // `listed` here and not in the statement above: an UPDATE's
        // expressions all read the row as it was, so this is the first point
        // at which `written` holds its new value.
        "UPDATE contacts
            SET sort_key = ?2, name_key = ?3,
                listed = CASE WHEN source <> 'mail' OR written > 0 THEN 1 ELSE 0 END
          WHERE id = ?1",
        bind![person, sort_key(&displayed), name_key(&displayed)],
    )
    .await?;

    let owned: Vec<String> = sql::all(
        connection,
        "SELECT address FROM addresses WHERE contact_id = ?1 LIMIT 1000",
        [person],
        |row| row.col(0),
    )
    .await?;
    let mut terms: BTreeSet<String> = BTreeSet::new();
    for text in [&name, &organization, &seen_name].into_iter().flatten() {
        terms.extend(words(text));
    }
    for address in &owned {
        terms.extend(address_terms(&EmailAddress::new(
            None::<String>,
            address.clone(),
        )));
    }
    sql::execute(
        connection,
        "DELETE FROM contact_terms WHERE contact_id = ?1",
        [person],
    )
    .await?;
    insert_terms(connection, person, &terms).await?;
    Ok(())
}

/// After `person` has lost addresses to someone else: refreshed if they
/// still own one, and folded into `receiver` -- `merged`, offered nowhere --
/// if they are left with none.
///
/// Folded rather than removed, and the state they were in handed back,
/// because undo has to be able to give the address back to *them*: a
/// person removed here would leave the inverse with nobody to return it to
/// (specs/005-contacts research R8).
pub(super) async fn settle_after_losing_addresses(
    connection: &Connection,
    person: i64,
    receiver: Option<i64>,
) -> Result<Option<ContactState>> {
    let remaining = sql::scalar(
        connection,
        "SELECT count(*) FROM addresses WHERE contact_id = ?1",
        [person],
    )
    .await?;
    if remaining > 0 {
        refresh(connection, person).await?;
        return Ok(None);
    }
    let state: Option<String> = sql::first(
        connection,
        "SELECT state FROM contacts WHERE id = ?1",
        [person],
        |row| row.col(0),
    )
    .await?;
    sql::execute(
        connection,
        "UPDATE contacts SET state = 'merged', merged_into = ?2 WHERE id = ?1",
        bind![person, receiver],
    )
    .await?;
    sql::execute(
        connection,
        "DELETE FROM contact_terms WHERE contact_id = ?1",
        [person],
    )
    .await?;
    Ok(state.as_deref().and_then(ContactState::from_name))
}

/// Adds `terms` to a person's filter index; one statement for all of them.
async fn insert_terms(
    connection: &Connection,
    person: i64,
    terms: &BTreeSet<String>,
) -> Result<()> {
    if terms.is_empty() {
        return Ok(());
    }
    let values = (0..terms.len())
        .map(|i| format!("(?{}, ?1)", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    let mut arguments = vec![turso::Value::Integer(person)];
    arguments.extend(terms.iter().map(|t| turso::Value::Text(t.clone())));
    sql::execute(
        connection,
        &format!("INSERT OR IGNORE INTO contact_terms (term, contact_id) VALUES {values}"),
        arguments,
    )
    .await?;
    Ok(())
}

/// The words of a name or an organisation, lowercased: what a person types
/// to find it, first name or surname alike.
pub(crate) fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
}

/// The ways a person types an address to find it: the whole thing, the local
/// part, the domain, and each word of either — so `lovelace` finds
/// `ada.lovelace@example.com` and `example` finds its whole domain.
pub(crate) fn address_terms(address: &EmailAddress) -> BTreeSet<String> {
    let normalized = address.normalized();
    let mut terms = BTreeSet::new();
    if let Some((local, domain)) = normalized.rsplit_once('@') {
        terms.insert(local.to_owned());
        terms.insert(domain.to_owned());
        terms.extend(words(local));
        terms.extend(words(domain));
    }
    terms.insert(normalized);
    terms.retain(|t| !t.is_empty());
    terms
}

/// The smallest string greater than every string starting with `prefix`, for
/// an index range `term >= prefix AND term < bound`.
pub(crate) fn term_upper_bound(prefix: &str) -> String {
    let mut bound = prefix.to_owned();
    bound.push(char::MAX);
    bound
}

/// A displayed name, folded for ordering the list.
pub(crate) fn sort_key(displayed: &str) -> String {
    displayed.trim().to_lowercase()
}

/// A displayed name, folded for noticing that two people share it (R11).
pub(crate) fn name_key(displayed: &str) -> String {
    displayed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Moves the preferred address to the front, the order completion offers them.
fn put_preferred_first(person: &mut Contact) {
    if let Some(at) = person
        .addresses
        .iter()
        .position(|a| a.id == person.preferred)
    {
        let preferred = person.addresses.remove(at);
        person.addresses.insert(0, preferred);
    }
}

/// A person, without addresses — [`ContactRepository::with_addresses`] adds
/// them.
pub(super) fn read_person(row: &Row) -> Result<Contact> {
    let source: String = row.col(4)?;
    let state: String = row.col(5)?;
    Ok(Contact {
        id: ContactId::new(row.col(0)?),
        name: row.col(1)?,
        organization: row.col(2)?,
        note: row.col(3)?,
        source: ContactSource::from_name(&source).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.source",
            value: source.clone(),
        })?,
        state: ContactState::from_name(&state).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.state",
            value: state.clone(),
        })?,
        preferred: AddressId::new(row.col::<Option<i64>>(6)?.unwrap_or(0)),
        addresses: Vec::new(),
        seen_name: row.col(7)?,
        times_seen: row.col(8)?,
        last_seen_at: row.col::<Option<i64>>(9)?.map(from_millis),
        written: row.col(10)?,
    })
}

/// One owned address from [`ADDRESS_COLUMNS`], starting at column `at`.
fn read_address(row: &Row, at: usize) -> Result<ContactAddress> {
    Ok(ContactAddress {
        id: AddressId::new(row.col(at)?),
        address: EmailAddress::new(
            row.col::<Option<String>>(at + 2)?,
            row.col::<String>(at + 1)?,
        ),
        times_seen: row.col(at + 3)?,
        last_seen_at: row.col::<Option<i64>>(at + 4)?.map(from_millis),
        written: row.col(at + 5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_found_by_its_whole_self_its_parts_and_their_words() {
        let terms = address_terms(&EmailAddress::new(
            None::<String>,
            "Ada.Lovelace@Mail.Example",
        ));
        for expected in [
            "ada.lovelace@mail.example",
            "ada.lovelace",
            "ada",
            "lovelace",
            "mail.example",
            "mail",
            "example",
        ] {
            assert!(
                terms.contains(expected),
                "{expected} missing from {terms:?}"
            );
        }
    }

    #[test]
    fn the_upper_bound_sorts_after_every_extension_of_the_prefix() {
        let bound = term_upper_bound("ada");
        for term in ["ada", "adam", "ada.lovelace@example.com", "adaz\u{10ffff}"] {
            assert!(term < bound.as_str(), "{term} must fall inside the range");
        }
        assert!("adb" > bound.as_str());
    }

    #[test]
    fn names_fold_to_the_same_key_whatever_their_case_and_spacing() {
        assert_eq!(name_key("  Ada   Lovelace "), name_key("ada lovelace"));
        assert_eq!(sort_key(" Ada Lovelace"), "ada lovelace");
    }
}
