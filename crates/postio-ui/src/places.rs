//! The places Focus can go (mailboxes, folders, labels, Outbox, Filtered), ranked and sectioned
//! with no toolkit;
//! the folders popover and the command bar's "Go to" rows read one rule.

use postio_core::{ActionId, CommandId};
use postio_model::{Label, Mailbox, MailboxId, MailboxRole};

use crate::finder::{Destination, Place, PlaceKind};

pub const OUTBOX: &str = "Outbox";

pub const FILTER_PLACEHOLDER: &str = "Go to folder or label";

pub fn footer(highlighted: Option<&Entry>) -> String {
    let Some(entry) = highlighted else {
        return "Esc close".to_owned();
    };
    let name = &entry.name;
    if entry.command.is_some() || matches!(entry.destination, Destination::Outbox(_)) {
        format!("\u{21b5} open {name} \u{b7} Esc close")
    } else {
        format!("\u{21b5} open {name} \u{b7} Esc close \u{b7} same as in:{name} in the command bar")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    Mailboxes,
    Folders,
    Labels,
}

impl Section {
    pub fn title(self) -> &'static str {
        match self {
            Section::Mailboxes => "Mailboxes",
            Section::Folders => "Folders",
            Section::Labels => "Labels",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Role(MailboxRole),
    Dot(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub section: Section,
    pub rank: usize,
    pub name: String,
    pub count: Option<String>,
    pub go: Option<CommandId>,
    /// Run this command rather than go to `destination`: a place that is a view of its own.
    pub command: Option<CommandId>,
    pub destination: Destination,
    pub mark: Mark,
    /// Why its last sync pass failed (#1731); the row carries a warning and this is its tooltip.
    pub failure: Option<String>,
}

pub fn mailbox_entry(mailbox: &Mailbox) -> Entry {
    let (section, rank) = match mailbox.role {
        MailboxRole::Regular => (Section::Folders, 0),
        role => (Section::Mailboxes, role_rank(role)),
    };
    Entry {
        section,
        rank,
        name: place_name(mailbox),
        count: Some(mailbox.counts.total.to_string()),
        command: None,
        go: go_to(mailbox.role),
        destination: Destination::Mailbox(mailbox.id),
        mark: Mark::Role(mailbox.role),
        failure: None,
    }
}

/// A view over Drafts with no mailbox row of its own; the caller lists it only while `waiting` is
/// more than nothing.
pub fn outbox_entry(account: postio_model::AccountId, waiting: u32) -> Entry {
    Entry {
        section: Section::Mailboxes,
        rank: role_rank(MailboxRole::Outbox),
        name: OUTBOX.to_owned(),
        count: Some(waiting.to_string()),
        command: None,
        go: Some(CommandId::GoToOutbox),
        destination: Destination::Outbox(account),
        mark: Mark::Role(MailboxRole::Outbox),
        failure: None,
    }
}

pub fn label_entry(label: &Label, count: Option<u32>) -> Entry {
    Entry {
        section: Section::Labels,
        rank: 0,
        name: label.name.clone(),
        count: count.map(|count| count.to_string()),
        command: None,
        go: None,
        destination: Destination::Label(label.id),
        mark: Mark::Dot(label.color.clone()),
        failure: None,
    }
}

pub fn filtered_entry(today: u32) -> Entry {
    Entry {
        section: Section::Mailboxes,
        rank: role_rank(MailboxRole::Archive) + 1,
        name: crate::filtered::TITLE.to_owned(),
        count: Some(crate::filtered::today_short(today)),
        go: Some(CommandId::GoToFiltered),
        command: Some(CommandId::GoToFiltered),
        destination: Destination::Search(String::new()),
        mark: Mark::Role(MailboxRole::Archive),
        failure: None,
    }
}

/// A view over mail filed elsewhere, not a folder with an id, so it runs the command its key runs.
pub fn view_entry(role: MailboxRole, count: Option<u32>) -> Entry {
    let (name, command) = match role {
        MailboxRole::Snoozed => ("Snoozed", CommandId::GoToSnoozed),
        _ => ("Flagged", CommandId::GoToFlagged),
    };
    Entry {
        section: Section::Mailboxes,
        rank: role_rank(role),
        name: name.to_owned(),
        count: count.map(|count| count.to_string()),
        go: Some(command),
        command: Some(command),
        destination: Destination::Search(String::new()),
        mark: Mark::Role(role),
        failure: None,
    }
}

pub fn view_entries() -> Vec<Entry> {
    [MailboxRole::Snoozed, MailboxRole::Flagged]
        .into_iter()
        .map(|role| view_entry(role, None))
        .collect()
}

pub fn listed(all: &[Entry], filtered: Option<&Entry>, wanted: &str) -> Vec<Entry> {
    let wanted = wanted.to_lowercase();
    let views: Vec<Entry> = view_entries()
        .into_iter()
        .filter(|view| !all.iter().any(|entry| entry.go == view.go))
        .collect();
    let mut shown: Vec<Entry> = all
        .iter()
        .chain(&views)
        .chain(filtered)
        .filter(|entry| wanted.is_empty() || entry.name.to_lowercase().contains(&wanted))
        .cloned()
        .collect();
    shown.sort_by_key(|entry| (entry.section, entry.rank, entry.name.to_lowercase()));
    shown
}

pub fn mailbox_place(mailbox: &Mailbox) -> Place {
    Place {
        kind: if mailbox.role == MailboxRole::Regular {
            PlaceKind::Folder
        } else {
            PlaceKind::Mailbox
        },
        name: place_name(mailbox),
        count: None,
        go: go_to(mailbox.role).map(ActionId::from),
        destination: Destination::Mailbox(mailbox.id),
    }
}

pub fn label_place(label: &Label) -> Place {
    Place {
        kind: PlaceKind::Label,
        name: label.name.clone(),
        count: None,
        go: None,
        destination: Destination::Label(label.id),
    }
}

pub fn direct_commands() -> Vec<CommandId> {
    [
        MailboxRole::Inbox,
        MailboxRole::Drafts,
        MailboxRole::Outbox,
        MailboxRole::Sent,
        MailboxRole::Archive,
        MailboxRole::Snoozed,
        MailboxRole::Flagged,
        MailboxRole::Junk,
        MailboxRole::Trash,
    ]
    .into_iter()
    .filter_map(go_to)
    .collect()
}

pub fn role_rank(role: MailboxRole) -> usize {
    match role {
        MailboxRole::Inbox => 0,
        MailboxRole::Drafts => 1,
        MailboxRole::Outbox => 1,
        MailboxRole::Sent => 2,
        MailboxRole::Snoozed => 3,
        MailboxRole::Archive => 4,
        _ => 5,
    }
}

pub fn go_to(role: MailboxRole) -> Option<CommandId> {
    Some(match role {
        MailboxRole::Inbox => CommandId::GoToInbox,
        MailboxRole::Drafts => CommandId::GoToDrafts,
        MailboxRole::Sent => CommandId::GoToSent,
        MailboxRole::Archive => CommandId::GoToArchive,
        MailboxRole::Snoozed => CommandId::GoToSnoozed,
        MailboxRole::Flagged => CommandId::GoToFlagged,
        MailboxRole::Outbox => CommandId::GoToOutbox,
        MailboxRole::Junk => CommandId::GoToJunk,
        MailboxRole::Trash => CommandId::GoToTrash,
        _ => return None,
    })
}

pub fn place_name(mailbox: &Mailbox) -> String {
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_owned(),
        _ => mailbox.name.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Focus,
    Inbox(MailboxId),
}

/// `here` is `None` when the list shows something else: the cycle starts over. With one inbox there
/// is only Focus's list.
pub fn next_scope(inboxes: &[MailboxId], here: Option<Scope>) -> Scope {
    if inboxes.len() < 2 {
        return Scope::Focus;
    }
    match here {
        Some(Scope::Inbox(current)) => inboxes
            .iter()
            .position(|inbox| *inbox == current)
            .and_then(|at| inboxes.get(at + 1))
            .map_or(Scope::Focus, |next| Scope::Inbox(*next)),
        Some(Scope::Focus) => Scope::Inbox(inboxes[0]),
        None => Scope::Focus,
    }
}

#[cfg(test)]
mod tests {
    use postio_model::ids::{AccountId, LabelId, MailboxId};

    use super::*;

    fn mailbox(name: &str, role: MailboxRole) -> Mailbox {
        let mut mailbox = Mailbox::new(AccountId::new(1), name, None);
        mailbox.role = role;
        mailbox.id = MailboxId::new(name.len() as i64);
        mailbox
    }

    #[test]
    fn the_views_are_listed_always_and_once() {
        let inbox = mailbox_entry(&mailbox("Inbox", MailboxRole::Inbox));
        let names = |shown: Vec<Entry>| -> Vec<String> {
            shown.into_iter().map(|entry| entry.name).collect()
        };
        let shown = names(listed(std::slice::from_ref(&inbox), None, ""));
        assert_eq!(shown, ["Inbox", "Snoozed", "Flagged"], "by rank");

        let own = mailbox_entry(&mailbox("Starred", MailboxRole::Flagged));
        let shown = names(listed(&[inbox, own], None, ""));
        assert_eq!(
            shown
                .iter()
                .filter(|name| *name == "Flagged" || *name == "Starred")
                .count(),
            1,
            "a server's own Flagged mailbox is the view's row: {shown:?}"
        );
    }

    #[test]
    fn snoozed_and_flagged_are_views_that_run_their_go_to_commands() {
        let views = view_entries();
        let commands: Vec<_> = views.iter().map(|entry| entry.command).collect();
        assert_eq!(
            commands,
            [Some(CommandId::GoToSnoozed), Some(CommandId::GoToFlagged)]
        );
        assert!(views.iter().all(|entry| entry.go == entry.command));
    }

    #[test]
    fn an_inbox_is_called_inbox_whatever_the_server_names_it() {
        assert_eq!(place_name(&mailbox("INBOX", MailboxRole::Inbox)), "Inbox");
        assert_eq!(
            place_name(&mailbox("Receipts", MailboxRole::Regular)),
            "Receipts"
        );
    }

    #[test]
    fn places_list_by_section_then_role_then_name() {
        let mut label = Label::new(AccountId::new(1), "Travel");
        label.id = LabelId::new(1);
        let all = vec![
            label_entry(&label, None),
            mailbox_entry(&mailbox("Receipts", MailboxRole::Regular)),
            mailbox_entry(&mailbox("Archive", MailboxRole::Archive)),
            mailbox_entry(&mailbox("INBOX", MailboxRole::Inbox)),
            outbox_entry(AccountId::new(1), 2),
        ];
        let names = |entries: Vec<Entry>| {
            entries
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(listed(&all, Some(&filtered_entry(3)), "")),
            [
                "Inbox", "Outbox", "Snoozed", "Archive", "Filtered", "Flagged", "Receipts",
                "Travel"
            ]
        );
        assert_eq!(names(listed(&all, None, "REC")), ["Receipts"]);
    }

    #[test]
    fn every_role_with_a_row_has_a_key() {
        let junk = mailbox_entry(&mailbox("Junk", MailboxRole::Junk));
        let trash = mailbox_entry(&mailbox("Trash", MailboxRole::Trash));
        assert_eq!(junk.go, Some(CommandId::GoToJunk));
        assert_eq!(trash.go, Some(CommandId::GoToTrash));
        assert_eq!(
            outbox_entry(AccountId::new(1), 1).go,
            Some(CommandId::GoToOutbox)
        );
        for command in [
            CommandId::GoToOutbox,
            CommandId::GoToJunk,
            CommandId::GoToTrash,
        ] {
            assert!(direct_commands().contains(&command), "{command:?}");
        }
        let keymap = postio_core::Keymap::resolve(&postio_config::KeyBindings::default());
        for entry in [&junk, &trash, &outbox_entry(AccountId::new(1), 1)] {
            let key = entry.go.and_then(|go| crate::hints::key(&keymap, go));
            assert!(key.is_some(), "{} shows no key", entry.name);
        }
    }

    #[test]
    fn the_footer_describes_the_highlighted_row() {
        let receipts = mailbox_entry(&mailbox("Receipts", MailboxRole::Regular));
        let label = {
            let mut label = Label::new(AccountId::new(1), "Travel");
            label.id = LabelId::new(1);
            label_entry(&label, None)
        };
        let outbox = outbox_entry(AccountId::new(1), 1);
        assert!(footer(Some(&receipts)).contains("in:Receipts"));
        assert!(footer(Some(&label)).contains("in:Travel"));
        assert!(footer(Some(&outbox)).contains("Outbox"));
        assert!(!footer(Some(&outbox)).contains("in:Receipts"));
        assert!(footer(None).contains("Esc close"));
    }

    #[test]
    fn filtered_runs_its_command_rather_than_going_to_a_place() {
        let entry = filtered_entry(4);
        assert_eq!(entry.command, Some(CommandId::GoToFiltered));
        assert_eq!(entry.go, Some(CommandId::GoToFiltered));
    }

    #[test]
    fn next_scope_walks_each_inbox_then_back_to_focus() {
        let inboxes = [MailboxId::new(1), MailboxId::new(9)];
        let mut here = Some(Scope::Focus);
        let mut walked = Vec::new();
        for _ in 0..4 {
            let next = next_scope(&inboxes, here);
            walked.push(next);
            here = Some(next);
        }
        assert_eq!(
            walked,
            [
                Scope::Inbox(MailboxId::new(1)),
                Scope::Inbox(MailboxId::new(9)),
                Scope::Focus,
                Scope::Inbox(MailboxId::new(1)),
            ]
        );
    }

    #[test]
    fn next_scope_from_somewhere_else_starts_over_at_focus() {
        let inboxes = [MailboxId::new(1), MailboxId::new(9)];
        assert_eq!(next_scope(&inboxes, None), Scope::Focus);
        assert_eq!(
            next_scope(&inboxes, Some(Scope::Inbox(MailboxId::new(77)))),
            Scope::Focus,
            "an inbox that is not one of the accounts' is somewhere else"
        );
    }

    #[test]
    fn next_scope_with_one_account_is_only_focus() {
        assert_eq!(next_scope(&[], Some(Scope::Focus)), Scope::Focus);
        assert_eq!(
            next_scope(&[MailboxId::new(1)], Some(Scope::Focus)),
            Scope::Focus
        );
    }
}
