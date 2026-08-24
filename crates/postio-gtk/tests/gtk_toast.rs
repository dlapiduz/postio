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
//! Cargo gives every integration test its own process, so the process-wide
//! init is safe here. Anything in this crate that needs a display belongs in
//! `tests/`, not in a `#[cfg(test)] mod tests`.

use postio_gtk::toast::Toast;

/// Returns false when there is no display to talk to — CI, mostly.
fn ready() -> bool {
    adw::init().is_ok() && gtk::gdk::Display::default().is_some()
}

#[test]
fn a_second_action_replaces_the_first_toasts_text_rather_than_stacking() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }
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
}

#[test]
fn only_an_undoable_completion_offers_the_button() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }
    let toast = Toast::new();
    toast.show_action_completed("Marked 1 message as read", false);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label(), None);
    assert_eq!(current.action_name(), None);
}

#[test]
fn an_undoable_completion_names_the_win_undo_action() {
    if !ready() {
        eprintln!("skipping: no display");
        return;
    }
    let toast = Toast::new();
    toast.show_action_completed("Archived 12 messages", true);
    let current = toast.showing().unwrap();
    assert_eq!(current.button_label().as_deref(), Some("Undo"));
    assert_eq!(current.action_name().as_deref(), Some("win.undo"));
}
