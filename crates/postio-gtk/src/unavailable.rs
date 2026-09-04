//! The screen for a store Postio could not open.
//!
//! ADR 0014 Q3 made a locked keyring a hard stop: the master key cannot be
//! read, so the store does not open, and there is no plaintext fallback. That
//! is the right posture and it produced a wrong surface — #299 could only say
//! so in a toast, which disappears while the condition does not, and left an
//! otherwise blank window behind it.
//!
//! This is the sibling of [`crate::onboarding`]: that one is "there is no
//! account yet", this one is "there is no store yet", and both replace the
//! window's content with a plate rather than opening a dialog. A modal would
//! be a claim that nothing else matters until this is resolved — true here,
//! as it happens, but the app has exactly one modal and the pattern is the
//! plate.
//!
//! # What this widget will not do
//!
//! It does not read the keyring and it does not open anything. It cannot:
//! `postio-gtk` may not link `rusqlite` or the secret store
//! (`scripts/checks/check-crate-boundaries.py`). So this is the words and the
//! states, and [`Unavailable::connect_retry`] is where the composition root
//! does the work — the same arrangement [`crate::onboarding`] has.
//!
//! # Why the sentence comes from outside
//!
//! `SecretError` already writes the sentence the user needs, unlock hint and
//! all, and a second copy of that wording here would be one to keep in step.
//! The screen shows what it is handed and never composes an explanation of
//! its own — which is also what lets a store that failed to open for some
//! *other* reason arrive at the same surface with its own words.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

type RetryHandler = Box<dyn Fn()>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Unavailable {
        pub(super) reason: gtk::Label,
        /// The one action, as the same [`KeycapButton`] every other surface
        /// draws its verbs with (#1002) — a `OnceCell` because the widget is
        /// built in `constructed`, and `KeycapButton` has no `Default`.
        ///
        /// [`KeycapButton`]: crate::widgets::KeycapButton
        pub(super) retry: std::cell::OnceCell<Rc<crate::widgets::KeycapButton>>,
        pub(super) busy: Cell<bool>,
        pub(super) on_retry: RefCell<Vec<RetryHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Unavailable {
        const NAME: &'static str = "PostioUnavailable";
        type Type = super::Unavailable;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Unavailable {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Unavailable {}
    impl BinImpl for Unavailable {}
}

glib::wrapper! {
    /// The screen shown instead of the mail when the store will not open.
    pub struct Unavailable(ObjectSubclass<imp::Unavailable>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Unavailable {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Unavailable {
    /// A screen with nothing to say yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The sentence explaining why the store did not open.
    ///
    /// Shown as it was given, but as a *sentence* — see [`sentence`]. The
    /// screen never writes one of its own; see the module docs.
    pub fn set_reason(&self, reason: &str) {
        self.imp().reason.set_text(&sentence(reason));
    }

    /// What the screen is currently saying.
    pub fn reason(&self) -> String {
        self.imp().reason.text().to_string()
    }

    /// Whether a retry is in flight.
    ///
    /// The one state here that is not instant: re-reading the keyring is a
    /// D-Bus round trip against a service that may be waiting for the user to
    /// type a passphrase into a prompt of its own. A button that stayed
    /// pressable would collect clicks the app has nothing to do with.
    pub fn is_busy(&self) -> bool {
        self.imp().busy.get()
    }

    /// Says whether a retry is in flight; see [`is_busy`](Self::is_busy).
    pub fn set_busy(&self, busy: bool) {
        let imp = self.imp();
        imp.busy.set(busy);
        let retry = self.retry_button();
        retry.set_sensitive(!busy);
        retry.set_label(if busy { "Trying…" } else { "Try again" });
    }

    /// Runs `handler` when the user asks to try again.
    pub fn connect_retry(&self, handler: impl Fn() + 'static) {
        self.imp().on_retry.borrow_mut().push(Box::new(handler));
    }

    /// Asks to try again, as pressing the button does.
    ///
    /// Public so the composition root can drive it from a test without a
    /// synthetic click, exactly as `Onboarding::submit` is.
    pub fn retry(&self) {
        if self.is_busy() {
            return;
        }
        for handler in self.imp().on_retry.borrow().iter() {
            handler();
        }
    }

    /// Puts the keyboard where the only action is.
    pub fn focus_retry(&self) {
        self.retry_button().widget().grab_focus();
    }

    /// The retry button, built in `constructed` and therefore always there.
    fn retry_button(&self) -> Rc<crate::widgets::KeycapButton> {
        self.imp()
            .retry
            .get()
            .expect("built in constructed")
            .clone()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-unavailable");
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Center);
        self.set_accessible_role(gtk::AccessibleRole::Group);

        let kicker = gtk::Label::new(Some("Cannot open"));
        kicker.add_css_class("postio-kicker");
        kicker.set_xalign(0.0);
        kicker.set_hexpand(true);
        kicker.set_accessible_role(gtk::AccessibleRole::Presentation);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-unavailable-header");
        header.append(&kicker);

        let title = gtk::Label::new(Some("Postio cannot open your mail"));
        title.add_css_class("postio-unavailable-title");
        title.set_xalign(0.0);
        title.set_wrap(true);

        // Wrapped, selectable and left aligned: it is a sentence somebody has
        // to read and may want to paste into a search or a bug report.
        imp.reason.add_css_class("postio-unavailable-reason");
        imp.reason.set_xalign(0.0);
        imp.reason.set_wrap(true);
        imp.reason.set_selectable(true);
        imp.reason.set_max_width_chars(56);

        // The one thing that is always true and is not in `SecretError`'s
        // sentence: the mail is not gone. A store that will not open is
        // frightening in a way it does not need to be.
        let reassurance = gtk::Label::new(Some(
            "Your mail is still here. Postio will not open the store \
             unencrypted, so nothing is lost by trying again once the keyring \
             is unlocked.",
        ));
        reassurance.add_css_class("postio-unavailable-note");
        reassurance.set_xalign(0.0);
        reassurance.set_wrap(true);
        reassurance.set_max_width_chars(56);

        // `Ret` is written down rather than read from a keymap because
        // Enter here is not a bound command — it is the default action of
        // the only button on a screen with one button. Everything else about
        // the cap is the shared one, so it matches the reader's and the
        // composer's exactly.
        let retry = Rc::new(crate::widgets::KeycapButton::new(
            None,
            "Try again",
            "postio-unavailable-retry",
            true,
        ));
        crate::widgets::KeycapButton::arm(&retry);
        retry.set_key(Some("Ret"));
        let button = retry.widget();
        button.set_halign(gtk::Align::Start);
        button.set_tooltip_text(Some("Read the keyring again"));
        let screen = self.clone();
        retry.connect_clicked(move || screen.retry());
        let _ = imp.retry.set(retry);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.add_css_class("postio-unavailable-body");
        body.append(&title);
        body.append(&imp.reason);
        body.append(&reassurance);
        body.append(&button);

        let plate = gtk::Box::new(gtk::Orientation::Vertical, 0);
        plate.append(&header);
        plate.append(&body);
        self.set_child(Some(&plate));
    }
}

/// The same words, starting like a sentence.
///
/// `SecretError`'s messages are written to be embedded — "the login keyring is
/// locked, so Postio cannot read…" reads correctly after "failed because",
/// which is where a log puts it. On a screen it is the first thing the eye
/// lands on, and a lowercase opening reads as a fragment somebody forgot to
/// finish.
///
/// Only the first character, and only when it is lowercase: the rest is the
/// error's own, including any address or path it names, and case-folding one
/// of those would be showing the user something they did not type.
fn sentence(reason: &str) -> String {
    let mut characters = reason.chars();
    match characters.next() {
        Some(first) if first.is_lowercase() => {
            first.to_uppercase().collect::<String>() + characters.as_str()
        }
        _ => reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::sentence;

    #[test]
    fn a_reason_is_shown_as_a_sentence() {
        assert_eq!(
            sentence("the login keyring is locked, so Postio cannot read it"),
            "The login keyring is locked, so Postio cannot read it"
        );
    }

    #[test]
    fn a_reason_that_already_reads_as_one_is_left_alone() {
        assert_eq!(sentence("Postio could not open"), "Postio could not open");
        assert_eq!(sentence(""), "");
        assert_eq!(
            sentence("~/.local/share is unreadable"),
            "~/.local/share is unreadable"
        );
    }

    #[test]
    fn only_the_first_character_moves() {
        assert_eq!(
            sentence("the keyring will not give up ada@Example.com's password"),
            "The keyring will not give up ada@Example.com's password"
        );
    }
}
