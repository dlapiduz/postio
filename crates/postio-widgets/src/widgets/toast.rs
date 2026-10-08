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

use gtk::prelude::*;

use postio_ui::observe::Tone;

/// How long an undo toast stays on screen before it dismisses itself.
///
/// A deliberate, named choice rather than whatever `AdwToast` defaults to:
/// long enough to read the sentence and reach for the button, far short of
/// [`postio_core::undo::UndoStack::EXPIRY`] — `u` keeps working long after
/// the toast is gone, which is the acceptance criterion this constant must
/// not quietly break.
pub const TOAST_TIMEOUT: u32 = 8;

/// What makes a toast again, the same each time it is called.
type Remake = Rc<dyn Fn() -> adw::Toast>;

/// The undo toast, and the overlay it appears over.
///
/// Not a widget of its own: [`Toast::overlay`] is what a window puts its
/// content inside, and this struct is only the bookkeeping that overlay
/// needs — which toast, if any, is still showing.
pub struct Toast {
    overlay: adw::ToastOverlay,
    /// An overlay inside a dialog, preferred while it is on screen: a toast
    /// in the window's own overlay is drawn under the dialog.
    over: RefCell<Option<adw::ToastOverlay>>,
    /// The overlay the toast now showing was added to.
    host: RefCell<Option<adw::ToastOverlay>>,
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
    /// How to make the toast now showing again, for [`Self::rehome`], and
    /// the key its button shows.
    remake: RefCell<Option<(Remake, Option<String>)>>,
    /// The key that undoes, for the cap on an Undo button: the window's
    /// keymap says it, and says it again when that changes.
    undo_key: RefCell<Option<String>>,
}

impl Toast {
    /// A fresh overlay, nothing showing yet.
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
            over: RefCell::new(None),
            host: RefCell::new(None),
            current: RefCell::new(None),
            pending_undo: RefCell::new(None),
            shown: Rc::new(RefCell::new(None)),
            remake: RefCell::new(None),
            undo_key: RefCell::new(None),
        }
    }

    /// The overlay: put the window's real content inside it with
    /// [`adw::ToastOverlay::set_child`].
    pub fn overlay(&self) -> &adw::ToastOverlay {
        &self.overlay
    }

    /// The key an Undo button shows beside its word, or none while nothing
    /// undoes.
    pub fn set_undo_key(&self, key: Option<String>) {
        *self.undo_key.borrow_mut() = key;
    }

    /// Name the overlay of a dialog that opens over the window: while it is
    /// on screen, toasts are shown in it, above the dialog.
    pub fn set_over(&self, over: Option<adw::ToastOverlay>) {
        *self.over.borrow_mut() = over;
    }

    /// The overlay the toast on screen was added to.
    pub fn host(&self) -> Option<adw::ToastOverlay> {
        self.host.borrow().clone()
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
        let description = description.to_owned();
        let key = undoable.then(|| self.undo_key.borrow().clone()).flatten();
        self.push(
            Rc::new(move || {
                let toast = adw::Toast::builder()
                    .title(&description)
                    .timeout(seconds)
                    .build();
                if undoable {
                    toast.set_button_label(Some("Undo"));
                    toast.set_action_name(Some("win.undo"));
                }
                toast
            }),
            Tone::Info,
            undoable,
            key,
        );
    }

    /// A sentence, with nothing to press.
    ///
    /// For a gesture that could not run and has to say why rather than go
    /// quiet — #1114's key pressed before the store is open, where there is
    /// nothing to retry because the work is already in flight. No button, for
    /// the same reason the plate that says the same sentence carries no key
    /// hint.
    pub fn show_notice(&self, sentence: &str) {
        let sentence = sentence.to_owned();
        // A warning: a notice is a gesture that could not run.
        self.push(
            Rc::new(move || {
                adw::Toast::builder()
                    .title(&sentence)
                    .timeout(TOAST_TIMEOUT)
                    .build()
            }),
            Tone::Warning,
            false,
            None,
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
        let description = description.to_owned();
        let on_undo = std::rc::Rc::new(on_undo);
        self.push(
            Rc::new({
                let on_undo = Rc::clone(&on_undo);
                move || {
                    let toast = adw::Toast::builder()
                        .title(&description)
                        .timeout(TOAST_TIMEOUT)
                        .button_label("Undo")
                        .build();
                    toast.connect_button_clicked({
                        let on_undo = Rc::clone(&on_undo);
                        move |_| on_undo()
                    });
                    toast
                }
            }),
            Tone::Info,
            true,
            self.undo_key.borrow().clone(),
        );
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
        let description = description.to_owned();
        self.push(
            Rc::new(move || {
                adw::Toast::builder()
                    .title(&description)
                    .timeout(TOAST_TIMEOUT)
                    .build()
            }),
            Tone::Success,
            false,
            None,
        );
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
    pub fn show_prompt(
        &self,
        sentence: &str,
        button_label: &str,
        key: Option<String>,
        on_click: impl Fn() + 'static,
    ) {
        let (sentence, button_label) = (sentence.to_owned(), button_label.to_owned());
        let on_click = Rc::new(on_click);
        // A warning, as `show_notice`'s: a gesture that could not run. The
        // button fixes what was missing; it is no undo.
        self.push(
            Rc::new(move || {
                let toast = adw::Toast::builder()
                    .title(&sentence)
                    .timeout(TOAST_TIMEOUT)
                    .button_label(&button_label)
                    .build();
                toast.connect_button_clicked({
                    let on_click = Rc::clone(&on_click);
                    move |_| on_click()
                });
                toast
            }),
            Tone::Warning,
            false,
            key,
        );
    }

    /// Dismisses whatever is showing and shows what `make` makes instead.
    fn push(
        &self,
        make: Rc<dyn Fn() -> adw::Toast>,
        tone: Tone,
        offers_undo: bool,
        key: Option<String>,
    ) {
        let host = self
            .over
            .borrow()
            .clone()
            .filter(gtk::prelude::WidgetExt::is_mapped)
            .unwrap_or_else(|| self.overlay.clone());
        self.show_in(host, make, tone, offers_undo, key);
    }

    fn show_in(
        &self,
        host: adw::ToastOverlay,
        make: Rc<dyn Fn() -> adw::Toast>,
        tone: Tone,
        offers_undo: bool,
        key: Option<String>,
    ) {
        let toast = make();
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
        host.add_toast(toast.clone());
        // A toast's buttons are answered by the mouse and by the key their
        // command has: never by the keyboard focus, which would land on one
        // whenever the focus had nowhere else to go (an empty list behind a
        // closed message) and make the next Return answer the toast. The
        // key is drawn beside the button's word, as every control that runs
        // a command draws it.
        dress(host.upcast_ref(), key.as_deref());
        let weak = host.downgrade();
        let drawn = key.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(host) = weak.upgrade() {
                dress(host.upcast_ref(), drawn.as_deref());
            }
        });
        *self.host.borrow_mut() = Some(host);
        *self.current.borrow_mut() = Some(toast);
        *self.remake.borrow_mut() = Some((make, key));
    }

    /// Carry the toast on screen over to the window's own overlay, for when
    /// the dialog it was raised in has gone: what a message's send or
    /// archive said goes on being said over the list, with its Undo.
    /// Answers whether there was a toast to carry.
    pub fn rehome(&self) -> bool {
        let in_a_dialog = self
            .host
            .borrow()
            .as_ref()
            .is_some_and(|host| *host != self.overlay);
        if !in_a_dialog || self.current.borrow().is_none() {
            return false;
        }
        let Some((_, tone, offers_undo)) = self.shown.borrow().clone() else {
            return false;
        };
        let Some((make, key)) = self.remake.borrow().clone() else {
            return false;
        };
        // The undo `u` reaches belongs to the toast, and survives it.
        let pending = self.pending_undo.borrow().clone();
        self.show_in(self.overlay.clone(), make, tone, offers_undo, key);
        *self.pending_undo.borrow_mut() = pending;
        true
    }
}

/// Take every button of the toasts drawn under `widget` out of the focus
/// chain, and give the labelled one its `key`.
fn dress(widget: &gtk::Widget, key: Option<&str>) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        if current.type_().name() == "AdwToastWidget" {
            if let Some(key) = key {
                super::keyhint::dress_labelled_buttons(&current, key);
            }
            let mut inside = vec![current.clone()];
            while let Some(widget) = inside.pop() {
                if widget.is::<gtk::Button>() {
                    widget.set_focusable(false);
                }
                let mut next = widget.first_child();
                while let Some(sibling) = next {
                    next = sibling.next_sibling();
                    inside.push(sibling);
                }
            }
        } else {
            dress(&current, key);
        }
        child = current.next_sibling();
    }
}

impl Default for Toast {
    fn default() -> Self {
        Self::new()
    }
}
