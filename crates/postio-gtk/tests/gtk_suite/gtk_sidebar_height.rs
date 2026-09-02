//! A sidebar full of folders still fits in the window.
//!
//! `postio-qhz.4`: a live run with fifteen folders logged
//!
//! ```text
//! AdwToastOverlay ... exceeds PostioWindow height: requested 949px, 700 available
//! ```
//!
//! The folder lists were `GtkListBox`es in a plain box with no scroller, so
//! the sidebar's height was however many folders the account had. GTK
//! answered by clipping: four folders were unreachable with no scrollbar to
//! say so, and the sync status line — which is last in the column — was
//! pushed off the bottom of the window entirely.
//!
//! One test function per binary, for the reason `gtk_style.rs` gives: GTK is
//! initialised once and the tests in one binary run on separate threads, so a
//! second `#[test]` here would quietly skip rather than run. That is not
//! hypothetical — this test was written into `gtk_sidebar.rs` first and
//! passed against the unfixed code by never executing.
//!
//! Skips without a display. Nothing here touches the network.

use std::time::Instant;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_gtk::sidebar::{Sidebar, SyncStatus};
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// The shortest window Postio supports, from `crate::shell`'s narrow
/// breakpoint. The sidebar has to fit inside it whatever the account holds.
const SHORTEST_WINDOW: i32 = 600;

pub fn a_sidebar_full_of_folders_still_fits_in_the_window() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let sidebar = Sidebar::new();
    let window = gtk::Window::new();
    window.set_child(Some(&sidebar));
    window.set_default_size(212, SHORTEST_WINDOW);
    window.present();
    pump();

    // An ordinary account, not a large one. `postio-qhz.4` came from a live
    // run with fifteen folders: the lists had no scroller, so the sidebar
    // asked for 949px of a 700px window and GTK answered by clipping — four
    // folders unreachable with no scrollbar to say so, and the sync status
    // pushed off the bottom entirely.
    let mut folders = canvas_mailboxes();
    for index in 0..20 {
        let mut folder = Mailbox::new(AccountId::new(1), format!("list-{index}"), Some('/'));
        folder.id = MailboxId::new(100 + index);
        folder.role = MailboxRole::Regular;
        folder.counts = MailboxCounts {
            total: 400,
            unread: index as u32,
            flagged: 0,
            snoozed: 0,
        };
        folders.push(folder);
    }
    sidebar.set_mailboxes(&folders);
    sidebar.set_status(SyncStatus {
        state: ConnectionState::Online,
        last_sync: Some(Instant::now()),
        ..SyncStatus::default()
    });
    pump();

    let (minimum, _, _, _) = sidebar.measure(gtk::Orientation::Vertical, 212);
    assert!(
        minimum <= SHORTEST_WINDOW,
        "a sidebar with {} folders insists on {minimum}px in a {SHORTEST_WINDOW}px \
         window. Something in it will not shrink, so GTK clips it — which \
         costs the folders past the fold *and* the sync status line, with no \
         scrollbar to say either is there.",
        folders.len()
    );

    // The status line is the half that goes missing first, because it is
    // last in the column. It has to be on screen, not merely present.
    let (state, detail) = status_text(&sidebar);
    assert!(
        !state.is_empty() && !detail.is_empty(),
        "the sync status is what answers `is anything happening`, and it has \
         to survive a full folder list: {state:?} / {detail:?}"
    );
    assert!(
        status_labels(&sidebar)
            .iter()
            .all(|label| label.property::<bool>("visible")),
        "the status line is on screen rather than scrolled away with the folders"
    );

    window.destroy();
}

/// The canvas' own special-use folders, before any list folders are added.
fn canvas_mailboxes() -> Vec<Mailbox> {
    [
        ("INBOX", MailboxRole::Inbox),
        ("Drafts", MailboxRole::Drafts),
        ("Sent", MailboxRole::Sent),
        ("Archive", MailboxRole::Archive),
        ("Junk", MailboxRole::Junk),
        ("Trash", MailboxRole::Trash),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (path, role))| {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.id = MailboxId::new(index as i64 + 1);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total: 900,
            unread: 12,
            flagged: 0,
            snoozed: 0,
        };
        mailbox
    })
    .collect()
}

/// The two lines of the sync status, as the widgets hold them.
fn status_text(sidebar: &Sidebar) -> (String, String) {
    let labels = status_labels(sidebar);
    let text = |index: usize| {
        labels
            .get(index)
            .map(|label: &gtk::Label| label.text().to_string())
            .unwrap_or_default()
    };
    (text(0), text(1))
}

fn status_labels(sidebar: &Sidebar) -> Vec<gtk::Label> {
    collect(&sidebar.clone().upcast(), "postio-status")
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Label>().ok())
        .collect()
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
