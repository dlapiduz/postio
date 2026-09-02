//! `h`/`Left` steps back out of a thread, from the keyboard (#765).
//!
//! `PrevView` was a registry command — binding, title, doc comment promising
//! "step back to the previous view without leaving the keyboard" — with no
//! handler anywhere: not in `Window::handled_here`, not in
//! `follow_drill_in`, not on the bus. Invoking it did nothing. It reads as
//! `Back`'s keyboard-only sibling (`Back` is `Escape`, reachable everywhere;
//! `PrevView` is `h`/`Left`, reachable only from the message surfaces
//! `Back` already knows how to leave a thread from), so this proves the
//! same round trip `gtk_thread.rs` proves for `Back`: the real command,
//! through `Window::act`, actually closes the thread.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display. Nothing here touches the network.

use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest,
};
use postio_gtk::list::Row;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{AccountId, MailboxId, MessageId, ThreadId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

/// One thread, two messages — just enough for a drill-in to have something
/// to open and something to leave.
struct TinyThread;

impl MessageSource for TinyThread {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let rows: Vec<Row> = (1..=2)
            .map(|index| Row {
                id: MessageId::new(index),
                thread: Some(ThreadId::new(1)),
                from: Some(EmailAddress::new(
                    Some("Correspondent".to_owned()),
                    "person@example.org".to_owned(),
                )),
                subject: Some("Re: the thread".to_owned()),
                preview: Some("…".to_owned()),
                received_at: chrono::Utc::now(),
                seen: true,
                flagged: false,
                answered: false,
                draft: false,
                has_attachments: false,
                thread_count: 2,
                participants: Vec::new(),
            })
            .collect();
        let start = (request.offset as usize).min(rows.len());
        let end = (start + request.limit as usize).min(rows.len());
        let page = rows[start..end].to_vec();
        let total = rows.len() as u32;
        Box::pin(async move { Ok(Page { total, rows: page }) })
    }
}

impl MailboxSource for TinyThread {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
        inbox.id = MailboxId::new(1);
        inbox.role = MailboxRole::Inbox;
        inbox.counts = MailboxCounts {
            total: 2,
            unread: 0,
            flagged: 0,
            snoozed: 0,
        };
        Box::pin(async move { Ok(vec![inbox]) })
    }
}

fn pump() {
    while glib::MainContext::default().iteration(false) {}
}

fn settle(window: &Window, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        pump();
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(done(), "timed out waiting for {what} in window {window:?}");
}

pub fn h_steps_back_out_of_a_thread_the_same_way_escape_does() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let account = AccountId::new(1);
    let source = std::rc::Rc::new(TinyThread);
    let feeds = window.install_feeds(account, "lena@example.com", source.clone(), source);
    window.present();
    pump();
    settle(&window, "the list to have a row to drill into", || {
        window.list().model().n_items() > 0
    });

    // A bare invocation, with no row named, means the cursor's own thread —
    // same as `t` from the keyboard.
    assert!(!window.thread_open());
    window.act(postio_core::Command::Thread { thread: None });
    pump();
    assert!(window.thread_open(), "the drill-in should have opened");

    // ── the real command, not a widget call ────────────────────────────
    window.act(postio_core::Command::PrevView);
    pump();
    assert!(
        !window.thread_open(),
        "PrevView through Window::act should have closed the thread"
    );
    assert_eq!(window.context(), Context::List);

    // A second PrevView, with nowhere left to step back from, is a no-op —
    // not a panic, not a phantom close of something already closed.
    window.act(postio_core::Command::PrevView);
    pump();
    assert!(!window.thread_open());

    drop(feeds);
    window.destroy();
}
