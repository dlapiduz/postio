//! Mailboxes: the folder tree, its special-use roles, and the sidebar's counts.

use postio_model::{
    AccountId, Mailbox, MailboxCounts, MailboxId, MailboxRole, ModSeq, SignatureId, Uid,
    UidValidity,
};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{from_millis, require_persisted, to_millis, unknown_enum};
use crate::error::{Error, Result};

/// Reads and writes [`Mailbox`] rows.
///
/// A mailbox's UID state lives in `sync_state` rather than on the row the
/// sidebar reads, so the sync engine can update it in the same transaction as
/// the message writes it describes without touching what the UI is rendering.
/// The repository joins the two back together, because a [`Mailbox`] is one
/// thing to everybody above this layer.
#[derive(Debug)]
pub struct MailboxRepository<'a> {
    connection: &'a Connection,
}

const MAILBOX_COLUMNS: &str = "\
m.id, m.account_id, m.parent_id, m.name, m.path, m.delimiter, m.role, m.selectable,
m.subscribed, m.total_count, m.unread_count, m.flagged_count, m.last_synced_at,
s.uid_validity, s.uid_next, s.highest_mod_seq, m.signature_id, m.backfill_excluded,
m.snoozed_count";

const FROM_MAILBOXES: &str = "\
FROM mailboxes m LEFT JOIN sync_state s ON s.mailbox_id = m.id";

/// What counts as a message for the sidebar: one that the list would show.
///
/// A message hidden pending a remote delete or a snooze not yet due is not
/// in the list, so counting it would put a number on screen the user cannot
/// reconcile with what they see. `snoozed_until` is compared against
/// SQLite's own clock rather than a bound parameter, matching the trigger
/// this mirrors (migration 0021) — both are the cached-count half of the
/// same two-tier arrangement the live list query (`where_clause`) is the
/// other half of.
const VISIBLE: &str =
    "deleted_locally = 0 AND (snoozed_until IS NULL OR snoozed_until <= (strftime('%s','now') * 1000))";

/// What counts as snoozed for the sidebar's own badge: the inverse of the
/// snooze half of [`VISIBLE`], still gated on `deleted_locally` the same way.
const SNOOZED: &str =
    "deleted_locally = 0 AND snoozed_until IS NOT NULL AND snoozed_until > (strftime('%s','now') * 1000)";

impl<'a> MailboxRepository<'a> {
    /// Borrows a connection.
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    /// Inserts a mailbox and its sync-state row, assigning its id.
    pub fn create(&self, mailbox: &mut Mailbox) -> Result<MailboxId> {
        let account_id = require_persisted(mailbox.account_id.get(), "account")?;
        let transaction = super::Scope::open(self.connection)?;

        transaction.execute(
            "INSERT INTO mailboxes (account_id, parent_id, name, path, delimiter, role,
                                    selectable, subscribed, total_count, unread_count,
                                    flagged_count, last_synced_at, signature_id,
                                    backfill_excluded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                account_id,
                optional_id(mailbox.parent_id),
                mailbox.name,
                mailbox.path,
                mailbox.delimiter.map(String::from),
                mailbox.role.as_str(),
                mailbox.selectable,
                mailbox.subscribed,
                mailbox.counts.total,
                mailbox.counts.unread,
                mailbox.counts.flagged,
                mailbox.last_synced_at.map(to_millis),
                optional_signature_id(mailbox.signature_id),
                mailbox.backfill_excluded,
            ],
        )?;
        let id = MailboxId::new(transaction.last_insert_rowid());
        mailbox.id = id;

        // Always written, even when every value is NULL: the sync engine can
        // then UPDATE its state without first having to wonder whether the row
        // exists, and "never synced" stays a readable, explicit state.
        write_sync_state(&transaction, id, account_id, mailbox)?;

        transaction.commit()?;
        Ok(id)
    }

    /// Writes a mailbox and its sync state back.
    pub fn update(&self, mailbox: &Mailbox) -> Result<()> {
        let id = require_persisted(mailbox.id.get(), "mailbox")?;
        let account_id = require_persisted(mailbox.account_id.get(), "account")?;
        let transaction = super::Scope::open(self.connection)?;

        let changed = transaction.execute(
            "UPDATE mailboxes
                SET account_id = ?2, parent_id = ?3, name = ?4, path = ?5, delimiter = ?6,
                    role = ?7, selectable = ?8, subscribed = ?9, total_count = ?10,
                    unread_count = ?11, flagged_count = ?12, last_synced_at = ?13,
                    signature_id = ?14, backfill_excluded = ?15, snoozed_count = ?16
              WHERE id = ?1",
            params![
                id,
                account_id,
                optional_id(mailbox.parent_id),
                mailbox.name,
                mailbox.path,
                mailbox.delimiter.map(String::from),
                mailbox.role.as_str(),
                mailbox.selectable,
                mailbox.subscribed,
                mailbox.counts.total,
                mailbox.counts.unread,
                mailbox.counts.flagged,
                mailbox.last_synced_at.map(to_millis),
                optional_signature_id(mailbox.signature_id),
                mailbox.backfill_excluded,
                mailbox.counts.snoozed,
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "mailbox",
                id,
            });
        }

        write_sync_state(&transaction, mailbox.id, account_id, mailbox)?;
        transaction.commit()?;
        Ok(())
    }

    /// One mailbox.
    pub fn get(&self, id: MailboxId) -> Result<Option<Mailbox>> {
        self.one("WHERE m.id = ?1", [id.get()])
    }

    /// Whether `id` is excluded from the background backfill lane (ADR 0016,
    /// #350).
    ///
    /// A single-column read rather than [`MailboxRepository::get`] plus a
    /// field access: `postio-sync::backfill::seed` calls this on every
    /// top-up pass for every mailbox and does not otherwise need the row.
    /// A mailbox that no longer exists answers `false` rather than an error
    /// — the caller's own query over `messages` already answers "nothing to
    /// seed" for that case, and failing open toward "still backfills" is the
    /// safer direction for a mechanism nobody asked to turn off.
    pub fn backfill_excluded(&self, id: MailboxId) -> Result<bool> {
        Ok(self
            .connection
            .query_row(
                "SELECT backfill_excluded FROM mailboxes WHERE id = ?1",
                [id.get()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    }

    /// Flips whether `id` participates in background backfill — the
    /// settings-surface counterpart of [`AccountRepository`]'s
    /// [`set_enabled`](super::AccountRepository::set_enabled), for the same
    /// reason: the caller here is a folder's own settings toggle, not code
    /// holding a full, freshly-loaded [`Mailbox`].
    ///
    /// Does not touch anything already pulled, and does not affect an
    /// interactive, on-open fetch — only what [`MailboxRepository::backfill_excluded`]
    /// answers for the background lane's own seeding pass.
    pub fn set_backfill_excluded(&self, id: MailboxId, excluded: bool) -> Result<bool> {
        let changed = self.connection.execute(
            "UPDATE mailboxes SET backfill_excluded = ?2 WHERE id = ?1",
            params![id.get(), excluded],
        )?;
        Ok(changed > 0)
    }

    /// The mailbox at `path` within an account.
    pub fn by_path(&self, account_id: AccountId, path: &str) -> Result<Option<Mailbox>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} WHERE m.account_id = ?1 AND m.path = ?2"
        ))?;
        let mut rows = statement.query(params![account_id.get(), path])?;
        rows.next()?
            .map(read_mailbox)
            .transpose()
            .map_err(Into::into)
    }

    /// The account's mailbox for a special-use role.
    ///
    /// Routing is by role and never by name: iCloud calls its sent folder
    /// `Sent Messages` and advertises no `SPECIAL-USE` attribute for it. When a
    /// server has somehow reported two, the first by path wins, so the answer
    /// is at least stable.
    pub fn by_role(&self, account_id: AccountId, role: MailboxRole) -> Result<Option<Mailbox>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES}
              WHERE m.account_id = ?1 AND m.role = ?2 ORDER BY m.path LIMIT 1"
        ))?;
        let mut rows = statement.query(params![account_id.get(), role.as_str()])?;
        rows.next()?
            .map(read_mailbox)
            .transpose()
            .map_err(Into::into)
    }

    /// Every mailbox in an account, ordered by path.
    ///
    /// Path order is hierarchy order for the sidebar: a folder sorts
    /// immediately before its children.
    pub fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Mailbox>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} WHERE m.account_id = ?1 ORDER BY m.path"
        ))?;
        let rows = statement.query_map([account_id.get()], read_mailbox)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Deletes a mailbox and its messages, returning whether there was one.
    pub fn delete(&self, id: MailboxId) -> Result<bool> {
        let deleted = self
            .connection
            .execute("DELETE FROM mailboxes WHERE id = ?1", [id.get()])?;
        Ok(deleted > 0)
    }

    /// The cached counts on a mailbox row.
    pub fn counts(&self, id: MailboxId) -> Result<Option<MailboxCounts>> {
        let mut statement = self.connection.prepare(
            "SELECT total_count, unread_count, flagged_count, snoozed_count
               FROM mailboxes WHERE id = ?1",
        )?;
        let mut rows = statement.query([id.get()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        Ok(Some(MailboxCounts {
            total: row.get(0)?,
            unread: row.get(1)?,
            flagged: row.get(2)?,
            snoozed: row.get(3)?,
        }))
    }

    /// Overwrites a mailbox's cached counts.
    ///
    /// For a server `STATUS` response, which reports on messages that have not
    /// been fetched yet and therefore have no local rows to count.
    ///
    /// `counts.snoozed` is never a server's to report — `STATUS` has no
    /// concept of it — so the only real caller is [`Self::recount`], whose
    /// own live scan computes a genuine value.
    pub fn set_counts(&self, id: MailboxId, counts: MailboxCounts) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE mailboxes SET total_count = ?2, unread_count = ?3, flagged_count = ?4,
                snoozed_count = ?5
              WHERE id = ?1",
            params![
                id.get(),
                counts.total,
                counts.unread,
                counts.flagged,
                counts.snoozed
            ],
        )?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "mailbox",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Recomputes a mailbox's counts from its messages and caches them.
    ///
    /// **A repair tool, not routine maintenance.** Migration 0003's triggers
    /// keep `total_count`, `unread_count` and `flagged_count` correct on
    /// every write to `messages` — that is what made `recount` and
    /// [`recount_account`](Self::recount_account) dead code with no
    /// production caller in the first place (postio-qhz.7), and calling
    /// either after an ordinary write only redoes what the trigger already
    /// did. What is left for these to be good for: repairing a store the
    /// triggers were not there for (the migration's own backfill), and
    /// standing as the independent, scan-based ground truth a test can check
    /// the trigger-maintained columns against — see
    /// `a_seeded_store_still_agrees_with_a_recount` in
    /// `tests/mailbox_counts.rs`, which is what would notice if the two ever
    /// drifted apart. postio-qhz.8.
    pub fn recount(&self, id: MailboxId) -> Result<MailboxCounts> {
        let now_millis = "(strftime('%s','now') * 1000)";
        let not_snoozed = format!("(snoozed_until IS NULL OR snoozed_until <= {now_millis})");
        let counts = self.connection.query_row(
            &format!(
                "SELECT coalesce(sum(deleted_locally = 0 AND {not_snoozed}), 0),
                        coalesce(sum(deleted_locally = 0 AND {not_snoozed} AND seen = 0), 0),
                        coalesce(sum(deleted_locally = 0 AND {not_snoozed} AND flagged = 1), 0),
                        coalesce(sum(deleted_locally = 0 AND snoozed_until IS NOT NULL
                                      AND snoozed_until > {now_millis}), 0)
                   FROM messages WHERE mailbox_id = ?1"
            ),
            [id.get()],
            |row| {
                Ok(MailboxCounts {
                    total: row.get::<_, i64>(0)? as u32,
                    unread: row.get::<_, i64>(1)? as u32,
                    flagged: row.get::<_, i64>(2)? as u32,
                    snoozed: row.get::<_, i64>(3)? as u32,
                })
            },
        )?;
        self.set_counts(id, counts)?;
        Ok(counts)
    }

    /// Recomputes every mailbox in an account, in one pass over its messages.
    ///
    /// [`recount`](Self::recount)'s repair-path doc applies here too: the
    /// triggers maintain these columns on every write, so this is for
    /// repairing an account whose counts have drifted, not something a
    /// normal write path should call.
    pub fn recount_account(&self, account_id: AccountId) -> Result<()> {
        self.connection.execute(
            &format!(
                "UPDATE mailboxes
                    SET total_count = coalesce((SELECT count(*) FROM messages
                                                 WHERE mailbox_id = mailboxes.id AND {VISIBLE}), 0),
                        unread_count = coalesce((SELECT count(*) FROM messages
                                                  WHERE mailbox_id = mailboxes.id AND {VISIBLE}
                                                    AND seen = 0), 0),
                        flagged_count = coalesce((SELECT count(*) FROM messages
                                                   WHERE mailbox_id = mailboxes.id AND {VISIBLE}
                                                     AND flagged = 1), 0),
                        snoozed_count = coalesce((SELECT count(*) FROM messages
                                                   WHERE mailbox_id = mailboxes.id AND {SNOOZED}), 0)
                  WHERE account_id = ?1"
            ),
            [account_id.get()],
        )?;
        Ok(())
    }

    /// The account's totals, summed from its mailboxes' cached counts.
    pub fn account_counts(&self, account_id: AccountId) -> Result<MailboxCounts> {
        self.connection
            .query_row(
                "SELECT coalesce(sum(total_count), 0), coalesce(sum(unread_count), 0),
                    coalesce(sum(flagged_count), 0), coalesce(sum(snoozed_count), 0)
               FROM mailboxes WHERE account_id = ?1",
                [account_id.get()],
                |row| {
                    Ok(MailboxCounts {
                        total: row.get::<_, i64>(0)? as u32,
                        unread: row.get::<_, i64>(1)? as u32,
                        flagged: row.get::<_, i64>(2)? as u32,
                        snoozed: row.get::<_, i64>(3)? as u32,
                    })
                },
            )
            .map_err(Into::into)
    }

    fn one<P: rusqlite::Params>(&self, filter: &str, parameters: P) -> Result<Option<Mailbox>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} {filter}"
        ))?;
        let mut rows = statement.query(parameters)?;
        rows.next()?
            .map(read_mailbox)
            .transpose()
            .map_err(Into::into)
    }
}

fn write_sync_state(
    connection: &Connection,
    id: MailboxId,
    account_id: i64,
    mailbox: &Mailbox,
) -> Result<()> {
    connection.execute(
        "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, uid_next, highest_mod_seq)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (mailbox_id) DO UPDATE
            SET account_id = excluded.account_id,
                uid_validity = excluded.uid_validity,
                uid_next = excluded.uid_next,
                highest_mod_seq = excluded.highest_mod_seq",
        params![
            id.get(),
            account_id,
            mailbox.uid_validity.map(|value| i64::from(value.get())),
            mailbox.uid_next.map(|value| i64::from(value.get())),
            mailbox.highest_mod_seq.map(|value| value.get() as i64),
        ],
    )?;
    Ok(())
}

fn read_mailbox(row: &Row<'_>) -> rusqlite::Result<Mailbox> {
    let role: String = row.get(6)?;
    let delimiter: Option<String> = row.get(5)?;

    Ok(Mailbox {
        id: MailboxId::new(row.get(0)?),
        account_id: AccountId::new(row.get(1)?),
        parent_id: row.get::<_, Option<i64>>(2)?.map(MailboxId::new),
        name: row.get(3)?,
        path: row.get(4)?,
        delimiter: delimiter.and_then(|value| value.chars().next()),
        role: MailboxRole::from_name(&role).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                6,
                rusqlite::types::Type::Text,
                Box::new(unknown_enum("mailboxes.role", role)),
            )
        })?,
        selectable: row.get(7)?,
        subscribed: row.get(8)?,
        counts: MailboxCounts {
            total: row.get(9)?,
            unread: row.get(10)?,
            flagged: row.get(11)?,
            snoozed: row.get(18)?,
        },
        uid_validity: row
            .get::<_, Option<i64>>(13)?
            .map(|value| UidValidity::new(value as u32)),
        uid_next: row
            .get::<_, Option<i64>>(14)?
            .map(|value| Uid::new(value as u32)),
        highest_mod_seq: row
            .get::<_, Option<i64>>(15)?
            .map(|value| ModSeq::new(value as u64)),
        last_synced_at: row.get::<_, Option<i64>>(12)?.map(from_millis),
        signature_id: row.get::<_, Option<i64>>(16)?.map(SignatureId::new),
        backfill_excluded: row.get(17)?,
    })
}

fn optional_id(id: Option<MailboxId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(MailboxId::get)
}

fn optional_signature_id(id: Option<SignatureId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(SignatureId::get)
}
