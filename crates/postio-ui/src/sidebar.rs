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

use postio_model::{AccountId, Mailbox, MailboxCounts, MailboxRole};

/// Where a role sits in the sidebar, or `None` for an ordinary folder.
///
/// The canvas' order — Inbox, Flagged, Drafts, Sent, Archive — with the two
/// folders it does not happen to draw after them. Snoozed joins right after
/// Flagged: the same client-only, no-`SPECIAL-USE` shape, and the same kind
/// of "things you will come back to soon" list.
///
/// The Outbox sits between Drafts and Sent, which is the order the column
/// reads in: what you are still writing, what is on its way, what has gone.
/// Its place is fixed rather than earned, because the row is hidden when the
/// Outbox is empty and a position that moved would reorder its neighbours
/// every time a message was sent.
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

/// How many messages each of the sidebar's views holds.
///
/// Gathered by the caller because the three come from different places:
/// `flagged` and `snoozed` are sums over the account's folders, which the
/// store's cached counts already have, and `outbox` is a question about draft
/// state that only a query can answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewCounts {
    /// Everything flagged in the account, wherever it is filed.
    pub flagged: u32,
    /// Everything currently snoozed.
    pub snoozed: u32,
    /// Drafts whose send is under way.
    pub outbox: u32,
}

/// Whether `mailbox` is a view over messages filed elsewhere rather than a
/// folder on the server.
///
/// A view is unpersisted by construction — it has no row, because there is
/// nothing to store — so an unassigned id is what says so. Every mailbox the
/// sidebar is handed otherwise comes from the store and has one.
///
/// This replaces the negative-id sentinels the GTK feed used to invent
/// (`MailboxId::new(-1)` and `-2`). A sentinel is a value that means something
/// only to whoever remembers it, and the frontend that did not remember —
/// macOS — simply never had these rows.
pub fn is_view(mailbox: &Mailbox) -> bool {
    !mailbox.id.is_assigned()
}

/// The view rows this account's sidebar draws, in no particular order —
/// [`sections`] places them.
///
/// # Why this is here and not in a widget
///
/// Every frontend needs the same answer, and the one that had to invent it
/// locally did not: `Flagged` and `Snoozed` were built inside
/// `postio-gtk::feed`, so the macOS sidebar has never had either row. Building
/// them in the toolkit-free layer both frontends already consume is what makes
/// "the same account draws the same rows" true rather than aspirational.
///
/// # What is invented, and what is not
///
/// `Snoozed` and `Outbox` are always Postio's own: no `SPECIAL-USE` attribute
/// names either, so no server can advertise one and `MailboxRole::kind`
/// answers `View` for both.
///
/// `Flagged` is the awkward one and the reason this takes the account's
/// folders. RFC 6154 *does* define `\Flagged`, so a server can have a real
/// one — Gmail's "Starred" — and inventing a second row beside it would draw
/// the word twice. Worse, a view's empty path sorts before any real path, so
/// the invented row would win [`primary_within`] and the user's actual folder
/// would be demoted to an ordinary row underneath it.
///
/// The `Outbox` row is absent when it holds nothing, which is its ordinary
/// state (spec 003 FR-012).
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

/// One view row: a query wearing a folder's clothes.
///
/// `path` is empty because there is nothing to `SELECT`, the id is left
/// unassigned because there is no row, and `last_synced_at` stays `None`
/// because a question is never out of date.
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
    };
    row
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

// ── What a row is called, and the number beside it ──────────────────────────
//
// Both moved out of `postio-gtk::sidebar` by spec 003, for the reason
// `role_order` and `sections` moved in #1155: they are product decisions, not
// widget details, and the frontend that had to re-derive them did not. The
// FFI sent `mailbox.name` raw, which is empty for a view row — so even once
// Flagged and Snoozed crossed the boundary, macOS had two rows with no label.

///
/// Straight off the canvas: Inbox 12 unread, Flagged 3 flagged, Drafts 2 in
/// total, and nothing at all beside Sent or Archive. A count of zero is not
/// drawn — an empty column is quieter than a row of noughts.
pub fn count_for(mailbox: &Mailbox) -> Option<u32> {
    let counts = &mailbox.counts;
    let count = match mailbox.role {
        // A draft you have not finished is not "unread".
        MailboxRole::Drafts => counts.total,
        MailboxRole::Flagged => counts.flagged,
        MailboxRole::Snoozed => counts.snoozed,
        // How many are on their way. The row is hidden entirely when this is
        // zero, which is its ordinary state -- see spec 003 FR-012.
        MailboxRole::Outbox => counts.total,
        // Nothing arrives in these unread, so a count would only ever be
        // "how much have you kept", which is not a thing to nag about.
        MailboxRole::Sent | MailboxRole::Archive | MailboxRole::Trash | MailboxRole::Junk => 0,
        MailboxRole::Inbox | MailboxRole::Regular => counts.unread,
    };
    (count > 0).then_some(count)
}

/// What a folder is called in the sidebar.
///
/// The special-use folders get the name Postio uses for the role, not the one
/// the server happens to have picked: an iCloud account calls its archive
/// "Archive" but its junk folder "Junk E-mail", and the sidebar is not the
/// place to learn that.
///
/// Public because the list pane's header names the same folder, and two
/// places calling one mailbox by two names is exactly the vocabulary drift
/// this function exists to prevent.
pub fn display_name(mailbox: &Mailbox, among: &[Mailbox]) -> String {
    if !primary_within(mailbox, among) {
        // The role's *twin* (#501): a second folder the server reports with
        // the same role. It renders as an ordinary folder, and an ordinary
        // folder is called what the server calls it — the role name belongs
        // to exactly one row, or the sidebar reads `Sent, Sent`.
        return mailbox.name.clone();
    }
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_string(),
        MailboxRole::Flagged => "Flagged".to_string(),
        MailboxRole::Snoozed => "Snoozed".to_string(),
        MailboxRole::Drafts => "Drafts".to_string(),
        MailboxRole::Outbox => "Outbox".to_string(),
        MailboxRole::Sent => "Sent".to_string(),
        MailboxRole::Archive => "Archive".to_string(),
        MailboxRole::Junk => "Junk".to_string(),
        MailboxRole::Trash => "Trash".to_string(),
        MailboxRole::Regular => mailbox.name.clone(),
    }
}

#[cfg(test)]
mod tests {

    // ── The view rows (spec 003, US4) ────────────────────────────────────

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
                is_view(row),
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
        // RFC 6154 defines `\Flagged`, so a server can have a real one --
        // Gmail's "Starred". Synthesising a second row beside it would draw
        // "Flagged, Flagged", and because a view's empty path sorts first the
        // *synthetic* one would win `primary_within` and the real folder would
        // be demoted to an ordinary row. The account's own mail would then be
        // one click further away than on an account whose server has nothing.
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
        // Even handed a folder a careless server called "Snoozed": a role is
        // resolved from `SPECIAL-USE`, and there is no attribute for this.
        let folders = vec![folder(3, "Snoozed", MailboxRole::Regular)];

        let roles: Vec<MailboxRole> = view_rows(
            account,
            &folders,
            ViewCounts {
                flagged: 0,
                snoozed: 0,
                outbox: 0,
            },
        )
        .into_iter()
        .map(|row| row.role)
        .collect();

        assert_eq!(roles, vec![MailboxRole::Flagged, MailboxRole::Snoozed]);
    }

    #[test]
    fn the_view_rows_take_their_place_in_the_shared_order() {
        // Built here *and* ordered here: a frontend appending them to the end
        // of the list is how the two frontends came to disagree.
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
        // Where a message on its way belongs in the reading of the column:
        // after what you are still writing, before what has gone. Its
        // position is fixed so that appearing and disappearing -- it is
        // hidden when empty -- never reorders the rows around it.
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
