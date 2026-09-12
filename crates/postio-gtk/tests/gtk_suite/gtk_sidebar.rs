//! The sidebar on a real display: the folders it lists, the row the selection
//! marks, and the status line.
//!
//! One test function, for the reason `gtk_style.rs` gives. Skips without a
//! display. Nothing here touches the network.

use crate::pump;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::ConnectionState;
use postio_gtk::sidebar::SidebarChoice;
use postio_gtk::sidebar::{Sidebar, SyncStatus};
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

pub fn the_sidebar_lists_folders_and_says_where_sync_stands() {
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
        move |choice| match choice {
            SidebarChoice::Folder(id) => picked.borrow_mut().push(id),
            SidebarChoice::View(role) => panic!("a folder was expected, got the {role:?} view"),
        }
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
        state: ConnectionState::Failing {
            reason: postio_core::FailureReason::Auth,
        },
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
            snoozed: 0,
            attention: 0,
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
///
/// By CSS class, not row position: the ordinary section's rows (#324) carry
/// an indent spacer and a disclosure button ahead of the name, which the
/// special-use section's flat rows do not, so `first_child`/`next_sibling`
/// no longer names the same two widgets in both.
fn labels(sidebar: &Sidebar) -> Vec<(String, Option<String>)> {
    rows(sidebar)
        .iter()
        .map(|row| {
            let widget = row.clone().upcast::<gtk::Widget>();
            let name: gtk::Label = collect(&widget, "postio-folder-name")[0]
                .clone()
                .downcast()
                .unwrap();
            let count: gtk::Label = collect(&widget, "postio-folder-count")[0]
                .clone()
                .downcast()
                .unwrap();
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

/// A manual sync is reachable at all times, and above all while syncing
/// (#495).
///
/// Reported directly: *"when downloading stuff the indicator hides the sync
/// button. I want to be able to sync even with longer processes running in
/// the background."* `Refresh` was always a real, correctly wired command —
/// what it had no persistent surface. The one hint anywhere on screen lived
/// in `list_state`'s banner, which is drawn only for `Offline` and
/// `Failing` and hides itself the moment the account connects. So the
/// affordance vanished at exactly the moment a person wants it: mid-backfill
/// on a long first sync, wanting to nudge a folder that looks stuck.
///
/// The states below are the whole point. A test that only checked the
/// default would have passed throughout the bug.
pub fn a_manual_sync_is_reachable_in_every_connection_state() {
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
    window.set_default_size(320, 600);
    window.present();
    pump();

    let asked: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&asked);
    sidebar.connect_refresh_requested(move || *counter.borrow_mut() += 1);

    let now = Instant::now();
    for (name, status) in [
        (
            "connecting",
            SyncStatus {
                state: ConnectionState::Connecting,
                ..SyncStatus::default()
            },
        ),
        (
            "idle",
            SyncStatus {
                state: ConnectionState::Online,
                last_sync: now.checked_sub(Duration::from_secs(12)),
                ..SyncStatus::default()
            },
        ),
        (
            // The reported case: connected, and busy for a long time.
            "online mid-backfill",
            SyncStatus {
                state: ConnectionState::Online,
                progress: Some((43, 100)),
                ..SyncStatus::default()
            },
        ),
        (
            "offline",
            SyncStatus {
                state: ConnectionState::Offline,
                ..SyncStatus::default()
            },
        ),
        (
            "failing",
            SyncStatus {
                state: ConnectionState::Failing {
                    reason: postio_core::FailureReason::Auth,
                },
                detail: Some("app-specific password rejected".into()),
                ..SyncStatus::default()
            },
        ),
    ] {
        sidebar.set_status(status);
        pump();
        let button = refresh_button(&sidebar);
        assert!(
            button.is_visible(),
            "the manual sync trigger is gone while {name}, which is the state \
             a person is in when they want it"
        );
        assert!(
            button.is_sensitive(),
            "the trigger is drawn but refuses to be pressed while {name}"
        );
    }

    // ── and pressing it asks, every time ─────────────────────────────────
    // No rejection of a second pass: `refresh()` starts another one, which
    // is what "nudge the sync" means. A button that quietly did nothing the
    // second time would be the same bug wearing a different hat.
    let button = refresh_button(&sidebar);
    button.emit_clicked();
    pump();
    button.emit_clicked();
    pump();
    assert_eq!(
        *asked.borrow(),
        2,
        "each press is a request; the second must not be swallowed"
    );

    // It says what it is, for a screen reader and for the pointer.
    let button = refresh_button(&sidebar);
    assert!(
        button
            .tooltip_text()
            .is_some_and(|text| text.contains("F5") || text.contains('R')),
        "the tooltip names the key that does the same thing, the way every \
         other affordance here does"
    );

    window.close();
}

/// The sidebar's manual-sync control.
fn refresh_button(sidebar: &Sidebar) -> gtk::Button {
    collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-status-refresh")
        .into_iter()
        .find_map(|widget| widget.downcast::<gtk::Button>().ok())
        .expect("the status line offers a manual sync")
}

/// The sidebar draws the model it was given, and does not re-derive it
/// (spec 003, FR-016 and FR-012).
///
/// Which rows exist, what each is called and what number sits beside it are
/// `postio_ui::sidebar`'s answers, so that the GTK sidebar and the macOS one
/// draw the same thing. The expectations below are therefore computed from
/// that module rather than written out: a literal here would pass while the
/// two frontends disagreed, which is the shape #1155 left behind — the
/// *ordering* moved to the shared layer and the rows did not, so macOS had
/// no Flagged or Snoozed at all.
pub fn the_sidebar_draws_the_shared_model_rather_than_its_own_idea_of_it() {
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
    sidebar.set_account("lena@example.com");

    let account = AccountId::new(1);
    let folders = canvas_mailboxes(12);

    // What the sidebar should show, asked of the shared layer.
    let expected = |counts: postio_ui::sidebar::ViewCounts| {
        let mut all = folders.clone();
        all.extend(postio_ui::sidebar::view_rows(account, &folders, counts));
        all
    };
    let rendered_against = |all: &[Mailbox]| {
        sidebar.set_mailboxes(all);
        pump();
        let mut drawn = labels(&sidebar);
        let mut wanted: Vec<(String, Option<String>)> = all
            .iter()
            .map(|mailbox| {
                (
                    postio_ui::sidebar::display_name(mailbox, all),
                    postio_ui::sidebar::count_for(mailbox).map(|count| count.to_string()),
                )
            })
            .collect();
        // Order is the existing test's subject; this one is about membership.
        drawn.sort();
        wanted.sort();
        (drawn, wanted)
    };

    // ── the ordinary state: nothing on its way, so no Outbox row ─────────
    let quiet = expected(postio_ui::sidebar::ViewCounts {
        flagged: 3,
        snoozed: 2,
        outbox: 0,
        drafts: 2,
        attention: 0,
    });
    let (drawn, wanted) = rendered_against(&quiet);
    assert_eq!(
        drawn, wanted,
        "the sidebar drew something the model did not"
    );
    assert!(
        !drawn.iter().any(|(name, _)| name == "Outbox"),
        "an empty Outbox is not a row (FR-012): {drawn:?}"
    );
    // The account's server has a real `\Flagged` folder, so the model
    // synthesises none — and the sidebar must not have one of its own.
    assert_eq!(
        drawn.iter().filter(|(name, _)| name == "Flagged").count(),
        1,
        "one Flagged row, the server's own: {drawn:?}"
    );

    // ── three on their way ───────────────────────────────────────────────
    let sending = expected(postio_ui::sidebar::ViewCounts {
        flagged: 3,
        snoozed: 2,
        outbox: 3,
        drafts: 2,
        attention: 0,
    });
    let (drawn, wanted) = rendered_against(&sending);
    assert_eq!(
        drawn, wanted,
        "the sidebar drew something the model did not"
    );
    let outbox = drawn
        .iter()
        .find(|(name, _)| name == "Outbox")
        .expect("three messages on their way, and no Outbox row to say so");
    assert_eq!(
        outbox.1,
        Some("3".to_string()),
        "the badge says how many are waiting"
    );

    window.close();
}

/// A view row can be opened by name, the way a folder can be opened by id.
///
/// `select` takes a `MailboxId` and searches for it. Every view row shares
/// the unassigned id — that is ADR 0036's stated cost — so searching for it
/// finds whichever view is drawn first, which is Flagged. That is the same
/// mistake the keyboard walk made when it stuck on Snoozed for ever, and
/// until now the only way around it was to hold the widget already.
pub fn a_view_row_is_selectable_by_role_rather_than_by_a_shared_id() {
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
    sidebar.set_account("lena@example.com");

    let account = AccountId::new(1);
    // No Flagged folder on the server, so the model synthesises one — which
    // makes Flagged the first view row and the one a search by id lands on.
    let folders: Vec<Mailbox> = canvas_mailboxes(12)
        .into_iter()
        .filter(|mailbox| mailbox.role != MailboxRole::Flagged)
        .collect();
    let mut all = folders.clone();
    all.extend(postio_ui::sidebar::view_rows(
        account,
        &folders,
        postio_ui::sidebar::ViewCounts {
            flagged: 3,
            snoozed: 2,
            outbox: 1,
            drafts: 2,
            attention: 0,
        },
    ));
    sidebar.set_mailboxes(&all);
    pump();

    for role in [
        MailboxRole::Flagged,
        MailboxRole::Snoozed,
        MailboxRole::Outbox,
    ] {
        sidebar.select_view(role);
        pump();
        assert_eq!(
            sidebar.selected_choice(),
            Some(SidebarChoice::View(role)),
            "selecting the {role:?} view landed somewhere else"
        );
    }

    // And the folder path still works, which is what says this is a second
    // door rather than a replacement for the first.
    let inbox = folders[0].id;
    sidebar.select(inbox);
    pump();
    assert_eq!(
        sidebar.selected_choice(),
        Some(SidebarChoice::Folder(inbox)),
        "a folder is still selectable by its id"
    );

    window.close();
}
