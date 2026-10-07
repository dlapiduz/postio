//! The places Focus can go -- mailboxes, the person's folders, labels, the
//! Outbox and Filtered -- how they are ranked, filtered and sectioned, with
//! no toolkit in it.
//!
//! The folders popover lists [`Entry`]s from [`listed`]; the command bar's
//! "Go to" rows are the same mailboxes and labels as [`Place`]s. Both read
//! one rule for what a mailbox is called, where it sorts and which key goes
//! straight to it.

use postio_core::{ActionId, CommandId};
use postio_model::{Label, Mailbox, MailboxRole};

use crate::finder::{Destination, Place, PlaceKind};

/// What the Outbox is called, in the list and the header.
pub const OUTBOX: &str = "Outbox";

/// The placeholder over the places' filter.
pub const FILTER_PLACEHOLDER: &str = "Go to folder or label";

/// The line under the places while `highlighted` is the row under the
/// highlight: what Return does, and the words that do the same in the
/// command bar.
pub fn footer(highlighted: Option<&Entry>) -> String {
    let Some(entry) = highlighted else {
        return "Esc close".to_owned();
    };
    let name = &entry.name;
    // The command bar's `in:` completes folders and labels; the views have
    // no `in:` of their own.
    if entry.command.is_some() || matches!(entry.destination, Destination::Outbox(_)) {
        format!("\u{21b5} open {name} \u{b7} Esc close")
    } else {
        format!("\u{21b5} open {name} \u{b7} Esc close \u{b7} same as in:{name} in the command bar")
    }
}

/// Which section a place is listed under, in the order they appear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// Mailboxes with a role, with their direct keys.
    Mailboxes,
    /// The person's own folders.
    Folders,
    /// Labels.
    Labels,
}

impl Section {
    /// The section's heading.
    pub fn title(self) -> &'static str {
        match self {
            Section::Mailboxes => "Mailboxes",
            Section::Folders => "Folders",
            Section::Labels => "Labels",
        }
    }
}

/// What sits before a place's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    /// A mailbox's mark, by its role.
    Role(MailboxRole),
    /// A label's dot, in its stored colour if it has one.
    Dot(Option<String>),
}

/// One place the list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its section.
    pub section: Section,
    /// Where it sorts within its section: a mailbox's role order.
    pub rank: usize,
    /// Its name.
    pub name: String,
    /// What it says on the right: its count, or "186 today".
    pub count: Option<String>,
    /// The command that goes there directly, whose key its row shows.
    pub go: Option<CommandId>,
    /// Run this command rather than go to [`Self::destination`]: a place
    /// that is a view of its own, like Filtered.
    pub command: Option<CommandId>,
    /// Where it takes the list.
    pub destination: Destination,
    /// How it is marked.
    pub mark: Mark,
}

/// The entry for a mailbox: a folder of the person's own or one with a role.
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
    }
}

/// The entry for an account's Outbox, which the caller lists only while
/// `waiting` is more than nothing: a view over Drafts, with no mailbox row
/// of its own.
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
    }
}

/// The entry for a label.
pub fn label_entry(label: &Label) -> Entry {
    Entry {
        section: Section::Labels,
        rank: 0,
        name: label.name.clone(),
        count: None,
        command: None,
        go: None,
        destination: Destination::Label(label.id),
        mark: Mark::Dot(label.color.clone()),
    }
}

/// The entry for Filtered, listed while filtering is on, with how many it
/// held back today. It runs `GoToFiltered` rather than going to a place.
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
    }
}

/// The entries for Snoozed and Flagged: views over mail filed elsewhere, not
/// folders with an id of their own, so they are listed always, whatever the
/// accounts' mailboxes say. Each runs the command its key runs, which lists
/// the same cross-account view.
pub fn view_entries() -> Vec<Entry> {
    [
        (MailboxRole::Snoozed, "Snoozed", CommandId::GoToSnoozed),
        (MailboxRole::Flagged, "Flagged", CommandId::GoToFlagged),
    ]
    .into_iter()
    .map(|(role, name, command)| Entry {
        section: Section::Mailboxes,
        rank: role_rank(role),
        name: name.to_owned(),
        count: None,
        go: Some(command),
        command: Some(command),
        destination: Destination::Search(String::new()),
        mark: Mark::Role(role),
    })
    .collect()
}

/// The places to list: every entry, Snoozed and Flagged (the views, unless a
/// server's own mailbox with that role already has the row), and Filtered
/// while it is one, whose name contains `wanted`, case aside, by section,
/// rank, then name.
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

/// The command bar's place for a mailbox.
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

/// The command bar's place for a label.
pub fn label_place(label: &Label) -> Place {
    Place {
        kind: PlaceKind::Label,
        name: label.name.clone(),
        count: None,
        go: None,
        destination: Destination::Label(label.id),
    }
}

/// The mailboxes that have a key of their own, in the order their commands
/// are listed.
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

/// Where a mailbox of `role` sorts: the order the places are listed in.
pub fn role_rank(role: MailboxRole) -> usize {
    match role {
        MailboxRole::Inbox => 0,
        MailboxRole::Drafts => 1,
        // What Drafts sent on its way, listed under it.
        MailboxRole::Outbox => 1,
        MailboxRole::Sent => 2,
        MailboxRole::Snoozed => 3,
        MailboxRole::Archive => 4,
        _ => 5,
    }
}

/// The command that goes to a mailbox of `role` directly.
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

/// What a mailbox is called here: "Inbox" for an inbox, whatever the server
/// names it ("INBOX"), and its own name otherwise.
pub fn place_name(mailbox: &Mailbox) -> String {
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_owned(),
        _ => mailbox.name.clone(),
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
            label_entry(&label),
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

    /// Every mailbox the popover lists shows the key that goes there, the
    /// Outbox, Junk and Trash included.
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

    /// The line under the places says what Return does on the row the
    /// highlight is on, in that row's own words.
    #[test]
    fn the_footer_describes_the_highlighted_row() {
        let receipts = mailbox_entry(&mailbox("Receipts", MailboxRole::Regular));
        let label = {
            let mut label = Label::new(AccountId::new(1), "Travel");
            label.id = LabelId::new(1);
            label_entry(&label)
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
}
