//! What agents did, read for Settings (ADR 0010 Q6, #1832): `postio-mcp` logs ids only, so this
//! resolves them to the rows they name.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_storage::Store;
use postio_storage::repository::{AccountRepository, MailboxRepository, MessageRepository};
use postio_ui::agent_access::{self as agent, Activity, Names};

/// The access log `postio-mcp` writes beside the store, or `None` for an in-memory store.
pub fn log_path(store: &Path) -> Option<PathBuf> {
    store
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|dir| dir.join("mcp-access.log"))
}

/// What an agent's client should launch to reach this Postio, or `None` when there is nothing to
/// name. Inside a Flatpak it is the `flatpak run` command (docs/mcp.md), not the sandbox's path.
pub fn entry_point(
    exe: &Path,
    flatpak_id: Option<&str>,
    exists: impl Fn(&Path) -> bool,
) -> Option<String> {
    if let Some(id) = flatpak_id.filter(|id| !id.is_empty()) {
        return Some(format!("flatpak run --command=postio-mcp {id}"));
    }
    let beside = exe.parent()?.join("postio-mcp");
    exists(&beside).then(|| beside.display().to_string())
}

/// The recent calls in `log` as of `now`, ids resolved in `database`. A deleted row is left
/// unnamed.
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

pub async fn activity_beside(database: Option<&Store>, store: &Path) -> Activity {
    let log = log_path(store)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    activity(database, &log, Utc::now()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_a_flatpak_the_agent_is_pointed_at_flatpak_run() {
        assert_eq!(
            entry_point(
                Path::new("/app/bin/postio"),
                Some("dev.postio.Postio"),
                |_| true
            ),
            Some("flatpak run --command=postio-mcp dev.postio.Postio".to_owned())
        );
    }

    #[test]
    fn otherwise_it_is_the_server_beside_the_app_when_there_is_one() {
        assert_eq!(
            entry_point(Path::new("/opt/postio/postio"), None, |path| path
                == Path::new("/opt/postio/postio-mcp")),
            Some("/opt/postio/postio-mcp".to_owned())
        );
        assert_eq!(
            entry_point(Path::new("/opt/postio/postio"), None, |_| false),
            None,
            "a build with no server beside it points at nothing"
        );
    }

    #[test]
    fn the_log_is_beside_the_store_and_an_in_memory_store_has_none() {
        assert_eq!(
            log_path(Path::new("/home/ada/.local/share/postio/postio.db")),
            Some(PathBuf::from(
                "/home/ada/.local/share/postio/mcp-access.log"
            ))
        );
        assert_eq!(log_path(Path::new("postio.db")), None);
    }
}
