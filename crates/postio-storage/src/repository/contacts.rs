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
use postio_model::card::{ImportSummary, Member, ParsedCard, ParsedGroup, ParsedPerson};
use postio_model::{
    AddressId, AddressMove, Contact, ContactAddress, ContactDetail, ContactId, ContactKind,
    ContactListRow, ContactSource, ContactState, ContactView, EmailAddress, JoinReceipt,
    JoinSuggestion, KindSignals, KindSource, Message, PersonEdit, PersonFields, ShowAs,
    SuggestionReason,
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
     preferred_address, seen_name, times_seen, last_seen_at, written, kind, kind_source, \
     saved_at, given_name, family_name, show_as, title";

/// [`PERSON_COLUMNS`], qualified by `c.` for a join.
pub(super) const PERSON_COLUMNS_C: &str = "c.id, c.name, c.organization, c.note, c.source, \
     c.state, c.preferred_address, c.seen_name, c.times_seen, c.last_seen_at, c.written, \
     c.kind, c.kind_source, c.saved_at, c.given_name, c.family_name, c.show_as, c.title";

/// How many columns [`PERSON_COLUMNS`] is, so a joined row can find what
/// follows it.
const PERSON_WIDTH: usize = 18;

/// How many of a sender's newest messages their kind is decided from.
pub const KIND_WINDOW: i64 = 50;

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
    /// Whether the address is among the message's senders: only mail from an
    /// address counts as opened or not.
    sender: bool,
}

/// What recording one message knows beyond one address on it.
struct Recording<'m> {
    account: i64,
    at: i64,
    own: &'m [String],
    /// The message's id, or 0 for one never stored (which has no ring entry).
    message: i64,
    read: bool,
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

        let own_normalized: Vec<String> = own.iter().map(EmailAddress::normalized).collect();
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
                sender: message.from.iter().any(|from| from.same_address(address)),
            });
        }

        let recording = Recording {
            account,
            at,
            own: &own_normalized,
            message: message.id.get(),
            read: message.flags.contains(&postio_model::Flag::Seen),
        };
        let recording = &recording;
        sql::in_scope(self.connection, |transaction| async move {
            for one in &seen {
                record_in(&transaction, recording, one).await?;
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
                refuse_own(&transaction, address).await?;
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

    /// The person who owns `address`, made now from the mail if nobody does
    /// yet (specs/011-focus-contacts plan 4.2).
    ///
    /// Mail that was stored before contacts existed has addresses and no
    /// people; this is what `i` or a card on such a sender runs, so nothing
    /// waits on the catch-up. What the person's sightings say is folded in
    /// SQL, one row per account, so the cost does not follow the number of
    /// messages. The user's own addresses are refused.
    pub async fn ensure_for_address(&self, address: AddressId) -> Result<ContactId> {
        sql::in_scope(self.connection, |transaction| async move {
            match adopt(&transaction, address.get()).await? {
                Some(person) => Ok(ContactId::new(person)),
                None => Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "that address is one of yours".into(),
                }),
            }
        })
        .await
    }

    /// The person who owns `email`, made now if nobody does: what a setting
    /// given to an address needs, whether or not mail from it was ever
    /// stored. Never saved, so the setting does not put them in the
    /// Contacts list. Refuses the user's own address.
    pub async fn person_for_email(&self, email: &EmailAddress) -> Result<ContactId> {
        let email = email.clone();
        sql::in_scope(self.connection, |transaction| async move {
            refuse_own(&transaction, &email).await?;
            let (address, owner) = address_and_owner(&transaction, &email).await?;
            if let Some((owner, _)) = owner {
                return Ok(ContactId::new(owner));
            }
            match adopt(&transaction, address).await? {
                Some(person) => Ok(ContactId::new(person)),
                None => Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "that address is one of yours".into(),
                }),
            }
        })
        .await
    }

    /// What the mail says about `person`, summed over their addresses: the
    /// tiles, the strip of months since `since_month` (a
    /// [`postio_model::month_key`]), and the newest messages. Four reads at
    /// most, by key over the person's addresses, so what it costs follows
    /// their addresses and never their mail.
    pub async fn stats(
        &self,
        person: ContactId,
        since_month: i64,
    ) -> Result<postio_model::ContactStats> {
        let owned: Vec<i64> = sql::all(
            self.connection,
            &Self::explain_stats(1)[0],
            [person.get()],
            |row| row.col(0),
        )
        .await?;
        if owned.is_empty() {
            return Ok(postio_model::ContactStats::default());
        }
        let queries = Self::explain_stats(owned.len());
        let ids =
            || -> Vec<turso::Value> { owned.iter().map(|id| turso::Value::Integer(*id)).collect() };

        let (messages, from_you, opened, last): (i64, i64, i64, Option<i64>) =
            sql::one(self.connection, &queries[1], ids(), |row| {
                Ok((row.col(0)?, row.col(1)?, row.col(2)?, row.col(3)?))
            })
            .await?;

        let mut bound = ids();
        bound.push(turso::Value::Integer(since_month));
        let months: Vec<(i64, u32)> =
            sql::all_unbounded(self.connection, &queries[2], bound, |row| {
                Ok((
                    row.col(0)?,
                    u32::try_from(row.col::<i64>(1)?.max(0)).unwrap_or(u32::MAX),
                ))
            })
            .await?;

        let recent: Vec<postio_model::RecentMessage> =
            sql::all_unbounded(self.connection, &queries[3], ids(), |row| {
                Ok(postio_model::RecentMessage {
                    message: postio_model::MessageId::new(row.col(0)?),
                    at: from_millis(row.col(1)?),
                    outgoing: row.col::<String>(2)? == "out",
                })
            })
            .await?;

        let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        Ok(postio_model::ContactStats {
            messages: count(messages),
            from_you: count(from_you),
            opened: count(opened),
            last: last.map(from_millis),
            months,
            recent,
        })
    }

    /// The SQL [`Self::stats`] runs, in order, for a person with `addresses`
    /// addresses: their ids, the sightings summed, the months (the last
    /// placeholder is the first month wanted) and the ring.
    pub fn explain_stats(addresses: usize) -> [String; 4] {
        let ids = (1..=addresses.max(1))
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let since = addresses.max(1) + 1;
        [
            "SELECT id FROM addresses WHERE contact_id = ?1 ORDER BY id LIMIT 1000".to_owned(),
            format!(
                "SELECT coalesce(sum(times_seen), 0), coalesce(sum(times_written), 0),
                        coalesce(sum(seen_count), 0), max(last_seen_at)
                   FROM contact_sightings WHERE address_id IN ({ids})"
            ),
            format!(
                "SELECT month, sum(n) FROM address_months
                  WHERE address_id IN ({ids}) AND month >= ?{since}
                  GROUP BY month ORDER BY month"
            ),
            format!(
                "SELECT message_id, at, direction FROM address_recent
                  WHERE address_id IN ({ids})
                  ORDER BY at DESC, message_id DESC LIMIT {}",
                postio_model::RECENT
            ),
        ]
    }

    /// What is open between the user and `person`, at most `limit` things,
    /// the ones waiting on the user first: undismissed markers on their mail
    /// and unsettled reminders on what was sent to them, both found through
    /// the ring of each address's newest messages, so what it reads is
    /// bounded by that and never by the person's mail.
    pub async fn open_items(
        &self,
        person: ContactId,
        limit: u32,
    ) -> Result<Vec<postio_model::OpenItem>> {
        use postio_model::{OpenItem, OpenSide};
        let owned: Vec<i64> = sql::all(
            self.connection,
            &Self::explain_stats(1)[0],
            [person.get()],
            |row| row.col(0),
        )
        .await?;
        if owned.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let queries = Self::explain_open_items(owned.len());
        let mut bound: Vec<turso::Value> =
            owned.iter().map(|id| turso::Value::Integer(*id)).collect();
        bound.push(turso::Value::Integer(i64::from(limit)));
        let you: Vec<(i64, String)> =
            sql::all_unbounded(self.connection, &queries[0], bound.clone(), |row| {
                Ok((row.col(0)?, row.col(1)?))
            })
            .await?;
        let them: Vec<(i64, String)> =
            sql::all_unbounded(self.connection, &queries[1], bound, |row| {
                Ok((row.col(0)?, row.col(1)?))
            })
            .await?;
        let item = |side, (message, text): (i64, String)| OpenItem {
            side,
            text,
            message: postio_model::MessageId::new(message),
        };
        // One of each first, so neither side crowds the other out.
        let (mut you, mut them) = (you.into_iter(), them.into_iter());
        let mut items = Vec::new();
        while items.len() < limit as usize {
            let before = items.len();
            if let Some(next) = you.next() {
                items.push(item(OpenSide::WaitingOnYou, next));
            }
            if items.len() < limit as usize
                && let Some(next) = them.next()
            {
                items.push(item(OpenSide::WaitingOnThem, next));
            }
            if items.len() == before {
                break;
            }
        }
        Ok(items)
    }

    /// Where the default rules put `person`'s mail: what the engine did with
    /// their newest inbound message, found through the address rings. Filtered
    /// when it was filed away and not restored, Digest when a digest rule is
    /// holding it, Inbox otherwise and for someone with no mail.
    pub async fn default_arrival(&self, person: ContactId) -> Result<postio_model::ContactArrival> {
        use postio_model::ContactArrival;
        let owned: Vec<i64> = sql::all(
            self.connection,
            &Self::explain_stats(1)[0],
            [person.get()],
            |row| row.col(0),
        )
        .await?;
        if owned.is_empty() {
            return Ok(ContactArrival::Inbox);
        }
        let bound: Vec<turso::Value> = owned.iter().map(|id| turso::Value::Integer(*id)).collect();
        let found: Vec<String> = sql::all_unbounded(
            self.connection,
            &Self::explain_default_arrival(owned.len())[0],
            bound,
            |row| row.col(0),
        )
        .await?;
        Ok(found
            .first()
            .and_then(|name| ContactArrival::from_name(name))
            .unwrap_or(ContactArrival::Inbox))
    }

    /// The SQL [`Self::default_arrival`] runs for a person with `addresses`
    /// addresses.
    pub fn explain_default_arrival(addresses: usize) -> [String; 1] {
        let ids = (1..=addresses.max(1))
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        [format!(
            "SELECT CASE WHEN f.message_id IS NOT NULL THEN 'filtered'
                        WHEN h.message_id IS NOT NULL THEN 'digest'
                        ELSE 'inbox' END
               FROM address_recent r
               LEFT JOIN filter_decisions f ON f.message_id = r.message_id AND f.restored_at IS NULL
               LEFT JOIN digest_holds h ON h.message_id = r.message_id
              WHERE r.address_id IN ({ids}) AND r.direction = 'in'
              ORDER BY r.at DESC, r.message_id DESC LIMIT 1"
        )]
    }

    /// The SQL [`Self::open_items`] runs for a person with `addresses`
    /// addresses: what waits on the user, then what the user waits on.
    pub fn explain_open_items(addresses: usize) -> [String; 2] {
        let ids = (1..=addresses.max(1))
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let limit = addresses.max(1) + 1;
        [
            format!(
                "SELECT r.message_id, coalesce(k.excerpt, 'An invitation') FROM address_recent r
                   JOIN markers k ON k.message_id = r.message_id AND k.dismissed_at IS NULL
                  WHERE r.address_id IN ({ids}) AND r.direction = 'in'
                  ORDER BY r.at DESC, r.message_id DESC LIMIT ?{limit}"
            ),
            format!(
                "SELECT r.message_id, coalesce(m.subject, '(no subject)') FROM address_recent r
                   JOIN reminders d ON d.anchor_message_id = r.message_id
                    AND d.cancelled_at IS NULL AND d.settled_at IS NULL
                   JOIN messages m ON m.id = r.message_id
                  WHERE r.address_id IN ({ids}) AND r.direction = 'out'
                  ORDER BY r.at DESC, r.message_id DESC LIMIT ?{limit}"
            ),
        ]
    }

    /// How many live people of each kind there are and how many of those are
    /// saved: the Contacts tabs' counts, read from a handful of rows the
    /// triggers keep.
    pub async fn kind_counts(&self) -> Result<Vec<(ContactKind, u32, u32)>> {
        let rows: Vec<(String, i64, i64)> = sql::all_unbounded(
            self.connection,
            "SELECT kind, total, saved FROM contact_counts WHERE total > 0 ORDER BY kind",
            (),
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
        )
        .await?;
        let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        rows.into_iter()
            .map(|(kind, total, saved)| {
                ContactKind::from_name(&kind)
                    .map(|kind| (kind, count(total), count(saved)))
                    .ok_or(Error::UnknownEnum {
                        column: "contact_counts.kind",
                        value: kind,
                    })
            })
            .collect()
    }

    /// One batch of the catch-up: up to `limit` addresses with mail and no
    /// person, newest first and strictly below `before`, each made a person
    /// in one transaction.
    ///
    /// The walk is one read whatever the store holds, so a pass with nothing
    /// to do costs that one read. `last` is where the next batch starts;
    /// `done` says this one reached the end. Safe to run beside live sync:
    /// an address that already has a person is not in the walk.
    pub async fn catch_up_batch(&self, before: Option<AddressId>, limit: u32) -> Result<CatchUp> {
        let before = before.map_or(i64::MAX, AddressId::get);
        sql::in_scope(self.connection, |transaction| async move {
            let walk: Vec<i64> = sql::all(
                &transaction,
                Self::explain_unowned(),
                bind![before, i64::from(limit)],
                |row| row.col(0),
            )
            .await?;
            let mut made = 0;
            for address in &walk {
                if adopt(&transaction, *address).await?.is_some() {
                    made += 1;
                }
            }
            Ok(CatchUp {
                made,
                last: walk.last().copied().map(AddressId::new),
                done: walk.len() < limit as usize,
            })
        })
        .await
    }

    /// How many addresses with mail still wait for a person: the "m" of
    /// "Building contacts, n of m".
    pub async fn unowned_count(&self) -> Result<u64> {
        let count = sql::scalar(
            self.connection,
            "SELECT count(*) FROM addresses a
              WHERE a.contact_id IS NULL
                AND a.address_normalized NOT IN (SELECT lower(address) FROM accounts)
                AND a.address_normalized NOT IN (SELECT lower(address) FROM identities)
                AND EXISTS (SELECT 1 FROM recipients r
                             WHERE r.address_id = a.id AND r.message_id IS NOT NULL)",
            (),
        )
        .await?;
        Ok(count.max(0) as u64)
    }

    /// The SQL [`Self::catch_up_batch`] walks.
    pub fn explain_unowned() -> &'static str {
        "SELECT a.id FROM addresses a
          WHERE a.contact_id IS NULL AND a.id < ?1
            AND a.address_normalized NOT IN (SELECT lower(address) FROM accounts)
            AND a.address_normalized NOT IN (SELECT lower(address) FROM identities)
            AND EXISTS (SELECT 1 FROM recipients r
                         WHERE r.address_id = a.id AND r.message_id IS NOT NULL)
          ORDER BY a.id DESC
          LIMIT ?2"
    }

    /// What the sender's mail says, for deciding their kind: counted over the
    /// newest [`KIND_WINDOW`] messages their preferred address sent, so the
    /// cost is the same for a sender of one message and of ten thousand.
    /// `None` for someone who is not live.
    ///
    /// Three reads: the person, the window, and the people at their domain.
    pub async fn kind_signals(&self, person: ContactId) -> Result<Option<KindSignals>> {
        type Head = (Option<String>, Option<String>, Option<String>, i64, i64);
        let head: Option<Head> = sql::first(
            self.connection,
            "SELECT a.address, c.name, c.seen_name, c.written, a.id
               FROM contacts c JOIN addresses a ON a.id = c.preferred_address
              WHERE c.id = ?1 AND c.state = 'live'",
            [person.get()],
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
        let Some((address, name, seen_name, written, preferred)) = head else {
            return Ok(None);
        };
        let address = address.unwrap_or_default();
        let window: (i64, i64, i64, i64, i64, i64, i64, i64) = sql::one(
            self.connection,
            Self::explain_kind_window(),
            bind![preferred, KIND_WINDOW],
            |row| {
                Ok((
                    row.col(0)?,
                    row.col(1)?,
                    row.col(2)?,
                    row.col(3)?,
                    row.col(4)?,
                    row.col(5)?,
                    row.col(6)?,
                    row.col(7)?,
                ))
            },
        )
        .await?;
        let domain = address.rsplit_once('@').map(|(_, d)| d.to_lowercase());
        let people_at_domain =
            match domain {
                Some(domain) => sql::scalar(
                    self.connection,
                    "SELECT count(*) FROM contact_terms t JOIN contacts c ON c.id = t.contact_id
                      WHERE t.term = ?1 AND c.state = 'live' AND c.kind = 'person' AND c.id <> ?2",
                    bind![domain, person.get()],
                )
                .await?,
                None => 0,
            };
        let count = |n: i64| u32::try_from(n.max(0)).unwrap_or(u32::MAX);
        Ok(Some(KindSignals {
            address,
            name: name.or(seen_name),
            messages: count(window.0),
            with_list_id: count(window.1),
            with_list_precedence: count(window.2),
            with_unsubscribe: count(window.3),
            with_bulk: count(window.4),
            with_junk: count(window.5),
            with_auto_generated: count(window.6),
            with_auto_replied: count(window.7),
            written_to: count(written),
            people_at_domain: count(people_at_domain),
        }))
    }

    /// The SQL [`Self::kind_signals`] counts the window with. `?1` is the
    /// address, `?2` the window's size. Walks the address's rows newest first
    /// along `idx_recipients_address`, so it reads the window and no more.
    pub fn explain_kind_window() -> &'static str {
        "SELECT count(*),
                coalesce(sum(CASE WHEN m.list_id IS NOT NULL THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN (m.automation & 2) <> 0 THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN m.unsubscribe_offered = 1 THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN (m.automation & 1) <> 0 THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN (m.automation & 4) <> 0 THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN (m.automation & 8) <> 0 THEN 1 ELSE 0 END), 0),
                coalesce(sum(CASE WHEN (m.automation & 16) <> 0 THEN 1 ELSE 0 END), 0)
           FROM (SELECT message_id FROM recipients
                  WHERE address_id = ?1 AND kind = 'from' AND message_id IS NOT NULL
                  ORDER BY id DESC LIMIT ?2) w
           JOIN messages m ON m.id = w.message_id"
    }

    /// Up to `limit` live people whose kind the detector decides and has not
    /// decided at `version` yet: the kind pass's walk. A kind the user chose
    /// is not in it.
    ///
    /// No cursor: deciding someone takes them out of the walk, so the next
    /// call finds the next ones, and a start where everyone is decided is one
    /// seek that finds nothing.
    pub async fn pending_kinds(&self, version: u32, limit: u32) -> Result<Vec<ContactId>> {
        sql::all(
            self.connection,
            Self::explain_pending_kinds(),
            bind![i64::from(version), i64::from(limit)],
            |row| Ok(ContactId::new(row.col(0)?)),
        )
        .await
    }

    /// The SQL [`Self::pending_kinds`] runs, over `idx_contacts_kind_pending`.
    pub fn explain_pending_kinds() -> &'static str {
        "SELECT id FROM contacts
          WHERE state = 'live' AND kind_source = 'detected' AND kind_version < ?1
          LIMIT ?2"
    }

    /// Records what the detector decided, at `version`. Does nothing for a
    /// person whose kind the user chose.
    pub async fn set_detected_kind(
        &self,
        person: ContactId,
        kind: ContactKind,
        version: u32,
    ) -> Result<()> {
        sql::execute(
            self.connection,
            "UPDATE contacts SET kind = ?2, kind_version = ?3
              WHERE id = ?1 AND kind_source = 'detected'",
            bind![person.get(), kind.as_str(), i64::from(version)],
        )
        .await?;
        Ok(())
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

/// The columns of a kind-tab page, and its seek: `idx_contacts_kind_list`.
fn kind_first_sql() -> String {
    format!(
        "SELECT {LIST_COLUMNS} FROM contacts c
           {LIST_JOINS}
          WHERE c.state = 'live' AND c.kind = ?1
          ORDER BY c.sort_key, c.id LIMIT ?2"
    )
}

fn kind_next_sql() -> String {
    format!(
        "SELECT {LIST_COLUMNS} FROM contacts c
           {LIST_JOINS}
          WHERE c.state = 'live' AND c.kind = ?1
            AND c.sort_key >= ?2 AND (c.sort_key > ?2 OR c.id > ?3)
          ORDER BY c.sort_key, c.id LIMIT ?4"
    )
}

impl ContactRepository<'_> {
    /// One page of the people of `kinds`, by name, starting `skip` rows past
    /// `after`. One seek over the kind index per kind, each at most
    /// `skip + limit` rows, merged in name order, so a tab holding two kinds
    /// costs two statements and a page of it is the same size as any other.
    pub async fn page_kinds(
        &self,
        kinds: &[ContactKind],
        after: Option<&ContactCursor>,
        skip: u32,
        limit: u32,
    ) -> Result<Vec<ContactListRow>> {
        let wanted = i64::from(skip) + i64::from(limit);
        let mut rows: Vec<ContactListRow> = Vec::new();
        for kind in kinds {
            let found = match after {
                None => {
                    sql::all(
                        self.connection,
                        &kind_first_sql(),
                        bind![kind.as_str(), wanted],
                        read_list_row,
                    )
                    .await?
                }
                Some(cursor) => {
                    sql::all(
                        self.connection,
                        &kind_next_sql(),
                        bind![kind.as_str(), cursor.sort_key, cursor.id.get(), wanted],
                        read_list_row,
                    )
                    .await?
                }
            };
            rows.extend(found);
        }
        rows.sort_by(|a, b| (&a.sort_key, a.id.get()).cmp(&(&b.sort_key, b.id.get())));
        Ok(rows
            .into_iter()
            .skip(skip as usize)
            .take(limit as usize)
            .collect())
    }

    /// Where `person` is in the list of `kinds`: how many live people of
    /// those kinds come before them by name. `None` for someone not in the
    /// list (another kind, deleted, or no one). One seek per kind.
    pub async fn position_in(
        &self,
        kinds: &[ContactKind],
        person: ContactId,
    ) -> Result<Option<u32>> {
        let found: Option<(String, String)> = sql::first(
            self.connection,
            "SELECT sort_key, kind FROM contacts WHERE id = ?1 AND state = 'live'",
            [person.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let Some((key, kind)) = found else {
            return Ok(None);
        };
        if !kinds.iter().any(|k| k.as_str() == kind) {
            return Ok(None);
        }
        let mut before = 0_i64;
        for kind in kinds {
            before += sql::scalar(
                self.connection,
                "SELECT count(*) FROM contacts
                  WHERE state = 'live' AND kind = ?1
                    AND (sort_key < ?2 OR (sort_key = ?2 AND id < ?3))",
                bind![kind.as_str(), key.clone(), person.get()],
            )
            .await?;
        }
        Ok(Some(u32::try_from(before).unwrap_or(u32::MAX)))
    }

    /// The people of `kinds` matching every word of `text`, by name, at most
    /// `cap` of them: [`filtered`](Self::filtered) within a tab.
    pub async fn filtered_kinds(
        &self,
        kinds: &[ContactKind],
        text: &str,
        cap: u32,
    ) -> Result<Vec<ContactListRow>> {
        self.filtered_in(ContactView::Everyone, kinds, text, cap)
            .await
    }

    /// What the strip and the footer say: each tab's count and the totals
    /// from `contact_counts`, the review queue, and how many addresses with
    /// mail still wait for a person. A handful of rows and three capped
    /// counts, whatever the number of people.
    pub async fn overview(&self) -> Result<postio_model::ContactsOverview> {
        use postio_model::{ContactTab, ContactsOverview, REVIEW_CAP, ReviewSummary};
        let counts = self.kind_counts().await?;
        let of = |kinds: &[ContactKind]| -> u32 {
            counts
                .iter()
                .filter(|(kind, _, _)| kinds.contains(kind))
                .map(|(_, total, _)| *total)
                .sum()
        };
        let saved: u32 = counts.iter().map(|(_, _, saved)| *saved).sum();
        let capped = |count: i64| {
            u32::try_from(count.max(0))
                .unwrap_or(u32::MAX)
                .min(REVIEW_CAP)
        };
        let duplicates = {
            let pairs: Vec<i64> = sql::all_unbounded(
                self.connection,
                &format!("SELECT count(*) FROM ({SUGGESTION_PAIRS})"),
                bind![i64::from(REVIEW_CAP)],
                |row| row.col(0),
            )
            .await?;
            capped(pairs.first().copied().unwrap_or(0))
        };
        let frequent = sql::scalar(
            self.connection,
            Self::explain_review()[0],
            bind![i64::from(REVIEW_CAP)],
        )
        .await?;
        let waiting = self.unowned_count().await?;
        let people = u64::from(of(&ContactKind::ALL));
        Ok(ContactsOverview {
            total: of(&ContactKind::ALL),
            saved,
            tabs: ContactTab::ALL
                .iter()
                .map(|tab| (*tab, of(tab.kinds())))
                .collect(),
            review: ReviewSummary {
                duplicates,
                bounced: capped(
                    sql::scalar(
                        self.connection,
                        "SELECT count(*) FROM (SELECT 1 FROM addresses
                          WHERE bounced_at > ?1 AND contact_id IS NOT NULL LIMIT ?2)",
                        bind![
                            to_millis(Utc::now() - chrono::Duration::days(BOUNCE_RECENT_DAYS)),
                            i64::from(REVIEW_CAP)
                        ],
                    )
                    .await?,
                ),
                frequent_unsaved: capped(frequent),
            },
            building: (waiting > 0).then(|| {
                (
                    u32::try_from(people).unwrap_or(u32::MAX),
                    u32::try_from(people + waiting).unwrap_or(u32::MAX),
                )
            }),
        })
    }

    /// The SQL the review counts run that are not the suggestion read.
    pub fn explain_review() -> [&'static str; 1] {
        ["SELECT count(*) FROM (SELECT 1 FROM contacts
           WHERE saved_at IS NULL AND kind = 'person' AND written >= 3
             AND ignored_at IS NULL AND state = 'live' LIMIT ?1)"]
    }

    /// Everything a person's card shows, in a fixed number of statements over
    /// their addresses: the person and their groups, the stats, what is open
    /// between you, what they decided, and the files and issues of their
    /// recent messages. `None` for someone who is not there.
    pub async fn card(
        &self,
        person: ContactId,
        since_month: i64,
    ) -> Result<Option<postio_model::ContactCardRead>> {
        use postio_model::{CARD_RECENT, FileChip, IssueRead};
        let Some(detail) = self.detail(person).await? else {
            return Ok(None);
        };
        let stats = self.stats(person, since_month).await?;
        let open = self.open_items(person, CARD_RECENT).await?;
        let settings = super::ContactSettingsRepository::new(self.connection)
            .get(person)
            .await?;
        let default_arrival = self.default_arrival(person).await?;
        let owned: Vec<i64> = sql::all(
            self.connection,
            &Self::explain_stats(1)[0],
            [person.get()],
            |row| row.col(0),
        )
        .await?;
        let (mut files, mut issues) = (Vec::new(), Vec::new());
        if !owned.is_empty() {
            let queries = Self::explain_card_recent(owned.len());
            let mut bound: Vec<turso::Value> =
                owned.iter().map(|id| turso::Value::Integer(*id)).collect();
            bound.push(turso::Value::Integer(i64::from(CARD_RECENT)));
            files = sql::all_unbounded(self.connection, &queries[0], bound.clone(), |row| {
                Ok(FileChip {
                    name: row.col::<Option<String>>(0)?.unwrap_or_default(),
                    mime: row.col(1)?,
                })
            })
            .await?;
            issues = sql::all_unbounded(self.connection, &queries[1], bound, |row| {
                Ok(IssueRead {
                    message: postio_model::MessageId::new(row.col(0)?),
                    subject: row.col(1)?,
                    at: from_millis(row.col(2)?),
                    opened: row.col::<i64>(3)? != 0,
                })
            })
            .await?;
        }
        Ok(Some(postio_model::ContactCardRead {
            person: detail.person,
            groups: detail.groups,
            stats,
            open,
            settings,
            default_arrival,
            files,
            issues,
            address_notes: self.address_notes(person).await?,
        }))
    }

    /// The SQL the card's recent files and issues run for a person with
    /// `addresses` addresses: both walk the ring's newest messages.
    pub fn explain_card_recent(addresses: usize) -> [String; 2] {
        let ids = (1..=addresses.max(1))
            .map(|n| format!("?{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let limit = addresses.max(1) + 1;
        [
            format!(
                "SELECT t.filename, t.mime_type FROM address_recent r
                   JOIN attachments t ON t.message_id = r.message_id
                                     AND t.disposition = 'attachment'
                  WHERE r.address_id IN ({ids})
                  ORDER BY r.at DESC, r.message_id DESC LIMIT ?{limit}"
            ),
            format!(
                "SELECT r.message_id, coalesce(m.subject, '(no subject)'), r.at, m.seen
                   FROM address_recent r
                   JOIN messages m ON m.id = r.message_id
                  WHERE r.address_id IN ({ids}) AND r.direction = 'in'
                  ORDER BY r.at DESC, r.message_id DESC LIMIT ?{limit}"
            ),
        ]
    }
}

/// Where the next page of a Contacts view starts: just past this row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCursor {
    sort_key: String,
    id: ContactId,
}

impl ContactCursor {
    /// The cursor just past the row with this sort key and id.
    pub fn from_parts(sort_key: String, id: ContactId) -> Self {
        Self { sort_key, id }
    }

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
        self.filtered_in(view, &ContactKind::ALL, text, cap).await
    }

    async fn filtered_in(
        &self,
        view: ContactView,
        kinds: &[ContactKind],
        text: &str,
        cap: u32,
    ) -> Result<Vec<ContactListRow>> {
        let words: Vec<String> = words(text).collect();
        if words.is_empty() {
            return if kinds.len() == ContactKind::ALL.len() {
                self.page(view, None, cap).await
            } else {
                self.page_kinds(kinds, None, 0, cap).await
            };
        }
        let kind_clause = if kinds.len() == ContactKind::ALL.len() {
            String::new()
        } else {
            format!(
                " AND c.kind IN ({})",
                kinds
                    .iter()
                    .map(|kind| format!("'{}'", kind.as_str()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
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
                   {LIST_JOINS}
                  WHERE {} AND {}{kind_clause}
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
       c.last_seen_at, c.source, c.state, c.sort_key,
       c.kind, c.times_seen, c.saved_at IS NOT NULL,
       cs.arrival, coalesce(cs.notify, 0), cs.html_treatment, cs.digest_cadence";

/// What a list row joins to: the preferred address and what the user decided.
const LIST_JOINS: &str = "LEFT JOIN addresses pa ON pa.id = c.preferred_address
           LEFT JOIN contact_settings cs ON cs.contact_id = c.id";

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
           {LIST_JOINS}
          WHERE {}
          ORDER BY c.sort_key, c.id LIMIT ?1 OFFSET ?2",
        view_predicate(view)
    )
}

fn next_page_sql(view: ContactView) -> String {
    format!(
        "SELECT {LIST_COLUMNS} FROM contacts c
           {LIST_JOINS}
          WHERE {} AND c.sort_key >= ?1 AND (c.sort_key > ?1 OR c.id > ?2)
          ORDER BY c.sort_key, c.id LIMIT ?3 OFFSET ?4",
        view_predicate(view)
    )
}

/// The pairs a suggestion read considers: live people who share a name key,
/// and people whose addresses answered one another -- minus any pair a
/// dismissed address pair spans. `replies` is NULL for a name match.
///
/// Every step is an index seek on `contacts` and `addresses`; the one walk is
/// over the reply candidates, which are the evidence and are read whole.
/// `CROSS JOIN` pins that order: left to itself the planner walked
/// `addresses` and probed the candidates, which is the wrong way round for a
/// table that is every address the user has ever seen.
const SUGGESTION_PAIRS: &str = "
    SELECT a.id, b.id, NULL FROM contacts a
      JOIN contacts b ON b.state = 'live' AND b.name_key = a.name_key AND b.id > a.id
     WHERE a.state = 'live' AND a.name_key <> ''
       AND NOT EXISTS (
           SELECT 1 FROM addresses nx
             JOIN addresses ny ON ny.contact_id = b.id
             JOIN contact_join_dismissals d
               ON d.address_low = min(nx.id, ny.id) AND d.address_high = max(nx.id, ny.id)
            WHERE nx.contact_id = a.id)
    UNION ALL
    SELECT min(x.contact_id, y.contact_id), max(x.contact_id, y.contact_id), cand.replies
      FROM contact_join_candidates cand
      CROSS JOIN addresses x ON x.id = cand.address_low
      CROSS JOIN addresses y ON y.id = cand.address_high
      CROSS JOIN contacts px ON px.id = x.contact_id AND px.state = 'live'
      CROSS JOIN contacts py ON py.id = y.contact_id AND py.state = 'live'
     WHERE x.contact_id <> y.contact_id
       AND NOT EXISTS (
           SELECT 1 FROM addresses u
             JOIN addresses v ON v.contact_id = y.contact_id
             JOIN contact_join_dismissals d
               ON d.address_low = min(u.id, v.id) AND d.address_high = max(u.id, v.id)
            WHERE u.contact_id = x.contact_id)
    LIMIT ?1";

/// The suggestion read's statements, for the budget that asks the planner.
#[cfg(feature = "test-support")]
pub(crate) fn suggestion_statements() -> Vec<String> {
    vec![SUGGESTION_PAIRS.replace("?1", "50")]
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
        ("kind first", kind_first_sql()),
        ("kind after", kind_next_sql()),
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
        kind: {
            let kind: String = row.col(8)?;
            ContactKind::from_name(&kind).ok_or_else(|| Error::UnknownEnum {
                column: "contacts.kind",
                value: kind.clone(),
            })?
        },
        messages: u32::try_from(row.col::<i64>(9)?.max(0)).unwrap_or(u32::MAX),
        saved: row.col::<i64>(10)? != 0,
        arrival: row
            .col::<Option<String>>(11)?
            .and_then(|name| postio_model::ContactArrival::from_name(&name)),
        notify: row.col::<i64>(12)? != 0,
        html_original: row.col::<Option<String>>(13)?.as_deref() == Some("paper"),
        digest_cadence: row.col(14)?,
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
            let prior_settings = read_settings(&transaction, into.get()).await?;
            let mut settings = prior_settings.clone();
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
                settings.absorb(&read_settings(&transaction, other.get()).await?);
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
            if settings != prior_settings {
                write_settings(&transaction, into.get(), &settings).await?;
            }
            Ok(JoinReceipt {
                into,
                restore,
                prior: survivor,
                added_memberships: added,
                prior_settings: Box::new(prior_settings),
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
            write_settings(&transaction, receipt.into.get(), &receipt.prior_settings).await?;
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
    pub async fn add_address(
        &self,
        person: ContactId,
        address: &EmailAddress,
    ) -> Result<AddressId> {
        sql::in_scope(self.connection, |transaction| async move {
            refuse_own(&transaction, address).await?;
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
                "SELECT a.address_normalized, {shown}
                   FROM addresses a
                   JOIN contacts c ON c.id = a.contact_id
                  WHERE a.address_normalized IN ({placeholders})
                    AND c.state = 'live' AND {shown} IS NOT NULL",
                shown = crate::shown_name!("c")
            ),
            wanted,
            |row| Ok((row.col::<String>(0)?, row.col::<String>(1)?)),
        )
        .await?;
        Ok(rows.into_iter().collect())
    }

    /// Sets a person's name, organisation and note, making them the user's
    /// if they were only the mail's (FR-021, FR-022). Hands back the fields
    /// it replaced, which [`put_fields`](Self::put_fields) restores.
    pub async fn edit(&self, person: ContactId, edit: &PersonEdit) -> Result<PersonFields> {
        let filled = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned)
        };
        let (name, organization, note) = (
            filled(&edit.name),
            filled(&edit.organization),
            filled(&edit.note),
        );
        sql::in_scope(self.connection, |transaction| async move {
            let prior = fields(&transaction, person).await?;
            sql::execute(
                &transaction,
                "UPDATE contacts SET name = ?2, organization = ?3, note = ?4, updated_at = ?5
                  WHERE id = ?1",
                bind![
                    person.get(),
                    name,
                    organization,
                    note,
                    to_millis(Utc::now())
                ],
            )
            .await?;
            promote(&transaction, person).await?;
            refresh(&transaction, person.get()).await?;
            Ok(prior)
        })
        .await
    }

    /// Puts a person's fields back exactly as `fields` has them -- source
    /// and preferred address included. What undoes an edit.
    pub async fn put_fields(&self, person: ContactId, fields: &PersonFields) -> Result<()> {
        let fields = fields.clone();
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "UPDATE contacts
                    SET name = ?2, organization = ?3, note = ?4, source = ?5,
                        preferred_address = ?6, updated_at = ?7
                  WHERE id = ?1",
                bind![
                    person.get(),
                    fields.name,
                    fields.organization,
                    fields.note,
                    fields.source.as_str(),
                    fields.preferred.get(),
                    to_millis(Utc::now())
                ],
            )
            .await?;
            // `refresh` keeps the preferred address only if it is still one
            // of theirs, which is the guard a stale inverse needs.
            refresh(&transaction, person.get()).await
        })
        .await
    }

    /// Deletes a person: offered nowhere, listed only in the Deleted view,
    /// still counting their addresses' mail (FR-023, SC-005). Hands back the
    /// state they were in.
    pub async fn delete(&self, person: ContactId) -> Result<ContactState> {
        sql::in_scope(self.connection, |transaction| async move {
            let state = state_of(&transaction, person).await?;
            if state != ContactState::Live {
                return Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "only someone in the address book can be deleted".into(),
                });
            }
            sql::execute(
                &transaction,
                "UPDATE contacts SET state = 'deleted', updated_at = ?2 WHERE id = ?1",
                bind![person.get(), to_millis(Utc::now())],
            )
            .await?;
            Ok(state)
        })
        .await
    }

    /// Brings a deleted person back in `state` -- what they were before.
    pub async fn restore(&self, person: ContactId, state: ContactState) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            if state_of(&transaction, person).await? != ContactState::Deleted {
                return Err(Error::ForbiddenTransition {
                    what: "contact",
                    reason: "only a deleted contact can be restored".into(),
                });
            }
            sql::execute(
                &transaction,
                "UPDATE contacts SET state = ?2, merged_into = NULL, updated_at = ?3
                  WHERE id = ?1",
                bind![person.get(), state.as_str(), to_millis(Utc::now())],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// A group's live members as list rows, by name, at most `cap` -- what
    /// the list shows while the cursor is on the group (FR-040).
    pub async fn group_rows(
        &self,
        group: postio_model::ContactGroupId,
        cap: u32,
    ) -> Result<Vec<ContactListRow>> {
        sql::all_unbounded(
            self.connection,
            &format!(
                "SELECT {LIST_COLUMNS} FROM contact_group_members m
                   JOIN contacts c ON c.id = m.contact_id
                   {LIST_JOINS}
                  WHERE m.group_id = ?1 AND c.state = 'live'
                  ORDER BY c.sort_key, c.id LIMIT ?2"
            ),
            bind![group.get(), i64::from(cap)],
            read_list_row,
        )
        .await
    }

    /// Who might be the same person, with the evidence (FR-018, R11): at
    /// most `limit` pairs, an answered-from-another-address pair before a
    /// name match. Reads only; a suggestion joins nobody (FR-019).
    pub async fn suggestions(&self, limit: u32) -> Result<Vec<JoinSuggestion>> {
        let pairs: Vec<(i64, i64, Option<i64>)> = sql::all_unbounded(
            self.connection,
            SUGGESTION_PAIRS,
            [i64::from(limit) * 2],
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
        )
        .await?;
        // A pair can be both a name match and an answer; the answer is the
        // stronger evidence, so it is the one kept.
        let mut kept: Vec<(i64, i64, SuggestionReason)> = Vec::new();
        for (a, b, replies) in pairs {
            let reason = match replies {
                Some(replies) => SuggestionReason::Replied {
                    replies: u32::try_from(replies).unwrap_or(u32::MAX),
                },
                None => SuggestionReason::SameName,
            };
            match kept.iter_mut().find(|(x, y, _)| (*x, *y) == (a, b)) {
                Some(existing) if matches!(reason, SuggestionReason::Replied { .. }) => {
                    existing.2 = reason;
                }
                Some(_) => {}
                None => kept.push((a, b, reason)),
            }
        }
        kept.sort_by_key(|(a, b, reason)| {
            let replies = match reason {
                SuggestionReason::Replied { replies } => *replies,
                SuggestionReason::SameName => 0,
            };
            (std::cmp::Reverse(replies), *a, *b)
        });
        kept.truncate(limit as usize);
        let mut ids: Vec<i64> = kept.iter().flat_map(|(a, b, _)| [*a, *b]).collect();
        ids.sort_unstable();
        ids.dedup();
        let people = self.people_by_ids(&ids).await?;
        let find = |id: i64| people.iter().find(|p| p.id.get() == id).cloned();
        Ok(kept
            .into_iter()
            .filter_map(|(a, b, reason)| {
                Some(JoinSuggestion {
                    people: [find(a)?, find(b)?],
                    reason,
                })
            })
            .collect())
    }

    /// People by id, with their addresses: two statements whatever the
    /// number.
    async fn people_by_ids(&self, ids: &[i64]) -> Result<Vec<Contact>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let people = sql::all_unbounded(
            self.connection,
            &format!("SELECT {PERSON_COLUMNS} FROM contacts WHERE id IN ({placeholders})"),
            ids.iter()
                .map(|id| turso::Value::Integer(*id))
                .collect::<Vec<_>>(),
            read_person,
        )
        .await?;
        self.with_addresses(people).await
    }

    /// The user says `a` and `b` are not the same person: never offered
    /// together again. Kept as the pair of their preferred addresses, which
    /// is what lets it outlive a join on either side (R11).
    pub async fn dismiss(&self, a: ContactId, b: ContactId) -> Result<()> {
        sql::in_scope(self.connection, |transaction| async move {
            let pa = fields(&transaction, a).await?.preferred.get();
            let pb = fields(&transaction, b).await?.preferred.get();
            sql::execute(
                &transaction,
                "INSERT INTO contact_join_dismissals (address_low, address_high)
                 VALUES (?1, ?2) ON CONFLICT DO NOTHING",
                bind![pa.min(pb), pa.max(pb)],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// Mail the user sent to `asked` was answered from `answered`: evidence
    /// that the two addresses may be one person (R11). Nothing when either is
    /// unknown or one person already owns both.
    pub async fn note_reply(&self, asked: &str, answered: &str) -> Result<()> {
        let (asked, answered) = (asked.to_lowercase(), answered.to_lowercase());
        if asked == answered {
            return Ok(());
        }
        sql::in_scope(self.connection, |transaction| async move {
            let lookup = async |address: &str| -> Result<Option<(i64, Option<i64>)>> {
                sql::first(
                    &transaction,
                    "SELECT id, contact_id FROM addresses WHERE address_normalized = ?1",
                    [address],
                    |row| Ok((row.col(0)?, row.col(1)?)),
                )
                .await
            };
            let (Some((x, px)), Some((y, py))) = (lookup(&asked).await?, lookup(&answered).await?)
            else {
                return Ok(());
            };
            if px.is_some() && px == py {
                return Ok(());
            }
            sql::execute(
                &transaction,
                "INSERT INTO contact_join_candidates (address_low, address_high, replies)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT (address_low, address_high) DO UPDATE SET replies = replies + 1",
                bind![x.min(y), x.max(y)],
            )
            .await?;
            Ok(())
        })
        .await
    }

    /// If `message` answers mail the user sent to exactly one address, and
    /// answers it from a different one, notes the pair (R11). One read, and
    /// only for a reply that is not the user's own.
    pub async fn record_reply(&self, message: &Message, own: &[EmailAddress]) -> Result<()> {
        let (Some(parent), Some(from)) = (&message.in_reply_to, message.from.first()) else {
            return Ok(());
        };
        let is_own = |address: &str| own.iter().any(|o| o.normalized() == address);
        let answered = from.normalized();
        if is_own(&answered) {
            return Ok(());
        }
        let rows: Vec<(String, String)> = sql::all(
            self.connection,
            "SELECT r.kind, a.address_normalized
               FROM messages p
               JOIN recipients r ON r.message_id = p.id
               JOIN addresses a ON a.id = r.address_id
              WHERE p.account_id = ?1 AND p.rfc_message_id = ?2
                AND r.kind IN ('from', 'to', 'cc')
              LIMIT 64",
            bind![message.account_id.get(), parent.as_str()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        let sent_by_user = rows
            .iter()
            .any(|(kind, address)| kind == "from" && is_own(address));
        let asked: Vec<&String> = rows
            .iter()
            .filter(|(kind, address)| kind != "from" && !is_own(address))
            .map(|(_, address)| address)
            .collect();
        match asked.as_slice() {
            [asked] if sent_by_user && **asked != answered => {
                self.note_reply(asked, &answered).await
            }
            _ => Ok(()),
        }
    }

    /// Applies a vCard import in one transaction (FR-053, R9): people first,
    /// so a group card can name them by `UID`, then groups.
    ///
    /// A card whose addresses nobody owns is a new person. A card whose
    /// addresses one person owns is that person, and takes the card's name
    /// unless the user named them -- which the summary counts. A card whose
    /// addresses several people own joins them, and the summary lists the
    /// join. Every card is kept whole in `contacts.vcard` for export.
    pub async fn apply_import(&self, cards: &[ParsedCard]) -> Result<ImportSummary> {
        let cards = cards.to_vec();
        sql::in_scope(self.connection, |transaction| async move {
            let contacts = ContactRepository::new(&transaction);
            let mut summary = ImportSummary::default();
            let mut by_uid: BTreeMap<String, ContactId> = BTreeMap::new();
            for card in &cards {
                if let ParsedCard::Person(person) = card {
                    let id = contacts.import_person(person, &mut summary).await?;
                    if let Some(uid) = &person.uid {
                        by_uid.insert(uid.clone(), id);
                    }
                }
            }
            for card in &cards {
                if let ParsedCard::Group(group) = card {
                    import_group(&transaction, group, &by_uid, &mut summary).await?;
                }
            }
            Ok(summary)
        })
        .await
    }

    async fn import_person(
        &self,
        card: &ParsedPerson,
        summary: &mut ImportSummary,
    ) -> Result<ContactId> {
        let mut owners: Vec<Contact> = Vec::new();
        for (address, _) in &card.emails {
            if let Some(owner) = self.by_address(&address.address).await?
                && owner.state == ContactState::Live
                && !owners.iter().any(|o| o.id == owner.id)
            {
                owners.push(owner);
            }
        }
        let addresses: Vec<EmailAddress> = card.emails.iter().map(|(a, _)| a.clone()).collect();
        let card_name = card
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let person = match owners.as_slice() {
            [] => {
                summary.people_created += 1;
                let made = self.create(card_name, &addresses).await?;
                summary.made.push(made);
                made
            }
            [first, rest @ ..] => {
                summary.people_updated += 1;
                // The user's own name for any of them wins over the card's.
                let chosen = owners.iter().find_map(|o| o.name.clone());
                if let (Some(chosen), Some(card_name)) = (&chosen, card_name)
                    && chosen != card_name
                {
                    summary.name_conflicts += 1;
                }
                if !rest.is_empty() {
                    // The card lists the addresses of several people you
                    // have. An import never merges people behind your back:
                    // the card is the evidence the review shows.
                    summary.offered_merges += 1;
                    let key = card.uid.clone().unwrap_or_else(|| card.raw.clone());
                    for owner in &owners {
                        self.note_evidence(owner.id, postio_model::EvidenceKind::Card, &key)
                            .await?;
                    }
                } else if chosen.is_none()
                    && let Some(card_name) = card_name
                {
                    sql::execute(
                        self.connection,
                        "UPDATE contacts SET name = ?2 WHERE id = ?1",
                        bind![first.id.get(), card_name],
                    )
                    .await?;
                }
                for address in &addresses {
                    // An address another person has stays theirs: moving it
                    // is the merge the user is offered, not the import's.
                    if let Some(owner) = self.by_address(&address.address).await?
                        && owner.state == ContactState::Live
                        && owner.id != first.id
                    {
                        continue;
                    }
                    self.add_address(first.id, address).await?;
                }
                first.id
            }
        };
        summary.touched.push(person);
        for owner in &owners {
            if !summary.touched.contains(&owner.id) {
                summary.touched.push(owner.id);
            }
        }
        // Which of the card's addresses it prefers.
        if let Some((preferred, _)) = card.emails.iter().find(|(_, p)| *p)
            && let Some(owned) = self.get(person).await?.and_then(|p| {
                p.addresses
                    .iter()
                    .find(|a| a.address.address.eq_ignore_ascii_case(&preferred.address))
                    .map(|a| a.id)
            })
        {
            self.set_preferred(person, owned).await?;
        }
        sql::execute(
            self.connection,
            "UPDATE contacts
                SET source = CASE WHEN source = 'mail' OR ?6 THEN 'import' ELSE source END,
                    organization = coalesce(organization, ?2),
                    note = coalesce(note, ?3),
                    uid = coalesce(?4, uid), vcard = ?5
              WHERE id = ?1",
            bind![
                person.get(),
                card.organization.clone(),
                card.note.clone(),
                card.uid.clone(),
                card.raw.clone(),
                owners.is_empty()
            ],
        )
        .await?;
        refresh(self.connection, person.get()).await?;
        // What the card says, noted as the body stage notes a sender, so the
        // review finds a second record with the same phone or signature.
        for phone in &card.phones {
            self.note_evidence(person, postio_model::EvidenceKind::Phone, phone)
                .await?;
        }
        let signature = postio_model::SignatureDetails {
            name: card_name.map(str::to_owned),
            title: card.title.clone(),
            company: card.organization.clone(),
            ..postio_model::SignatureDetails::default()
        };
        if let Some(key) = signature.key() {
            self.note_evidence(person, postio_model::EvidenceKind::Signature, &key)
                .await?;
        }
        // A new person who has the name of someone you already have is
        // counted, and the review offers the merge.
        if owners.is_empty() {
            let same = sql::scalar(
                self.connection,
                "SELECT count(*) FROM contacts o JOIN contacts n ON n.id = ?1
                  WHERE o.id <> n.id AND o.state = 'live' AND n.name_key <> ''
                    AND o.name_key = n.name_key",
                [person.get()],
            )
            .await?;
            if same > 0 {
                summary.look_alikes += 1;
            }
        }
        Ok(person)
    }

    /// A person's vCard identity and stored card, if any.
    pub async fn card_of(&self, person: ContactId) -> Result<(Option<String>, Option<String>)> {
        let found: Option<(Option<String>, Option<String>)> = sql::first(
            self.connection,
            "SELECT uid, vcard FROM contacts WHERE id = ?1",
            [person.get()],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        found.ok_or(Error::NotFound {
            entity: "contact",
            id: person.get(),
        })
    }

    /// Every live person in `view`, by id, in list order -- what export
    /// writes when nothing is selected and nothing filtered (FR-050).
    pub async fn view_ids(&self, view: ContactView) -> Result<Vec<ContactId>> {
        sql::all_unbounded(
            self.connection,
            &format!(
                "SELECT c.id FROM contacts c WHERE {} ORDER BY c.sort_key, c.id",
                view_predicate(view)
            ),
            (),
            |row| Ok(ContactId::new(row.col(0)?)),
        )
        .await
    }

    /// What export writes for each of `ids` that is live: the person, their
    /// `UID` -- given one now, and kept, if they had none, so a second export
    /// names them the same -- and their stored card.
    pub async fn export_people(&self, ids: &[ContactId]) -> Result<Vec<ExportRow>> {
        let ids = ids.to_vec();
        sql::in_scope(self.connection, |transaction| async move {
            let contacts = ContactRepository::new(&transaction);
            let raw: Vec<i64> = ids.iter().map(|id| id.get()).collect();
            let people = contacts.people_by_ids(&raw).await?;
            let mut rows = Vec::with_capacity(people.len());
            for id in ids {
                let Some(person) = people.iter().find(|p| p.id == id) else {
                    continue;
                };
                if person.state != ContactState::Live {
                    continue;
                }
                let (uid, vcard) = contacts.card_of(id).await?;
                let uid = match uid {
                    Some(uid) => uid,
                    None => {
                        let uid = new_uid();
                        sql::execute(
                            &transaction,
                            "UPDATE contacts SET uid = ?2 WHERE id = ?1",
                            bind![id.get(), uid.clone()],
                        )
                        .await?;
                        uid
                    }
                };
                rows.push(ExportRow {
                    person: person.clone(),
                    uid,
                    vcard,
                });
            }
            Ok(rows)
        })
        .await
    }

    /// Every group with a member among `rows`, as export writes it: a `UID`
    /// -- given now and kept, as for people -- its name, its stored card,
    /// and the `UID`s of the members being exported with it. A group whose
    /// members are all left out of this export is left out too.
    pub async fn export_groups(&self, rows: &[ExportRow]) -> Result<Vec<GroupExport>> {
        let by_person: BTreeMap<ContactId, String> = rows
            .iter()
            .map(|row| (row.person.id, row.uid.clone()))
            .collect();
        sql::in_scope(self.connection, |transaction| async move {
            let groups = super::ContactGroupRepository::new(&transaction);
            let mut out = Vec::new();
            for group in groups.list().await? {
                let members: Vec<String> = groups
                    .members(group.id)
                    .await?
                    .iter()
                    .filter_map(|person| by_person.get(&person.id).cloned())
                    .collect();
                if members.is_empty() {
                    continue;
                }
                let vcard: Option<String> = sql::first(
                    &transaction,
                    "SELECT vcard FROM contact_groups WHERE id = ?1",
                    [group.id.get()],
                    |row| row.col(0),
                )
                .await?
                .flatten();
                let uid = match group.uid.clone() {
                    Some(uid) => uid,
                    None => {
                        let uid = new_uid();
                        sql::execute(
                            &transaction,
                            "UPDATE contact_groups SET uid = ?2 WHERE id = ?1",
                            bind![group.id.get(), uid.clone()],
                        )
                        .await?;
                        uid
                    }
                };
                out.push(GroupExport {
                    uid,
                    name: group.name,
                    members,
                    vcard,
                });
            }
            Ok(out)
        })
        .await
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
                Some(previous) => {
                    settle_after_losing_addresses(&transaction, previous, None).await?
                }
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

/// One person as export writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRow {
    /// The person, with their addresses, preferred first.
    pub person: Contact,
    /// Their vCard `UID`.
    pub uid: String,
    /// The card they were imported from, if they were.
    pub vcard: Option<String>,
}

/// One group as export writes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupExport {
    /// Its vCard `UID`.
    pub uid: String,
    /// Its name.
    pub name: String,
    /// The `UID`s of its members in this export.
    pub members: Vec<String>,
    /// The card it was imported from, if it was.
    pub vcard: Option<String>,
}

/// A fresh vCard `UID`: a version 4 UUID from the standard library's
/// per-process random hash keys, so no generator crate is needed for one
/// identifier per exported person.
fn new_uid() -> String {
    use std::hash::{BuildHasher, Hasher};
    let half = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        hasher.finish()
    };
    let bytes = (u128::from(half()) << 64 | u128::from(half())).to_be_bytes();
    let mut b = bytes;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "urn:uuid:{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7],
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}

/// Makes a group card's group, or finds the one already called that, and
/// puts its members in it: by `UID` among the cards just imported or the
/// people already here, or by address.
async fn import_group(
    connection: &Connection,
    card: &ParsedGroup,
    by_uid: &BTreeMap<String, ContactId>,
    summary: &mut ImportSummary,
) -> Result<()> {
    let groups = super::ContactGroupRepository::new(connection);
    let existing = groups
        .list()
        .await?
        .into_iter()
        .find(|g| g.name.eq_ignore_ascii_case(card.name.trim()));
    let group = match existing {
        Some(group) => group.id,
        None => {
            summary.groups_created += 1;
            let mut group = postio_model::ContactGroup::new(card.name.trim(), Utc::now());
            group.uid = card.uid.clone();
            groups.create(&mut group).await?
        }
    };
    sql::execute(
        connection,
        "UPDATE contact_groups SET uid = coalesce(uid, ?2), vcard = ?3 WHERE id = ?1",
        bind![group.get(), card.uid.clone(), card.raw.clone()],
    )
    .await?;
    let contacts = ContactRepository::new(connection);
    let mut members = Vec::new();
    for member in &card.members {
        let found = match member {
            Member::Uid(uid) => match by_uid.get(uid) {
                Some(id) => Some(*id),
                None => {
                    sql::first(
                        connection,
                        "SELECT id FROM contacts WHERE uid = ?1 AND state = 'live'",
                        [uid.as_str()],
                        |row| Ok(ContactId::new(row.col(0)?)),
                    )
                    .await?
                }
            },
            Member::Address(address) => contacts
                .by_address(address)
                .await?
                .filter(|p| p.state == ContactState::Live)
                .map(|p| p.id),
        };
        members.extend(found);
    }
    groups.add_members(group, &members).await?;
    Ok(())
}

/// A person's state.
async fn state_of(connection: &Connection, person: ContactId) -> Result<ContactState> {
    let state: Option<String> = sql::first(
        connection,
        "SELECT state FROM contacts WHERE id = ?1",
        [person.get()],
        |row| row.col(0),
    )
    .await?;
    let state = state.ok_or(Error::NotFound {
        entity: "contact",
        id: person.get(),
    })?;
    ContactState::from_name(&state).ok_or(Error::UnknownEnum {
        column: "contacts.state",
        value: state,
    })
}

/// Refuses one of the user's own addresses -- an account's or an
/// identity's. The user is never their own contact (spec edge case), and
/// saying so beats a person who silently never appears.
async fn refuse_own(connection: &Connection, address: &EmailAddress) -> Result<()> {
    let own = sql::scalar(
        connection,
        "SELECT (SELECT count(*) FROM accounts WHERE lower(address) = ?1)
              + (SELECT count(*) FROM identities WHERE lower(address) = ?1)",
        bind![address.normalized()],
    )
    .await?;
    if own > 0 {
        return Err(Error::ForbiddenTransition {
            what: "contact",
            reason: "that address is one of yours".into(),
        });
    }
    Ok(())
}

/// A person's editable fields, for a receipt or an inverse.
async fn fields(connection: &Connection, person: ContactId) -> Result<PersonFields> {
    type Row = (
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<i64>,
    );
    let row: Option<Row> = sql::first(
        connection,
        "SELECT name, organization, note, source, preferred_address FROM contacts WHERE id = ?1",
        [person.get()],
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
async fn record_in(connection: &Connection, now: &Recording<'_>, seen: &Seen<'_>) -> Result<()> {
    let Recording {
        account, at, own, ..
    } = *now;
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
    let existed = found.is_some();
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
             (address_id, account_id, times_seen, last_seen_at, last_name, times_written,
              seen_count)
         VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6)
         ON CONFLICT (address_id, account_id) DO UPDATE SET
             times_seen = times_seen + 1,
             seen_count = seen_count + excluded.seen_count,
             last_name = CASE WHEN excluded.last_seen_at >= coalesce(last_seen_at, 0)
                              THEN coalesce(excluded.last_name, last_name)
                              ELSE last_name END,
             last_seen_at = max(coalesce(last_seen_at, excluded.last_seen_at),
                                excluded.last_seen_at),
             times_written = times_written + excluded.times_written",
        bind![
            address_id,
            account,
            at,
            header_name,
            written,
            i64::from(seen.sender && now.read)
        ],
    )
    .await?;
    note_month_and_ring(connection, address_id, now, seen).await?;

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
            // An address that had mail before contacts began has history
            // nothing recorded; add it to this first sighting.
            if existed && history_through(connection).await? > 0 {
                fold_sightings(connection, address_id, Some(own), true).await?;
                refresh(connection, person).await?;
            }
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

/// What one batch of the catch-up did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CatchUp {
    /// People made.
    pub made: usize,
    /// The lowest address this batch reached: pass it as `before` for the
    /// next. `None` when the walk found nothing.
    pub last: Option<AddressId>,
    /// Whether the walk ran out before the batch filled.
    pub done: bool,
}

/// The person owning `address`, made from its stored mail if there is none;
/// `None` for the user's own address.
async fn adopt(connection: &Connection, address: i64) -> Result<Option<i64>> {
    type Found = (Option<i64>, String, i64);
    let found: Option<Found> = sql::first(
        connection,
        "SELECT contact_id, address, (SELECT count(*) FROM accounts WHERE lower(address) = a.address_normalized)
                 + (SELECT count(*) FROM identities WHERE lower(address) = a.address_normalized)
           FROM addresses a WHERE id = ?1",
        [address],
        |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
    )
    .await?;
    let Some((owner, _text, own)) = found else {
        return Err(Error::NotFound {
            entity: "address",
            id: address,
        });
    };
    if let Some(owner) = owner {
        return Ok(Some(owner));
    }
    if own > 0 {
        return Ok(None);
    }

    let now = to_millis(Utc::now());
    sql::execute(
        connection,
        "INSERT INTO contacts (source, state, preferred_address, sort_key, name_key,
                               created_at, updated_at)
         VALUES ('mail', 'live', ?1, '', '', ?2, ?2)",
        bind![address, now],
    )
    .await?;
    let person = connection.last_insert_rowid();
    sql::execute(
        connection,
        "UPDATE addresses SET contact_id = ?2 WHERE id = ?1",
        bind![address, person],
    )
    .await?;
    fold_sightings(connection, address, None, false).await?;
    refresh(connection, person).await?;
    Ok(Some(person))
}

/// Sets the address's sightings to what its stored mail says, folded per
/// account in one statement: how many messages, the newest, how many the
/// user sent to it, and the newest name a header gave it. `own` is who
/// counts as the user -- a caller that was told (sync) says so; `None`
/// reads the accounts' and identities' addresses from the store.
///
/// Only mail stored before contacts began ([`history_through`]) is folded: a
/// message stored since is recorded by the pass that stored it, and counting
/// it here too would count it twice. `add` adds to the sightings already
/// there (an address recorded since), where otherwise they are set.
async fn fold_sightings(
    connection: &Connection,
    address: i64,
    own: Option<&[String]>,
    add: bool,
) -> Result<()> {
    let through = history_through(connection).await?;
    if through == 0 {
        return Ok(());
    }
    let mine = match own {
        Some(list) => {
            let placeholders: Vec<String> =
                (0..list.len()).map(|n| format!("?{}", n + 2)).collect();
            format!(
                "fa.address_normalized IN ({})",
                if placeholders.is_empty() {
                    "''".to_owned()
                } else {
                    placeholders.join(", ")
                }
            )
        }
        None => "(fa.address_normalized IN (SELECT lower(address) FROM accounts)
                  OR fa.address_normalized IN (SELECT lower(address) FROM identities))"
            .to_owned(),
    };
    let through_at = own.map_or(0, <[String]>::len) + 2;
    let mut parameters: Vec<turso::Value> = vec![turso::Value::Integer(address)];
    parameters.extend(
        own.unwrap_or_default()
            .iter()
            .map(|one| turso::Value::Text(one.clone())),
    );
    parameters.push(turso::Value::Integer(through));
    type Sighting = (i64, i64, i64, i64, Option<String>, i64);
    let sightings: Vec<Sighting> = sql::all_unbounded(
        connection,
        &format!(
            "SELECT m.account_id,
                    count(DISTINCT m.id),
                    max(coalesce(m.date, m.received_at)),
                    count(DISTINCT CASE WHEN r.kind IN ('to', 'cc', 'bcc') AND EXISTS (
                            SELECT 1 FROM recipients f JOIN addresses fa ON fa.id = f.address_id
                             WHERE f.message_id = m.id AND f.kind = 'from' AND {mine})
                        THEN m.id END),
                    (SELECT r2.name FROM recipients r2 JOIN messages m2 ON m2.id = r2.message_id
                      WHERE r2.address_id = ?1 AND m2.account_id = m.account_id
                        AND m2.id <= ?{through_at}
                        AND r2.name IS NOT NULL AND trim(r2.name) <> ''
                      ORDER BY coalesce(m2.date, m2.received_at) DESC LIMIT 1),
                    count(DISTINCT CASE WHEN r.kind = 'from' AND m.seen <> 0 THEN m.id END)
               FROM recipients r JOIN messages m ON m.id = r.message_id
              WHERE r.address_id = ?1 AND m.id <= ?{through_at}
              GROUP BY m.account_id"
        ),
        parameters.clone(),
        |row| {
            Ok((
                row.col(0)?,
                row.col(1)?,
                row.col(2)?,
                row.col(3)?,
                row.col(4)?,
                row.col(5)?,
            ))
        },
    )
    .await?;
    for (account, seen, last, written, name, opened) in sightings {
        sql::execute(
            connection,
            "INSERT INTO contact_sightings
                 (address_id, account_id, times_seen, last_seen_at, last_name, times_written,
                  seen_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?8)
             ON CONFLICT (address_id, account_id) DO UPDATE SET
                 seen_count = CASE WHEN ?7 = 1 THEN seen_count + excluded.seen_count
                                   ELSE excluded.seen_count END,
                 times_seen = CASE WHEN ?7 = 1 THEN times_seen + excluded.times_seen
                                   ELSE excluded.times_seen END,
                 last_seen_at = max(coalesce(last_seen_at, excluded.last_seen_at),
                                    excluded.last_seen_at),
                 last_name = coalesce(CASE WHEN ?7 = 1 THEN last_name ELSE excluded.last_name END,
                                      excluded.last_name),
                 times_written = CASE WHEN ?7 = 1 THEN times_written + excluded.times_written
                                      ELSE excluded.times_written END",
            bind![
                address,
                account,
                seen,
                last,
                name,
                written,
                i64::from(add),
                opened
            ],
        )
        .await?;
    }
    fold_months_and_ring(connection, address, &mine, &parameters, through_at, add).await
}

/// The months and the ring of the mail stored before contacts began,
/// written the way recording would have.
async fn fold_months_and_ring(
    connection: &Connection,
    address: i64,
    mine: &str,
    parameters: &[turso::Value],
    through_at: usize,
    add: bool,
) -> Result<()> {
    let when = "coalesce(m.date, m.received_at)";
    sql::execute(
        connection,
        &format!(
            "INSERT INTO address_months (address_id, month, n)
             SELECT ?1,
                    CAST(strftime('%Y', {when} / 1000, 'unixepoch') AS INTEGER) * 12
                      + CAST(strftime('%m', {when} / 1000, 'unixepoch') AS INTEGER) - 1 AS month,
                    count(DISTINCT m.id)
               FROM recipients r JOIN messages m ON m.id = r.message_id
              WHERE r.address_id = ?1 AND m.id <= ?{through_at}
              GROUP BY month
             ON CONFLICT (address_id, month) DO UPDATE SET n = {n}",
            n = if add { "n + excluded.n" } else { "excluded.n" }
        ),
        parameters.to_vec(),
    )
    .await?;
    sql::execute(
        connection,
        &format!(
            "INSERT OR IGNORE INTO address_recent (address_id, message_id, at, direction)
             SELECT ?1, m.id, {when},
                    CASE WHEN max(r.kind IN ('to', 'cc', 'bcc') AND EXISTS (
                            SELECT 1 FROM recipients f JOIN addresses fa ON fa.id = f.address_id
                             WHERE f.message_id = m.id AND f.kind = 'from' AND {mine}))
                         THEN 'out' ELSE 'in' END
               FROM recipients r JOIN messages m ON m.id = r.message_id
              WHERE r.address_id = ?1 AND m.id <= ?{through_at}
              GROUP BY m.id
              ORDER BY {when} DESC, m.id DESC LIMIT {limit}",
            limit = postio_model::RECENT
        ),
        parameters.to_vec(),
    )
    .await?;
    trim_ring(connection, address).await
}

/// What `person` decided (see `contact_settings`), the default for no row.
async fn read_settings(
    connection: &Connection,
    person: i64,
) -> Result<postio_model::ContactSettings> {
    super::ContactSettingsRepository::new(connection)
        .get(ContactId::new(person))
        .await
}

/// Records what `person` decided; the default removes the row.
async fn write_settings(
    connection: &Connection,
    person: i64,
    settings: &postio_model::ContactSettings,
) -> Result<()> {
    super::ContactSettingsRepository::new(connection)
        .put(ContactId::new(person), settings)
        .await
}

/// Writes one sighting's month and ring entry: the strip and the newest
/// fifty a card reads. A message never stored has no ring entry.
async fn note_month_and_ring(
    connection: &Connection,
    address: i64,
    now: &Recording<'_>,
    seen: &Seen<'_>,
) -> Result<()> {
    let month = postio_model::month_key(from_millis(now.at));
    sql::execute(
        connection,
        "INSERT INTO address_months (address_id, month, n) VALUES (?1, ?2, 1)
         ON CONFLICT (address_id, month) DO UPDATE SET n = n + 1",
        bind![address, month],
    )
    .await?;
    if now.message == 0 {
        return Ok(());
    }
    sql::execute(
        connection,
        "INSERT OR IGNORE INTO address_recent (address_id, message_id, at, direction)
         VALUES (?1, ?2, ?3, ?4)",
        bind![
            address,
            now.message,
            now.at,
            if seen.written { "out" } else { "in" }
        ],
    )
    .await?;
    trim_ring(connection, address).await
}

/// Keeps an address's ring to its newest [`postio_model::RECENT`].
async fn trim_ring(connection: &Connection, address: i64) -> Result<()> {
    sql::execute(
        connection,
        "DELETE FROM address_recent
          WHERE address_id = ?1 AND message_id IN (
              SELECT message_id FROM address_recent WHERE address_id = ?1
               ORDER BY at DESC, message_id DESC LIMIT -1 OFFSET ?2)",
        bind![address, postio_model::RECENT as i64],
    )
    .await?;
    Ok(())
}

/// The newest message id the store held when contacts began, or 0 for a
/// store that began with them. Mail up to it was stored before anything
/// recorded who it was from; mail after it is recorded as it is stored.
async fn history_through(connection: &Connection) -> Result<i64> {
    let kept: Option<String> = sql::first(
        connection,
        "SELECT value FROM settings WHERE key = 'contacts.history_through' AND account_id IS NULL",
        (),
        |row| row.col(0),
    )
    .await?;
    Ok(kept.and_then(|text| text.trim().parse().ok()).unwrap_or(0))
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

/// What saving the Edit form did, and what puts it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedForm {
    /// Who was made or changed.
    pub person: ContactId,
    /// The form as it was before, for an existing person; `None` for one just
    /// made, whom undo deletes.
    pub prior: Option<postio_model::ContactForm>,
}

impl ContactRepository<'_> {
    /// The label and bounce mark of each of `person`'s addresses.
    pub async fn address_notes(&self, person: ContactId) -> Result<Vec<postio_model::AddressNote>> {
        sql::all(
            self.connection,
            "SELECT id, label, bounced_at FROM addresses WHERE contact_id = ?1 ORDER BY id",
            [person.get()],
            |row| {
                Ok(postio_model::AddressNote {
                    address: AddressId::new(row.col(0)?),
                    label: row
                        .col::<Option<String>>(1)?
                        .as_deref()
                        .and_then(postio_model::AddressLabel::from_name),
                    bounced: row.col::<Option<i64>>(2)?.is_some(),
                    bounced_at: row.col::<Option<i64>>(2)?.map(from_millis),
                })
            },
        )
        .await
    }

    /// The Edit form for `person`, as the card would fill it.
    pub async fn form_of(&self, person: ContactId) -> Result<Option<postio_model::ContactForm>> {
        let Some(detail) = self.detail(person).await? else {
            return Ok(None);
        };
        let notes = self.address_notes(person).await?;
        let card = postio_model::ContactCardRead {
            person: detail.person,
            groups: detail.groups,
            stats: postio_model::ContactStats::default(),
            open: Vec::new(),
            settings: postio_model::ContactSettings::default(),
            default_arrival: postio_model::ContactArrival::Inbox,
            files: Vec::new(),
            issues: Vec::new(),
            address_notes: notes.clone(),
        };
        Ok(Some(postio_model::ContactForm::of(&card, &notes)))
    }

    /// Makes a person from the Edit form, or changes `person` to be what the
    /// form says (specs/011-focus-contacts S8), and hands back what puts it
    /// back.
    ///
    /// Everything the form holds is checked before anything is written: an
    /// address a live person owns is refused whole, naming no half-saved
    /// state. Names, kind (the user's from now on), style, work, addresses,
    /// which one is primary, their labels and their groups all land; an
    /// address the form no longer lists is let go, never deleted from the mail.
    pub async fn save_form(
        &self,
        person: Option<ContactId>,
        form: &postio_model::ContactForm,
    ) -> Result<SavedForm> {
        let form = form.tidy();
        if let Some(problem) = form.problem() {
            return Err(Error::ForbiddenTransition {
                what: "contact",
                reason: problem.text(),
            });
        }
        let wanted: Vec<EmailAddress> = form
            .addresses
            .iter()
            .map(|row| EmailAddress::new(None::<String>, row.address.clone()))
            .collect();
        let prior = match person {
            Some(person) => Some(self.form_of(person).await?.ok_or(Error::NotFound {
                entity: "contact",
                id: person.get(),
            })?),
            None => None,
        };
        let keeps = |address: &EmailAddress| {
            prior.as_ref().is_some_and(|prior| {
                prior
                    .addresses
                    .iter()
                    .any(|old| old.address.eq_ignore_ascii_case(&address.address))
            })
        };
        // Every address the form adds must be one nobody live owns.
        for address in wanted.iter().filter(|address| !keeps(address)) {
            if let Some(owner) = self.by_address(&address.address).await?
                && owner.state == ContactState::Live
                && Some(owner.id) != person
            {
                return Err(Error::AddressOwned {
                    address: owner
                        .addresses
                        .iter()
                        .find(|owned| owned.address.same_address(address))
                        .map_or(0, |owned| owned.id.get()),
                    owner: owner.id.get(),
                });
            }
        }
        let id = match person {
            Some(person) => {
                let held = self.get(person).await?.ok_or(Error::NotFound {
                    entity: "contact",
                    id: person.get(),
                })?;
                for owned in &held.addresses {
                    if !wanted
                        .iter()
                        .any(|address| address.same_address(&owned.address))
                    {
                        self.release_address(owned.id).await?;
                    }
                }
                for address in &wanted {
                    if !held
                        .addresses
                        .iter()
                        .any(|owned| owned.address.same_address(address))
                    {
                        self.add_address(person, address).await?;
                    }
                }
                person
            }
            None => self.create(Some(&form.full_name()), &wanted).await?,
        };
        let styled = form.shown_as(form.show_as);
        let now = to_millis(Utc::now());
        let primary = wanted.get(form.primary).cloned();
        let labels: Vec<(String, Option<&'static str>)> = form
            .addresses
            .iter()
            .map(|row| {
                (
                    row.address.clone(),
                    row.label.map(postio_model::AddressLabel::as_str),
                )
            })
            .collect();
        sql::in_scope(self.connection, |transaction| async move {
            let filled = |text: &str| {
                let text = text.trim();
                (!text.is_empty()).then(|| text.to_owned())
            };
            sql::execute(
                &transaction,
                "UPDATE contacts
                    SET name = ?2, given_name = ?3, family_name = ?4, show_as = ?5,
                        title = ?6, organization = ?7, updated_at = ?8
                  WHERE id = ?1",
                bind![
                    id.get(),
                    filled(&styled),
                    filled(&form.given),
                    filled(&form.family),
                    form.show_as.as_str(),
                    filled(&form.title),
                    filled(&form.company),
                    now
                ],
            )
            .await?;
            // The user's word on the kind wins from here on.
            sql::execute(
                &transaction,
                "UPDATE contacts SET kind = ?2, kind_source = 'user'
                  WHERE id = ?1 AND (kind <> ?2 OR kind_source <> 'user')",
                bind![id.get(), form.kind.as_str()],
            )
            .await?;
            for (address, label) in &labels {
                sql::execute(
                    &transaction,
                    "UPDATE addresses SET label = ?3
                      WHERE contact_id = ?1 AND address_normalized = ?2",
                    bind![id.get(), address.to_lowercase(), *label],
                )
                .await?;
            }
            if let Some(primary) = &primary {
                sql::execute(
                    &transaction,
                    "UPDATE contacts SET preferred_address =
                        (SELECT id FROM addresses
                          WHERE contact_id = ?1 AND address_normalized = ?2)
                      WHERE id = ?1
                        AND EXISTS (SELECT 1 FROM addresses
                                     WHERE contact_id = ?1 AND address_normalized = ?2)",
                    bind![id.get(), primary.normalized()],
                )
                .await?;
            }
            promote(&transaction, id).await?;
            sql::execute(
                &transaction,
                "UPDATE contacts SET saved_at = coalesce(saved_at, ?2) WHERE id = ?1",
                bind![id.get(), now],
            )
            .await?;
            // Groups: by name, made when new, shared across people.
            let held: Vec<(i64, String)> = sql::all(
                &transaction,
                "SELECT g.id, g.name FROM contact_group_members m
                   JOIN contact_groups g ON g.id = m.group_id WHERE m.contact_id = ?1",
                [id.get()],
                |row| Ok((row.col(0)?, row.col(1)?)),
            )
            .await?;
            for (group, name) in &held {
                if !form
                    .groups
                    .iter()
                    .any(|wanted| wanted.eq_ignore_ascii_case(name))
                {
                    sql::execute(
                        &transaction,
                        "DELETE FROM contact_group_members WHERE group_id = ?1 AND contact_id = ?2",
                        bind![*group, id.get()],
                    )
                    .await?;
                }
            }
            for name in &form.groups {
                if held.iter().any(|(_, held)| held.eq_ignore_ascii_case(name)) {
                    continue;
                }
                let existing: Option<i64> = sql::first(
                    &transaction,
                    "SELECT id FROM contact_groups WHERE name = ?1 COLLATE NOCASE",
                    [name.clone()],
                    |row| row.col(0),
                )
                .await?;
                let group = match existing {
                    Some(group) => group,
                    None => {
                        sql::execute(
                            &transaction,
                            "INSERT INTO contact_groups (name, created_at) VALUES (?1, ?2)",
                            bind![name.clone(), now],
                        )
                        .await?;
                        transaction.last_insert_rowid()
                    }
                };
                sql::execute(
                    &transaction,
                    "INSERT OR IGNORE INTO contact_group_members (group_id, contact_id)
                     VALUES (?1, ?2)",
                    bind![group, id.get()],
                )
                .await?;
            }
            refresh(&transaction, id.get()).await
        })
        .await?;
        Ok(SavedForm { person: id, prior })
    }
}

impl ContactRepository<'_> {
    /// The phones and websites kept for `person`, or refused.
    pub async fn details(&self, person: ContactId) -> Result<Vec<postio_model::KeptDetail>> {
        sql::all(
            self.connection,
            "SELECT kind, value, state FROM contact_details WHERE contact_id = ?1 ORDER BY id",
            [person.get()],
            |row| {
                let kind: String = row.col(0)?;
                Ok(postio_model::KeptDetail {
                    kind: postio_model::DetailKind::from_name(&kind).ok_or_else(|| {
                        Error::UnknownEnum {
                            column: "contact_details.kind",
                            value: kind.clone(),
                        }
                    })?,
                    value: row.col(1)?,
                    added: row.col::<String>(2)? == "added",
                })
            },
        )
        .await
    }

    /// Keeps a detail (`Some(true)`), refuses it so it is not offered again
    /// (`Some(false)`), or forgets it (`None`).
    pub async fn set_detail(
        &self,
        person: ContactId,
        kind: postio_model::DetailKind,
        value: &str,
        keep: Option<bool>,
    ) -> Result<()> {
        match keep {
            None => {
                sql::execute(
                    self.connection,
                    "DELETE FROM contact_details WHERE contact_id = ?1 AND kind = ?2 AND value = ?3",
                    bind![person.get(), kind.as_str(), value],
                )
                .await?;
            }
            Some(added) => {
                sql::execute(
                    self.connection,
                    "INSERT INTO contact_details (contact_id, kind, value, state)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT (contact_id, kind, value) DO UPDATE SET state = excluded.state",
                    bind![
                        person.get(),
                        kind.as_str(),
                        value,
                        if added { "added" } else { "ignored" }
                    ],
                )
                .await?;
            }
        }
        Ok(())
    }

    /// For the new-sender popover: the newest message `person` sent, when it
    /// arrived, and the names of up to three saved people who were also on
    /// it. `None` when they have sent nothing that is still here.
    pub async fn introduction_facts(
        &self,
        person: &postio_model::Contact,
    ) -> Result<Option<(postio_model::MessageId, chrono::DateTime<Utc>, Vec<String>)>> {
        let ids: Vec<i64> = person.addresses.iter().map(|a| a.id.get()).collect();
        if ids.is_empty() {
            return Ok(None);
        }
        let marks = vec!["?"; ids.len()].join(", ");
        let newest: Option<(i64, i64)> = sql::first(
            self.connection,
            &format!(
                "SELECT m.id, m.received_at FROM recipients r
                   JOIN messages m ON m.id = r.message_id
                  WHERE r.kind = 'from' AND r.address_id IN ({marks}) AND m.deleted_locally = 0
                  ORDER BY m.received_at DESC, m.id DESC LIMIT 1"
            ),
            ids.iter()
                .map(|id| turso::Value::Integer(*id))
                .collect::<Vec<_>>(),
            |row| Ok((row.col::<i64>(0)?, row.col::<i64>(1)?)),
        )
        .await?;
        let Some((message, received)) = newest else {
            return Ok(None);
        };
        let with: Vec<String> = sql::all(
            self.connection,
            "SELECT DISTINCT coalesce(c.name, c.seen_name, a.address) FROM recipients r
               JOIN addresses a ON a.id = r.address_id
               JOIN contacts c ON c.id = a.contact_id AND c.state = 'live'
              WHERE r.message_id = ?1 AND r.kind IN ('to', 'cc')
                AND c.id <> ?2 AND (c.saved_at IS NOT NULL OR c.source <> 'mail')
              LIMIT 3",
            [message, person.id.get()],
            |row| row.col::<String>(0),
        )
        .await?;
        Ok(Some((
            postio_model::MessageId::new(message),
            from_millis(received),
            with,
        )))
    }

    /// "Add contact" in the new-sender popover: the person is saved, and what
    /// their signature said becomes theirs -- title and company where they
    /// had none, phones and websites as details. Hands back what undo needs.
    pub async fn add_sender(
        &self,
        person: ContactId,
        details: &postio_model::SignatureDetails,
    ) -> Result<postio_model::SenderPrior> {
        use postio_model::DetailKind;
        let now = to_millis(Utc::now());
        type Held = (Option<i64>, Option<String>, Option<String>);
        let held: Held = sql::first(
            self.connection,
            "SELECT saved_at, title, organization FROM contacts WHERE id = ?1",
            [person.get()],
            |row| Ok((row.col(0)?, row.col(1)?, row.col(2)?)),
        )
        .await?
        .ok_or(Error::NotFound {
            entity: "contact",
            id: person.get(),
        })?;
        let mut added = Vec::new();
        for (kind, value) in details
            .phones
            .iter()
            .map(|v| (DetailKind::Phone, v))
            .chain(details.websites.iter().map(|v| (DetailKind::Url, v)))
        {
            let known = self
                .details(person)
                .await?
                .iter()
                .any(|d| d.kind == kind && &d.value == value);
            if !known {
                self.set_detail(person, kind, value, Some(true)).await?;
                added.push((kind, value.clone()));
            }
        }
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "UPDATE contacts
                    SET saved_at = coalesce(saved_at, ?2),
                        title = coalesce(title, ?3),
                        organization = coalesce(organization, ?4),
                        listed = 1, updated_at = ?2
                  WHERE id = ?1",
                bind![
                    person.get(),
                    now,
                    details.title.clone(),
                    details.company.clone()
                ],
            )
            .await?;
            promote(&transaction, person).await
        })
        .await?;
        Ok(postio_model::SenderPrior {
            saved_at: held.0.map(from_millis),
            title: held.1,
            organization: held.2,
            added,
        })
    }

    /// Puts a person back as they were before [`add_sender`](Self::add_sender).
    pub async fn restore_sender(
        &self,
        person: ContactId,
        prior: &postio_model::SenderPrior,
    ) -> Result<()> {
        for (kind, value) in &prior.added {
            self.set_detail(person, *kind, value, None).await?;
        }
        sql::execute(
            self.connection,
            "UPDATE contacts SET saved_at = ?2, title = ?3, organization = ?4, updated_at = ?5
              WHERE id = ?1",
            bind![
                person.get(),
                prior.saved_at.map(to_millis),
                prior.title.clone(),
                prior.organization.clone(),
                to_millis(Utc::now())
            ],
        )
        .await?;
        Ok(())
    }
}

impl ContactRepository<'_> {
    /// Notes a fact about `person` the way the body stage does for a sender:
    /// what an import read from their card.
    pub async fn note_evidence(
        &self,
        person: ContactId,
        kind: postio_model::EvidenceKind,
        value: &str,
    ) -> Result<()> {
        sql::execute(
            self.connection,
            "INSERT OR IGNORE INTO contact_evidence (contact_id, kind, value) VALUES (?1, ?2, ?3)",
            bind![person.get(), kind.as_str(), value],
        )
        .await?;
        Ok(())
    }

    /// Notes what a message said about its sender (specs/011-focus-contacts
    /// S10): a signature, a phone, a `Resent-From`. Kept against the sender's
    /// person, once however often the mail says it; what is noted is a fact
    /// about them, never the message.
    pub async fn record_evidence(
        &self,
        message: postio_model::MessageId,
        items: &[(postio_model::EvidenceKind, String)],
    ) -> Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        let sender: Option<i64> = sql::first(
            self.connection,
            "SELECT a.contact_id FROM recipients r JOIN addresses a ON a.id = r.address_id
              WHERE r.message_id = ?1 AND r.kind = 'from' AND a.contact_id IS NOT NULL
              LIMIT 1",
            [message.get()],
            |row| row.col(0),
        )
        .await?;
        let Some(sender) = sender else {
            return Ok(());
        };
        for (kind, value) in items {
            sql::execute(
                self.connection,
                "INSERT OR IGNORE INTO contact_evidence (contact_id, kind, value) VALUES (?1, ?2, ?3)",
                bind![sender, kind.as_str(), value.clone()],
            )
            .await?;
        }
        Ok(())
    }

    /// Pairs of live people the mail says may be one, found without being
    /// told: the same signature or phone noted on both, a `Resent-From` that
    /// names the other's address, or one stopping as the other begins in at
    /// least two shared threads. Lower id first; pairs the user said are two
    /// people are left out.
    pub async fn discovered_pairs(&self, limit: u32) -> Result<Vec<(ContactId, ContactId)>> {
        use std::collections::BTreeSet;
        let cap = i64::from(limit) * 4;
        let mut found: BTreeSet<(i64, i64)> = BTreeSet::new();
        let shared: Vec<(i64, i64)> = sql::all_unbounded(
            self.connection,
            "SELECT e1.contact_id, e2.contact_id FROM contact_evidence e1
               JOIN contact_evidence e2
                 ON e2.kind = e1.kind AND e2.value = e1.value AND e2.contact_id > e1.contact_id
               JOIN contacts c1 ON c1.id = e1.contact_id AND c1.state = 'live'
               JOIN contacts c2 ON c2.id = e2.contact_id AND c2.state = 'live'
              WHERE e1.kind IN ('signature', 'phone', 'card') LIMIT ?1",
            [cap],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        found.extend(shared);
        let resent: Vec<(i64, i64)> = sql::all_unbounded(
            self.connection,
            "SELECT e.contact_id, a.contact_id FROM contact_evidence e
               JOIN addresses a ON a.address_normalized = lower(e.value)
               JOIN contacts c1 ON c1.id = e.contact_id AND c1.state = 'live'
               JOIN contacts c2 ON c2.id = a.contact_id AND c2.state = 'live'
              WHERE e.kind = 'resent' AND a.contact_id <> e.contact_id LIMIT ?1",
            [cap],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        found.extend(resent.into_iter().map(|(a, b)| (a.min(b), a.max(b))));
        let took: Vec<(i64, i64)> = sql::all_unbounded(
            self.connection,
            "WITH f AS (
               SELECT m.thread_id AS t, a.contact_id AS c,
                      min(m.received_at) AS lo, max(m.received_at) AS hi
                 FROM recipients r
                 JOIN messages m ON m.id = r.message_id
                 JOIN addresses a ON a.id = r.address_id
                 JOIN contacts k ON k.id = a.contact_id AND k.state = 'live'
                WHERE r.kind = 'from' AND m.thread_id IS NOT NULL
                GROUP BY m.thread_id, a.contact_id)
             SELECT x.c, y.c FROM f x JOIN f y ON y.t = x.t AND y.c <> x.c AND x.hi < y.lo
              GROUP BY x.c, y.c HAVING count(*) >= 2 LIMIT ?1",
            [cap],
            |row| Ok((row.col(0)?, row.col(1)?)),
        )
        .await?;
        found.extend(took.into_iter().map(|(a, b)| (a.min(b), a.max(b))));
        let mut out = Vec::new();
        for (a, b) in found {
            let dismissed = sql::exists(
                self.connection,
                "SELECT 1 FROM addresses nx
                   JOIN addresses ny ON ny.contact_id = ?2
                   JOIN contact_join_dismissals d
                     ON d.address_low = min(nx.id, ny.id) AND d.address_high = max(nx.id, ny.id)
                  WHERE nx.contact_id = ?1",
                bind![a, b],
            )
            .await?;
            if !dismissed {
                out.push((ContactId::new(a), ContactId::new(b)));
            }
            if out.len() >= limit as usize {
                break;
            }
        }
        Ok(out)
    }

    /// What the noted evidence says the two records share.
    pub async fn pair_evidence(
        &self,
        a: &postio_model::Contact,
        b: &postio_model::Contact,
    ) -> Result<postio_model::PairEvidence> {
        let shared = |kind: &'static str| {
            let (a, b) = (a.id.get(), b.id.get());
            async move {
                sql::first(
                    self.connection,
                    "SELECT e1.value FROM contact_evidence e1
                       JOIN contact_evidence e2 ON e2.kind = e1.kind AND e2.value = e1.value
                      WHERE e1.kind = ?3 AND e1.contact_id = ?1 AND e2.contact_id = ?2 LIMIT 1",
                    bind![a, b, kind],
                    |row| row.col::<String>(0),
                )
                .await
            }
        };
        let signature = shared("signature").await?;
        let card = shared("card").await?;
        let phone = shared("phone").await?;
        let mut resent = None;
        for (sender, other) in [(a, b), (b, a)] {
            let noted: Vec<String> = sql::all_unbounded(
                self.connection,
                "SELECT value FROM contact_evidence WHERE contact_id = ?1 AND kind = 'resent'",
                [sender.id.get()],
                |row| row.col(0),
            )
            .await?;
            if let Some(from) = noted.into_iter().find(|value| {
                other
                    .addresses
                    .iter()
                    .any(|owned| owned.address.address.eq_ignore_ascii_case(value))
            }) {
                let to = sender
                    .addresses
                    .first()
                    .map(|owned| owned.address.address.clone())
                    .unwrap_or_default();
                resent = Some((from, to));
                break;
            }
        }
        Ok(postio_model::PairEvidence {
            signature,
            phone,
            resent,
            card,
        })
    }

    /// A delivery failure named `address`: it is marked, at the time of the
    /// report, unless a later failure already is. Returns whether the address
    /// is one the store knows.
    pub async fn mark_bounced(&self, address: &str, at: chrono::DateTime<Utc>) -> Result<bool> {
        let at = to_millis(at);
        let found = sql::first(
            self.connection,
            "SELECT id FROM addresses WHERE address_normalized = ?1",
            [address.to_lowercase()],
            |row| row.col::<i64>(0),
        )
        .await?;
        let Some(id) = found else {
            return Ok(false);
        };
        sql::execute(
            self.connection,
            "UPDATE addresses SET bounced_at = ?2 WHERE id = ?1 AND (bounced_at IS NULL OR bounced_at < ?2)",
            bind![id, at],
        )
        .await?;
        Ok(true)
    }

    /// Whether one of two people stopped writing as the other began, in the
    /// same threads: the address that stopped, the one that began, and in how
    /// many threads. `None` when it did not happen in at least one.
    pub async fn took_over(
        &self,
        a: &postio_model::Contact,
        b: &postio_model::Contact,
    ) -> Result<Option<(String, String, u32)>> {
        for (old, new) in [(a, b), (b, a)] {
            let (olds, news) = (
                old.addresses.iter().map(|x| x.id.get()).collect::<Vec<_>>(),
                new.addresses.iter().map(|x| x.id.get()).collect::<Vec<_>>(),
            );
            if olds.is_empty() || news.is_empty() {
                continue;
            }
            let marks = |from: usize, n: usize| {
                (from..from + n)
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let params: Vec<turso::Value> = olds
                .iter()
                .chain(news.iter())
                .map(|id| turso::Value::Integer(*id))
                .collect();
            let threads = sql::scalar(
                self.connection,
                &format!(
                    "SELECT count(*) FROM (
                       SELECT m.thread_id AS t, max(m.received_at) AS last
                         FROM recipients r JOIN messages m ON m.id = r.message_id
                        WHERE r.kind = 'from' AND m.thread_id IS NOT NULL
                          AND r.address_id IN ({})
                        GROUP BY m.thread_id) x
                     JOIN (
                       SELECT m.thread_id AS t, min(m.received_at) AS first
                         FROM recipients r JOIN messages m ON m.id = r.message_id
                        WHERE r.kind = 'from' AND m.thread_id IS NOT NULL
                          AND r.address_id IN ({})
                        GROUP BY m.thread_id) y ON y.t = x.t
                     WHERE x.last < y.first",
                    marks(0, olds.len()),
                    marks(olds.len(), news.len())
                ),
                params,
            )
            .await?;
            if threads > 0 {
                let first = |p: &postio_model::Contact| {
                    p.addresses
                        .first()
                        .map(|x| x.address.address.clone())
                        .unwrap_or_default()
                };
                return Ok(Some((
                    first(old),
                    first(new),
                    u32::try_from(threads).unwrap_or(u32::MAX),
                )));
            }
        }
        Ok(None)
    }

    /// The suggestions as pairs with the answers that crossed, the
    /// strongest first, for the review.
    pub async fn suggestion_pairs(
        &self,
        limit: u32,
    ) -> Result<Vec<(postio_model::Contact, postio_model::Contact, u32)>> {
        let suggestions = self.suggestions(limit).await?;
        Ok(suggestions
            .into_iter()
            .map(|suggestion| {
                let replies = match suggestion.reason {
                    SuggestionReason::Replied { replies } => replies,
                    SuggestionReason::SameName => 0,
                };
                let [a, b] = suggestion.people;
                (a, b, replies)
            })
            .collect())
    }
}

/// How long a bounce counts as recent in the review.
const BOUNCE_RECENT_DAYS: i64 = 30;

/// A person, without addresses — [`ContactRepository::with_addresses`] adds
/// them.
pub(super) fn read_person(row: &Row) -> Result<Contact> {
    let source: String = row.col(4)?;
    let state: String = row.col(5)?;
    let kind: String = row.col(11)?;
    let kind_source: String = row.col(12)?;
    let show_as: String = row.col(16)?;
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
        kind: ContactKind::from_name(&kind).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.kind",
            value: kind.clone(),
        })?,
        kind_source: KindSource::from_name(&kind_source).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.kind_source",
            value: kind_source.clone(),
        })?,
        saved_at: row.col::<Option<i64>>(13)?.map(from_millis),
        given_name: row.col(14)?,
        family_name: row.col(15)?,
        show_as: ShowAs::from_name(&show_as).ok_or_else(|| Error::UnknownEnum {
            column: "contacts.show_as",
            value: show_as.clone(),
        })?,
        title: row.col(17)?,
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
