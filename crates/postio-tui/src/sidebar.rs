//! The sidebar's rows: folders, views and saved searches.
//!
//! What a sidebar holds, in what order, and what each row is called is
//! `postio_ui::sidebar`'s -- the same answer the desktop and macOS sidebars
//! get (`sections`, `view_rows`, `display_name`, `count_for`). This only lays
//! those out as lines, one account after another, and remembers which list
//! each line opens.

use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{Account, AccountId, ListScope};
use postio_ui::sidebar::{ViewCounts, count_for, display_name, is_view, sections, view_rows};
use postio_ui::terminal::SafeText;

/// What the sidebar is built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contents {
    /// Every account, in the sidebar's order.
    pub accounts: Vec<Account>,
    /// Every account's folders.
    pub folders: Vec<Mailbox>,
    /// Each account's view counts.
    pub counts: Vec<(AccountId, ViewCounts)>,
    /// The saved searches pinned to the sidebar.
    pub saved: Vec<Saved>,
}

/// A saved search from `config.toml`'s `[filters]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    /// What the sidebar calls it.
    pub name: String,
    /// The query it runs.
    pub query: String,
}

/// One line of the sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// What it says.
    pub label: SafeText,
    /// The number beside it, when there is one worth drawing.
    pub count: Option<u32>,
    /// The list it opens; `None` for a heading.
    pub opens: Option<ListScope>,
    /// The saved search it runs, for a saved search's line.
    pub searches: Option<String>,
    /// Whether it is a heading rather than a row.
    pub heading: bool,
}

/// The lines for `contents`, top to bottom.
pub fn lines(contents: &Contents) -> Vec<Line> {
    let mut lines = Vec::new();
    for account in &contents.accounts {
        // By address, as the desktop sidebar heads its folders.
        lines.push(heading(&account.address.address));
        let counts = contents
            .counts
            .iter()
            .find(|(id, _)| *id == account.id)
            .map(|(_, counts)| *counts)
            .unwrap_or_default();
        let mut folders: Vec<Mailbox> = contents
            .folders
            .iter()
            .filter(|folder| folder.account_id == account.id)
            .cloned()
            .collect();
        folders.extend(view_rows(account.id, &folders, counts));
        let (special, ordinary) = sections(&folders);
        for mailbox in special.iter().chain(&ordinary) {
            lines.push(Line {
                label: SafeText::new(&display_name(mailbox, &folders)),
                count: count_for(mailbox),
                opens: Some(opens(account.id, mailbox)),
                searches: None,
                heading: false,
            });
        }
    }
    if !contents.saved.is_empty() {
        lines.push(heading("Saved searches"));
        for saved in &contents.saved {
            lines.push(Line {
                label: SafeText::new(&saved.name),
                count: None,
                opens: None,
                searches: Some(saved.query.clone()),
                heading: false,
            });
        }
    }
    lines
}

fn heading(text: &str) -> Line {
    Line {
        label: SafeText::new(text),
        count: None,
        opens: None,
        searches: None,
        heading: true,
    }
}

/// The list a row opens: a folder by its id, a view by what it asks.
fn opens(account: AccountId, mailbox: &Mailbox) -> ListScope {
    if !is_view(mailbox) {
        return ListScope::Mailbox(mailbox.id);
    }
    match mailbox.role {
        MailboxRole::Snoozed => ListScope::Snoozed(account),
        MailboxRole::Outbox => ListScope::Outbox(account),
        _ => ListScope::Flagged(account),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::{EmailAddress, MailboxId};

    fn account(id: i64, name: &str) -> Account {
        let mut account = Account::new(
            name,
            EmailAddress::new(None::<String>, format!("{name}@example.com")),
        );
        account.id = AccountId::new(id);
        account.enabled = true;
        account
    }

    fn folder(id: i64, account: i64, name: &str, role: MailboxRole, unread: u32) -> Mailbox {
        let mut folder = Mailbox::new(AccountId::new(account), name, None);
        folder.id = MailboxId::new(id);
        folder.role = role;
        folder.selectable = true;
        folder.counts.unread = unread;
        folder
    }

    fn one_account() -> Contents {
        Contents {
            accounts: vec![account(1, "ada")],
            folders: vec![
                folder(10, 1, "INBOX", MailboxRole::Inbox, 3),
                folder(11, 1, "Archive", MailboxRole::Archive, 0),
                folder(12, 1, "Receipts", MailboxRole::Regular, 0),
            ],
            counts: vec![(
                AccountId::new(1),
                ViewCounts {
                    flagged: 2,
                    ..Default::default()
                },
            )],
            saved: vec![Saved {
                name: "Unread from Ada".into(),
                query: "from:ada is:unread".into(),
            }],
        }
    }

    #[test]
    fn folders_views_and_saved_searches_are_all_there() {
        let lines = lines(&one_account());
        let labels: Vec<&str> = lines.iter().map(|line| line.label.as_str()).collect();
        for wanted in [
            "Inbox",
            "Flagged",
            "Snoozed",
            "Archive",
            "Receipts",
            "Unread from Ada",
        ] {
            assert!(labels.contains(&wanted), "{wanted} missing from {labels:?}");
        }
        let inbox = lines
            .iter()
            .find(|line| line.label.as_str() == "Inbox")
            .unwrap();
        assert_eq!(inbox.opens, Some(ListScope::Mailbox(MailboxId::new(10))));
        assert_eq!(inbox.count, Some(3));
        let flagged = lines
            .iter()
            .find(|line| line.label.as_str() == "Flagged")
            .unwrap();
        assert_eq!(flagged.opens, Some(ListScope::Flagged(AccountId::new(1))));
    }

    #[test]
    fn inbox_comes_before_ordinary_folders() {
        let lines = lines(&one_account());
        let at = |label: &str| {
            lines
                .iter()
                .position(|line| line.label.as_str() == label)
                .unwrap()
        };
        assert!(at("Inbox") < at("Receipts"));
    }

    #[test]
    fn even_one_account_is_headed_by_its_address() {
        let lines = lines(&one_account());
        assert!(lines[0].heading, "{:?}", lines[0]);
        assert_eq!(lines[0].label.as_str(), "ada@example.com");
        assert_eq!(lines[0].opens, None, "a heading opens nothing");
    }

    #[test]
    fn two_accounts_each_get_a_heading() {
        let mut contents = one_account();
        contents.accounts.push(account(2, "bea"));
        contents
            .folders
            .push(folder(20, 2, "INBOX", MailboxRole::Inbox, 0));
        let lines = lines(&contents);
        let headings: Vec<&str> = lines
            .iter()
            .filter(|line| line.heading)
            .map(|line| line.label.as_str())
            .collect();
        assert!(
            headings.iter().any(|heading| heading.contains("ada")),
            "{headings:?}"
        );
        assert!(
            headings.iter().any(|heading| heading.contains("bea")),
            "{headings:?}"
        );
    }
}
