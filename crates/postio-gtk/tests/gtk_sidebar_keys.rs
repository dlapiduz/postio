//! Changing folder without the mouse.
//!
//! `postio-cfd.2`: `ToggleSidebar` was the only sidebar command in the
//! registry and `Context` had no sidebar variant, so the folder list — the
//! main axis of navigation after the message list — could only be reached by
//! pointing at it. That is a direct violation of docs/PRODUCT.md §8 and of the
//! project's second principal.
//!
//! What is asserted here is the round trip a keyboard user actually makes:
//! `g f` to reach the folders, `j` and `k` to move between them, `Esc` to come
//! back to the messages. The commands themselves are registry entries, so the
//! palette and the `?` sheet pick them up without being told — that half is
//! covered by `postio-core`'s own registry tests and by
//! `cheatsheet::tests::the_sections_are_the_ones_the_registry_actually_uses`.
//!
//! One test function: GTK is single-threaded and initialised once per binary.
//! Skips without a display. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. These tests set it before the app under test starts, which
// is the one moment it is sound. The crate's library code forbids `unsafe`.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

fn pump() {
    while glib::MainContext::default().iteration(false) {}
}

fn press(window: &Window, key: &str) {
    window.handle_key(
        gdk::Key::from_name(key).unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();
}

/// An ordinary account: special-use folders first, then user folders. The
/// split matters — they are two `GtkListBox`es, and `j` has to cross it.
fn folders() -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let folder = |id: i64, path: &str, role, unread| {
        let mut mailbox = Mailbox::new(account, path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            unread,
            ..MailboxCounts::default()
        };
        mailbox
    };
    vec![
        folder(1, "INBOX", MailboxRole::Inbox, 12),
        folder(2, "Archive", MailboxRole::Archive, 0),
        folder(3, "Taxes", MailboxRole::Regular, 3),
        folder(4, "Garagiste", MailboxRole::Regular, 0),
    ]
}

#[test]
fn a_mailbox_can_be_chosen_without_touching_the_mouse() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-sidebar-keys-{}", std::process::id()));
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
    pump();
    window.sidebar().set_mailboxes(&folders());
    pump();

    // Every folder the keyboard lands on, in order, so the assertions can be
    // about what the *application* was told rather than about widget state.
    let opened: Rc<RefCell<Vec<i64>>> = Default::default();
    window.sidebar().connect_selected({
        let opened = Rc::clone(&opened);
        move |id| opened.borrow_mut().push(id.get())
    });

    assert_eq!(
        window.context(),
        Context::List,
        "the list starts with the keyboard"
    );

    // ── `g f` — go to folders ───────────────────────────────────────────
    press(&window, "g");
    press(&window, "f");
    assert_eq!(
        window.context(),
        Context::Sidebar,
        "`g f` did not put the keyboard in the folder list, so nothing below \
         this can mean anything"
    );

    // ── `j` and `k` move between folders, and opening follows ───────────
    press(&window, "j");
    press(&window, "j");
    assert_eq!(
        *opened.borrow(),
        vec![1, 2],
        "moving down the folders did not open them. Selection *is* the open \
         folder here, exactly as it is for a click."
    );

    // Across the section boundary. Archive is the last special-use folder;
    // the ordinary section is sorted by path, so Garagiste comes before
    // Taxes and is the next row on screen. The point is that `j` crosses at
    // all — the two list boxes are a visual split and the keyboard must not
    // know about it.
    press(&window, "j");
    assert_eq!(
        opened.borrow().last().copied(),
        Some(4),
        "`j` stopped at the end of the special-use section"
    );

    press(&window, "k");
    assert_eq!(
        opened.borrow().last().copied(),
        Some(2),
        "`k` did not cross back into the special-use section"
    );

    // ── and it does not run off the end ─────────────────────────────────
    for _ in 0..10 {
        press(&window, "k");
    }
    assert_eq!(
        opened.borrow().last().copied(),
        Some(1),
        "`k` past the top wrapped or fell off; it should stop at the first \
         folder — wrapping a short list is how you end up in Trash when you \
         meant to stop at Inbox"
    );

    // ── `Esc` gives the keyboard back ───────────────────────────────────
    press(&window, "Escape");
    assert_eq!(
        window.context(),
        Context::List,
        "`Esc` left the keyboard in the folder list, so `j` would still be \
         moving folders while the message list looks focused"
    );
}
