//! Undo toasts: *Archived 12 messages — Undo*.
//!
//! spec.md §16 and canvas 3b. The undo stack itself — coalescing a burst of
//! archives into one entry, remembering it for ten minutes — is
//! `postio_core::undo::UndoStack`; this is only the two things a stack has
//! no opinion about: how the confirmation looks, and where the mouse reaches
//! the same `u` already reaches.
//!
//! # Coalescing follows the stack, not the other way round
//!
//! [`Toast::show_action_completed`] dismisses whatever undo toast is still
//! showing before it shows the next one, so a second archive inside the
//! stack's own coalescing window replaces the toast's text rather than
//! stacking a second banner on top of the first — one action, one toast,
//! the same rule the stack already applies to the entry underneath it.
//!
//! # The toast's timeout is not the undo stack's
//!
//! [`TOAST_TIMEOUT`] is how long the *confirmation* stays on screen — long
//! enough to read "Archived 12 messages" and reach for the button, short
//! enough not to sit there once the moment has passed. `u` is not on a
//! clock: [`postio_core::undo::UndoStack::EXPIRY`] is ten minutes, and nothing
//! here shortens that. The toast disappearing is not the undo window
//! closing.

use std::cell::RefCell;

/// How long an undo toast stays on screen before it dismisses itself.
///
/// A deliberate, named choice rather than whatever `AdwToast` defaults to:
/// long enough to read the sentence and reach for the button, far short of
/// [`postio_core::undo::UndoStack::EXPIRY`] — `u` keeps working long after
/// the toast is gone, which is the acceptance criterion this constant must
/// not quietly break.
pub const TOAST_TIMEOUT: u32 = 8;

/// The undo toast, and the overlay it appears over.
///
/// Not a widget of its own: [`Toast::overlay`] is what a window puts its
/// content inside, and this struct is only the bookkeeping that overlay
/// needs — which toast, if any, is still showing.
pub struct Toast {
    overlay: adw::ToastOverlay,
    current: RefCell<Option<adw::Toast>>,
}

impl Toast {
    /// A fresh overlay, nothing showing yet.
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
            current: RefCell::new(None),
        }
    }

    /// The overlay: put the window's real content inside it with
    /// [`adw::ToastOverlay::set_child`].
    pub fn overlay(&self) -> &adw::ToastOverlay {
        &self.overlay
    }

    /// *Archived 12 messages — Undo.* `description` is already user-facing
    /// prose (`UndoEntry::description`); `undoable` decides whether the
    /// button appears at all — some completions have nothing to take back.
    ///
    /// The button names `win.undo`, the same action `u` reaches through
    /// [`postio_core::CommandId::Undo`] — one path, whichever one the user
    /// takes.
    pub fn show_action_completed(&self, description: &str, undoable: bool) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(TOAST_TIMEOUT)
            .build();
        if undoable {
            toast.set_button_label(Some("Undo"));
            toast.set_action_name(Some("win.undo"));
        }
        self.push(toast);
    }

    /// *Archived 12 messages, undone.* What `u` (or the toast's own button)
    /// leaves behind: confirmation, not a second offer to undo the undo.
    pub fn show_undo_performed(&self, description: &str) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(TOAST_TIMEOUT)
            .build();
        self.push(toast);
    }

    /// Dismisses whatever is showing and shows `toast` instead.
    fn push(&self, toast: adw::Toast) {
        if let Some(previous) = self.current.borrow_mut().take() {
            previous.dismiss();
        }
        self.overlay.add_toast(toast.clone());
        *self.current.borrow_mut() = Some(toast);
    }
}

impl Default for Toast {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> bool {
        adw::init().is_ok() && gtk::gdk::Display::default().is_some()
    }

    #[test]
    fn a_second_action_replaces_the_first_toasts_text_rather_than_stacking() {
        if !init() {
            eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
            return;
        }
        let toast = Toast::new();
        toast.show_action_completed("Archived 3 messages", true);
        let first = toast.current.borrow().clone();
        assert!(first.is_some());

        toast.show_action_completed("Archived 5 messages", true);
        let second = toast.current.borrow().clone();
        assert!(second.is_some());
        assert_ne!(
            first.unwrap(),
            second.unwrap(),
            "coalescing swaps in a new toast rather than editing the old one in place"
        );
    }

    #[test]
    fn only_an_undoable_completion_offers_the_button() {
        if !init() {
            eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
            return;
        }
        let toast = Toast::new();
        toast.show_action_completed("Marked 1 message as read", false);
        let current = toast.current.borrow();
        let current = current.as_ref().unwrap();
        assert_eq!(current.button_label(), None);
        assert_eq!(current.action_name(), None);
    }

    #[test]
    fn an_undoable_completion_names_the_win_undo_action() {
        if !init() {
            eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
            return;
        }
        let toast = Toast::new();
        toast.show_action_completed("Archived 12 messages", true);
        let current = toast.current.borrow();
        let current = current.as_ref().unwrap();
        assert_eq!(current.button_label().as_deref(), Some("Undo"));
        assert_eq!(current.action_name().as_deref(), Some("win.undo"));
    }
}
