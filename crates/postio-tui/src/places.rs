//! The places a person can go: folders, views and saved searches.
//!
//! What they are, in what order, and what each is called is
//! `postio_ui::sidebar`'s (`sections`, `view_rows`, `display_name`,
//! `count_for`). The command bar's `#` and `in:` finder, the `alt+1`..`alt+4`
//! saved searches and `g o` read them.

use std::collections::HashSet;

use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{Account, AccountId, ListScope};
use postio_ui::sidebar::{
    ViewCounts, count_for, display_name, folder_rows, is_view, sections, view_rows,
};
use postio_ui::terminal::SafeText;

/// Everything the places are built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Places {
    /// Every account, in the settings' order.
    pub accounts: Vec<Account>,
    /// Every account's folders.
    pub folders: Vec<Mailbox>,
    /// Each account's view counts.
    pub counts: Vec<(AccountId, ViewCounts)>,
    /// The saved searches pinned in `config.toml`.
    pub saved: Vec<Saved>,
    /// Which of Focus's features `config.toml` has in use.
    pub features: Features,
}

/// Which of Focus's features are in use, for the strip's counts: a count
/// shows only while its feature is (C10).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Features {
    /// Whether filtering is on, so "N filtered today" means something.
    pub filtering: bool,
    /// How many digest rules there are.
    pub digest_rules: usize,
}

/// A saved search from `config.toml`'s `[filters]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Saved {
    /// Its key in `[filters]`, which a rename leaves alone.
    pub key: String,
    /// What it is called.
    pub name: String,
    /// The query it runs.
    pub query: String,
}

/// One place a person can go: a folder, a view, or a saved search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// What it is called.
    pub label: SafeText,
    /// The number beside it, when there is one worth drawing.
    pub count: Option<u32>,
    /// The list it opens; `None` for a saved search.
    pub opens: Option<ListScope>,
    /// The query it runs, for a saved search.
    pub searches: Option<String>,
}

/// The places in `places`, in the order the command bar offers them: each
/// account's roles, then its other folders as the server nests them, then
/// the saved searches.
pub fn entries(places: &Places) -> Vec<Place> {
    let mut entries = Vec::new();
    for account in &places.accounts {
        let counts = places
            .counts
            .iter()
            .find(|(id, _)| *id == account.id)
            .map(|(_, counts)| *counts)
            .unwrap_or_default();
        let mut folders: Vec<Mailbox> = places
            .folders
            .iter()
            .filter(|folder| folder.account_id == account.id)
            .cloned()
            .collect();
        folders.extend(view_rows(account.id, &folders, counts));
        let (special, _) = sections(&folders);
        for mailbox in &special {
            entries.push(Place {
                label: SafeText::new(&display_name(mailbox, &folders)),
                count: count_for(mailbox),
                opens: Some(opens(account.id, mailbox)),
                searches: None,
            });
        }
        for folder in folder_rows(&folders, &HashSet::new()) {
            let mailbox = &folder.mailbox;
            // A `\Noselect` container holds folders, not mail.
            if mailbox.selectable {
                entries.push(Place {
                    label: SafeText::new(&display_name(mailbox, &folders)),
                    count: count_for(mailbox),
                    opens: Some(opens(account.id, mailbox)),
                    searches: None,
                });
            }
        }
    }
    for saved in &places.saved {
        entries.push(Place {
            label: SafeText::new(&saved.name),
            count: None,
            opens: None,
            searches: Some(saved.query.clone()),
        });
    }
    entries
}

/// What the strip calls the place `scope` is: `Inbox`, a folder's display
/// name, `Flagged`.
pub fn name_of(places: &Places, scope: ListScope) -> SafeText {
    let name = match scope {
        ListScope::Mailbox(id) => places
            .folders
            .iter()
            .find(|folder| folder.id == id)
            .map(|folder| display_name(folder, &places.folders))
            .unwrap_or_default(),
        ListScope::Flagged(_) => "Flagged".to_owned(),
        ListScope::Snoozed(_) => "Snoozed".to_owned(),
        ListScope::Outbox(_) => "Outbox".to_owned(),
        ListScope::Account(account) => places
            .accounts
            .iter()
            .find(|candidate| candidate.id == account)
            .map(|account| account.address.address.clone())
            .unwrap_or_default(),
        ListScope::Unified | ListScope::Focus(_) => "Inbox".to_owned(),
        ListScope::Thread(_) => "Conversation".to_owned(),
    };
    SafeText::new(&name)
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

    fn one_account() -> Places {
        Places {
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
            features: Features::default(),
            saved: vec![Saved {
                key: "unread-from-ada".into(),
                name: "Unread from Ada".into(),
                query: "from:ada is:unread".into(),
            }],
        }
    }

    #[test]
    fn folders_views_and_saved_searches_are_all_there() {
        let places = entries(&one_account());
        let labels: Vec<&str> = places.iter().map(|place| place.label.as_str()).collect();
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
        let inbox = places
            .iter()
            .find(|place| place.label.as_str() == "Inbox")
            .unwrap();
        assert_eq!(inbox.opens, Some(ListScope::Mailbox(MailboxId::new(10))));
        assert_eq!(inbox.count, Some(3));
        let flagged = places
            .iter()
            .find(|place| place.label.as_str() == "Flagged")
            .unwrap();
        assert_eq!(flagged.opens, Some(ListScope::Flagged(AccountId::new(1))));
        let saved = places.last().unwrap();
        assert_eq!(saved.searches.as_deref(), Some("from:ada is:unread"));
        assert_eq!(saved.opens, None);
    }

    #[test]
    fn inbox_comes_before_ordinary_folders() {
        let places = entries(&one_account());
        let at = |label: &str| {
            places
                .iter()
                .position(|place| place.label.as_str() == label)
                .unwrap()
        };
        assert!(at("Inbox") < at("Receipts"));
    }

    #[test]
    fn a_container_that_holds_no_mail_is_not_a_place() {
        let mut places = one_account();
        let mut container = folder(13, 1, "Projects", MailboxRole::Regular, 0);
        container.selectable = false;
        places.folders.push(container);
        let labels: Vec<String> = entries(&places)
            .iter()
            .map(|place| place.label.as_str().to_owned())
            .collect();
        assert!(
            !labels.iter().any(|label| label == "Projects"),
            "{labels:?}"
        );
    }
}
