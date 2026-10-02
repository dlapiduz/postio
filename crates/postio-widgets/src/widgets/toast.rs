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
use std::rc::Rc;

use postio_ui::observe::Tone;

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
    /// The undo behind the toast now showing, if it has one.
    ///
    /// Held so `u` can reach it as well as the button (#471). The closure is
    /// the same one the button runs -- one behaviour with two ways in, not
    /// two implementations that have to agree.
    pending_undo: RefCell<Option<std::rc::Rc<dyn Fn()>>>,
    /// How the toast now showing reads, and whether it offers undo,
    /// recorded when it was shown rather than read back off its button
    /// (specs/008-storyboards research R6). Cleared when that toast is
    /// dismissed, so an observation never reports a toast nobody can see.
    shown: Rc<RefCell<Option<(adw::Toast, Tone, bool)>>>,
}

impl Toast {
    /// A fresh overlay, nothing showing yet.
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
            current: RefCell::new(None),
            pending_undo: RefCell::new(None),
            shown: Rc::new(RefCell::new(None)),
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

    /// How the toast on screen reads, as it was shown: `None` when nothing
    /// is showing. For a storyboard's `notice.tone`.
    pub fn tone(&self) -> Option<Tone> {
        self.shown.borrow().as_ref().map(|(_, tone, _)| *tone)
    }

    /// Whether the toast on screen offers undo. For a storyboard's
    /// `notice.undo`.
    pub fn offers_undo(&self) -> bool {
        self.shown
            .borrow()
            .as_ref()
            .is_some_and(|(_, _, undo)| *undo)
    }

    /// *Archived 12 messages — Undo.* `description` is already user-facing
    /// prose (`UndoEntry::description`); `undoable` decides whether the
    /// button appears at all — some completions have nothing to take back.
    ///
    /// The button names `win.undo`, the same action `u` reaches through
    /// [`postio_core::CommandId::Undo`] — one path, whichever one the user
    /// takes.
    pub fn show_action_completed(&self, description: &str, undoable: bool) {
        self.show_action_completed_for(description, undoable, TOAST_TIMEOUT);
    }

    /// [`Self::show_action_completed`], staying `seconds` rather than
    /// [`TOAST_TIMEOUT`]: for an action whose undo lasts a window of its own
    /// -- an answer to an invitation, taken back only until its reply leaves
    /// (specs/007-postio-focus FR-102) -- so the toast offers Undo exactly as
    /// long as Undo works.
    pub fn show_action_completed_for(&self, description: &str, undoable: bool, seconds: u32) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(seconds)
            .build();
        if undoable {
            toast.set_button_label(Some("Undo"));
            toast.set_action_name(Some("win.undo"));
        }
        self.push(toast, Tone::Info, undoable);
    }

    /// A sentence, with nothing to press.
    ///
    /// For a gesture that could not run and has to say why rather than go
    /// quiet — #1114's key pressed before the store is open, where there is
    /// nothing to retry because the work is already in flight. No button, for
    /// the same reason the plate that says the same sentence carries no key
    /// hint.
    pub fn show_notice(&self, sentence: &str) {
        // A warning: a notice is a gesture that could not run.
        self.push(
            adw::Toast::builder()
                .title(sentence)
                .timeout(TOAST_TIMEOUT)
                .build(),
            Tone::Warning,
            false,
        );
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
    /// 0005 Q6a. Its undo is real (Q6 requires it) and local to this one
    /// toast rather than to the global stack -- but no longer local to the
    /// *button*: #471 made removal a command with `Recovery::Undo`, so `u`
    /// in `Context::Accounts` reaches it through [`Self::activate_undo`].
    pub fn show_removable(&self, description: &str, on_undo: impl Fn() + 'static) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(TOAST_TIMEOUT)
            .button_label("Undo")
            .build();
        let on_undo = std::rc::Rc::new(on_undo);
        toast.connect_button_clicked({
            let on_undo = std::rc::Rc::clone(&on_undo);
            move |_| on_undo()
        });
        self.push(toast, Tone::Info, true);
        // After `push`, which clears whatever the last toast left here.
        *self.pending_undo.borrow_mut() = Some(on_undo);
    }

    /// Runs the showing toast's undo, if it has one, and dismisses it.
    ///
    /// Answers whether there was anything to undo, so a caller can tell "the
    /// toast is up and I ran it" from "there was no toast" -- `u` with
    /// nothing showing must not be reported as an undo that happened.
    pub fn activate_undo(&self) -> bool {
        let Some(on_undo) = self.pending_undo.borrow_mut().take() else {
            return false;
        };
        on_undo();
        if let Some(toast) = self.current.borrow_mut().take() {
            toast.dismiss();
        }
        true
    }

    /// *Archived 12 messages, undone.* What `u` (or the toast's own button)
    /// leaves behind: confirmation, not a second offer to undo the undo.
    pub fn show_undo_performed(&self, description: &str) {
        let toast = adw::Toast::builder()
            .title(description)
            .timeout(TOAST_TIMEOUT)
            .build();
        self.push(toast, Tone::Success, false);
    }

    /// A sentence with one button that runs `on_click`, replacing whatever
    /// toast was showing (`push`'s rule).
    ///
    /// Unlike [`Self::show_removable`], not reachable through `u` -- the
    /// button is not an undo, so it never joins `pending_undo`. For a
    /// gesture that could not run for a reason with a fix on offer, such as
    /// Focus's compose with no account yet to write from
    /// (specs/007-postio-focus T172): the sentence names what is missing,
    /// and the button starts fixing it.
    pub fn show_prompt(&self, sentence: &str, button_label: &str, on_click: impl Fn() + 'static) {
        let toast = adw::Toast::builder()
            .title(sentence)
            .timeout(TOAST_TIMEOUT)
            .button_label(button_label)
            .build();
        toast.connect_button_clicked(move |_| on_click());
        // A warning, as `show_notice`'s: a gesture that could not run. The
        // button fixes what was missing; it is no undo.
        self.push(toast, Tone::Warning, false);
    }

    /// Dismisses whatever is showing and shows `toast` instead.
    fn push(&self, toast: adw::Toast, tone: Tone, offers_undo: bool) {
        // A new toast replaces the old one's offer too: an undo whose toast
        // is gone is one the person can no longer see, and `u` must not
        // reach back past what is on screen.
        self.pending_undo.borrow_mut().take();
        if let Some(previous) = self.current.borrow_mut().take() {
            previous.dismiss();
        }
        *self.shown.borrow_mut() = Some((toast.clone(), tone, offers_undo));
        toast.connect_dismissed({
            let shown = Rc::clone(&self.shown);
            move |gone| {
                let mut shown = shown.borrow_mut();
                if shown.as_ref().is_some_and(|(toast, _, _)| toast == gone) {
                    *shown = None;
                }
            }
        });
        self.overlay.add_toast(toast.clone());
        *self.current.borrow_mut() = Some(toast);
    }
}

impl Default for Toast {
    fn default() -> Self {
        Self::new()
    }
}
