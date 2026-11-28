use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Generation, Mailbox, MailboxCounts, MailboxId, MailboxRole, ModSeq, RoleKind,
    SignatureId, Uid,
};

use super::{from_millis, require_persisted, to_millis, unknown_enum};

use crate::error::{Error, Result};
use crate::sql::{self, RowExt as _, bind};
use crate::store::Connection;
use turso::Row;

/// UID state lives in `sync_state`, so the sync engine updates it in the same transaction as its
/// message writes without touching what the UI renders; the repository joins them back.
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

use super::VISIBLE;

const SNOOZED: &str = "deleted_locally = 0 AND snoozed_until IS NOT NULL AND snoozed_until > (strftime('%s','now') * 1000)";

/// Not left to the schema's `CHECK`: a violation does not name the role, and the `CHECK` enumerates
/// spellings so it silently stops enforcing when a role is added to the enum only
/// (`scripts/checks/check-view-roles-are-not-storable.py`).
fn refuse_a_view(role: MailboxRole) -> Result<()> {
    match role.kind() {
        RoleKind::Folder => Ok(()),
        RoleKind::View => Err(Error::RoleIsAView {
            role: role.as_str(),
        }),
    }
}

pub use postio_model::listing::DraftCounts;

impl<'a> MailboxRepository<'a> {
    pub fn new(connection: &'a Connection) -> Self {
        Self { connection }
    }

    pub async fn create(&self, mailbox: &mut Mailbox) -> Result<MailboxId> {
        refuse_a_view(mailbox.role)?;
        let account_id = require_persisted(mailbox.account_id.get(), "account")?;
        sql::in_scope(self.connection, |transaction| async move {
            sql::execute(
                &transaction,
                "INSERT INTO mailboxes (account_id, parent_id, name, path, delimiter, role,
                                    selectable, subscribed, total_count, unread_count,
                                    flagged_count, last_synced_at, signature_id,
                                    backfill_excluded)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                bind![
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
            )
            .await?;
            let id = MailboxId::new(transaction.last_insert_rowid());
            mailbox.id = id;

            // Always written, even all NULL, so the engine can UPDATE without checking the row
            // exists.
            write_sync_state(&transaction, id, account_id, mailbox).await?;

            Ok(id)
        })
        .await
    }

    pub async fn update(&self, mailbox: &Mailbox) -> Result<()> {
        refuse_a_view(mailbox.role)?;
        let id = require_persisted(mailbox.id.get(), "mailbox")?;
        let account_id = require_persisted(mailbox.account_id.get(), "account")?;
        sql::in_scope(self.connection, |transaction| async move {
            let changed = sql::execute(
                &transaction,
                "UPDATE mailboxes
                SET account_id = ?2, parent_id = ?3, name = ?4, path = ?5, delimiter = ?6,
                    role = ?7, selectable = ?8, subscribed = ?9, total_count = ?10,
                    unread_count = ?11, flagged_count = ?12, last_synced_at = ?13,
                    signature_id = ?14, backfill_excluded = ?15, snoozed_count = ?16
              WHERE id = ?1",
                bind![
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
            )
            .await?;
            if changed == 0 {
                return Err(Error::NotFound {
                    entity: "mailbox",
                    id,
                });
            }

            write_sync_state(&transaction, mailbox.id, account_id, mailbox).await?;
            Ok(())
        })
        .await
    }

    pub async fn get(&self, id: MailboxId) -> Result<Option<Mailbox>> {
        self.one("WHERE m.id = ?1", [id.get()]).await
    }

    /// A single-column read: `postio-sync::backfill::seed` calls this every top-up pass. A missing
    /// mailbox answers `false`, failing open toward "still backfills".
    pub async fn backfill_excluded(&self, id: MailboxId) -> Result<bool> {
        Ok(sql::first(
            self.connection,
            "SELECT backfill_excluded FROM mailboxes WHERE id = ?1",
            [id.get()],
            |row| row.col(0),
        )
        .await?
        .unwrap_or(false))
    }

    /// A folder's own settings toggle. Does not touch anything already pulled or an interactive
    /// on-open fetch.
    pub async fn set_backfill_excluded(&self, id: MailboxId, excluded: bool) -> Result<bool> {
        let changed = sql::execute(
            self.connection,
            "UPDATE mailboxes SET backfill_excluded = ?2 WHERE id = ?1",
            bind![id.get(), excluded],
        )
        .await?;
        Ok(changed > 0)
    }

    pub async fn by_path(&self, account_id: AccountId, path: &str) -> Result<Option<Mailbox>> {
        sql::first(
            self.connection,
            &format!(
                "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} WHERE m.account_id = ?1 AND m.path = ?2"
            ),
            bind![account_id.get(), path],
            read_mailbox,
        )
        .await
    }

    /// By role, never by name: iCloud's `Sent Messages` has no `SPECIAL-USE`. If a server reports
    /// two, the first by path wins. Never a retired row: a folder the server no longer lists
    /// refuses APPEND and MOVE (#943).
    pub async fn by_role(
        &self,
        account_id: AccountId,
        role: MailboxRole,
    ) -> Result<Option<Mailbox>> {
        sql::first(
            self.connection,
            &Self::explain_by_role(),
            bind![account_id.get(), role.as_str()],
            read_mailbox,
        )
        .await
    }

    pub fn explain_by_role() -> String {
        format!(
            "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES}
              WHERE m.account_id = ?1 AND m.role = ?2 AND m.selectable = 1
              ORDER BY m.path LIMIT 1"
        )
    }

    /// Path order is hierarchy order for the sidebar: a folder sorts immediately before its
    /// children.
    pub async fn list_for_account(&self, account_id: AccountId) -> Result<Vec<Mailbox>> {
        sql::all(
            self.connection,
            &format!(
                "SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} WHERE m.account_id = ?1 ORDER BY m.path"
            ),
            [account_id.get()],
            read_mailbox,
        )
        .await
    }

    pub async fn delete(&self, id: MailboxId) -> Result<bool> {
        let deleted = sql::execute(
            self.connection,
            "DELETE FROM mailboxes WHERE id = ?1",
            [id.get()],
        )
        .await?;
        Ok(deleted > 0)
    }

    pub async fn counts(&self, id: MailboxId) -> Result<Option<MailboxCounts>> {
        let mut statement = sql::statement(
            self.connection,
            "SELECT total_count, unread_count, flagged_count, snoozed_count
               FROM mailboxes WHERE id = ?1",
        )
        .await?;
        crate::sql::first_of(&mut statement, [id.get()], |row| {
            Ok(MailboxCounts {
                total: row.col(0)?,
                unread: row.col(1)?,
                flagged: row.col(2)?,
                snoozed: row.col(3)?,
                // Not a message count: filled by the sidebar's feed.
                attention: 0,
            })
        })
        .await
    }

    /// For a server `STATUS` response, which reports on messages not fetched yet. `counts.snoozed`
    /// is never a server's to report, so the only real caller is [`Self::recount`].
    pub async fn set_counts(&self, id: MailboxId, counts: MailboxCounts) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE mailboxes SET total_count = ?2, unread_count = ?3, flagged_count = ?4,
                snoozed_count = ?5
              WHERE id = ?1",
            bind![
                id.get(),
                counts.total,
                counts.unread,
                counts.flagged,
                counts.snoozed
            ],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "mailbox",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// Completion, not attempt: a failed pass has not synced this folder, and a moving timestamp
    /// would turn "when did this last work" into "when did this last try".
    ///
    /// A narrow `UPDATE`, not [`update`](Self::update): counts are maintained by migration 0003's
    /// triggers and a stale whole-row write would overwrite them (#1281).
    pub async fn record_sync(&self, id: MailboxId, at: DateTime<Utc>) -> Result<()> {
        let changed = sql::execute(
            self.connection,
            "UPDATE mailboxes SET last_synced_at = ?2 WHERE id = ?1",
            bind![id.get(), to_millis(at)],
        )
        .await?;
        if changed == 0 {
            return Err(Error::NotFound {
                entity: "mailbox",
                id: id.get(),
            });
        }
        Ok(())
    }

    /// **A repair tool, not routine maintenance.** Migration 0003's triggers keep `total_count`,
    /// `unread_count` and `flagged_count` correct on every write, so calling this after an ordinary
    /// write redoes the trigger. Good for repairing a store the triggers were not there for, and as
    /// the scan-based ground truth `a_seeded_store_still_agrees_with_a_recount` in
    /// `tests/mailbox_counts.rs` checks them against.
    pub async fn recount(&self, id: MailboxId) -> Result<MailboxCounts> {
        let now_millis = "(strftime('%s','now') * 1000)";
        let not_snoozed = format!("(snoozed_until IS NULL OR snoozed_until <= {now_millis})");
        let counts = sql::one(
            self.connection,
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
                    total: row.col::<i64>(0)? as u32,
                    unread: row.col::<i64>(1)? as u32,
                    flagged: row.col::<i64>(2)? as u32,
                    snoozed: row.col::<i64>(3)? as u32,
                    // Not a message count: filled by the sidebar's feed.
                    attention: 0,
                })
            },
        )
        .await?;
        self.set_counts(id, counts).await?;
        Ok(counts)
    }

    /// A repair path like [`recount`](Self::recount): not for normal write paths.
    pub async fn recount_account(&self, account_id: AccountId) -> Result<()> {
        sql::execute(self.connection,
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
                                                   WHERE mailbox_id = mailboxes.id AND {SNOOZED}), 0),
                        bodies_owed = coalesce((SELECT count(*) FROM messages
                                                 WHERE mailbox_id = mailboxes.id
                                                   AND deleted_locally = 0
                                                   AND body_state IN ('not_fetched', 'headers_only')), 0)
                  WHERE account_id = ?1"
            ),
            [account_id.get()],
        ).await?;
        Ok(())
    }

    /// Three numbers from one read that no cached column can hold: the Outbox is not a mailbox, and
    /// `mailboxes.total_count` counts in-flight Drafts rows, so the Drafts badge asks separately.
    ///
    /// Bounded: `idx_messages_send_state` covers `(account_id, send_state, deleted_locally)` so the
    /// planner reads no message row. It must not be partial: this planner will not read a partial
    /// index, and a walk of every row was paid on every arrival (#1614).
    pub async fn draft_counts(&self, account_id: AccountId) -> Result<DraftCounts> {
        sql::one(
            self.connection,
            self.explain_draft_counts(),
            [account_id.get()],
            |row| {
                Ok(DraftCounts {
                    outbox: row.col::<i64>(0)? as u32,
                    attention: row.col::<i64>(1)? as u32,
                    drafts: row.col::<i64>(2)? as u32,
                })
            },
        )
        .await
    }

    pub fn explain_draft_counts(&self) -> &'static str {
        "SELECT
                 coalesce(sum(send_state IN ('queued', 'sending')), 0),
                 coalesce(sum(send_state IN ('failed', 'unconfirmed')), 0),
                 coalesce(sum(send_state NOT IN ('queued', 'sending', 'sent')), 0)
               FROM messages
              WHERE account_id = ?1 AND send_state IS NOT NULL
                AND deleted_locally = 0"
    }

    /// Read from the cached columns rather than counted.
    pub async fn account_counts(&self, account_id: AccountId) -> Result<MailboxCounts> {
        sql::one(
            self.connection,
            "SELECT coalesce(sum(total_count), 0), coalesce(sum(unread_count), 0),
                    coalesce(sum(flagged_count), 0), coalesce(sum(snoozed_count), 0)
               FROM mailboxes WHERE account_id = ?1",
            [account_id.get()],
            |row| {
                Ok(MailboxCounts {
                    total: row.col::<i64>(0)? as u32,
                    unread: row.col::<i64>(1)? as u32,
                    flagged: row.col::<i64>(2)? as u32,
                    snoozed: row.col::<i64>(3)? as u32,
                    // Not a message count: filled by the sidebar's feed.
                    attention: 0,
                })
            },
        )
        .await
    }

    async fn one(
        &self,
        filter: &str,
        parameters: impl turso::IntoParams,
    ) -> Result<Option<Mailbox>> {
        sql::first(
            self.connection,
            &format!("SELECT {MAILBOX_COLUMNS} {FROM_MAILBOXES} {filter}"),
            parameters,
            read_mailbox,
        )
        .await
    }
}

async fn write_sync_state(
    connection: &Connection,
    id: MailboxId,
    account_id: i64,
    mailbox: &Mailbox,
) -> Result<()> {
    sql::execute(
        connection,
        "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, uid_next, highest_mod_seq)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (mailbox_id) DO UPDATE
            SET account_id = excluded.account_id,
                uid_validity = excluded.uid_validity,
                uid_next = excluded.uid_next,
                highest_mod_seq = excluded.highest_mod_seq",
        bind![
            id.get(),
            account_id,
            mailbox.generation.map(|value| i64::from(value.get())),
            mailbox.uid_next.map(|value| i64::from(value.get())),
            mailbox.highest_mod_seq.map(|value| value.get() as i64),
        ],
    )
    .await?;
    Ok(())
}

fn read_mailbox(row: &Row) -> Result<Mailbox> {
    let role: String = row.col(6)?;
    let delimiter: Option<String> = row.col(5)?;

    Ok(Mailbox {
        id: MailboxId::new(row.col(0)?),
        account_id: AccountId::new(row.col(1)?),
        parent_id: row.col::<Option<i64>>(2)?.map(MailboxId::new),
        name: row.col(3)?,
        path: row.col(4)?,
        delimiter: delimiter.and_then(|value| value.chars().next()),
        role: MailboxRole::from_name(&role).ok_or_else(|| unknown_enum("mailboxes.role", role))?,
        selectable: row.col(7)?,
        subscribed: row.col(8)?,
        counts: MailboxCounts {
            total: row.col(9)?,
            unread: row.col(10)?,
            flagged: row.col(11)?,
            snoozed: row.col(18)?,
            // Not a message count: filled by the sidebar's feed.
            attention: 0,
        },
        generation: row
            .col::<Option<i64>>(13)?
            .map(|value| Generation::new(value as u32)),
        uid_next: row
            .col::<Option<i64>>(14)?
            .map(|value| Uid::new(value as u32)),
        highest_mod_seq: row
            .col::<Option<i64>>(15)?
            .map(|value| ModSeq::new(value as u64)),
        last_synced_at: row.col::<Option<i64>>(12)?.map(from_millis),
        signature_id: row.col::<Option<i64>>(16)?.map(SignatureId::new),
        backfill_excluded: row.col(17)?,
    })
}

fn optional_id(id: Option<MailboxId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(MailboxId::get)
}

fn optional_signature_id(id: Option<SignatureId>) -> Option<i64> {
    id.filter(|id| id.is_assigned()).map(SignatureId::get)
}
