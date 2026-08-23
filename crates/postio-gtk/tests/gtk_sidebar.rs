//! The sidebar on a real display: the folders it lists, the row the selection
//! marks, and the status line.
//!
//! One test function, for the reason `gtk_style.rs` gives. Skips without a
//! display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_gtk::sidebar::{Sidebar, SyncStatus};
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

#[test]
fn the_sidebar_lists_folders_and_says_where_sync_stands() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
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

    // ── an account with nothing in it yet ─────────────────────────────────
    assert!(rows(&sidebar).is_empty(), "no folders, no rows");
    assert_eq!(
        status_text(&sidebar),
        ("offline · imap".into(), "never synced".into()),
        "before the first sync, the sidebar says exactly that"
    );

    // ── the canvas' folders ───────────────────────────────────────────────
    sidebar.set_account("lena@example.com");
    sidebar.set_mailboxes(&canvas_mailboxes(12));
    pump();

    assert_eq!(
        labels(&sidebar),
        [
            ("Inbox".to_string(), Some("12".to_string())),
            ("Flagged".to_string(), Some("3".to_string())),
            ("Drafts".to_string(), Some("2".to_string())),
            ("Sent".to_string(), None),
            ("Archive".to_string(), None),
            ("lkml".to_string(), Some("204".to_string())),
            ("wayland-devel".to_string(), Some("37".to_string())),
        ],
        "the canvas' folders, in the canvas' order, with the canvas' counts"
    );

    // ── selecting ─────────────────────────────────────────────────────────
    let picked: Rc<RefCell<Vec<MailboxId>>> = Rc::new(RefCell::new(Vec::new()));
    sidebar.connect_selected({
        let picked = picked.clone();
        move |id| picked.borrow_mut().push(id)
    });

    sidebar.select(MailboxId::new(1));
    pump();
    assert_eq!(sidebar.selected(), Some(MailboxId::new(1)));
    assert!(
        picked.borrow().is_empty(),
        "restoring a selection is not the user choosing one"
    );

    let inbox = rows(&sidebar)[0].clone();
    assert!(
        inbox.is_selected(),
        "the selected row is the one the CSS marks with the steel edge"
    );

    // A row picked in the list — by pointer or by arrow key — does report.
    let lkml = rows(&sidebar)[5].clone();
    let list: gtk::ListBox = lkml.parent().and_then(|p| p.downcast().ok()).unwrap();
    list.select_row(Some(&lkml));
    pump();
    assert_eq!(*picked.borrow(), [MailboxId::new(6)]);
    assert_eq!(sidebar.selected(), Some(MailboxId::new(6)));
    assert!(
        !inbox.is_selected(),
        "the two blocks are one list: selecting in either clears the other"
    );

    // ── counts update live, without disturbing anything ───────────────────
    let before: Vec<gtk::ListBoxRow> = rows(&sidebar);
    sidebar.set_mailboxes(&canvas_mailboxes(13));
    pump();

    assert_eq!(
        labels(&sidebar)[0],
        ("Inbox".to_string(), Some("13".to_string())),
        "a flag change should reach the count"
    );
    assert_eq!(
        rows(&sidebar),
        before,
        "the rows are updated in place, not rebuilt — rebuilding would drop \
         the selection and the keyboard focus"
    );
    assert_eq!(
        sidebar.selected(),
        Some(MailboxId::new(6)),
        "and the selection survives"
    );
    assert_eq!(
        *picked.borrow(),
        [MailboxId::new(6)],
        "restoring it must not look like a second click"
    );

    // An inbox with nothing unread shows no number at all.
    sidebar.set_mailboxes(&canvas_mailboxes(0));
    pump();
    assert_eq!(labels(&sidebar)[0], ("Inbox".to_string(), None));

    // ── the status line ───────────────────────────────────────────────────
    let now = Instant::now();
    sidebar.set_status(SyncStatus {
        state: ConnectionState::Online,
        last_sync: now.checked_sub(Duration::from_secs(12)),
        ..SyncStatus::default()
    });
    pump();
    assert_eq!(
        status_text(&sidebar),
        ("idle · imap".into(), "last sync 12s".into())
    );
    assert!(!status_labels(&sidebar)[0].has_css_class("error"));

    sidebar.set_status(SyncStatus {
        state: ConnectionState::Failing,
        last_sync: now.checked_sub(Duration::from_secs(12)),
        detail: Some("app-specific password rejected".into()),
        ..SyncStatus::default()
    });
    pump();
    assert_eq!(
        status_text(&sidebar),
        (
            "error · imap".into(),
            "app-specific password rejected".into()
        ),
        "an error has to say what to do about it"
    );
    assert!(
        status_labels(&sidebar)[0].has_css_class("error"),
        "and it is the one thing in this sidebar that is allowed to shout"
    );

    window.destroy();
}

/// Canvas 1b's account, with the inbox's unread count as a parameter.
fn canvas_mailboxes(unread: u32) -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let folder = |id: i64, path: &str, role, (total, unread, flagged)| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total,
            unread,
            flagged,
        };
        mailbox
    };
    vec![
        folder(1, "INBOX", MailboxRole::Inbox, (940, unread, 3)),
        folder(2, "Flagged", MailboxRole::Flagged, (940, unread, 3)),
        folder(3, "Drafts", MailboxRole::Drafts, (2, 0, 0)),
        folder(4, "Sent", MailboxRole::Sent, (4021, 0, 0)),
        folder(5, "Archive", MailboxRole::Archive, (38122, 0, 0)),
        folder(6, "lkml", MailboxRole::Regular, (9004, 204, 0)),
        folder(7, "wayland-devel", MailboxRole::Regular, (880, 37, 0)),
    ]
}

/// Every folder row, in the order the sidebar draws them.
fn rows(sidebar: &Sidebar) -> Vec<gtk::ListBoxRow> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder")
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect()
}

/// Each row's name and the count beside it, when it shows one.
fn labels(sidebar: &Sidebar) -> Vec<(String, Option<String>)> {
    rows(sidebar)
        .iter()
        .map(|row| {
            let line = row.child().unwrap();
            let name: gtk::Label = line.first_child().unwrap().downcast().unwrap();
            let count: gtk::Label = name.next_sibling().unwrap().downcast().unwrap();
            (
                name.text().to_string(),
                count.is_visible().then(|| count.text().to_string()),
            )
        })
        .collect()
}

fn status_labels(sidebar: &Sidebar) -> Vec<gtk::Label> {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-status")
        .into_iter()
        .filter_map(|w| w.downcast().ok())
        .collect()
}

fn status_text(sidebar: &Sidebar) -> (String, String) {
    let labels = status_labels(sidebar);
    (labels[0].text().to_string(), labels[1].text().to_string())
}

/// Every widget in the tree carrying `class`, depth first.
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

fn pump() {
    for _ in 0..80 {
        glib::MainContext::default().iteration(false);
    }
}
