//! One gesture, one delivery.
//!
//! The window has two seams out: [`Window::connect_command`], which carries a
//! [`CommandId`], and [`Window::connect_action`], which carries a whole
//! [`Command`] — the verb *and* what it is aimed at. Whoever wires the command
//! bus subscribes to the second one, because it is the only one that can tell
//! "archive what is selected" from "archive the row under the pointer".
//!
//! What this file holds down is that the two seams do not both fire for one
//! gesture with *different* invocations. A mouse-originated command used to go
//! out twice — once as an id, from the window's own fallthrough, and once as
//! the command itself — so a bus subscribed to both would archive the
//! selection *and* the hovered row from one click. Deduplicating that in the
//! composition root is not possible after the fact: by the time the specific
//! invocation arrives, the vague one has already been acted on.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_composer.rs`.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use postio_core::{Command, CommandId, MessageTarget};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::MessageId;

fn settle() {
    while glib::MainContext::default().iteration(false) {}
}

#[test]
fn every_gesture_reaches_the_bus_exactly_once() {
    let state_dir = std::env::temp_dir().join(format!("postio-dispatch-{}", std::process::id()));
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

    let window = Window::default();
    window.present();
    settle();

    let ids: Rc<RefCell<Vec<CommandId>>> = Default::default();
    let actions: Rc<RefCell<Vec<Command>>> = Default::default();
    window.connect_command({
        let ids = Rc::clone(&ids);
        move |id| ids.borrow_mut().push(id)
    });
    window.connect_action({
        let actions = Rc::clone(&actions);
        move |command| actions.borrow_mut().push(command)
    });

    // ── The keyboard: a verb with no target of its own ──────────────────
    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert_eq!(*ids.borrow(), vec![CommandId::Archive]);
    assert_eq!(
        *actions.borrow(),
        vec![Command::Archive {
            target: MessageTarget::Selection
        }],
        "the keyboard has to reach the whole-command seam too, or the bus \
         would have to subscribe to both and dedupe them"
    );
    ids.borrow_mut().clear();
    actions.borrow_mut().clear();

    // ── The mouse: the same verb, aimed at one row ──────────────────────
    let one = MessageTarget::Messages(vec![MessageId::new(7)]);
    window.act(Command::Archive {
        target: one.clone(),
    });
    settle();
    assert_eq!(
        *actions.borrow(),
        vec![Command::Archive { target: one }],
        "a hover action must arrive once, still naming its row"
    );
    assert_eq!(
        ids.borrow().len(),
        1,
        "and the id seam sees it once, not twice"
    );
    ids.borrow_mut().clear();
    actions.borrow_mut().clear();

    // ── What the window answers itself never leaves it ───────────────────
    window.handle_key(
        gdk::Key::from_name("j").unwrap(),
        gdk::ModifierType::empty(),
    );
    settle();
    assert!(
        ids.borrow().is_empty() && actions.borrow().is_empty(),
        "moving the cursor is the window's own business"
    );
}
