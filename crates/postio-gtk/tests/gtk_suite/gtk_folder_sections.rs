//! The folder feed can read more than one account's tree. #185.
//!
//! ADR 0005 Q4 wants the sidebar to draw one collapsible section per account.
//! The blocker was never the widget: `Folders` held a single
//! `account: Cell<Option<AccountId>>` and asked its source for exactly one
//! tree, so there was nothing for a section to be *of*. This is that half —
//! the composition-root plumbing the sidebar work sits on.
//!
//! # What it deliberately does not assert
//!
//! How the sidebar *draws* those sections. That is the other half, and it is
//! not written: the safe shape is one `GtkListBox` per account so that
//! `sync_folder_rows` keeps syncing one kind of row by index, and reaching it
//! means turning the sidebar's selection coordination from three hardcoded
//! list boxes into a registry over N. Doing that badly breaks which row an
//! action lands on, which is the failure `/ux-architect` names as the usual
//! one. So the feed lands first and the drawing follows.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use crate::settle;
use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::glib;
use postio_gtk::feed::{Folders, MailboxFuture, MailboxSource};
use postio_gtk::sidebar::Sidebar;
use postio_gtk::{app, fonts, style};
use postio_model::mailbox::{Mailbox, MailboxRole};
use postio_model::{AccountId, MailboxId};

/// Answers with two folders per account, named so the account is legible in
/// the result, and records which accounts were asked about.
struct PerAccount {
    asked: Rc<RefCell<Vec<AccountId>>>,
}

impl MailboxSource for PerAccount {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        self.asked.borrow_mut().push(account);
        let id = account.get();
        let folder = |offset: i64, path: &str, role| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id * 100 + offset);
            mailbox.role = role;
            mailbox
        };
        let folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox),
            folder(2, &format!("Project {id}"), MailboxRole::Regular),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

pub fn the_feed_reads_every_account_it_is_given_and_keeps_their_order() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let asked: Rc<RefCell<Vec<AccountId>>> = Rc::new(RefCell::new(Vec::new()));
    let sidebar = Sidebar::new();
    let folders = Folders::new(
        &sidebar,
        Rc::new(PerAccount {
            asked: Rc::clone(&asked),
        }),
    );

    let work = AccountId::new(1);
    let home = AccountId::new(2);

    // ── one account: exactly the shape it has always had ────────────────
    folders.open(work, "ada@work.example");
    settle();
    assert_eq!(*asked.borrow(), vec![work]);
    assert!(
        folders
            .mailboxes()
            .iter()
            .all(|mailbox| mailbox.account_id == work),
        "opening one account must not start reading others: a store with one \
         account cannot be made to pay for a loop it has no use for"
    );

    // ── sections: every account, in the order given ─────────────────────
    asked.borrow_mut().clear();
    folders.open_sections(&[work, home], work, "ada@work.example");
    settle();

    assert_eq!(
        *asked.borrow(),
        vec![work, home],
        "the order is the caller's, and it has to be: the sidebar keys an \
         account's hue off its position, so a tree that arrived first must \
         not be able to reorder them"
    );

    let read = folders.mailboxes();
    for account in [work, home] {
        assert!(
            read.iter().any(|mailbox| mailbox.account_id == account),
            "account {account:?} contributed no folders, so a section drawn \
             for it would be empty: {read:?}"
        );
    }

    // `Mailbox` carries its own account, which is what lets the sidebar group
    // a flat list back into sections without a second shape to keep in step.
    assert!(
        read.iter().filter(|m| m.account_id == work).count() >= 2
            && read.iter().filter(|m| m.account_id == home).count() >= 2,
        "both trees have to survive the concatenation whole: {read:?}"
    );

    // ── and back: sections are not a one-way door ───────────────────────
    asked.borrow_mut().clear();
    folders.open(home, "ada@home.example");
    settle();
    assert_eq!(*asked.borrow(), vec![home]);
    assert!(
        folders
            .mailboxes()
            .iter()
            .all(|mailbox| mailbox.account_id == home),
        "opening a single account after sections must drop the others, or the \
         sidebar goes on drawing a tree nothing is pointing at"
    );
}
