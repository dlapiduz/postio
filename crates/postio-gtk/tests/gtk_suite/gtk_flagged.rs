//! The sidebar's Flagged folder: a query that looks like a folder.
//!
//! `postio-uoy`. The design canvas puts "Flagged" under Inbox and the sidebar
//! already knew how to draw it — a label, a count source, a sort position —
//! but nothing ever created one, because IMAP has no such folder and the
//! store has no such row. It is a query over a role.
//!
//! # What this is really testing
//!
//! That the row **opens**. Drawing it was never the hard part; the hard part
//! is that selecting a folder has to show its messages, and the list was keyed
//! by a `MailboxId` that a smart folder does not have. So the assertions below
//! are mostly about the *scope* travelling with the selection, and about the
//! synthetic row's stand-in id not escaping the one hop it is allowed.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_gtk::feed::{
    FeedScope, MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;
const ARCHIVE: i64 = 5;

/// Records what the list asked for, so "which scope" is checkable.
#[derive(Default)]
struct Store {
    asked: RefCell<Vec<FeedScope>>,
}

impl Store {
    fn folders(&self) -> Vec<Mailbox> {
        let folder = |id: i64, path: &str, role, total, flagged| {
            let mut mailbox = Mailbox::new(AccountId::new(ACCOUNT), path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total,
                unread: 0,
                flagged,
                snoozed: 0,
            };
            mailbox
        };
        vec![
            // Flagged mail lives in more than one folder, which is the whole
            // reason the sidebar's count cannot come from any single one.
            folder(INBOX, "INBOX", MailboxRole::Inbox, 940, 3),
            folder(2, "Drafts", MailboxRole::Drafts, 2, 0),
            folder(ARCHIVE, "Archive", MailboxRole::Archive, 38_122, 4),
        ]
    }
}

impl MailboxSource for Store {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        let folders = self.folders();
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Store {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        self.asked.borrow_mut().push(request.scope);
        // Seven flagged messages across the account; the inbox holds 940.
        let total = match request.scope {
            FeedScope::Flagged(_) => 7,
            FeedScope::Mailbox(id) if id.get() == INBOX => 940,
            FeedScope::Mailbox(_) => 0,
            // This store is about which scope the sidebar asked for; a
            // drill-in is `gtk_thread_scope.rs`.
            FeedScope::Snoozed(_) | FeedScope::Thread(_) => 0,
        };
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: MessageId::new(position as i64 + 1),
                    thread: None,
                    from: None,
                    subject: Some(format!("message {position}")),
                    preview: None,
                    received_at: Utc
                        .timestamp_opt(1_700_000_000 - position as i64, 0)
                        .unwrap(),
                    seen: false,
                    flagged: true,
                    answered: false,
                    draft: false,
                    has_attachments: false,
                    thread_count: 1,
                    participants: Vec::new(),
                })
                .collect();
            Ok(Page { total, rows })
        })
    }
}

pub fn the_sidebar_offers_flagged_and_opening_it_lists_the_flagged_mail() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let store = Rc::new(Store::default());
    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "lena@example.com",
        store.clone(),
        store.clone(),
    );
    let _ = &feeds;
    // The folder read crosses to the source and back, so wait for the row to
    // exist rather than for a number of frames.
    pump_until(|| labels(&window).len() == 5);

    // ── it is in the sidebar, where the canvas puts it ───────────────────
    assert_eq!(
        labels(&window),
        ["Inbox", "Flagged", "Snoozed", "Drafts", "Archive"],
        "canvas 1b: Flagged sits under Inbox, above the rest"
    );
    assert!(
        counts(&window).contains(&"7".to_string()),
        "the count is the account's flagged mail — 3 in the inbox and 4 in \
         the archive — not any one folder's: {:?}",
        counts(&window)
    );

    // ── opening it lists the flagged mail ────────────────────────────────
    store.asked.borrow_mut().clear();
    click_folder(&window, "Flagged");
    pump_until(|| window.list().model().n_items() == 7);

    assert_eq!(
        store.asked.borrow().first(),
        Some(&FeedScope::Flagged(AccountId::new(ACCOUNT))),
        "the scope travels with the selection; a smart folder has no \
         MailboxId to send instead"
    );
    assert_eq!(
        window.list().model().n_items(),
        7,
        "and the rows that came back are the ones showing"
    );

    // ── the stand-in id does not escape ──────────────────────────────────
    assert_eq!(
        feeds.messages.mailbox(),
        None,
        "this is what `commands::mirror` feeds to AppState::open_mailbox. A \
         smart folder claiming to be a mailbox would aim Ctrl+A at a folder \
         that does not exist, and a drop onto it at a foreign key that does \
         not resolve."
    );

    // ── a real folder still works the way it did ─────────────────────────
    store.asked.borrow_mut().clear();
    click_folder(&window, "Inbox");
    pump_until(|| feeds.messages.mailbox() == Some(MailboxId::new(INBOX)));

    assert_eq!(
        store.asked.borrow().first(),
        Some(&FeedScope::Mailbox(MailboxId::new(INBOX)))
    );
    assert_eq!(
        feeds.messages.mailbox(),
        Some(MailboxId::new(INBOX)),
        "and a folder that is a folder still says so"
    );

    window.destroy();
}

/// Select a folder the way a click does: through the row widget, not through
/// an id. The synthetic row's stand-in id is deliberately private, and a test
/// that reached for it would be testing something no user can do.
fn click_folder(window: &Window, label: &str) {
    let row = rows(window)
        .into_iter()
        .find(|(text, _)| text == label)
        .map(|(_, row)| row)
        .unwrap_or_else(|| panic!("the sidebar draws a {label} row"));
    let list = row
        .parent()
        .and_then(|parent| parent.downcast::<gtk::ListBox>().ok())
        .expect("a folder row lives in a list box");
    list.select_row(Some(&row));
}

fn labels(window: &Window) -> Vec<String> {
    rows(window).into_iter().map(|(label, _)| label).collect()
}

/// Every folder row's label and its widget, in sidebar order.
fn rows(window: &Window) -> Vec<(String, gtk::ListBoxRow)> {
    let mut found = Vec::new();
    walk(
        window.sidebar().upcast_ref::<gtk::Widget>(),
        &mut |widget| {
            if let Some(row) = widget.downcast_ref::<gtk::ListBoxRow>()
                && let Some(label) = first_label(row.upcast_ref::<gtk::Widget>())
            {
                found.push((label, row.clone()));
            }
        },
    );
    found
}

/// The numbers drawn beside the folder names.
fn counts(window: &Window) -> Vec<String> {
    let mut found = Vec::new();
    walk(
        window.sidebar().upcast_ref::<gtk::Widget>(),
        &mut |widget| {
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && label.text().chars().all(|c| c.is_ascii_digit())
                && !label.text().is_empty()
            {
                found.push(label.text().to_string());
            }
        },
    );
    found
}

fn first_label(widget: &gtk::Widget) -> Option<String> {
    let mut found = None;
    walk(widget, &mut |node| {
        if found.is_none()
            && let Some(label) = node.downcast_ref::<gtk::Label>()
            && !label.text().is_empty()
        {
            found = Some(label.text().to_string());
        }
    });
    found
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(node) = child {
        walk(&node, visit);
        child = node.next_sibling();
    }
}

/// Drive the main loop until `done`, or give up after a deadline.
///
/// A fixed number of frames is a bet that the work will have finished by the
/// time the test looks. The page read here crosses to a source and back
/// through `glib::spawn_future_local`, so with several sessions building the
/// bet loses and the value read is stale rather than wrong — which is
/// `postio-1ff` and `postio-mdu1`, both of which were exactly this.
///
/// A genuine regression still fails: the deadline runs out and the assertion
/// after this call reports the real values. A slow machine only makes it
/// slower, which is the difference between strict and flaky.
fn pump_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(5), || {
        glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(3000);
    while !done() && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}
