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
use postio_gtk::reader::Reader;
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};

/// A reader with nothing behind it — no blob source worth naming, since
/// nothing here asks it to resolve a `cid:`. Good enough to stand in for the
/// hardened one everywhere this file only cares that a reader was built, not
/// what it can render.
fn stub_reader() -> Reader {
    Reader::new(Rc::new(|_content_id: &str| None))
}

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
        // A bare reader: this test is about the stack, and `gtk_reader.rs`
        // covers what goes in each slot.
        stub_reader()
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

    // A dwell nobody could out-wait, so "has anything been read *yet*" is a
    // question about the code rather than about how loaded the machine is.
    // The first version of this asserted the same thing with a 30ms dwell and
    // a 10ms settle, which passed alone and failed in the full suite: under
    // load the settle outran the timer.
    pane.set_dwell_delay(std::time::Duration::from_secs(30));

    let unread: Vec<Row> = (20..24).map(|id| message(id, false)).collect();
    pane.open(unread);
    settle_for(std::time::Duration::from_millis(50));
    assert!(
        dwelled.borrow().is_empty(),
        "opening a conversation read something before anybody had rested on \
         it: {:?}",
        dwelled.borrow()
    );

    // Opening *does* start the clock on the message it focused, and that is
    // right: you pressed `t`, the message is expanded in front of you, and
    // resting on it is reading it. What must never happen is the whole
    // conversation going read because it was opened -- so exactly one
    // message is readable at a time, and it is the focused one.
    pane.set_dwell_delay(std::time::Duration::from_millis(30));

    // Walk past two without resting on either, then rest on the third. Each
    // move cancels the last one's timer; a `glib` timeout that merely lost
    // its handle would still fire and read a message nobody looked at.
    pane.focus_message(MessageId::new(21));
    pane.focus_message(MessageId::new(22));
    pane.focus_message(MessageId::new(23));
    settle_for(std::time::Duration::from_millis(200));

    assert_eq!(
        dwelled.borrow().as_slice(),
        &[MessageId::new(23)],
        "dwell has to read the message that was rested on, and nothing the \
         cursor passed over on the way to it"
    );

    window.close();
}

/// `reader_for` finds the reader already built for an expanded entry, and
/// nothing for anything else (#739).
///
/// This is the seam a body or a payload landing for a message the
/// conversation pane is already showing has to come back through:
/// `expand` only ever builds a reader once, so re-drawing an arrival into
/// the *same* one — rather than tearing an entry down to rebuild it — starts
/// with finding it again.
pub fn reader_for_finds_only_an_expanded_entrys_own_reader() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(move |_message| stub_reader());

    window.set_child(Some(&pane.widget()));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    // Two read (collapsed), then two unread (expanded up to the cap).
    let messages: Vec<Row> = (0..4).map(|id| message(id, id < 2)).collect();
    pane.open(messages);
    while gtk::glib::MainContext::default().iteration(false) {}

    let expanded = MessageId::new(2);
    let collapsed = MessageId::new(0);
    let absent = MessageId::new(99);
    assert!(
        pane.is_expanded(expanded),
        "the setup for this test changed"
    );
    assert!(
        !pane.is_expanded(collapsed),
        "the setup for this test changed"
    );

    assert!(
        pane.reader_for(collapsed).is_none(),
        "a collapsed entry has no reader to find — expand builds one, this \
         does not"
    );
    assert!(
        pane.reader_for(absent).is_none(),
        "a message outside the conversation should have nothing to find"
    );

    let reader = pane
        .reader_for(expanded)
        .expect("an expanded entry has a reader");
    assert_eq!(
        reader.paints(),
        0,
        "the factory in this test never rendered anything"
    );

    // Draw into it directly, the way a repaint on an arrival would.
    reader.render(
        &postio_model::MessageBody {
            text: Some("a body that landed".into()),
            html: None,
        },
        None,
    );

    // Asking again returns the *same* reader — the point of keeping it,
    // rather than `expand`'s factory being called a second time — so the
    // paint just made is still on it.
    let same = pane
        .reader_for(expanded)
        .expect("the entry is still expanded");
    assert_eq!(
        same.paints(),
        1,
        "reader_for handed back a different reader than the one drawn into, \
         so the entry does not carry the paint forward"
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
