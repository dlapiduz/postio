//! Which folders the sidebar draws where.
//!
//! The canvas' order — Inbox, Flagged, Drafts, Sent, Archive — and the rule
//! that keeps a role from appearing twice. Both frontends draw a sidebar and
//! neither should decide this for itself: a second ordering is a second
//! answer to "where is my inbox", and the duplicate rule in particular took a
//! bug report to find (#501).
//!
//! It lived in `postio-gtk::sidebar` until #1155, which is where the macOS
//! sidebar could not reach it — so that one sorted alphabetically and drew
//! `Archive, Archive … Sent, Sent … Trash, Trash`, exactly the failure #501
//! had already fixed on the other platform. Nothing here touches a toolkit:
//! `Vec<Mailbox>` in, `Vec<Mailbox>` out.

use postio_model::{Mailbox, MailboxRole};

/// Where a role sits in the sidebar, or `None` for an ordinary folder.
///
/// The canvas' order — Inbox, Flagged, Drafts, Sent, Archive — with the two
/// folders it does not happen to draw after them. Snoozed joins right after
/// Flagged: the same client-only, no-`SPECIAL-USE` shape, and the same kind
/// of "things you will come back to soon" list.
pub fn role_order(role: MailboxRole) -> Option<u8> {
    match role {
        MailboxRole::Inbox => Some(0),
        MailboxRole::Flagged => Some(1),
        MailboxRole::Snoozed => Some(2),
        MailboxRole::Drafts => Some(3),
        MailboxRole::Sent => Some(4),
        MailboxRole::Archive => Some(5),
        MailboxRole::Junk => Some(6),
        MailboxRole::Trash => Some(7),
        MailboxRole::Regular => None,
    }
}

/// Whether `mailbox` is the folder its role actually routes to, among its
/// account's mailboxes.
///
/// The same answer `MailboxRepository::by_role` gives — first by path — so
/// the folder the sidebar crowns with the role name is the folder `a`
/// archives into and `d` deletes into. Two rules diverging here is how a
/// sidebar says `Archive` over one folder while the key files into another.
///
/// A role-less mailbox is trivially primary: there is nothing to be the
/// twin of.
pub fn primary_within(mailbox: &Mailbox, among: &[Mailbox]) -> bool {
    if role_order(mailbox.role).is_none() {
        return true;
    }
    // Identity by path, not id: paths are unique within an account and are
    // what `by_role` orders by, while ids are storage rowids a fixture never
    // sets.
    !among.iter().any(|other| {
        other.account_id == mailbox.account_id
            && other.role == mailbox.role
            && other.path < mailbox.path
    })
}

/// Split the mailboxes into the two sections the canvas draws, each in order.
///
/// Unselectable folders — `\Noselect` containers that exist only to hold a
/// hierarchy — are dropped: a row you cannot open is a row that wastes a
/// keystroke.
pub fn sections(mailboxes: &[Mailbox]) -> (Vec<Mailbox>, Vec<Mailbox>) {
    let mut special: Vec<Mailbox> = Vec::new();
    let mut ordinary: Vec<Mailbox> = Vec::new();

    for mailbox in mailboxes.iter().filter(|m| m.selectable) {
        // One row per role (#501): an account that has passed through more
        // than one client holds two folders per role, and a special section
        // that renamed both to the role drew `Sent, Sent, Archive, Archive`.
        // Only the primary — the mailbox actions route to — gets the role
        // treatment; its twin is an ordinary folder under its server name.
        match role_order(mailbox.role) {
            Some(_) if primary_within(mailbox, mailboxes) => special.push(mailbox.clone()),
            _ => ordinary.push(mailbox.clone()),
        }
    }

    special.sort_by_key(|m| (role_order(m.role).unwrap_or(u8::MAX), m.name.clone()));
    ordinary.sort_by_key(|m| m.path.to_lowercase());
    (special, ordinary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::ids::{AccountId, MailboxId};

    fn folder(id: i64, path: &str, role: MailboxRole) -> Mailbox {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.selectable = true;
        mailbox
    }

    /// An account that has passed through more than one client: two folders
    /// per role, which is the shape #501 was reported from and the shape the
    /// macOS sidebar drew as `Archive, Archive … Sent, Sent`.
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
        // The folder a mail client opens on. Sorted by name it is sixth,
        // below a user folder called Garagiste, which is what the macOS
        // sidebar did before #1155.
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
        // #501. Two clients leave two folders per role, and a section that
        // gave both the role's treatment drew each role twice with no way to
        // tell them apart.
        let (special, ordinary) = sections(&two_clients());
        for role in [MailboxRole::Archive, MailboxRole::Sent, MailboxRole::Trash] {
            assert_eq!(
                special.iter().filter(|m| m.role == role).count(),
                1,
                "{role:?} appears more than once in the special section"
            );
        }
        // ...and the twin is not dropped: it is still reachable, under the
        // name the server gave it.
        let paths: Vec<&str> = ordinary.iter().map(|m| m.path.as_str()).collect();
        assert!(paths.contains(&"Archives"), "the twin vanished: {paths:?}");
        assert!(
            paths.contains(&"Sent Messages"),
            "the twin vanished: {paths:?}"
        );
        assert!(paths.contains(&"Trash"), "the twin vanished: {paths:?}");
    }

    #[test]
    fn an_unselectable_container_gets_no_row() {
        // A `\Noselect` folder holds a hierarchy and opens onto nothing.
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
            "a row that cannot be opened wastes a keystroke"
        );
    }
}
