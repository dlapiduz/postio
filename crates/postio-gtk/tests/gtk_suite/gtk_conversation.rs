//! The conversation pane on a real display (ADR 0015 Q4, #308).
//!
//! The two decisions with consequences — where focus opens and how much
//! expands — are pure and unit-tested in `conversation.rs`. What needs a
//! display is everything they do not cover: that the pane actually builds an
//! entry per message, that focus is *drawn*, that jumping to a message
//! expands it, that a reader is created only for what is expanded, and that
//! reply and forward carry the message they were drawn on rather than the
//! conversation's.
//!
//! The last of those is the one worth a display test on its own. Reply,
//! reply-all and forward are the only per-message verbs in an otherwise
//! thread-level pane (ADR 0015 Q4), and answering the wrong message of a
//! conversation is the mistake the whole arrangement exists to prevent.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::conversation::{ConversationView, EAGER_EXPANSION_CAP};
use postio_gtk::list::Row;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// One message of the conversation, oldest first by id.
fn message(id: i64, seen: bool) -> Row {
    Row {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some(format!("Tide gate interlock {id}")),
        preview: Some(format!("Snippet {id}")),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    }
}

pub fn the_conversation_pane_stacks_a_thread_and_acts_per_message() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();

    // How many readers the pane asked for. Every expanded message is a
    // `WebKitWebView`, so this counter is the cost the cap exists to bound —
    // and a factory that is never called is a pane that draws nothing.
    let built: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    let counter = Rc::clone(&built);
    pane.set_reader_factory(move |message| {
        counter.borrow_mut().push(message);
        // A stand-in for the hardened reader: this test is about the stack,
        // and `gtk_reader.rs` covers what goes in each slot.
        gtk::Label::new(Some("body")).upcast::<gtk::Widget>()
    });

    window.set_child(Some(&pane.widget()));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    // Two read, then four unread: opening lands on the third.
    let messages: Vec<Row> = (0..6).map(|id| message(id, id < 2)).collect();
    pane.open(messages.clone());
    while gtk::glib::MainContext::default().iteration(false) {}

    // ── every message is in the pane ────────────────────────────────────
    assert_eq!(
        pane.len(),
        6,
        "the pane holds the whole conversation, not a window over it"
    );

    // ── focus opens on the first unread, and is drawn ───────────────────
    assert_eq!(
        pane.focused(),
        Some(MessageId::new(2)),
        "the pane opens where reading stopped, not at the end"
    );
    assert!(
        pane.is_focus_drawn(),
        "focus has to be visible: an unmarked current message is the one \
         thing a per-message verb needs the user to be sure of"
    );

    // ── read messages are collapsed, and the cap holds ──────────────────
    assert!(
        !pane.is_expanded(MessageId::new(0)) && !pane.is_expanded(MessageId::new(1)),
        "read messages open collapsed, which is what makes a long \
         conversation readable"
    );
    assert_eq!(
        built.borrow().len(),
        EAGER_EXPANSION_CAP,
        "opening a conversation must not build a reader per message: {:?}",
        built.borrow()
    );

    // ── jumping to a collapsed message expands it ───────────────────────
    // What the drill-in column does to this pane: the column is an index and
    // this is the content, so landing on a one-line header would be a dead
    // end.
    pane.focus_message(MessageId::new(0));
    while gtk::glib::MainContext::default().iteration(false) {}
    assert_eq!(pane.focused(), Some(MessageId::new(0)));
    assert!(
        pane.is_expanded(MessageId::new(0)),
        "jumping to a message has to open it — you went there to read it"
    );

    // ── reply and forward carry the message they were drawn on ──────────
    let replied: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&replied);
    pane.connect_reply(move |message, all| {
        assert!(!all, "this call is plain reply");
        seen.borrow_mut().push(message);
    });
    let forwarded: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    let seen_forward = Rc::clone(&forwarded);
    pane.connect_forward(move |message| seen_forward.borrow_mut().push(message));

    // Not the focused message, and not the newest: the fourth one, because
    // "reply to the message you clicked reply on" is the whole point.
    pane.test_click_reply(MessageId::new(3));
    pane.test_click_forward(MessageId::new(4));
    while gtk::glib::MainContext::default().iteration(false) {}

    assert_eq!(
        replied.borrow().as_slice(),
        &[MessageId::new(3)],
        "reply answered a different message than the one it was drawn on"
    );
    assert_eq!(
        forwarded.borrow().as_slice(),
        &[MessageId::new(4)],
        "forward carried a different message than the one it was drawn on"
    );

    // ── a fully-read conversation opens on its newest ───────────────────
    let read: Vec<Row> = (10..14).map(|id| message(id, true)).collect();
    pane.open(read);
    while gtk::glib::MainContext::default().iteration(false) {}
    assert_eq!(
        pane.focused(),
        Some(MessageId::new(13)),
        "with nothing unread, the newest is what you came back for"
    );
    assert!(
        pane.is_expanded(MessageId::new(13)),
        "the focused message is never left as a one-line header"
    );

    // ── dwell reads the focused message, and only that one ─────────────
    // #71's rule, one surface over. Opening a conversation must not mark
    // anything read, and walking the index must not read what it passes over
    // — the timer is cancelled when focus moves, and a `glib` timeout that
    // merely loses its handle still fires.
    let dwelled: Rc<RefCell<Vec<MessageId>>> = Rc::new(RefCell::new(Vec::new()));
    let seen_dwell = Rc::clone(&dwelled);
    pane.connect_dwelled(move |message| seen_dwell.borrow_mut().push(message));
    pane.set_dwell_delay(std::time::Duration::from_millis(30));

    let unread: Vec<Row> = (20..24).map(|id| message(id, false)).collect();
    pane.open(unread);
    settle_for(std::time::Duration::from_millis(10));
    assert!(
        dwelled.borrow().is_empty(),
        "opening a conversation read something: {:?}",
        dwelled.borrow()
    );

    // Walk past two without resting on either.
    pane.focus_message(MessageId::new(21));
    pane.focus_message(MessageId::new(22));
    // And rest on the third.
    pane.focus_message(MessageId::new(23));
    settle_for(std::time::Duration::from_millis(120));

    assert_eq!(
        dwelled.borrow().as_slice(),
        &[MessageId::new(23)],
        "dwell has to read the message that was rested on and nothing the \
         cursor passed over on the way"
    );

    window.close();
}

/// Pump the main loop for `how_long`, so a timer can fire.
fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + how_long;
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
