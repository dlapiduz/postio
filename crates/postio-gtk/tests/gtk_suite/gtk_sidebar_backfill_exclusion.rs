//! A folder's own context menu can skip or resume background backfill
//! (ADR 0016, #350), on a real display: right-clicking a folder row offers
//! exactly one entry, worded for whichever direction it currently does, and
//! picking it reports the mailbox and the new state.
//!
//! Covers both sidebar sections deliberately: ADR 0016's own motivating
//! example, Junk, is a special-use folder and lives in the section next to
//! Inbox and Sent, not in the ordinary tree with everything else.

use crate::pump;
use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::sidebar::Sidebar;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxRole};

fn two_folders() -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let mut junk = Mailbox::new(account, "Junk", Some('/'));
    junk.id = MailboxId::new(1);
    junk.role = MailboxRole::Junk;
    let mut archive = Mailbox::new(account, "Old mailing list", Some('/'));
    archive.id = MailboxId::new(2);
    archive.backfill_excluded = true;
    vec![junk, archive]
}

fn collect(widget: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    if widget.has_css_class(class) {
        found.push(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        found.extend(collect(&current, class));
        child = current.next_sibling();
    }
    found
}

/// The special-use section's own rows — everything carrying `postio-folder`
/// without also carrying `postio-folder-tree`, which only the ordinary
/// tree's rows add.
fn special_rows(sidebar: &Sidebar) -> Vec<gtk::ListBoxRow> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder")
        .into_iter()
        .filter(|w| !w.has_css_class("postio-folder-tree"))
        .filter_map(|w| w.downcast().ok())
        .collect()
}

fn tree_rows(sidebar: &Sidebar) -> Vec<gtk::ListBoxRow> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder-tree")
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect()
}

pub fn the_menu_offers_one_entry_worded_for_the_current_state() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let sidebar = Sidebar::new();
    let window = gtk::Window::new();
    style::track(&window);
    window.set_child(Some(&sidebar));
    window.set_default_size(212, 700);
    window.present();
    pump();

    sidebar.set_mailboxes(&two_folders());
    pump();
    let junk_rows = special_rows(&sidebar);
    assert_eq!(junk_rows.len(), 1, "Junk is a special-use folder");
    let ordinary_rows = tree_rows(&sidebar);
    assert_eq!(
        ordinary_rows.len(),
        1,
        "the mailing-list archive has no special role"
    );

    let heard: Rc<RefCell<Vec<(MailboxId, bool)>>> = Rc::new(RefCell::new(Vec::new()));
    sidebar.connect_backfill_exclusion_changed({
        let heard = heard.clone();
        move |mailbox, excluded| heard.borrow_mut().push((mailbox, excluded))
    });

    // ── Junk (included by default): the offer is to skip it ────────────
    sidebar.test_open_special_folder_menu(&junk_rows[0]);
    assert!(
        sidebar
            .activate_action("folder.toggle-backfill", None)
            .is_ok(),
        "the toggle entry exists on an included special-use folder"
    );
    sidebar.test_close_folder_menu();

    // ── the excluded folder (Old mailing list): the offer is to resume ──
    sidebar.test_open_ordinary_folder_menu(&ordinary_rows[0]);
    assert!(
        sidebar
            .activate_action("folder.toggle-backfill", None)
            .is_ok(),
        "the toggle entry exists on an excluded ordinary folder too"
    );
    sidebar.test_close_folder_menu();

    assert_eq!(
        *heard.borrow(),
        vec![(MailboxId::new(1), true), (MailboxId::new(2), false)],
        "each row's own current state decides which way its one entry toggles"
    );

    window.destroy();
}
