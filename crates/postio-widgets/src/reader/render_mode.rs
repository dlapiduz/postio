//! The render-mode line: one quiet line above an HTML body that names the
//! treatment it is drawn in, says why, and offers the other one
//! (specs/007-postio-focus T213; the handoff's screens 03, 11-13).
//!
//! "App colours · sender colours and fonts removed · Show original O", or
//! "Original layout, on paper · this message sets its own background · Use
//! app colours O". A treatment that is never named is a rewrite nobody can
//! see, which is the rule reader view's notice was written for (#1009).
//!
//! Native chrome above the body, never inside its document, like the
//! notices: what it reports on is that document. The words are
//! `postio_ui::reader::document::render_mode_words`, proven without a
//! display; this only draws them.

use std::rc::Rc;

use adw::prelude::*;
use postio_core::CommandId;
use postio_ui::reader::document::RenderModeWords;

use crate::widgets::KeycapButton;

/// The line, and its two controls.
pub struct RenderModeLine {
    root: gtk::Box,
    title: gtk::Label,
    detail: gtk::Label,
    always: gtk::Button,
    switch: Rc<KeycapButton>,
    /// Whether it is saying the body fell back to plain text, rather than
    /// naming a treatment.
    fell_back: std::cell::Cell<bool>,
}

impl RenderModeLine {
    /// A line, hidden until a body with a treatment to name is drawn.
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S2);
        root.add_css_class("postio-render-mode");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        // The handoff's half-filled circle: one sheet, two ways to draw it.
        let mark = gtk::Label::new(Some("\u{25D1}"));
        mark.add_css_class("postio-render-mode-mark");
        mark.set_accessible_role(gtk::AccessibleRole::Presentation);
        root.append(&mark);

        let title = gtk::Label::new(None);
        title.add_css_class("postio-render-mode-title");
        root.append(&title);

        let detail = gtk::Label::new(None);
        detail.add_css_class("postio-render-mode-detail");
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_xalign(0.0);
        detail.set_hexpand(true);
        root.append(&detail);

        let always = gtk::Button::with_label("Always for this sender");
        always.add_css_class("postio-render-mode-always");
        crate::widgets::button::style(
            &always,
            crate::widgets::button::Kind::Ghost,
            crate::widgets::button::Size::Regular,
        );
        always.set_valign(gtk::Align::Center);
        always.set_visible(false);
        root.append(&always);

        let switch = Rc::new(KeycapButton::new(
            Some(CommandId::SwitchTreatment),
            "Show original",
            "postio-render-mode-switch",
            false,
        ));
        KeycapButton::arm(&switch);
        root.append(&switch.widget());

        Rc::new(RenderModeLine {
            root,
            title,
            detail,
            always,
            switch,
            fell_back: std::cell::Cell::new(false),
        })
    }

    /// The line, to place above the body.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Say `words`, or hide the line when there are none: plain text, or no
    /// body at all.
    pub fn show(&self, words: Option<RenderModeWords>) {
        self.fell_back.set(false);
        let Some(words) = words else {
            self.root.set_visible(false);
            return;
        };
        self.title.set_label(words.title);
        self.detail.set_label(words.detail);
        self.switch.set_label(words.action);
        self.switch.widget().set_visible(true);
        self.always.set_visible(words.offer_always);
        self.root
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "{}, {}",
                words.title, words.detail
            ))]);
        self.root.set_visible(true);
    }

    /// Say the body fell back to plain text, and `why` ("this message took
    /// too long to lay out") -- in place of a treatment, because neither
    /// treatment is what is drawn (T218). Nothing to switch to: the line
    /// offers no control.
    pub fn show_fallback(&self, why: &str) {
        self.fell_back.set(true);
        self.title.set_label("Plain text");
        self.detail.set_label(why);
        self.switch.widget().set_visible(false);
        self.always.set_visible(false);
        self.root
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Plain text, {why}"
            ))]);
        self.root.set_visible(true);
    }

    /// Whether the line is saying the body fell back.
    pub fn is_fallback(&self) -> bool {
        self.fell_back.get()
    }

    /// The switch's key, from the keymap: `None` hides the cap.
    pub fn set_key(&self, key: Option<&str>) {
        self.switch.set_key(key);
    }

    /// Run `handler` when the switch is pressed.
    pub fn connect_switch(&self, handler: impl Fn() + 'static) {
        self.switch.connect_clicked(handler);
    }

    /// Run `handler` when "Always for this sender" is pressed.
    pub fn connect_always(&self, handler: impl Fn() + 'static) {
        self.always.connect_clicked(move |_| handler());
    }

    /// Whether the line is on screen.
    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

    /// What it says, as "title · detail · action key": test-facing.
    pub fn text(&self) -> String {
        if !self.switch.widget().is_visible() {
            return format!("{} · {}", self.title.label(), self.detail.label());
        }
        format!(
            "{} · {} · {} {}",
            self.title.label(),
            self.detail.label(),
            self.switch.label(),
            self.switch.key()
        )
        .trim_end()
        .to_owned()
    }

    /// Whether it offers "Always for this sender": test-facing.
    pub fn offers_always(&self) -> bool {
        self.root.is_visible() && self.always.is_visible()
    }

    /// Press "Always for this sender" as a click would: test-facing.
    pub fn press_always(&self) {
        self.always.emit_clicked();
    }
}
