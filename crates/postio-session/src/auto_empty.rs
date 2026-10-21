//! Auto-emptying Junk and Trash (#1932): the pass that purges what is old
//! enough, a page at a time.
//!
//! The rule is [`postio_core::retention::AutoEmpty`]; finding the mail is
//! `ExpiryRepository`; the delete is `actions::purge`, which hides the row at
//! once and queues the server's half like every other mutation. This only
//! joins them, off the interactive path and under the background write gate,
//! so a keystroke's write always goes first.

use chrono::{DateTime, Utc};
use postio_core::retention::AutoEmpty;
use postio_storage::repository::{AccountRepository, ExpiryRepository};
use postio_storage::{Store, WritePriority};

/// Messages purged per read-and-write round.
pub const PAGE: u32 = 200;

/// Pages one pass takes at most per account and folder kind: a mailbox of a
/// hundred thousand is emptied over a few runs, never in one long write.
pub const PAGES_PER_PASS: u32 = 10;

/// What one pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    /// Messages purged.
    pub purged: usize,
    /// More were due than the pass would take.
    pub more: bool,
}

/// Purge everything `policy` says is due at `now`, in every enabled account.
/// Off (the default) is one comparison and no database work.
pub async fn run(
    database: &Store,
    policy: AutoEmpty,
    now: DateTime<Utc>,
) -> Result<Report, Box<dyn std::error::Error>> {
    let mut report = Report::default();
    if !policy.is_on() {
        return Ok(report);
    }
    let connection = database.connect_background().await?;
    let accounts = AccountRepository::new(&connection).list_enabled().await?;
    for account in accounts {
        for role in AutoEmpty::ROLES {
            let Some(cutoff) = policy.cutoff(role, now) else {
                continue;
            };
            for _ in 0..PAGES_PER_PASS {
                let _permit = connection
                    .write_gate()
                    .acquire(WritePriority::Background)
                    .await;
                let page = ExpiryRepository::new(&connection)
                    .expired_in(account.id, role, cutoff, PAGE)
                    .await?;
                if page.messages.is_empty() {
                    break;
                }
                let mut by_mailbox = std::collections::BTreeMap::new();
                for (mailbox, message) in &page.messages {
                    by_mailbox
                        .entry(*mailbox)
                        .or_insert_with(Vec::new)
                        .push(*message);
                }
                let account_id = account.id;
                postio_storage::transaction::<_, postio_storage::Error, _, _>(
                    &connection,
                    |scope| async move {
                        for (mailbox, ids) in by_mailbox {
                            postio_storage::actions::purge(&scope, account_id, mailbox, &ids, now)
                                .await?;
                        }
                        Ok(())
                    },
                )
                .await?;
                report.purged += page.messages.len();
                if !page.more {
                    break;
                }
                report.more = true;
            }
        }
    }
    Ok(report)
}
