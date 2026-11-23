//! Undo toasts: *Archived 12 messages - Undo*. The undo stack is `postio_core::undo::UndoStack`;
//! this is how the confirmation looks and where the mouse reaches what `u` reaches.
//! `TOAST_TIMEOUT` is how long the confirmation shows, not the undo window: `u` works for
//! `UndoStack::EXPIRY`.
//! `show_action_completed` dismisses the previous undo toast first, so one action is one toast.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use postio_ui::observe::Tone;

/// Far short of `postio_core::undo::UndoStack::EXPIRY`: `u` keeps working after the toast is gone.
pub const TOAST_TIMEOUT: u32 = 8;

type Remake = Rc<dyn Fn() -> adw::Toast>;

type PendingOffer = Rc<RefCell<Option<(adw::Toast, Rc<dyn Fn()>)>>>;

/// Not a widget: bookkeeping for which toast, if any, is still showing.
pub struct Toast {
    overlay: adw::ToastOverlay,
    /// An overlay inside a dialog, preferred while it is on screen: a toast
    /// in the window's own overlay is drawn under the dialog.
    over: RefCell<Option<adw::ToastOverlay>>,
    /// The overlay the toast now showing was added to.
    host: RefCell<Option<adw::ToastOverlay>>,
    current: RefCell<Option<adw::Toast>>,
    /// Held so `u` can reach it as well as the button (#471); the same closure the button runs.
    pending_undo: RefCell<Option<std::rc::Rc<dyn Fn()>>>,
    /// Gone with its toast, so the key never takes up an offer nobody can see (#1953).
    pending_offer: PendingOffer,
    /// Recorded when shown rather than read back off the button; cleared on dismissal so an
    /// observation never reports an invisible toast.
    shown: Rc<RefCell<Option<(adw::Toast, Tone, bool)>>>,
    remake: RefCell<Option<(Remake, Option<String>)>>,
    undo_key: RefCell<Option<String>>,
}

impl Toast {
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
            over: RefCell::new(None),
            host: RefCell::new(None),
            current: RefCell::new(None),
            pending_undo: RefCell::new(None),
            pending_offer: Rc::new(RefCell::new(None)),
            shown: Rc::new(RefCell::new(None)),
            remake: RefCell::new(None),
            undo_key: RefCell::new(None),
        }
    }

    pub fn overlay(&self) -> &adw::ToastOverlay {
        &self.overlay
    }

    pub fn set_undo_key(&self, key: Option<String>) {
        *self.undo_key.borrow_mut() = key;
    }

    /// While a dialog's overlay is set, toasts show in it, above the dialog.
    pub fn set_over(&self, over: Option<adw::ToastOverlay>) {
        *self.over.borrow_mut() = over;
    }

    pub fn host(&self) -> Option<adw::ToastOverlay> {
        self.host.borrow().clone()
    }

    pub fn showing(&self) -> Option<adw::Toast> {
        self.current.borrow().clone()
    }

    pub fn tone(&self) -> Option<Tone> {
        self.shown.borrow().as_ref().map(|(_, tone, _)| *tone)
    }

    pub fn offers_undo(&self) -> bool {
        self.shown
            .borrow()
            .as_ref()
            .is_some_and(|(_, _, undo)| *undo)
    }

    /// `undoable` decides whether the button appears; it names `win.undo`, the action `u` reaches.
    pub fn show_action_completed(&self, description: &str, undoable: bool) {
        self.show_action_completed_for(description, undoable, TOAST_TIMEOUT);
    }

    /// For an action whose undo lasts a window of its own, so the toast offers Undo exactly as long
    /// as Undo works.
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

    /// For a gesture that could not run and has to say why, such as a key pressed before the store
    /// is open (#1114). No button: there is nothing to retry.
    pub fn show_notice(&self, sentence: &str) {
        let sentence = sentence.to_owned();
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

    /// The button calls `on_undo` directly, not the global `win.undo`: account removal (#464) has
    /// its own local undo, which `u` in `Context::Accounts` reaches through `activate_undo` (#471).
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

    /// Answers whether there was anything to undo, so `u` with nothing showing is not reported as
    /// an undo.
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

    /// Confirmation, not a second offer to undo the undo.
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

    /// Not reachable through `u`: the button is not an undo, so it never joins `pending_undo`. For
    /// a gesture that could not run for a reason with a fix on offer.
    pub fn show_prompt(
        &self,
        sentence: &str,
        button_label: &str,
        key: Option<String>,
        on_click: impl Fn() + 'static,
    ) {
        self.show_button(Tone::Warning, sentence, button_label, key, on_click);
    }

    /// News, not a warning, and non-blocking: the next toast replaces it. The offer is held until
    /// its toast goes so `take_offer` can run it from the keyboard (#1952).
    pub fn show_offer(
        &self,
        sentence: &str,
        button_label: &str,
        key: Option<String>,
        on_click: impl Fn() + 'static,
    ) {
        let on_click: Rc<dyn Fn()> = Rc::new(on_click);
        let again = Rc::clone(&on_click);
        self.show_button(Tone::Info, sentence, button_label, key, move || again());
        if let Some(toast) = self.current.borrow().clone() {
            *self.pending_offer.borrow_mut() = Some((toast, on_click));
        }
    }

    /// A notice belongs to the moment it was raised; a dialog opening over it hides it, so its key
    /// must not keep reaching what nobody can see.
    pub fn drop_offer(&self) -> bool {
        let Some((toast, _)) = self.pending_offer.borrow_mut().take() else {
            return false;
        };
        toast.dismiss();
        true
    }

    /// Returns whether there was an offer.
    pub fn take_offer(&self) -> bool {
        let Some((toast, on_click)) = self.pending_offer.borrow_mut().take() else {
            return false;
        };
        toast.dismiss();
        on_click();
        true
    }

    fn show_button(
        &self,
        tone: Tone,
        sentence: &str,
        button_label: &str,
        key: Option<String>,
        on_click: impl Fn() + 'static,
    ) {
        let (sentence, button_label) = (sentence.to_owned(), button_label.to_owned());
        let on_click = Rc::new(on_click);
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
            tone,
            false,
            key,
        );
    }

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
        // A new toast replaces the old one's offer too: `u` must not reach back past what is on
        // screen.
        self.pending_undo.borrow_mut().take();
        self.pending_offer.borrow_mut().take();
        if let Some(previous) = self.current.borrow_mut().take() {
            previous.dismiss();
        }
        *self.shown.borrow_mut() = Some((toast.clone(), tone, offers_undo));
        toast.connect_dismissed({
            let shown = Rc::clone(&self.shown);
            let offer = Rc::clone(&self.pending_offer);
            move |gone| {
                let mut offer = offer.borrow_mut();
                if offer.as_ref().is_some_and(|(toast, _)| toast == gone) {
                    *offer = None;
                }
                drop(offer);
                let mut shown = shown.borrow_mut();
                if shown.as_ref().is_some_and(|(toast, _, _)| toast == gone) {
                    *shown = None;
                }
            }
        });
        host.add_toast(toast.clone());
        // Buttons are answered by mouse and by their command's key, never keyboard focus, which
        // would land on one when nothing else could take it and make the next Return answer the
        // toast.
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

    /// For when the dialog the toast was raised in has gone. Answers whether there was a toast to
    /// carry.
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
        self.move_to(self.overlay.clone(), make, tone, offers_undo, key);
        true
    }

    /// Keeps what the toast's keys reach: the undo and the offer belong to the toast.
    fn move_to(
        &self,
        host: adw::ToastOverlay,
        make: Remake,
        tone: Tone,
        offers_undo: bool,
        key: Option<String>,
    ) {
        let pending = self.pending_undo.borrow().clone();
        let offer = self.pending_offer.borrow().clone();
        self.show_in(host, make, tone, offers_undo, key);
        *self.pending_undo.borrow_mut() = pending;
        if let (Some((_, on_click)), Some(toast)) = (offer, self.current.borrow().clone()) {
            *self.pending_offer.borrow_mut() = Some((toast, on_click));
        }
    }
}

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
