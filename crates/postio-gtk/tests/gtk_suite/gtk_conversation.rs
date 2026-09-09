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
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
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
        Some(stub_reader())
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
        EAGER_EXPANSION_CAP + 1,
        "opening a conversation must not build a reader per message. The cap, \
         plus exactly one: focus warms a reader for the message below it, so \
         moving down does not wait for a web process to start and flash black \
         while it does (#1216). One spare, and only after focus lands -- \
         during the expansions themselves there is no `next message` to \
         answer for: {:?}",
        built.borrow()
    );
    assert!(
        !built.borrow().contains(&MessageId::new(1)),
        "the spare is for the message *after* the focused one, not the last \
         collapsed one above it: {:?}",
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
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(move |_message| Some(stub_reader()));

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

/// An expanded entry's own reader does not draw its own action bar (#822).
///
/// The entry already carries a Reply/Reply all/Forward row of its own
/// (`build_entry`'s `conversation-actions`, deliberately without Archive —
/// every other verb is the conversation's, not one message in the stack's).
/// A factory that hands back a reader with its action bar still showing
/// draws that a second time, in a different style, with a fourth button
/// (Archive) nothing in this pane should offer per-message. This is what a
/// real factory (`postio_app::reading`'s `set_reader_factory` closure) must
/// suppress the same way it already hides the reader's own identity line.
pub fn an_expanded_entrys_reader_does_not_draw_its_own_action_bar() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(move |_message| {
        let reader = stub_reader();
        // What `postio_app::reading`'s real factory must do too.
        reader.set_actions_visible(false);
        Some(reader)
    });

    window.set_child(Some(&pane.widget()));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    let messages: Vec<Row> = (0..4).map(|id| message(id, id < 2)).collect();
    pane.open(messages);
    while gtk::glib::MainContext::default().iteration(false) {}

    let expanded = MessageId::new(2);
    assert!(
        pane.is_expanded(expanded),
        "the setup for this test changed"
    );
    let reader = pane
        .reader_for(expanded)
        .expect("an expanded entry has a reader");

    assert!(
        !reader.actions_visible(),
        "the factory suppressed the bar before handing the reader back"
    );

    // A body landing later re-renders into the same reader (#739) — the bar
    // must stay suppressed, not come back the moment something draws.
    reader.render(
        &postio_model::MessageBody {
            text: Some("a body that landed".into()),
            html: None,
        },
        None,
    );
    assert!(
        !reader.actions_visible(),
        "render() must not undo a suppression set before it"
    );

    window.close();
}

/// Pump the main loop for `how_long`, so a timer can fire.
fn settle_for(how_long: std::time::Duration) {
    let deadline = std::time::Instant::now() + postio_test_support::scaled(how_long);
    while std::time::Instant::now() < deadline {
        while gtk::glib::MainContext::default().iteration(false) {}
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// The header names the conversation, the dividers fold its middle, and the
/// footer carries its verbs (#1004, #1005, #1006).
///
/// Asserts on what a person would see — the words in the header, the text on
/// the divider, which rows are on screen — rather than on what the pane was
/// handed. A pane that was told about eight messages and drew none would pass
/// the second kind of test and fail this one.
pub fn the_pane_names_its_conversation_folds_its_middle_and_offers_its_verbs() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // Eight messages, all read but the last: the shape the canvas draws.
    // `expanded_on_open` opens the newest and collapses the seven before it.
    let mut messages: Vec<Row> = (1..=8).map(|id| message(id, true)).collect();
    messages[7].seen = false;
    let bo = EmailAddress::new(Some("Bo Ferris"), "bo@example.com");
    for message in messages.iter_mut().skip(4) {
        message.from = Some(bo.clone());
    }
    // Say the width. The header is two headers -- screen 28's, and screen
    // 29's shorter one for a window with no room for a rail -- and a test
    // that does not say which it is asking about gets whichever size the
    // compositor happened to give this window (#1380).
    pane.set_window_width(1400);
    pane.open(messages);
    crate::pump();

    // ── the header ────────────────────────────────────────────────────────
    assert_eq!(
        pane.header().subject(),
        "Tide gate interlock 1",
        "the subject is the conversation's, taken from its first message"
    );
    let meta = pane.header().meta();
    assert!(
        meta.starts_with("8 messages · "),
        "the header counts the stack it sits above: {meta}"
    );
    assert!(
        meta.contains("Ada, Bo"),
        "and names who is in it, shortened: {meta}"
    );

    // ── the folded run ────────────────────────────────────────────────────
    // Seven collapsed in a row, so one divider stands in for all of them.
    let dividers = pane.divider_labels();
    assert_eq!(dividers.len(), 1, "one run, one divider: {dividers:?}");
    assert!(
        dividers[0].starts_with("7 earlier messages · "),
        "the divider says how many it is hiding and who they are from: {}",
        dividers[0]
    );

    // `Show` puts the individual rows back — still collapsed, not opened.
    let before = pane.expanded_count();
    pane.show_run(0..7);
    crate::pump();
    assert!(
        pane.divider_labels().is_empty(),
        "showing a run replaces its divider with the rows themselves"
    );
    assert_eq!(
        pane.expanded_count(),
        before,
        "and opens nothing: `Show` is one step, not two"
    );

    // ── expand all ────────────────────────────────────────────────────────
    pane.expand_all();
    crate::pump();
    assert_eq!(
        pane.expanded_count(),
        8,
        "`O` opens every message, folded run included"
    );
    assert!(
        pane.divider_labels().is_empty(),
        "nothing is collapsed, so there is nothing left to fold"
    );

    // ── the footer ────────────────────────────────────────────────────────
    let footer = pane.footer();
    assert!(footer.is_visible(), "a thread has the thread's verbs");
    assert_eq!(
        footer
            .button(postio_core::CommandId::ArchiveThread)
            .expect("archive thread is in the footer")
            .key(),
        "A",
        "the footer button names the key that does the same thing"
    );
    // And nothing else. Reply is per message and is drawn on the message
    // (#1173): `Reply to conversation` ran the same command as the bar above
    // it, aimed at the same focused message, from a control that did not show
    // you which message that was.
    assert!(
        footer.button(postio_core::CommandId::Reply).is_none(),
        "the footer is the thread's bar, and reply is not a thread verb"
    );

    // An empty pane has nothing to name and no verbs to offer.
    pane.open(Vec::new());
    crate::pump();
    assert!(!footer.is_visible());

    window.close();
}

/// `J`/`K` walk the stack, `space` folds, and neither wraps (#1007).
///
/// Asserts on which message is focused and whether its body is showing —
/// what a person sees — rather than on the pane having been told to move.
pub fn the_keyboard_walks_the_stack_and_folds_what_it_lands_on() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // Four read messages: focus opens on the newest, per the pane's own
    // policy, which puts it at the end with nowhere further to go.
    pane.open((1..=4).map(|id| message(id, true)).collect());
    crate::pump();
    assert_eq!(
        pane.focused_index(),
        Some(3),
        "a fully-read conversation opens on its newest"
    );

    assert!(
        !pane.focus_next(),
        "there is nothing after the last message, and `J` says so rather \
         than wrapping round to the first"
    );
    assert_eq!(pane.focused_index(), Some(3), "and nothing moved");

    assert!(pane.focus_previous());
    assert_eq!(pane.focused_index(), Some(2));
    assert!(pane.focus_previous());
    assert!(pane.focus_previous());
    assert_eq!(pane.focused_index(), Some(0));
    assert!(
        !pane.focus_previous(),
        "and the same at the top: `K` stops at the first message"
    );

    // Landing expands, so what `J` walks onto is readable rather than a
    // one-line header you then have to open.
    let focused = pane.focused().expect("something is focused");
    assert!(
        pane.is_expanded(focused),
        "walking onto a message opens it -- a dead end is not a landing"
    );

    // `space` is the only way back to collapsed-and-focused.
    pane.toggle_fold();
    crate::pump();
    assert!(
        !pane.is_expanded(focused),
        "`space` folds the message the keyboard is on"
    );
    assert_eq!(
        pane.focused(),
        Some(focused),
        "and leaves the keyboard where it was"
    );

    pane.toggle_fold();
    crate::pump();
    assert!(
        pane.is_expanded(focused),
        "twice returns the pane to where it started"
    );

    // An empty pane has nowhere to walk, and says so rather than panicking.
    pane.open(Vec::new());
    crate::pump();
    assert!(!pane.focus_next());
    assert!(!pane.focus_previous());
    pane.toggle_fold();

    window.close();
}

/// Every widget under `root` whose CSS class ends in `-reply`, walking only
/// into subtrees that are actually on screen.
///
/// The class is what `widgets::Action` documents as "the CSS class a test
/// finds it by", and `-reply` excludes `-reply-all` because the class for
/// that one ends in `-all`. Counting what is *drawn* rather than asking the
/// two action tables what they hold is the whole point: the tables were each
/// right on their own, and the defect was the pair of them on screen at once.
fn reply_controls(root: &gtk::Widget) -> Vec<String> {
    let mut found = Vec::new();
    if !root.is_visible() {
        return found;
    }
    for class in root.css_classes() {
        if class.ends_with("-reply") {
            found.push(class.to_string());
        }
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        found.extend(reply_controls(&widget));
        child = widget.next_sibling();
    }
    found
}

/// One thread, one `Reply` on screen — at any length (#1173).
///
/// The one-message case was the visible half: both bars drew and `Reply`
/// appeared twice with `e` printed on each. The n>1 case is the same defect
/// with a pane's height between the two buttons, which is why it survived
/// three issues while the n=1 one was reported at once.
///
/// `Reply to conversation` runs `CommandId::Reply` aimed at the *focused*
/// message, so it is the same command and the same key as the bar drawn on
/// that message — under a label naming a scope Postio does not have. You
/// reply to a message, never to a thread, and a pinned control drawn a pane
/// away from the message it will answer is the mistake ADR 0015 Q4 gives as
/// its reason for making the reply verbs per-message: "answering the wrong
/// message of a conversation is a real and common mistake".
pub fn one_thread_offers_one_reply_however_long_it_is() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // ── a thread of one ───────────────────────────────────────────────────
    pane.open(vec![message(1, false)]);
    crate::pump();
    let found = reply_controls(&pane.widget());
    assert_eq!(
        found.len(),
        1,
        "a one-message thread drew {} controls bound to reply: {found:?}",
        found.len()
    );

    // ── and a thread of eight ─────────────────────────────────────────────
    let mut messages: Vec<Row> = (1..=8).map(|id| message(id, true)).collect();
    messages[7].seen = false;
    pane.open(messages);
    crate::pump();
    let found = reply_controls(&pane.widget());
    assert_eq!(
        found.len(),
        1,
        "a thread drew {} controls bound to reply: {found:?}. Both run \
         `CommandId::Reply` on the focused message, so the second is a \
         duplicate that hides which message it answers",
        found.len()
    );

    window.close();
}

/// `Expand all` is offered only when there is something to expand (#1173).
///
/// The same n=1 surface as the double `Reply`, and it was read as a second
/// bug in the issue — "the header still offers `Expand all 0`". The `0` is
/// the keycap for `O`, which the mono face draws close enough to a zero at
/// that size to look like a count, so there is no "expand nothing" arithmetic
/// to fix. What is real is the control: a one-message thread opens that
/// message expanded, so the button is offered with nothing left to do.
pub fn expand_all_is_offered_only_when_there_is_something_to_expand() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));
    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    pane.open(vec![message(1, false)]);
    crate::pump();
    assert!(
        !pane.header().offers_expand_all(),
        "a thread of one opens expanded, so there is nothing for `O` to do"
    );

    let mut messages: Vec<Row> = (1..=8).map(|id| message(id, true)).collect();
    messages[7].seen = false;
    pane.open(messages);
    crate::pump();
    assert!(
        pane.header().offers_expand_all(),
        "seven of the eight open collapsed, which is exactly what `O` is for"
    );

    window.close();
}

/// A draft is offered the one verb that is true of it (#1212).
///
/// Nothing in the pane used to branch on `row.draft`, so a message *you*
/// wrote and never sent was drawn under the ordinary bar: a primary `Reply`
/// that would quote your own unsent text back at you, and a `Reply all`
/// addressed to yourself. The list row knew — it draws the draft mark and
/// says "Draft" to a screen reader — and the pane the row opens into did not.
///
/// The verb that is right was reachable and unannounced: activating the row
/// resumes the composer on the draft. So the bar names it, and names the
/// message, because a draft inside a longer thread is not the row the list
/// cursor is on.
pub fn a_draft_is_offered_continue_editing_and_no_reply() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(stub_reader()));

    let ran: Rc<RefCell<Vec<postio_core::Command>>> = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&ran);
    pane.connect_command(move |command| seen.borrow_mut().push(command));

    window.set_child(Some(&pane.widget()));
    window.set_default_size(700, 600);
    window.present();
    crate::pump();

    // A thread of two: a message and the unsent reply to it. The draft is
    // not the row the list holds, which is what makes naming it matter.
    let mut messages: Vec<Row> = (1..=2).map(|id| message(id, true)).collect();
    messages[1].draft = true;
    let draft = messages[1].id;
    pane.open(messages);
    crate::pump();

    let found = reply_controls(&pane.widget());
    assert!(
        found.is_empty(),
        "a draft was offered {found:?}: you do not reply to a message you \
         wrote and never sent"
    );

    pane.focus_message(draft);
    crate::pump();
    pane.press_entry_command(draft, postio_core::CommandId::OpenMessage);
    crate::pump();
    assert_eq!(
        ran.borrow().as_slice(),
        [postio_core::Command::OpenMessage {
            message: Some(draft)
        }],
        "`Continue editing` names the draft it is drawn on, so a draft that \
         is not the thread's own row still opens the right one"
    );

    window.close();
}

/// #1241: a row knows whether the message is one the user sent.
///
/// `Design/screens/18-conversation-row-states.png` gives four states a mark
/// each, and three of them — read, unread, focused — the row could already
/// tell apart from `Row::seen` and its own selection. `mine` it could not:
/// nothing in `postio-gtk` knows the account's addresses, so the pane is
/// told them and folds the comparison once.
///
/// The mark itself is drawn into a `snapshot()` and cannot be read back from
/// a widget tree, so what is asserted is the state the drawing depends on.
/// `scripts/screens.sh --only conversation` is what puts the pixels beside
/// `17-conversation-view.png`.
pub fn a_row_knows_whether_the_message_is_the_users_own() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = gtk::Window::new();
    let pane = ConversationView::new();
    window.set_child(Some(&pane.widget()));
    window.present();

    let from = |id: i64, address: &str| Row {
        from: Some(EmailAddress::new(Some("Someone"), address)),
        ..message(id, true)
    };

    pane.open(vec![
        from(1, "ada@example.com"),
        from(2, "me@example.net"),
        from(3, "ADA@EXAMPLE.COM"),
    ]);
    settle_for(std::time::Duration::from_millis(20));

    // Nobody has said which addresses are the user's, so nothing is theirs.
    assert_eq!(mine_flags(&pane), vec![false, false, false]);

    pane.set_own_addresses(&[EmailAddress::new(None::<String>, "me@example.net")]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(
        mine_flags(&pane),
        vec![false, true, false],
        "only the message from the account's own address is the user's"
    );

    // Addresses are case-insensitive, and a thread quotes them however the
    // sender typed them.
    pane.set_own_addresses(&[EmailAddress::new(None::<String>, "Ada@Example.com")]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(
        mine_flags(&pane),
        vec![true, false, true],
        "ADA@EXAMPLE.COM and ada@example.com are the same person"
    );

    // Identities can change while a conversation is open, and a mark that
    // was right when the pane opened is not one anybody checks again.
    pane.set_own_addresses(&[]);
    settle_for(std::time::Duration::from_millis(20));
    assert_eq!(mine_flags(&pane), vec![false, false, false]);

    window.destroy();
}

/// Each row's `mine`, in stack order.
fn mine_flags(pane: &ConversationView) -> Vec<bool> {
    pane.rows()
        .iter()
        .filter_map(|row| pane.header_for(row.id))
        .map(|header| header.is_mine())
        .collect()
}

/// Reading down a long thread does not accumulate a body per message.
///
/// `EAGER_EXPANSION_CAP` bounds how many open when a conversation opens.
/// Nothing bounded how many accumulate as it is *scrolled*: `expand` built a
/// reader the first time each message opened and `collapse` deliberately kept
/// it, so a thirty-message thread ended with thirty `WebKitWebView`s and
/// thirty web processes, held until the thread changed. At roughly 50 MB each
/// that is over a gigabyte for one conversation (#1216).
pub fn a_long_thread_keeps_a_bounded_number_of_bodies() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let window = gtk::Window::new();
    let pane = ConversationView::new();

    let live: Rc<RefCell<usize>> = Rc::new(RefCell::new(0));
    let counter = Rc::clone(&live);
    pane.set_reader_factory(move |_message| {
        *counter.borrow_mut() += 1;
        Some(stub_reader())
    });

    window.set_child(Some(&pane.widget()));
    window.present();
    while gtk::glib::MainContext::default().iteration(false) {}

    let messages: Vec<Row> = (0..30).map(|id| message(id, true)).collect();
    pane.open(messages.clone());
    while gtk::glib::MainContext::default().iteration(false) {}

    // Read down the whole thread, as a person does.
    for id in 0..30 {
        pane.focus_message(MessageId::new(id));
        while gtk::glib::MainContext::default().iteration(false) {}
    }

    let held = pane.live_body_count();
    assert!(
        held <= postio_gtk::conversation::LIVE_BODY_CAP,
        "reading down a thirty-message thread left {held} live bodies, over \
         the {} cap -- each is a `WebKitWebView` and a web process",
        postio_gtk::conversation::LIVE_BODY_CAP
    );

    // And the ones kept are the ones near where the reading stopped, not an
    // arbitrary six: scrolling back a message must not rebuild.
    assert!(
        pane.reader_for(MessageId::new(29)).is_some(),
        "the message the reader stopped on must still have its body"
    );

    // Scrolling back rebuilds rather than showing an empty body.
    pane.focus_message(MessageId::new(0));
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(
        pane.reader_for(MessageId::new(0)).is_some(),
        "an entry whose body was released must rebuild when it is opened \
         again, not stay expanded and empty"
    );

    window.set_visible(false);
}
