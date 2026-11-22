//! Which folders the sidebar draws where, and the rule that keeps a role from appearing twice
//! (#501); shared by both frontends.

use postio_model::{AccountId, Mailbox, MailboxCounts, MailboxRole};

/// Snoozed follows Flagged. The Outbox sits between Drafts and Sent; its place is fixed because the
/// row hides when empty and a moving position would reorder its neighbours.
pub fn role_order(role: MailboxRole) -> Option<u8> {
    match role {
        MailboxRole::Inbox => Some(0),
        MailboxRole::Flagged => Some(1),
        MailboxRole::Snoozed => Some(2),
        MailboxRole::Drafts => Some(3),
        MailboxRole::Outbox => Some(4),
        MailboxRole::Sent => Some(5),
        MailboxRole::Archive => Some(6),
        MailboxRole::Junk => Some(7),
        MailboxRole::Trash => Some(8),
        MailboxRole::Regular => None,
    }
}

/// The same answer as `MailboxRepository::by_role` (first by path), so the folder shown with the
/// role name is the one `a` and `d` file into.
pub fn primary_within(mailbox: &Mailbox, among: &[Mailbox]) -> bool {
    if role_order(mailbox.role).is_none() {
        return true;
    }
    // By path, not id: ids are storage rowids that a fixture never sets.
    !among.iter().any(|other| {
        other.account_id == mailbox.account_id
            && other.role == mailbox.role
            && other.path < mailbox.path
    })
}

/// Gathered by the caller: `flagged` and `snoozed` sum the cached counts, `outbox` needs a
/// draft-state query.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewCounts {
    pub flagged: u32,
    pub snoozed: u32,
    pub outbox: u32,
    /// What the Drafts row should show: everything not in flight. Not
    /// `mailboxes.total_count`, which counts every message row filed there.
    pub drafts: u32,
    pub attention: u32,
}

/// A server may have a real `\Flagged` folder (RFC 6154, Gmail's "Starred"); a synthetic row beside
/// it would draw the word twice and win `primary_within`, since a view's empty path sorts first.
/// `Snoozed` and `Outbox` are always Postio's own. The `Outbox` row is absent when empty.
pub fn view_rows(account: AccountId, folders: &[Mailbox], counts: ViewCounts) -> Vec<Mailbox> {
    let mut rows = Vec::new();

    let server_has_flagged = folders
        .iter()
        .any(|folder| folder.account_id == account && folder.role == MailboxRole::Flagged);
    if !server_has_flagged {
        rows.push(view(account, MailboxRole::Flagged, counts.flagged));
    }
    rows.push(view(account, MailboxRole::Snoozed, counts.snoozed));
    if counts.outbox > 0 {
        rows.push(view(account, MailboxRole::Outbox, counts.outbox));
    }
    rows
}

/// `path` is empty, the id unassigned and `last_synced_at` `None`: a view is a query, not a folder.
fn view(account: AccountId, role: MailboxRole, count: u32) -> Mailbox {
    let mut row = Mailbox::new(account, "", None);
    row.role = role;
    row.selectable = true;
    row.counts = MailboxCounts {
        total: count,
        unread: 0,
        flagged: if role == MailboxRole::Flagged {
            count
        } else {
            0
        },
        snoozed: if role == MailboxRole::Snoozed {
            count
        } else {
            0
        },
        attention: 0,
    };
    row
}

pub fn sections(mailboxes: &[Mailbox]) -> (Vec<Mailbox>, Vec<Mailbox>) {
    let mut special: Vec<Mailbox> = Vec::new();
    let mut ordinary: Vec<Mailbox> = Vec::new();

    // An unselectable folder keeps its row when it holds one: dropping a `\Noselect` container
    // drops its children, which then match no tree predicate.
    let holds_a_folder = |container: &Mailbox| {
        mailboxes
            .iter()
            .any(|other| other.parent_id == Some(container.id))
    };
    for mailbox in mailboxes
        .iter()
        .filter(|m| m.selectable || holds_a_folder(m))
    {
        // One row per role (#501): only the primary gets the role treatment; its twin is an
        // ordinary folder under its server name.
        match role_order(mailbox.role) {
            Some(_) if primary_within(mailbox, mailboxes) => special.push(mailbox.clone()),
            _ => ordinary.push(mailbox.clone()),
        }
    }

    special.sort_by_key(|m| (role_order(m.role).unwrap_or(u8::MAX), m.name.clone()));
    ordinary.sort_by_key(|m| m.path.to_lowercase());
    (special, ordinary)
}

/// Special-use folders get Postio's role name, not the server's ("Junk E-mail"). Public because the
/// list header names the same folder.
pub fn display_name(mailbox: &Mailbox, among: &[Mailbox]) -> String {
    if !primary_within(mailbox, among) {
        // The role's twin (#501) is an ordinary folder under the server's name: the role name
        // belongs to one row.
        return mailbox.name.clone();
    }
    match role_name(mailbox.role) {
        Some(name) => name.to_string(),
        None => mailbox.name.clone(),
    }
}

pub fn path_label(mailbox: &Mailbox, among: &[Mailbox]) -> String {
    let shown = display_name(mailbox, among);
    if shown == mailbox.name {
        mailbox.path.clone()
    } else {
        shown
    }
}

/// `None` for `Regular`. Split out of `display_name` because a role needs a name where there is no
/// folder, e.g. "this account has no Drafts folder".
pub fn role_name(role: MailboxRole) -> Option<&'static str> {
    match role {
        MailboxRole::Inbox => Some("Inbox"),
        MailboxRole::Flagged => Some("Flagged"),
        MailboxRole::Snoozed => Some("Snoozed"),
        MailboxRole::Drafts => Some("Drafts"),
        MailboxRole::Outbox => Some("Outbox"),
        MailboxRole::Sent => Some("Sent"),
        MailboxRole::Archive => Some("Archive"),
        MailboxRole::Junk => Some("Junk"),
        MailboxRole::Trash => Some("Trash"),
        MailboxRole::Regular => None,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_folder_list_names_roles_and_paths() {
        let among = vec![
            folder(1, "INBOX", MailboxRole::Inbox),
            folder(2, "Archive", MailboxRole::Archive),
            folder(3, "Archives", MailboxRole::Archive),
            folder(4, "Work/Receipts", MailboxRole::Regular),
        ];
        let labels: Vec<String> = among.iter().map(|m| path_label(m, &among)).collect();
        assert_eq!(labels, ["Inbox", "Archive", "Archives", "Work/Receipts"]);
    }

    #[test]
    fn a_view_row_is_built_here_rather_than_by_a_frontend() {
        let account = AccountId::new(1);
        let folders = vec![folder(1, "INBOX", MailboxRole::Inbox)];

        let views = view_rows(
            account,
            &folders,
            ViewCounts {
                flagged: 3,
                snoozed: 2,
                outbox: 0,
                drafts: 0,
                attention: 0,
            },
        );

        let roles: Vec<MailboxRole> = views.iter().map(|row| row.role).collect();
        assert_eq!(
            roles,
            vec![MailboxRole::Flagged, MailboxRole::Snoozed],
            "no Outbox: it is hidden when empty (FR-012)"
        );
        for row in &views {
            assert!(
                !row.id.is_assigned(),
                "{:?} has an id, so something will try to SELECT it",
                row.role
            );
            assert!(row.path.is_empty(), "a view has nothing to SELECT");
            assert!(row.selectable, "a view row is one a person can open");
        }
        assert_eq!(views[0].counts.flagged, 3);
        assert_eq!(views[1].counts.snoozed, 2);
    }

    #[test]
    fn the_outbox_row_appears_only_when_it_holds_something() {
        let account = AccountId::new(1);
        let folders = vec![folder(1, "INBOX", MailboxRole::Inbox)];
        let with = |outbox| {
            view_rows(
                account,
                &folders,
                ViewCounts {
                    flagged: 0,
                    snoozed: 0,
                    outbox,
                    drafts: 0,
                    attention: 0,
                },
            )
            .into_iter()
            .map(|row| row.role)
            .collect::<Vec<_>>()
        };

        assert!(
            !with(0).contains(&MailboxRole::Outbox),
            "an empty Outbox is not drawn (FR-012)"
        );
        assert!(with(1).contains(&MailboxRole::Outbox));
    }

    #[test]
    fn no_flagged_view_is_invented_when_the_server_really_has_that_folder() {
        // A server's real `\Flagged` (Gmail's "Starred") must not get a synthetic twin that would
        // win `primary_within`.
        let account = AccountId::new(1);
        let folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox),
            folder(2, "Starred", MailboxRole::Flagged),
        ];

        let roles: Vec<MailboxRole> = view_rows(
            account,
            &folders,
            ViewCounts {
                flagged: 3,
                snoozed: 0,
                outbox: 0,
                drafts: 0,
                attention: 0,
            },
        )
        .into_iter()
        .map(|row| row.role)
        .collect();

        assert_eq!(
            roles,
            vec![MailboxRole::Snoozed],
            "the real Starred folder is the Flagged row; nothing is invented"
        );
    }

    #[test]
    fn snoozed_is_always_invented_because_no_server_can_have_one() {
        let account = AccountId::new(1);
        let folders = vec![folder(3, "Snoozed", MailboxRole::Regular)];

        let roles: Vec<MailboxRole> = view_rows(
            account,
            &folders,
            ViewCounts {
                flagged: 0,
                snoozed: 0,
                outbox: 0,
                drafts: 0,
                attention: 0,
            },
        )
        .into_iter()
        .map(|row| row.role)
        .collect();

        assert_eq!(roles, vec![MailboxRole::Flagged, MailboxRole::Snoozed]);
    }

    #[test]
    fn the_view_rows_take_their_place_in_the_shared_order() {
        let account = AccountId::new(1);
        let mut all = vec![
            folder(1, "INBOX", MailboxRole::Inbox),
            folder(4, "Sent", MailboxRole::Sent),
            folder(5, "Projects", MailboxRole::Regular),
        ];
        all.extend(view_rows(
            account,
            &all.clone(),
            ViewCounts {
                flagged: 1,
                snoozed: 1,
                outbox: 1,
                drafts: 0,
                attention: 0,
            },
        ));

        let (special, ordinary) = sections(&all);
        assert_eq!(
            special.iter().map(|m| m.role).collect::<Vec<_>>(),
            vec![
                MailboxRole::Inbox,
                MailboxRole::Flagged,
                MailboxRole::Snoozed,
                MailboxRole::Outbox,
                MailboxRole::Sent,
            ]
        );
        assert_eq!(ordinary.len(), 1, "Projects is the only ordinary folder");
    }

    #[test]
    fn the_outbox_sits_between_drafts_and_sent() {
        assert_eq!(
            role_order(MailboxRole::Outbox),
            Some(4),
            "the Outbox reads after Drafts and before Sent"
        );
        assert!(role_order(MailboxRole::Drafts) < role_order(MailboxRole::Outbox));
        assert!(role_order(MailboxRole::Outbox) < role_order(MailboxRole::Sent));
    }

    #[test]
    fn every_role_that_gets_a_row_has_a_distinct_place_in_the_order() {
        let mut seen: Vec<u8> = [
            MailboxRole::Inbox,
            MailboxRole::Flagged,
            MailboxRole::Snoozed,
            MailboxRole::Drafts,
            MailboxRole::Outbox,
            MailboxRole::Sent,
            MailboxRole::Archive,
            MailboxRole::Junk,
            MailboxRole::Trash,
        ]
        .into_iter()
        .map(|role| role_order(role).expect("a special row has a place"))
        .collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "two roles share a position: {seen:?}");
        assert_eq!(role_order(MailboxRole::Regular), None);
    }
    use super::*;
    use postio_model::ids::{AccountId, MailboxId};

    fn folder(id: i64, path: &str, role: MailboxRole) -> Mailbox {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.selectable = true;
        mailbox
    }

    fn two_clients() -> Vec<Mailbox> {
        vec![
            folder(1, "Archive", MailboxRole::Archive),
            folder(2, "Archives", MailboxRole::Archive),
            folder(3, "Deleted Messages", MailboxRole::Trash),
            folder(4, "Drafts", MailboxRole::Drafts),
            folder(5, "Garagiste", MailboxRole::Regular),
            folder(6, "INBOX", MailboxRole::Inbox),
            folder(7, "Junk", MailboxRole::Junk),
            folder(8, "Sent", MailboxRole::Sent),
            folder(9, "Sent Messages", MailboxRole::Sent),
            folder(10, "Trash", MailboxRole::Trash),
        ]
    }

    #[test]
    fn the_inbox_comes_first() {
        let (special, _) = sections(&two_clients());
        assert_eq!(
            special.first().map(|m| m.role),
            Some(MailboxRole::Inbox),
            "the sidebar's first row is {:?}",
            special.iter().map(|m| m.path.clone()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_special_section_is_the_canvas_order() {
        let (special, _) = sections(&two_clients());
        let roles: Vec<MailboxRole> = special.iter().map(|m| m.role).collect();
        assert_eq!(
            roles,
            vec![
                MailboxRole::Inbox,
                MailboxRole::Drafts,
                MailboxRole::Sent,
                MailboxRole::Archive,
                MailboxRole::Junk,
                MailboxRole::Trash,
            ]
        );
    }

    #[test]
    fn a_role_gets_one_row_however_many_folders_carry_it() {
        let (special, ordinary) = sections(&two_clients());
        for role in [MailboxRole::Archive, MailboxRole::Sent, MailboxRole::Trash] {
            assert_eq!(
                special.iter().filter(|m| m.role == role).count(),
                1,
                "{role:?} appears more than once in the special section"
            );
        }
        let paths: Vec<&str> = ordinary.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.contains(&"Archives"), "the twin vanished: {paths:?}");
        assert!(
            paths.contains(&"Sent Messages"),
            "the twin vanished: {paths:?}"
        );
        assert!(paths.contains(&"Trash"), "the twin vanished: {paths:?}");
    }

    #[test]
    fn an_unselectable_container_holding_nothing_gets_no_row() {
        let mut mailboxes = two_clients();
        let mut container = folder(11, "Archives/2024", MailboxRole::Regular);
        container.selectable = false;
        mailboxes.push(container);

        let (special, ordinary) = sections(&mailboxes);
        assert!(
            !special
                .iter()
                .chain(&ordinary)
                .any(|m| m.path == "Archives/2024"),
            "a row that cannot be opened and holds nothing wastes a keystroke"
        );
    }

    #[test]
    fn an_unselectable_container_holding_folders_keeps_its_row() {
        // Dropping a container also dropped its children: they matched no tree predicate and were
        // drawn nowhere.
        let mut mailboxes = two_clients();
        let mut container = folder(11, "Archives/2024", MailboxRole::Regular);
        container.selectable = false;
        let mut child = folder(12, "Archives/2024/Q1", MailboxRole::Regular);
        child.parent_id = Some(container.id);
        mailboxes.push(container);
        mailboxes.push(child);

        let (special, ordinary) = sections(&mailboxes);
        let paths: Vec<&str> = special
            .iter()
            .chain(&ordinary)
            .map(|m| m.path.as_str())
            .collect();
        assert!(
            paths.contains(&"Archives/2024"),
            "the container went and took its children's only route with it: {paths:?}"
        );
        assert!(paths.contains(&"Archives/2024/Q1"), "{paths:?}");
    }
}
