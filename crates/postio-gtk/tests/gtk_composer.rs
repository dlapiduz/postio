//! The composer on a real display: canvas 2a's takeover, end to end.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_cheatsheet.rs`.
//!
//! The rules that are pure — where the keyboard lands, what closing keeps,
//! how a bad address is counted — are unit-tested in `src/composer.rs` with no
//! display. What needs one is everything this file walks: that `c` really
//! takes the reading pane and nothing else, that the *list* comes through it
//! untouched (the whole argument for taking over a pane rather than opening a
//! window), that `Esc` gives the draft back rather than eating it, and that
//! `ctrl+Enter` hands the draft on and puts the pane back.
//!
//! The list here is a stand-in — a `GtkListBox` in the list pane rather than
//! the real windowed model — on purpose: the criterion is that the composer
//! does not disturb *whatever* is in that pane, and a stand-in with a
//! selection and a scroll offset states it without dragging storage into a
//! widget test.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::composer::{self, Closing, Field};
use postio_gtk::shell::Pane;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft, DraftKind, EmailAddress, MessageBody};

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// A draft with something in every field a user would have typed.
fn started() -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    draft.subject = "the mbox importer".to_owned();
    draft.body = MessageBody {
        text: Some("Looking now.".to_owned()),
        html: None,
    };
    draft
}
#[test]

fn the_composer_takes_the_reading_pane_and_gives_it_back() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    let shell = window.shell();

    // ── A list with a place in it, and something in the reading pane ──────
    let rows = gtk::ListBox::new();
    for index in 0..40 {
        rows.append(&gtk::Label::new(Some(&format!("message {index}"))));
    }
    let scroller = gtk::ScrolledWindow::builder()
        .vexpand(true)
        .child(&rows)
        .build();
    shell.list().append(&scroller);

    let reading = gtk::Label::new(Some("the message being read"));
    shell.reader().append(&reading);
    // Registered with the pane's arbiter as the reader surface (#502): the
    // pane has one owner now, and a stand-in that just sat in the box would
    // be invisible to it — hidden when the composer claims, and never
    // restored, because restoring is computed from who is active rather
    // than replayed from what was visible.
    shell.register_reader_occupant(
        postio_gtk::shell::ReaderOccupant::Reader,
        reading.upcast_ref(),
    );
    shell.claim_reading();

    window.present();
    settle();

    let chosen = rows.row_at_index(7).unwrap();
    rows.select_row(Some(&chosen));
    scroller.vadjustment().set_value(120.0);
    settle();
    let scroll = scroller.vadjustment().value();

    let composer = composer::install(&window);
    let sent: Rc<RefCell<Vec<Draft>>> = Rc::new(RefCell::new(Vec::new()));
    let closings: Rc<RefCell<Vec<Closing>>> = Rc::new(RefCell::new(Vec::new()));
    composer.connect_closed({
        let closings = Rc::clone(&closings);
        move |outcome| closings.borrow_mut().push(outcome)
    });

    assert!(!composer.is_open(), "not in the way until asked for");

    // ── `c` opens it, over the reader and nothing else ───────────────────
    press(&window, "c", gdk::ModifierType::empty());

    assert!(composer.is_open(), "`c` composes");
    assert_eq!(
        window.context(),
        Context::Composer,
        "the keyboard belongs to the composer"
    );
    assert!(
        shell.has_css_class(composer::COMPOSING_CLASS),
        "the shell says it is composing, which is what dims the list"
    );
    assert!(!reading.is_visible(), "the composer replaced the message");
    assert_eq!(
        composer.focused_field(),
        Some(Field::To),
        "new mail starts in To"
    );

    // ── …and the list is exactly where it was ────────────────────────────
    assert!(shell.list().is_visible(), "the list stays put");
    assert_eq!(
        rows.selected_row().map(|row| row.index()),
        Some(7),
        "composing must not disturb the list's selection"
    );
    assert_eq!(
        scroller.vadjustment().value(),
        scroll,
        "composing must not disturb the list's scroll"
    );

    // ── …and it costs a dispatch, not a rebuild ──────────────────────────
    //
    // This used to time the key press and assert it under the 16ms budget,
    // and it failed whenever another worktree was compiling (#796). A budget
    // asserted inside `cargo test` measures the machine, and this machine's
    // normal state is several sessions building at once — so the failure
    // arrived at the last gate before a merge and read as "the change I am
    // landing broke the composer".
    //
    // What the test can hold regardless of load is the *reason* the budget
    // holds, which is what the old comment here already said it was checking:
    // the widgets exist already and opening is a `set_visible`. Both halves
    // of that are asserted below, and the number itself now lives in
    // `postio-bench`'s `composer_open`, where a measurement is deliberate and
    // nothing is blocked on the result.
    press(&window, "Escape", gdk::ModifierType::empty());

    // The composer's own child, before. Widget equality in gtk-rs is object
    // identity, so this is "the same widget", not "a widget that looks the
    // same".
    let built_before = composer
        .first_child()
        .expect("the composer has its widgets before `c` is ever pressed");

    window.handle_key(
        gdk::Key::from_name("c").unwrap(),
        gdk::ModifierType::empty(),
    );

    // Deliberately before `settle()`: opening is a `set_visible`, so it has
    // happened by the time the key handler returns. If it ever became a
    // transition, a revealer or anything deferred to a frame, this is where
    // that shows — and *that* is what would put opening over the budget, on
    // any machine, load or no load.
    assert!(
        composer.is_open(),
        "the composer was not open when the key handler returned, so opening \
         is waiting for a frame -- an animation or an idle callback -- rather \
         than being the `set_visible` the interaction budget assumes"
    );
    assert!(
        composer.is_visible(),
        "the composer is open but not visible, which means something is \
         revealing it over time"
    );

    settle();
    assert!(composer.is_open());
    assert_eq!(
        composer.first_child().as_ref(),
        Some(&built_before),
        "opening the composer replaced its widgets instead of showing the \
         ones that were already there; a rebuild is what the interaction \
         budget cannot absorb, and it is invisible in a screenshot"
    );

    // ── An untouched composer closes with nothing to keep ────────────────
    press(&window, "Escape", gdk::ModifierType::empty());
    assert_eq!(
        closings.borrow().last().copied(),
        Some(Closing::Drop),
        "nothing was written, so nothing was discarded"
    );

    // ── `Esc` closes it and keeps every word ─────────────────────────────
    composer.open(started());
    settle();
    assert!(composer.is_open());
    press(&window, "Escape", gdk::ModifierType::empty());

    assert!(!composer.is_open(), "Esc leaves the composer");
    assert_eq!(
        closings.borrow().last().copied(),
        Some(Closing::Keep),
        "Esc keeps a draft that has anything in it"
    );
    assert_eq!(
        window.context(),
        Context::List,
        "and gives the keyboard back to where it came from"
    );
    assert!(reading.is_visible(), "the message is back in the pane");
    assert!(!shell.has_css_class(composer::COMPOSING_CLASS));

    press(&window, "c", gdk::ModifierType::empty());
    let kept = composer.draft();
    assert_eq!(kept.subject, "the mbox importer", "the draft came back");
    assert_eq!(
        kept.body.text.as_deref(),
        Some("Looking now."),
        "including the body, which exists nowhere else"
    );
    assert_eq!(kept.to.len(), 1, "including the recipient");

    // ── …but a forward asked for next is the forward, not the kept draft ─
    // #691, reported by a user: close a draft with anything in it, then ask
    // for a reply or a forward, and the retained draft won — the composer
    // reopened on the old composition and the forward was thrown away
    // without a word. `c` means "show me the draft I kept"; naming a
    // specific message does not.
    press(&window, "Escape", gdk::ModifierType::empty());
    settle();
    let mut forward = Draft::new(AccountId::UNASSIGNED);
    forward.kind = DraftKind::Forward;
    forward.subject = "Fwd: the tide gate interlock".to_owned();
    forward.body = MessageBody {
        text: Some("> the message being forwarded".to_owned()),
        html: None,
    };
    composer.open(forward);
    settle();
    let showing = composer.draft();
    assert_eq!(
        showing.subject, "Fwd: the tide gate interlock",
        "the forward the user asked for was discarded and the kept draft \
         shown instead"
    );
    assert_eq!(
        showing.kind,
        DraftKind::Forward,
        "and it is still a forward, not a composition wearing its subject"
    );

    // ── A reply starts in the body ───────────────────────────────────────
    composer.discard();
    settle();
    let mut reply = started();
    reply.kind = DraftKind::Reply;
    composer.open(reply);
    settle();
    assert!(composer.is_open());
    assert_eq!(
        composer.focused_field(),
        Some(Field::Body),
        "a reply starts in the body"
    );

    // ── Send with nothing wired keeps the mail rather than losing it ─────
    press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
    assert!(
        composer.is_open(),
        "with no send path the composer stays open"
    );
    assert!(
        composer.status().contains("no outgoing account"),
        "and says why: {:?}",
        composer.status()
    );

    // ── …and with a handler, it hands the draft on and lets the pane go ──
    composer.connect_send({
        let sent = Rc::clone(&sent);
        move |draft| sent.borrow_mut().push(draft.clone())
    });
    press(&window, "Return", gdk::ModifierType::CONTROL_MASK);

    assert_eq!(sent.borrow().len(), 1, "the draft went to the handler");
    assert_eq!(sent.borrow()[0].subject, "the mbox importer");
    assert_eq!(
        sent.borrow()[0].to,
        vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
        "the fields were read back as addresses, not as text"
    );
    assert!(!composer.is_open(), "sending closes the composer");
    assert!(reading.is_visible(), "and gives the reading pane back");
    assert_eq!(
        window.shell().focused_pane(),
        Pane::List,
        "and the pane the narrow layout would show"
    );
    assert!(
        composer.draft().subject.is_empty(),
        "a sent draft is not still sitting in the composer"
    );

    // ── ctrl+Return refuses what the Send button is greyed out for ───────
    //
    // The button has always been insensitive on `Draft::is_sendable`; the key
    // reached `send` directly. That cost nothing while the seam was unwired
    // (#423) because every send was a no-op. Now that a send queues, the two
    // have to agree — see `Composer::send` for what each disagreement costs.
    let mut unaddressed = started();
    unaddressed.to.clear();
    composer.open(unaddressed);
    settle();
    press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        sent.borrow().len(),
        1,
        "a draft addressed to nobody was handed to the send seam anyway; it \
         would clear the composer and drain as impossible, losing the words"
    );
    assert!(composer.is_open(), "so the composer keeps it");
    assert!(
        composer.status().contains("add a recipient"),
        "and says what is missing: {:?}",
        composer.status()
    );

    // Already on its way: reachable because a queued draft keeps its row in
    // the Drafts folder until the drainer deletes it, so it can be resumed
    // between the enqueue and the send. A second `Operation::Send` against
    // one draft is the recipient receiving the message twice.
    composer.discard();
    settle();
    let mut queued = started();
    queued.state = postio_model::DraftState::Queued;
    composer.open(queued);
    settle();
    press(&window, "Return", gdk::ModifierType::CONTROL_MASK);
    assert_eq!(
        sent.borrow().len(),
        1,
        "a draft already handed to the queue was queued a second time"
    );
    assert!(composer.is_open());
    assert!(
        composer.status().contains("already on its way"),
        "and says why, rather than naming a recipient that is right there: \
         {:?}",
        composer.status()
    );
    composer.discard();
    settle();

    // ── The header's Compose button reaches the same command ─────────────
    //
    // Every action needs a key *and* a control: `c` and this button are the
    // same `win.compose`, not two paths that could drift apart.
    assert_eq!(
        postio_gtk::header::build().compose.action_name().as_deref(),
        Some("win.compose"),
        "the button in the header names the action the composer installs"
    );
    gtk::prelude::WidgetExt::activate_action(&window, "win.compose", None).unwrap();
    settle();
    assert!(composer.is_open(), "and activating it composes");
    press(&window, "Escape", gdk::ModifierType::empty());

    // ── The list came through all of it untouched ────────────────────────
    assert_eq!(rows.selected_row().map(|row| row.index()), Some(7));
    assert_eq!(scroller.vadjustment().value(), scroll);
}
