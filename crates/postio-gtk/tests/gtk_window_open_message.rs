//! Opening a mailbox and a message from outside a click on a visible row.
//!
//! `postio-du6` shipped notifications whose click could only present the
//! window: nothing in `postio-gtk` could switch the sidebar to a mailbox and
//! put the cursor on a message the way a click on an already-visible row
//! does, from outside the window. `Window::open_mailbox` and
//! `Window::open_message` are that other way in, and the seam
//! `crates/postio-app/src/notifications.rs`'s click action now drives.
//! `gtk_list_select_message.rs` covers the row-selection half on its own;
//! this covers the whole path: sidebar, list source, and cursor together.
//!
//! One `#[test]`, like the rest of `gtk_*`: a window costs seconds to
//! realise, and GTK may be initialised once per process (#41). Skips
//! without a display. Nothing here touches the network.

use std::cell::Cell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::gdk;
use gtk::prelude::*;
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

struct Store {
    reads: Cell<usize>,
}

impl Store {
    fn new() -> Rc<Self> {
        Rc::new(Store {
            reads: Cell::new(0),
        })
    }
}

impl MailboxSource for Store {
    fn mailboxes(&self, _account: AccountId) -> MailboxFuture {
        self.reads.set(self.reads.get() + 1);
        let account = AccountId::new(ACCOUNT);
        let folder = |id: i64, path: &str, role| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total: 40,
                unread: 0,
                flagged: 0,
                snoozed: 0,
            };
            mailbox
        };
        let folders = vec![
            folder(INBOX, "INBOX", MailboxRole::Inbox),
            folder(ARCHIVE, "Archive", MailboxRole::Archive),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

/// A message id that says which mailbox and which position it came from, so
/// a test can name exactly the row it wants without reading anything back.
fn message_id(mailbox: i64, position: u32) -> MessageId {
    MessageId::new(mailbox * 1_000_000 + position as i64 + 1)
}

impl MessageSource for Store {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let mailbox = match request.scope {
            ListScope::Mailbox(id) => id,
            ListScope::Account(_)
            | ListScope::Unified
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            | ListScope::Thread(_) => MailboxId::new(0),
        };
        let total = 40;
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total);
            let rows = (request.offset..end)
                .map(|position| Row {
                    id: message_id(mailbox.get(), position),
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

fn pump() {
    let context = gtk::glib::MainContext::default();
    for _ in 0..64 {
        while context.iteration(false) {}
    }
}

#[test]
fn open_mailbox_and_open_message_switch_the_window_from_outside() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let store = Store::new();
    let window = Window::default();
    window.present();
    pump();

    window.install_feeds(
        AccountId::new(ACCOUNT),
        "ada@example.com",
        store.clone(),
        store.clone(),
    );
    pump();
    assert_eq!(
        window.sidebar().selected(),
        Some(MailboxId::new(INBOX)),
        "the window opened into a folder on its own, same as always"
    );

    // ── open_mailbox switches the sidebar and the list together ───────────
    window.open_mailbox(MailboxId::new(ARCHIVE));
    pump();
    assert_eq!(window.sidebar().selected(), Some(MailboxId::new(ARCHIVE)));
    assert_eq!(
        window.list().mailbox_name(),
        "Archive",
        "the list pane should be showing the folder that was opened, not \
         just the sidebar's own idea of it"
    );

    // ── open_message does the same, and also lands the cursor ─────────────
    // Back on the inbox, told about a message that arrived in the archive --
    // the case a notification for a non-inbox mailbox is.
    let target = message_id(ARCHIVE, 7);
    window.open_message(MailboxId::new(ARCHIVE), target);
    pump();
    assert_eq!(window.sidebar().selected(), Some(MailboxId::new(ARCHIVE)));
    assert_eq!(
        window.list().cursor_id(),
        Some(target),
        "the cursor should have landed on the specific message, not just \
         wherever the folder's autoselect put it"
    );

    window.destroy();
}
