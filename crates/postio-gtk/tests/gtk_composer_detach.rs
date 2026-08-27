//! Popping the composition out into its own window, and putting it back.
//!
//! Its own file: GTK is initialised once per process, so one `#[test]` per
//! integration binary. See `gtk_composer.rs`.
//!
//! In-place is the default and stays the default — the whole argument for
//! canvas 2a's takeover is that the list keeps its scroll and its selection
//! so you never lose your place. The one thing it genuinely cannot do is let
//! you read something *else* while you write, and that is what detaching is
//! for. So what this file walks is the pair of claims that make the opt-in
//! worth having:
//!
//! * detaching costs nothing — it is the same widget reparented, so every
//!   field, the identity override and the cursor position survive because
//!   they were never rebuilt; and
//! * the main window comes all the way back — the message returns to the
//!   reading pane, the keyboard context leaves `Composer`, and the list is
//!   untouched throughout, exactly as if the composer had closed.
//!
//! The second one is why the test keeps a list stand-in with a selection and
//! a scroll offset: "the list is untouched" is an acceptance criterion and
//! not something to take on faith from a reparent.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::{CommandId, Context, registry};
use postio_gtk::composer::{self, Closing, Field};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft, EmailAddress, MessageBody};

/// Whether anything under `widget` is an `AdwHeaderBar`.
///
/// `AdwWindow` draws no titlebar of its own — a bare `set_content` gives a
/// window with no title, no close button and nothing to drag it by, which
/// looks like a stray rectangle rather than a pop-out. The content has to
/// provide the chrome, and this is the only way to say so as an assertion.
fn has_header_bar(widget: &gtk::Widget) -> bool {
    if widget.is::<adw::HeaderBar>() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if has_header_bar(&current) {
            return true;
        }
        child = current.next_sibling();
    }
    false
}

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

/// A key press into the main window, the way `gtk_composer.rs` does it —
/// GTK4 gives no supported way to synthesize a GDK event.
fn press(window: &Window, key: &str, modifiers: gdk::ModifierType) {
    window.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// The same, but into the detached window: a different focus and a context
/// the main window is no longer in. Goes through the very call the detached
/// window's own controller makes, so this is the real keyboard path.
fn press_detached(composer: &composer::Composer, key: &str, modifiers: gdk::ModifierType) {
    composer.handle_key(gdk::Key::from_name(key).unwrap(), modifiers);
    settle();
}

/// A draft with something in every field a user would have typed.
fn started() -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    draft.subject = "the mbox importer".to_owned();
    draft.body = MessageBody {
        text: Some("Looking now — one moment.".to_owned()),
        html: None,
    };
    draft
}

#[test]
fn the_composer_detaches_into_its_own_window_and_comes_back() {
    let state_dir = std::env::temp_dir().join(format!("postio-detach-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── The registry is the single source the binding, the palette entry and
    //    the cheat sheet all come from, so it is where reachability starts ──
    let spec = registry::get(CommandId::DetachComposer);
    assert!(
        spec.available_in(Context::Composer),
        "it is only reachable while there is a composition to detach"
    );
    assert!(
        registry::reachable(Context::Composer).any(|action| action.title == spec.title),
        "and it is in the palette and the cheat sheet, from that one entry"
    );
    assert_eq!(
        spec.default_binding, "ctrl+shift+o",
        "and it has a key of its own, so it is not pointer-only"
    );

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
    // Registered as the pane's reader surface — see gtk_composer.rs (#502).
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
    assert!(
        !composer.is_detached(),
        "nothing opens detached; it is opt-in"
    );

    // ── Compose in place, and write something worth not losing ───────────
    composer.open(started());
    settle();
    composer.test_set_body("Looking now — one moment. Second paragraph.");
    // Into the body, which is not where a fresh composition starts — so
    // "the focus survived" below means it was carried, not re-defaulted.
    assert!(composer.test_focus_field(Field::Body));
    settle();
    assert_eq!(composer.focused_field(), Some(Field::Body));

    assert!(composer.is_open());
    assert!(!composer.is_detached(), "in-place is the default");
    let before = composer.draft();
    let cursor = composer.test_cursor_offset();
    assert!(
        cursor > 0,
        "the test needs a cursor that is not at the start"
    );

    // ── The pointer control is there too, per the acceptance criterion ────
    let button = composer.test_detach_button();
    assert!(
        button.is_visible(),
        "detaching must be reachable by pointer, not only by key"
    );

    // ── `ctrl+shift+o` pops it out ───────────────────────────────────────
    press(
        &window,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );

    assert!(composer.is_detached(), "the composition moved to a window");
    assert!(
        composer.is_open(),
        "moving it is not closing it — there is still exactly one composition"
    );

    let detached = composer
        .detached_window()
        .expect("a detached composer has a window");
    assert!(
        !detached.is_modal(),
        "a modal would block the main window, which is the point of detaching"
    );
    assert_eq!(
        detached.transient_for().map(|w| w.upcast::<gtk::Window>()),
        Some(window.clone().upcast::<gtk::Window>()),
        "it belongs to the main window rather than floating loose"
    );
    assert_eq!(
        composer.root().and_downcast::<gtk::Window>(),
        Some(detached.clone().upcast::<gtk::Window>()),
        "the composer really is inside it — one widget, reparented"
    );
    assert!(
        adw::prelude::AdwWindowExt::content(&detached)
            .is_some_and(|content| has_header_bar(&content)),
        "a pop-out with no titlebar cannot be closed, moved or named"
    );
    assert_eq!(
        gtk::prelude::GtkWindowExt::title(&detached).as_deref(),
        Some("Compose"),
        "and the titlebar says which composition it is holding"
    );

    // ── Nothing was lost on the way out ──────────────────────────────────
    let after = composer.draft();
    assert_eq!(after.to, before.to, "recipients survived the move");
    assert_eq!(after.subject, before.subject, "the subject survived");
    assert_eq!(after.body.text, before.body.text, "the body survived");
    assert_eq!(after.kind, before.kind, "and it is still the same kind");
    assert_eq!(
        composer.identity(),
        None,
        "the identity override is carried, not reset"
    );
    assert_eq!(
        composer.test_cursor_offset(),
        cursor,
        "the cursor did not jump — the buffer was never rebuilt"
    );
    assert_eq!(
        composer.focused_field(),
        Some(Field::Body),
        "and the keyboard is still in the field it was in"
    );

    // ── …and the main window is all the way back ─────────────────────────
    assert!(
        reading.is_visible(),
        "the reading pane returned to what it was showing"
    );
    assert_eq!(
        window.context(),
        Context::List,
        "the main window's keyboard left the composer with it"
    );
    assert!(
        !shell.has_css_class(composer::COMPOSING_CLASS),
        "and the shell stopped saying it was composing"
    );
    assert!(shell.list().is_visible(), "the list stays put");
    assert_eq!(
        rows.selected_row().map(|row| row.index()),
        Some(7),
        "detaching must not disturb the list's selection"
    );
    assert_eq!(
        scroller.vadjustment().value(),
        scroll,
        "detaching must not disturb the list's scroll"
    );

    // ── Only one composition: `c` in the main window raises it, never a
    //    second composer ────────────────────────────────────────────────
    press(&window, "c", gdk::ModifierType::empty());
    assert!(
        composer.is_detached(),
        "compose again while detached must not fork the composition"
    );
    assert_eq!(
        composer.draft().subject,
        before.subject,
        "and must not clobber what is being written"
    );
    assert_eq!(
        window.context(),
        Context::List,
        "nor quietly take the reading pane back for a composer that is not in it"
    );
    assert!(reading.is_visible(), "the message is still being read");

    // ── The same command in the detached window puts it back ─────────────
    press_detached(
        &composer,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );

    assert!(!composer.is_detached(), "it re-attached");
    assert!(composer.is_open(), "and it is still the same composition");
    assert!(
        composer.detached_window().is_none(),
        "the window went with it rather than being left empty"
    );
    assert_eq!(
        composer.root().and_downcast::<gtk::Window>(),
        Some(window.clone().upcast::<gtk::Window>()),
        "back in the main window"
    );
    assert_eq!(
        window.context(),
        Context::Composer,
        "and the keyboard came back with it"
    );
    assert!(
        !reading.is_visible(),
        "the composer has the reading pane again"
    );
    assert!(shell.has_css_class(composer::COMPOSING_CLASS));

    let back = composer.draft();
    assert_eq!(back.to, before.to, "recipients survived the return trip");
    assert_eq!(back.subject, before.subject, "the subject survived");
    assert_eq!(back.body.text, before.body.text, "the body survived");
    assert_eq!(
        composer.test_cursor_offset(),
        cursor,
        "and so did the cursor"
    );

    // ── …and the list is *still* untouched, after a full round trip ──────
    assert_eq!(rows.selected_row().map(|row| row.index()), Some(7));
    assert_eq!(scroller.vadjustment().value(), scroll);

    // ── `Esc` in the detached window keeps the draft, same as in place ───
    press(
        &window,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    assert!(composer.is_detached());

    press_detached(&composer, "Escape", gdk::ModifierType::empty());

    assert!(!composer.is_open(), "Esc leaves the composer");
    assert!(
        !composer.is_detached(),
        "and takes the window with it rather than leaving one behind"
    );
    assert!(composer.detached_window().is_none());
    assert!(
        reading.is_visible(),
        "the message is back in the reading pane"
    );
    assert_eq!(window.context(), Context::List);
    assert!(!shell.has_css_class(composer::COMPOSING_CLASS));

    press(&window, "c", gdk::ModifierType::empty());
    settle();
    assert_eq!(
        composer.draft().subject,
        before.subject,
        "Esc kept the draft — closing a detached composer is not discarding it"
    );
    assert_eq!(composer.close(), Closing::Keep);

    // ── Sending from the detached window is how most pop-outs end, and it
    //    has to leave the application in exactly the state Esc leaves it ──
    let sent: Rc<RefCell<Vec<Draft>>> = Rc::new(RefCell::new(Vec::new()));
    composer.connect_send({
        let sent = Rc::clone(&sent);
        move |draft| sent.borrow_mut().push(draft.clone())
    });

    composer.open(started());
    settle();
    press(
        &window,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    assert!(composer.is_detached());

    press_detached(&composer, "Return", gdk::ModifierType::CONTROL_MASK);

    assert_eq!(
        sent.borrow().len(),
        1,
        "ctrl+Enter in the detached window sends, through the same registry"
    );
    assert_eq!(sent.borrow()[0].subject, started().subject);
    assert!(!composer.is_open(), "and the composition is over");
    assert!(
        !composer.is_detached() && composer.detached_window().is_none(),
        "so its window went with it rather than being left up and empty"
    );
    assert!(reading.is_visible(), "the reading pane is usable again");
    assert_eq!(window.context(), Context::List);

    // ── Closing the detached window with its own close button is the same
    //    thing as Esc: it keeps the draft, it does not discard it ─────────
    composer.open(started());
    settle();
    press(
        &window,
        "o",
        gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
    );
    let host = composer.detached_window().expect("detached");
    host.close();
    settle();

    assert!(!composer.is_open(), "the window's close button closes it");
    assert!(!composer.is_detached());
    press(&window, "c", gdk::ModifierType::empty());
    settle();
    assert_eq!(
        composer.draft().subject,
        started().subject,
        "and keeps the draft, exactly as Esc does"
    );
}
