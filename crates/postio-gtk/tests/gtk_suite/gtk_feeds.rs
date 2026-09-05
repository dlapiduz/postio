//! Both panes fed from one runtime, on a real display.
//!
//! The three things postio-4e2 promises: the sidebar shows the account's
//! real folders with live counts, the status line follows a real connection
//! all the way round, and picking a folder changes what the message list
//! shows. Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives — and because the
//! replies are awaited on the thread-default main context, which the test
//! harness would otherwise drive from two threads at once.

use crate::pump;
use std::cell::Cell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
use postio_core::{ConnectionState, Event};
use postio_gtk::feed::{
    ListScope, MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const ACCOUNT: i64 = 1;
const INBOX: i64 = 1;
const ARCHIVE: i64 = 5;

/// The account as the repository would report it, with counts a test can move.
struct Store {
    unread: Cell<u32>,
    reads: Cell<usize>,
}

impl Store {
    fn new() -> Rc<Self> {
        Rc::new(Store {
            unread: Cell::new(12),
            reads: Cell::new(0),
        })
    }

    fn folders(&self) -> Vec<Mailbox> {
        let account = AccountId::new(ACCOUNT);
        let folder = |id: i64, path: &str, role, total, unread| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total,
                unread,
                flagged: 0,
                snoozed: 0,
            };
            // A folder that has synced, so the status line has an age to show.
            //
            // Five and a half minutes rather than twelve seconds. `age`
            // renders exact seconds below a minute, so a twelve-second
            // fixture re-read the wall clock at render time and said `13s`
            // on any run that crossed a second boundary — about one in three
            // (#49). In the minutes bucket the same assertion has thirty
            // seconds of slack and still proves the only thing it is for:
            // that the age came off the folder rather than from anywhere
            // else.
            mailbox.last_synced_at = Some(Utc::now() - chrono::Duration::seconds(330));
            mailbox
        };
        vec![
            folder(INBOX, "INBOX", MailboxRole::Inbox, 940, self.unread.get()),
            folder(2, "Drafts", MailboxRole::Drafts, 2, 0),
            folder(ARCHIVE, "Archive", MailboxRole::Archive, 38_122, 0),
        ]
    }
}

impl MailboxSource for Store {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        self.reads.set(self.reads.get() + 1);
        let folders = self.folders();
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Store {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        // Each mailbox holds a different amount of mail, so "the list shows
        // the folder you picked" is checkable by counting.
        // A folder id to count by. The flagged query has no folder, so it
        // stands in for one here: this store is about "the list shows what
        // you picked", and `gtk_flagged.rs` is about which scope was asked
        // for.
        let mailbox = match request.scope {
            ListScope::Mailbox(id) => id,
            ListScope::Account(_)
            | ListScope::Unified
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            | ListScope::Thread(_) => MailboxId::new(0),
        };
        // Each mailbox holds a different amount of mail, so "the list shows
        // the folder you picked" is checkable by counting.
        let total = match mailbox.get() {
            INBOX => 940,
            ARCHIVE => 7,
            _ => 0,
        };
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: MessageId::new(mailbox.get() * 1_000_000 + position as i64 + 1),
                    thread: None,
                    from: None,
                    subject: Some(format!("folder {} message {position}", mailbox.get())),
                    preview: None,
                    received_at: Utc
                        .timestamp_opt(1_700_000_000 - position as i64, 0)
                        .unwrap(),
                    seen: false,
                    flagged: false,
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

pub fn the_panes_follow_the_account_the_sync_and_the_folder_you_pick() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Store::new();
    let window = Window::default();
    window.present();
    pump();

    // ── before anything is fed ────────────────────────────────────────────
    assert!(labels(&window).is_empty(), "no account, no folders");
    assert_eq!(
        status_text(&window),
        ("offline · imap".to_string(), "never synced".to_string()),
        "the sidebar says what is true before the first sync"
    );

    let feeds = window.install_feeds(
        AccountId::new(ACCOUNT),
        "lena@example.com",
        store.clone(),
        store.clone(),
    );
    pump();

    // ── the real account's folders, with their counts ─────────────────────
    assert_eq!(
        labels(&window),
        [
            ("Inbox".to_string(), Some("12".to_string())),
            // Synthesised by `Folders`, not read from the store: a query over
            // the flagged role, which no server has a folder for. No count
            // because this account has no flagged mail, the same way Archive
            // shows none with nothing unread. See `gtk_flagged.rs`.
            ("Flagged".to_string(), None),
            // Snoozed is the same shape: a query over `snoozed_until`, no
            // count because nothing here is snoozed either.
            ("Snoozed".to_string(), None),
            ("Drafts".to_string(), Some("2".to_string())),
            ("Archive".to_string(), None),
        ]
    );
    assert_eq!(
        status_text(&window).1,
        "last sync 5m",
        "the age came off the folders' own last_synced_at"
    );

    // ── counts follow the mail, and one burst costs one read ──────────────
    store.unread.set(11);
    let reads = store.reads.get();
    for _ in 0..20 {
        feeds.apply(&Event::MessagesChanged {
            account: postio_model::AccountId::new(1),
            messages: vec![MessageId::new(1)],
        });
    }
    pump();
    assert_eq!(
        labels(&window)[0],
        ("Inbox".to_string(), Some("11".to_string())),
        "reading a message did not reach the count"
    );
    assert_eq!(
        store.reads.get() - reads,
        1,
        "a burst of twenty changes re-read the folders {} times",
        store.reads.get() - reads
    );

    // ── the status line, all the way round ────────────────────────────────
    let connection = |state| Event::ConnectionChanged {
        account: AccountId::new(ACCOUNT),
        state,
    };
    feeds.apply(&connection(ConnectionState::Connecting));
    pump();
    assert_eq!(status_text(&window).0, "connecting · imap");

    feeds.apply(&connection(ConnectionState::Online));
    feeds.apply(&Event::SyncProgress {
        account: AccountId::new(ACCOUNT),
        done: 30,
        total: 100,
    });
    pump();
    // Not a percentage: the denominator is an upper bound a pass routinely
    // finishes short of, so the count is the honest half. `postio-qhz.6`.
    assert_eq!(
        status_text(&window),
        ("syncing · imap".to_string(), "fetched 30".to_string())
    );

    // ── mail arriving is its own state, on the real widget ───────────────
    //
    // Issue #74: the backfill announced nothing, so the longest phase of a
    // first sync drew `idle` -- not merely unreported but reported as
    // nothing happening, which reads as stuck. It is a separate word from
    // `syncing` because the consequences differ: a mailbox whose list is
    // still arriving cannot be read, and one whose mail is still arriving
    // can.
    feeds.apply(&Event::SyncProgress {
        account: AccountId::new(ACCOUNT),
        done: 100,
        total: 100,
    });
    feeds.apply(&Event::BackfillProgress {
        account: AccountId::new(ACCOUNT),
        done: 412,
        total: 2000,
        footprint: None,
    });
    pump();
    assert_eq!(
        status_text(&window),
        (
            "downloading · imap".to_string(),
            "mail 412 of 2000".to_string()
        ),
        "the list is complete and the mail is not, and the line has to \
         say which"
    );

    // Drained, and it gets out of the way rather than sticking at 2000/2000.
    feeds.apply(&Event::BackfillProgress {
        account: AccountId::new(ACCOUNT),
        done: 2000,
        total: 2000,
        footprint: None,
    });
    pump();
    assert_eq!(
        status_text(&window).0,
        "idle · imap",
        "a drained body queue is not a backfill in progress"
    );

    feeds.apply(&Event::Error {
        message: "app-specific password rejected".to_string(),
    });
    feeds.apply(&connection(ConnectionState::Failing {
        reason: postio_core::FailureReason::Auth,
    }));
    pump();
    assert_eq!(
        status_text(&window),
        (
            "error · imap".to_string(),
            "app-specific password rejected".to_string()
        ),
        "a failing connection has to say what to do about it"
    );

    feeds.apply(&connection(ConnectionState::Offline));
    pump();
    assert_eq!(status_text(&window).0, "offline · imap");
    assert_ne!(
        status_text(&window).1,
        "app-specific password rejected",
        "a reason that outlived its failure is worse than none"
    );

    // ── the window opened into a folder, without being asked ──────────────
    feeds.apply(&connection(ConnectionState::Online));
    pump();
    assert_eq!(window.sidebar().selected(), Some(MailboxId::new(INBOX)));
    assert_eq!(window.list().model().n_items(), 940);
    assert_eq!(feeds.messages.mailbox(), Some(MailboxId::new(INBOX)));
    assert_eq!(
        header(&window),
        ("Inbox".to_string(), "12 unread".to_string())
    );

    // ── picking a folder changes what the list shows ──────────────────────
    // Selected the way a click or an arrow key does it, not through the
    // sidebar's own restore path, which deliberately reports nothing.
    pick(&window, MailboxId::new(ARCHIVE));
    pump();
    assert_eq!(
        window.list().model().n_items(),
        7,
        "the list is still showing the folder that was left"
    );
    assert_eq!(header(&window).0, "Archive");
    assert_eq!(
        header(&window).1,
        "",
        "an archive with nothing unread says nothing"
    );

    // ── and the list pane's named states read the same connection ─────────
    feeds.apply(&connection(ConnectionState::Failing {
        reason: postio_core::FailureReason::Auth,
    }));
    pump();
    assert!(
        window.list_state().state().is_some(),
        "a failing connection left the list pane saying nothing"
    );
    feeds.apply(&connection(ConnectionState::Online));
    pump();
    assert!(
        window.list_state().state().is_none(),
        "there are rows to show, so the named state should be out of the way"
    );

    window.destroy();
}

/// Pick a folder the way a pointer or an arrow key does.
fn pick(window: &Window, id: MailboxId) {
    let sidebar = window.sidebar();
    for row in collect(sidebar.upcast_ref::<gtk::Widget>(), "postio-folder") {
        let row: gtk::ListBoxRow = row.downcast().unwrap();
        let list: gtk::ListBox = row.parent().and_then(|p| p.downcast().ok()).unwrap();
        let before = sidebar.selected();
        list.select_row(Some(&row));
        if sidebar.selected() == Some(id) {
            return;
        }
        if let Some(before) = before {
            sidebar.select(before);
        }
    }
    panic!("no row for {id:?}");
}

fn labels(window: &Window) -> Vec<(String, Option<String>)> {
    collect(
        window.sidebar().upcast_ref::<gtk::Widget>(),
        "postio-folder",
    )
    .iter()
    .map(|row| {
        let line = row.first_child().unwrap();
        let name: gtk::Label = line.first_child().unwrap().downcast().unwrap();
        let count: gtk::Label = name.next_sibling().unwrap().downcast().unwrap();
        (
            name.text().to_string(),
            count.is_visible().then(|| count.text().to_string()),
        )
    })
    .collect()
}

fn status_text(window: &Window) -> (String, String) {
    let labels: Vec<gtk::Label> = collect(
        window.sidebar().upcast_ref::<gtk::Widget>(),
        "postio-status",
    )
    .into_iter()
    .filter_map(|w| w.downcast().ok())
    .collect();
    (labels[0].text().to_string(), labels[1].text().to_string())
}

/// The list pane's own header: the folder, and how much of it is unread.
fn header(window: &Window) -> (String, String) {
    let title: gtk::Label = collect(
        window.list().upcast_ref::<gtk::Widget>(),
        "postio-list-title",
    )
    .remove(0)
    .downcast()
    .unwrap();
    let meta: gtk::Label = collect(
        window.list().upcast_ref::<gtk::Widget>(),
        "postio-list-meta",
    )
    .remove(0)
    .downcast()
    .unwrap();
    (title.text().to_string(), meta.text().to_string())
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
