//! Tab moves between panes, deliberately.
//!
//! #494, reported directly: *"tab, shift+tab, ctrl+tab are inconsistent,
//! sometimes it changes panes, sometimes it changes items within a pane. I
//! need an easy way to go from the sidebar to the message list to the
//! preview pane."*
//!
//! Bare Tab had **no entry in the registry at all** — it appeared only in
//! `KEY_NAMES`, the list of key spellings the binding *parser* understands,
//! so a user could bind something to it but nothing was. Its top-level
//! behaviour was therefore whatever GTK's native focus-chain traversal
//! happened to produce, which is genuinely inconsistent pane to pane:
//! `list_view.rs` already special-cases Tab "arriving from outside" to patch
//! over it.
//!
//! What is asserted here is the round trip, in both directions, through
//! `window.context()` — which pane the application believes owns the
//! keyboard, not which widget GTK last focused. The registry half (that
//! `tab` resolves to `CyclePane` in each pane, and to nothing in the
//! composer or search) is covered by `postio-core`'s own registry tests;
//! this is the half that proves pressing the key actually moves anything.
//!
//! One test function: GTK is single-threaded and initialised once per
//! binary. Skips without a display. Nothing here touches the network.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread
// reading the environment. This sets it before the app under test starts,
// which is the one moment it is sound. The crate's library code forbids
// `unsafe`.

use crate::settle as pump;
use gtk::gdk;
use postio_core::Context;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::ids::{AccountId, MailboxId};
use postio_model::mailbox::{Mailbox, MailboxCounts, MailboxRole};

fn tab(window: &Window, shift: bool) {
    let modifiers = if shift {
        gdk::ModifierType::SHIFT_MASK
    } else {
        gdk::ModifierType::empty()
    };
    window.handle_key(gdk::Key::from_name("Tab").unwrap(), modifiers);
    pump();
}

fn folders() -> Vec<Mailbox> {
    let account = AccountId::new(1);
    let mut inbox = Mailbox::new(account, "INBOX", Some('/'));
    inbox.id = MailboxId::new(1);
    inbox.role = MailboxRole::Inbox;
    inbox.counts = MailboxCounts {
        unread: 3,
        ..MailboxCounts::default()
    };
    vec![inbox]
}

pub fn tab_walks_the_panes_and_shift_tab_walks_back() {
    let state_dir = std::env::temp_dir().join(format!("postio-pane-cycle-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", &state_dir) };

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
    pump();
    window.sidebar().set_mailboxes(&folders());
    pump();

    assert_eq!(
        window.context(),
        Context::List,
        "the list starts with the keyboard"
    );

    // ── forward: list → reader → sidebar → list ──────────────────────────
    tab(&window, false);
    assert_eq!(
        window.context(),
        Context::Reader,
        "Tab from the list should reach the reading pane"
    );

    tab(&window, false);
    assert_eq!(
        window.context(),
        Context::Sidebar,
        "Tab from the reader should reach the folder list"
    );

    tab(&window, false);
    assert_eq!(
        window.context(),
        Context::List,
        "Tab from the folders should come back to the message list, closing \
         the cycle rather than stopping"
    );

    // ── and back the way it came ─────────────────────────────────────────
    //
    // Asserted as its own walk rather than assumed from the forward one: a
    // cycle that only works in one direction still passes every forward
    // assertion above, and Shift+Tab is half of what was reported.
    tab(&window, true);
    assert_eq!(
        window.context(),
        Context::Sidebar,
        "Shift+Tab from the list should reach the folder list"
    );

    tab(&window, true);
    assert_eq!(
        window.context(),
        Context::Reader,
        "Shift+Tab from the folders should reach the reading pane"
    );

    tab(&window, true);
    assert_eq!(
        window.context(),
        Context::List,
        "Shift+Tab from the reader should come back to the message list"
    );
}
