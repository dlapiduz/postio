//! The undo toast, exercised in a process of its own.
//!
//! These were unit tests inside `src/toast.rs` until they aborted CI. GTK may
//! be initialized from exactly one thread, `cargo test` runs a crate's unit
//! tests on a thread pool in one process, and four of these called
//! `adw::init()`. Whether that aborted depended on which thread won and
//! whether a display existed, so it survived every developer machine and
//! failed the first time a display-less runner ran the suite:
//!
//! ```text
//! Gdk-ERROR **: gdk_display_manager_get() was called before gtk_init()
//! postio_gtk-... (signal: 6, SIGABRT)
//! ```
//!
//! Cargo gives every integration test *binary* its own process — but not
//! every test *function*. Those still share the binary's thread pool, so
//! three `#[test]`s here reproduced the same abort the move was meant to end:
//!
//! ```text
//! Gdk-ERROR **: gdk_display_open_default() was called before gtk_init()
//! ```
//!
//! So this file is deliberately *one* test function, the way `gtk_style.rs`
//! and `gtk_accessibility.rs` already are. Anything in this crate that needs
//! a display belongs in `tests/`, one test to a file, not in a
//! `#[cfg(test)] mod tests`.

use postio_gtk::toast::Toast;

/// Returns false when there is no display to talk to — CI, mostly.
fn ready() -> bool {
    adw::init().is_ok() && gtk::gdk::Display::default().is_some()
}

#[test]
fn the_undo_toast_coalesces_and_offers_undo_only_when_there_is_something_to_undo() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }

    // ── a second action replaces the first rather than stacking ──────────
    let toast = Toast::new();
    toast.show_action_completed("Archived 3 messages", true);
    let first = toast.showing();
    assert!(first.is_some());

    toast.show_action_completed("Archived 5 messages", true);
    let second = toast.showing();
    assert!(second.is_some());
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "coalescing swaps in a new toast rather than editing the old one in place"
    );

    // ── an undoable completion names the win.undo action ─────────────────
    let toast = Toast::new();
    toast.show_action_completed("Archived 12 messages", true);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label().as_deref(), Some("Undo"));
    assert_eq!(current.action_name().as_deref(), Some("win.undo"));

    // ── and one with nothing to take back offers no button ───────────────
    let toast = Toast::new();
    toast.show_action_completed("Marked 1 message as read", false);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label(), None);
    assert_eq!(current.action_name(), None);
}
