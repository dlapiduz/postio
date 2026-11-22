//! Auto-emptying Junk and Trash (#1932): purges what is old enough, a page at a time, off the
//! interactive path under the background write gate.

use chrono::{DateTime, Utc};
use postio_core::retention::AutoEmpty;
use postio_storage::repository::{AccountRepository, ExpiryRepository};
use postio_storage::{Store, WritePriority};

pub const PAGE: u32 = 200;

pub const PAGES_PER_PASS: u32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Report {
    pub purged: usize,
    pub more: bool,
}

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
