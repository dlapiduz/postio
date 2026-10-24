//! What agents did, read for Settings (ADR 0010 Q6, #1832).
//!
//! `postio-mcp` logs ids only (ARCHITECTURE.md §11), so a pane that shows
//! the log resolves them to the rows they name when it draws. That reading
//! is here, once, for both frontends: the words are
//! `postio_ui::agent_access`'s.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_storage::Store;
use postio_storage::repository::{AccountRepository, MailboxRepository, MessageRepository};
use postio_ui::agent_access::{self as agent, Activity, Names};

/// The access log `postio-mcp` writes beside the store at `store`, or `None`
/// for a store with no directory (an in-memory one).
pub fn log_path(store: &Path) -> Option<PathBuf> {
    store
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|dir| dir.join("mcp-access.log"))
}

/// The recent calls in `log` as of `now`, their ids resolved in `database`.
///
/// A row that has since been deleted is left unnamed rather than failing
/// the page, and a store that will not open names nothing.
pub async fn activity(database: Option<&Store>, log: &str, now: DateTime<Utc>) -> Activity {
    let calls = agent::parse_log(log, agent::RECENT);
    let wanted = agent::wanted(&calls);
    let mut names = Names::default();
    if let Some(database) = database
        && let Ok(connection) = database.connect().await
    {
        let accounts = AccountRepository::new(&connection);
        for id in &wanted.accounts {
            if let Ok(Some(account)) = accounts.get(AccountId::new(*id)).await {
                names.accounts.insert(*id, account.address.address.clone());
            }
        }
        let mailboxes = MailboxRepository::new(&connection);
        for id in &wanted.mailboxes {
            if let Ok(Some(mailbox)) = mailboxes.get(MailboxId::new(*id)).await {
                names.mailboxes.insert(*id, mailbox.path);
            }
        }
        let messages = MessageRepository::new(&connection);
        for id in &wanted.messages {
            if let Ok(Some(message)) = messages.get(MessageId::new(*id)).await
                && let Some(subject) = message.subject
            {
                names.messages.insert(*id, subject);
            }
        }
    }
    agent::activity(&calls, &names, now)
}

/// [`activity`] over the log beside `store`, as of now.
pub async fn activity_beside(database: Option<&Store>, store: &Path) -> Activity {
    let log = log_path(store)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    activity(database, &log, Utc::now()).await
}
