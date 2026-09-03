//! A folder reload must not move the list out from under the user.
//!
//! Open Flagged, then let anything make the sidebar re-read the folder tree
//! — a rename, a new folder, or just a resync finishing and emitting
//! `MailboxesChanged` — and the list used to jump back to the inbox (#813).
//!
//! # Why the obvious guard is the wrong one
//!
//! `install_feeds`' `connect_loaded` handler picks a default folder when
//! nothing is open yet, and asked `feed.mailbox().is_some()`. That is `None`
//! for every scope which is not a real folder — `Flagged`, `Snoozed`,
//! `Unified` — so while one of those was on screen the guard never fired and
//! the handler opened the inbox over the top. It runs on *every* load, not
//! only the first, which is what turns a startup convenience into a view
//! that will not stay put.
//!
//! Tightening it to `feed.scope().is_some()` is worse, and is the reason
//! this file has a third case. On a fresh account the sidebar's virtual rows
//! arrive a beat before the real folders, `GtkListBox` auto-selects the first
//! row it is handed, and that is the Flagged sentinel — so the scope is
//! *already* `Some(Flagged)` when the folders land, and a scope-based guard
//! opens the window into an empty smart folder instead of the inbox.
//!
//! The two cases are indistinguishable from the feed alone, which is why the
//! answer is the folder generation: pick at most once per generation, and
//! `open`/`open_sections` are the only things that bump it.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::glib;
use gtk::prelude::*;
use postio_core::Event;
use postio_gtk::feed::{
    ListScope, MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const OTHER_ACCOUNT: i64 = 2;
/// Every account's inbox carries its own id, so "whose inbox is showing"
/// is answerable rather than assumed.
const INBOX: i64 = ACCOUNT;
const OTHER_INBOX: i64 = OTHER_ACCOUNT;

/// Distinct totals per scope, so "which scope is showing" is readable off
/// the row count alone and a wrong answer cannot look like a right one.
const INBOX_TOTAL: u32 = 940;
const FLAGGED_TOTAL: u32 = 7;
const SNOOZED_TOTAL: u32 = 3;

#[derive(Default)]
struct Store {
    /// Every folder read, so a test can tell a reload actually happened
    /// rather than assert on the absence of an effect that never ran.
    reads: RefCell<u32>,
    /// Whether the first sync has landed. A real account has no folders at
    /// all until one has: `postio-app`'s `e2e` opens a window over an empty
    /// store and the tree arrives afterwards, so a handler that only ever
    /// sees a populated first read is not being tested against the case
    /// that matters.
    empty_until_synced: Cell<bool>,
}

impl MailboxSource for Store {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        *self.reads.borrow_mut() += 1;
        if self.empty_until_synced.get() {
            return Box::pin(async move { Ok(Vec::new()) });
        }
        let folder = |id: i64, path: &str, role| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total: INBOX_TOTAL,
                unread: 0,
                flagged: FLAGGED_TOTAL,
                snoozed: SNOOZED_TOTAL,
            };
            mailbox
        };
        let folders = vec![
            folder(account.get(), "INBOX", MailboxRole::Inbox),
            folder(100 + account.get(), "Archive", MailboxRole::Archive),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Store {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let total = match request.scope {
            ListScope::Flagged(_) => FLAGGED_TOTAL,
            ListScope::Snoozed(_) => SNOOZED_TOTAL,
            ListScope::Mailbox(id) if id.get() == INBOX || id.get() == OTHER_INBOX => INBOX_TOTAL,
            ListScope::Mailbox(_)
            | ListScope::Account(_)
            | ListScope::Unified
            | ListScope::Thread(_) => 0,
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

/// A window with the store's folders already in the sidebar.
fn opened() -> (Window, Rc<Store>, postio_gtk::feed::Feeds) {
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
    settle_until(|| labels(&window).len() == 4);
    (window, store, feeds)
}

/// A resync finishing: the event that makes the sidebar re-read the tree.
///
/// Sent through `Feeds::apply`, the way the composition root delivers it,
/// rather than by calling `reload` — a test that pokes the reload directly
/// would not fail if the event stopped reaching it.
fn resync_finishes(feeds: &postio_gtk::feed::Feeds, store: &Store) {
    let before = *store.reads.borrow();
    feeds.apply(&Event::MailboxesChanged {
        account: AccountId::new(ACCOUNT),
    });
    settle_until(|| *store.reads.borrow() > before);
    assert!(
        *store.reads.borrow() > before,
        "the folder tree was never re-read, so this proved nothing"
    );
    settle();
}

pub fn a_folder_reload_leaves_the_list_in_flagged() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, store, feeds) = opened();

    click_folder(&window, "Flagged");
    settle_until(|| window.list().model().n_items() == FLAGGED_TOTAL);

    resync_finishes(&feeds, &store);

    assert_eq!(
        feeds.messages.scope(),
        Some(ListScope::Flagged(AccountId::new(ACCOUNT))),
        "a folder reload threw the list out of Flagged"
    );
    assert_eq!(
        window.list().model().n_items(),
        FLAGGED_TOTAL,
        "the rows are the inbox's, so the view moved even if the scope did not"
    );
    window.destroy();
}

pub fn a_folder_reload_leaves_the_list_in_snoozed() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, store, feeds) = opened();

    click_folder(&window, "Snoozed");
    settle_until(|| window.list().model().n_items() == SNOOZED_TOTAL);

    resync_finishes(&feeds, &store);

    assert_eq!(
        feeds.messages.scope(),
        Some(ListScope::Snoozed(AccountId::new(ACCOUNT))),
        "a folder reload threw the list out of Snoozed"
    );
    assert_eq!(
        window.list().model().n_items(),
        SNOOZED_TOTAL,
        "the rows are the inbox's, so the view moved even if the scope did not"
    );
    window.destroy();
}

pub fn a_first_load_still_opens_the_inbox_over_the_auto_selected_sentinel() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    // The trap, and the reason the guard cannot simply ask for a scope.
    // Nobody clicks anything here: `GtkListBox` auto-selects the Flagged
    // sentinel as soon as the virtual rows exist, so by the time the real
    // folders land the feed already has a scope. A window that honours that
    // opens into an empty smart folder on every fresh account.
    let (window, _store, feeds) = opened();
    // Waited for the *rows*, not for the pick: the pick crosses to the
    // source and back, and a wait that stops at `mailbox().is_some()`
    // samples the turn before the page lands.
    settle_until(|| window.list().model().n_items() > 0);

    assert_eq!(
        feeds.messages.mailbox(),
        Some(MailboxId::new(INBOX)),
        "a fresh account must open into its inbox, not into whichever \
         virtual row GtkListBox happened to select first"
    );
    assert_eq!(
        feeds.messages.scope(),
        Some(ListScope::Mailbox(MailboxId::new(INBOX))),
        "and the list is aimed at the inbox, not at the sentinel"
    );
    assert!(
        window.list().model().n_items() > 0,
        "an inbox with {INBOX_TOTAL} messages showed none, so the pick \
         did not reach the list"
    );
    window.destroy();
}

/// Click a folder row by its label, the way the user reaches it.
fn click_folder(window: &Window, label: &str) {
    let row = rows(window)
        .into_iter()
        .find(|(text, _)| text == label)
        .map(|(_, row)| row)
        .unwrap_or_else(|| panic!("no folder row labelled {label}: {:?}", labels(window)));
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

fn first_label(widget: &gtk::Widget) -> Option<String> {
    let mut text = None;
    walk(widget, &mut |candidate| {
        if text.is_none()
            && let Some(label) = candidate.downcast_ref::<gtk::Label>()
            && !label.text().is_empty()
        {
            text = Some(label.text().to_string());
        }
    });
    text
}

fn walk(widget: &gtk::Widget, visit: &mut impl FnMut(&gtk::Widget)) {
    visit(widget);
    let mut child = widget.first_child();
    while let Some(current) = child {
        walk(&current, visit);
        child = current.next_sibling();
    }
}

fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..200 {
        while context.iteration(false) {}
    }
}

fn settle_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let heartbeat = glib::timeout_add_local(std::time::Duration::from_millis(5), || {
        glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now() + postio_test_support::patience();
    while !done() && std::time::Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

pub fn a_folder_reload_leaves_the_list_in_the_unified_view() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, store, feeds) = opened();

    // How the application itself opens it: Unified is a view over every
    // account rather than a folder in one, so there is no tree to re-point
    // and the list is addressed directly (`postio_app`'s scope handler).
    feeds.messages.open(ListScope::Unified);
    settle();

    resync_finishes(&feeds, &store);

    assert_eq!(
        feeds.messages.scope(),
        Some(ListScope::Unified),
        "a folder reload threw the list out of the unified view (#185)"
    );
    window.destroy();
}

pub fn switching_accounts_still_opens_the_new_accounts_inbox() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, _store, feeds) = opened();
    settle_until(|| feeds.messages.mailbox() == Some(MailboxId::new(INBOX)));

    // The other half of picking once per generation: `open` bumps it, so the
    // handler is owed another pick. Without that this fix would trade a list
    // that will not stay put for one that will not move.
    feeds
        .folders
        .open(AccountId::new(OTHER_ACCOUNT), "home@example.net");
    settle_until(|| feeds.messages.mailbox() == Some(MailboxId::new(OTHER_INBOX)));

    assert_eq!(
        feeds.messages.mailbox(),
        Some(MailboxId::new(OTHER_INBOX)),
        "switching accounts must open the new account's inbox"
    );
    window.destroy();
}

pub fn an_account_whose_folders_arrive_after_the_first_sync_still_opens_its_inbox() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    let store = Rc::new(Store {
        reads: RefCell::new(0),
        empty_until_synced: Cell::new(true),
    });
    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "lena@example.com",
        store.clone(),
        store.clone(),
    );
    // The window opens over an empty store: there is no tree yet, so there is
    // nothing to pick and the handler must not count that as its turn.
    settle_until(|| *store.reads.borrow() > 0);
    settle();
    assert_eq!(
        feeds.messages.mailbox(),
        None,
        "there were no folders to open yet"
    );

    // The first sync lands, and with it the folders.
    store.empty_until_synced.set(false);
    resync_finishes(&feeds, &store);
    settle_until(|| feeds.messages.mailbox().is_some());

    assert_eq!(
        feeds.messages.mailbox(),
        Some(MailboxId::new(INBOX)),
        "the folders arrived and nothing opened one: an account whose first \
         read came back empty has not had its turn yet, and is still owed a \
         pick when the tree shows up"
    );
    window.destroy();
}
