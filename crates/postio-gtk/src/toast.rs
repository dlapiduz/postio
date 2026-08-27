//! Undo toasts: *Archived 12 messages — Undo*.
//!
//! docs/PRODUCT.md §16 and canvas 3b. The undo stack itself — coalescing a burst of
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

    /// Which undo toast is on screen, if any.
    ///
    /// The bookkeeping this struct exists for, and what `tests/gtk_toast.rs`
    /// asserts against — it cannot reach a private field from its own
    /// process, and a display-touching test has to be in one.
    pub fn showing(&self) -> Option<adw::Toast> {
        self.current.borrow().clone()
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

    /// *Account removed — Undo.* Same shape as
    /// [`Toast::show_action_completed`], but the button calls `on_undo`
    /// directly rather than the global `win.undo` action.
    ///
    /// For something the undo *stack* has no opinion about at all: account
    /// removal (#464) is a `gio::SimpleActionGroup` action on the settings
    /// panel, not a [`postio_core::Command`], because it needs a specific
    /// account as its payload with no keystroke-derived default and there
    /// is no `Context::Settings` for the keymap to reach it in — see ADR
    /// 0005 Q6a. Its undo is real (Q6 requires it), just local to this one
    /// button rather than reachable from `u`.
    pub fn show_removable(&self, description: &str, on_undo: impl Fn() + 'static) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(TOAST_TIMEOUT)
            .button_label("Undo")
            .build();
        toast.connect_button_clicked(move |_| on_undo());
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
