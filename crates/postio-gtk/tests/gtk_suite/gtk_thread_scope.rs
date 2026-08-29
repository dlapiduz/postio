//! A drill-in shows the whole thread, not the part of it this folder held.
//!
//! `t` used to build the column by filtering the message list's own model —
//! which is why it worked with no new read path, and why it was never the
//! whole conversation. A message filed in Archive is not in the Inbox's
//! model, and a page the list has not scrolled to is not resident either. The
//! header said so, reading `3 of 6 messages here`, but saying so is not
//! showing it. That is #44.
//!
//! So the source is asked for `ListScope::Thread`, which `postio-storage`
//! answers from `idx_messages_thread` across every folder the thread touches.
//! The fake source here refuses to serve the thread from the mailbox scope —
//! it hands back only the two Inbox messages, exactly as the real store would
//! — so a drill-in that still filtered the list model would show two of four
//! and fail.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

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
use postio_model::EmailAddress;
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

const THREAD: i64 = 7;
/// The whole conversation: four messages, two of them filed away.
const WHOLE_THREAD: u32 = 4;
/// What the Inbox alone can see of it.
const IN_THIS_FOLDER: usize = 2;

/// A store with one thread of four, two of whose messages are in Archive.
struct Split {
    /// How many times the thread scope was asked for, so the test can tell a
    /// real read from a lucky filter of what was already on screen.
    thread_reads: Cell<u32>,
}

fn row(id: i64, minute: i64) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(THREAD)),
        from: Some(EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")),
        subject: Some("the whole conversation".into()),
        preview: Some("a snippet under it".into()),
        received_at: Utc.timestamp_opt(1_700_000_000 + minute * 60, 0).unwrap(),
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: WHOLE_THREAD,
        participants: Vec::new(),
    }
}

impl MailboxSource for Split {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let folder = |id: i64, path: &str, role| {
            let mut mailbox = Mailbox::new(account, path, Some('/'));
            mailbox.id = MailboxId::new(id);
            mailbox.role = role;
            mailbox.counts = MailboxCounts {
                total: IN_THIS_FOLDER as u32,
                unread: 0,
                flagged: 0,
                snoozed: 0,
            };
            mailbox
        };
        let folders = vec![
            folder(1, "INBOX", MailboxRole::Inbox),
            folder(2, "Archive", MailboxRole::Archive),
        ];
        Box::pin(async move { Ok(folders) })
    }
}

impl MessageSource for Split {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        // The Inbox holds two of the four. The other two are in Archive and
        // no mailbox-scoped read can see them, which is the whole point.
        let rows = match request.scope {
            ListScope::Thread(_) => {
                self.thread_reads.set(self.thread_reads.get() + 1);
                vec![row(1, 10), row(2, 20), row(3, 30), row(4, 40)]
            }
            _ => vec![row(1, 10), row(3, 30)],
        };
        let total = rows.len() as u32;
        Box::pin(async move {
            let end = (request.offset + request.limit).min(total) as usize;
            Ok(Page {
                total,
                rows: rows[(request.offset as usize).min(end)..end].to_vec(),
            })
        })
    }
}

pub fn drilling_in_shows_the_thread_and_not_just_this_folders_part_of_it() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let source = Rc::new(Split {
        thread_reads: Cell::new(0),
    });
    window.install_feeds(
        AccountId::new(1),
        "lena@example.com",
        source.clone(),
        source.clone(),
    );
    window.present();
    pump_until(|| window.list().model().n_items() > 0);

    // ── the list holds only this folder's half ───────────────────────────
    assert_eq!(
        window.list().model().n_items() as usize,
        IN_THIS_FOLDER,
        "the fixture is wrong if the Inbox can already see the whole thread"
    );

    // ── drill in ─────────────────────────────────────────────────────────
    window.list().first_row();
    let cursor = window.list().cursor_row().expect("a row to drill into");
    window.open_thread(&cursor);
    pump_until(|| window.thread().rows().len() as u32 == WHOLE_THREAD);

    assert!(
        source.thread_reads.get() > 0,
        "the drill-in never asked for the thread, so whatever it is showing \
         came from the list's own model — which is the bug"
    );
    assert_eq!(
        window.thread().rows().len() as u32,
        WHOLE_THREAD,
        "the column shows only the part of the thread that was in this \
         folder; the two archived messages are missing"
    );

    // ── and the header stops hedging ─────────────────────────────────────
    // `n of m messages here` is what the column says when it knows it cannot
    // see the whole thread. Having read the thread, it can.
    let meta = window.thread().meta();
    assert!(
        !meta.contains(" of "),
        "the header still hedges with `n of m`: {meta}"
    );
    assert!(
        meta.starts_with(&format!("{WHOLE_THREAD} messages")),
        "the header should count the whole thread: {meta}"
    );

    window.destroy();
}

fn pump_until(ready: impl Fn() -> bool) {
    let context = gtk::glib::MainContext::default();
    for _ in 0..2000 {
        while context.iteration(false) {}
        if ready() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
