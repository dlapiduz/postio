//! Leaving the search field has to give the keyboard back.
//!
//! `0.1.0` was reported as "single-key bindings stop working, seemingly at
//! random" — `?` and the rest silently doing nothing, with no way to get them
//! back. `postio-73` named three candidates and asked for the state the
//! window is in when it happens, rather than a guess.
//!
//! It is the second one. `Window::key_context` asks the finder first:
//!
//! ```text
//! match self.finder().context() {
//!     Some(context) => KeyContext::from(context),
//!     None => KeyContext::from(self.context()),
//! }
//! ```
//!
//! and `Finder::context` is `is_open().then(..)`. Focusing the field *opens*
//! the box — there is a `connect_enter` for exactly that — and nothing closes
//! it when focus leaves. Worse, staying open is deliberate after a search:
//! "its results *are* the message list, so the field stays up with the query
//! still in it". So the ordinary act of searching and then clicking a result
//! leaves the resolver in `Search` for the rest of the session, where `?` is
//! not bound, and every bare key is silently dropped.
//!
//! Nothing times out and nothing else clears it, which is why it reads as
//! random and why clicking a row does not bring the keys back.
//!
//! Its own file: GTK is single-threaded and initialised once, so one `#[test]`
//! per integration binary. See `gtk_dispatch.rs`.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::CommandId;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

fn settle() {
    let context = glib::MainContext::default();
    for _ in 0..40 {
        while context.iteration(false) {}
    }
}

pub fn leaving_the_search_field_gives_the_single_key_bindings_back() {
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
    window.present();
    settle();

    let press_question_mark = |window: &Window| {
        window.handle_key(
            gdk::Key::from_name("question").unwrap(),
            gdk::ModifierType::empty(),
        );
        settle();
    };

    // ── the keys work before anyone searches ─────────────────────────────
    press_question_mark(&window);
    assert!(
        window.cheatsheet().is_visible(),
        "`?` does not open the cheat sheet even on a fresh window — this test \
         is measuring something other than what it thinks"
    );
    window.close_cheatsheet();
    settle();

    // ── the user searches, which is what leaves the box open ─────────────
    field(&window).grab_focus();
    settle();
    assert!(
        window.finder().is_open(),
        "focusing the field is supposed to open the box — if it no longer \
         does, the bug this test guards has moved"
    );

    // ── and then goes back to the mail, as one does ──────────────────────
    // The query stays in the field on purpose: in search mode the results
    // *are* the message list, so the box does not close itself. What has to
    // happen is that the keyboard stops belonging to it.
    window.list().grab_focus();
    settle();

    // ── the keys still work ──────────────────────────────────────────────
    press_question_mark(&window);
    assert!(
        window.cheatsheet().is_visible(),
        "`?` did nothing after the focus left the search field. The finder \
         still reports itself open, so `key_context` is pinned to Search and \
         every single-key binding is silently dropped — for the rest of the \
         session, because nothing times out and nothing else clears it."
    );
    window.close_cheatsheet();
    settle();

    // ── and the box still owns them while it does have the keyboard ──────
    // The other direction, which matters just as much: `key_context` must
    // still answer with the box's context while the box holds the keyboard,
    // or the fix above trades one silent bug for another.
    //
    // `ctrl+a` is the chord that shows it. It is Select all, bound across the
    // list surfaces and *not* in the palette, and it survives a text entry
    // because it carries a modifier. So with the box in command mode it must
    // do nothing — and if the context fell back to the window's, typing a
    // command name would quietly select every message in the mailbox.
    let commands: Rc<RefCell<Vec<CommandId>>> = Default::default();
    window.connect_command({
        let commands = Rc::clone(&commands);
        move |id| commands.borrow_mut().push(id)
    });

    window.open_finder(postio_gtk::finder::Mode::Command);
    settle();
    assert!(
        window.finder().has_keyboard(),
        "opening the palette is supposed to put the keyboard in the field — \
         without that the assertion below passes for the wrong reason"
    );

    window.handle_key(
        gdk::Key::from_name("a").unwrap(),
        gdk::ModifierType::CONTROL_MASK,
    );
    settle();
    assert!(
        !commands.borrow().contains(&CommandId::SelectAll),
        "ctrl+a selected every message while the user was typing a command \
         into the palette: the box has the keyboard, so `key_context` should \
         have answered Palette, where ctrl+a is not bound"
    );
}

/// The header's one text box, which is also the finder's.
fn field(window: &Window) -> gtk::Text {
    fn find(widget: &gtk::Widget) -> Option<gtk::Text> {
        if let Some(text) = widget.downcast_ref::<gtk::Text>() {
            return Some(text.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = find(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }
    find(window.upcast_ref::<gtk::Widget>()).expect("the header's one box")
}
