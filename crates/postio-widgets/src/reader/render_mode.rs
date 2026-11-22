//! The render-mode line above an HTML body: names the treatment it is drawn in, says why, and
//! offers the other (a rewrite nobody can see is what #1009 forbids). Native chrome, never inside
//! the document; the words are `postio_ui::reader::document::render_mode_words`.

use std::rc::Rc;

use adw::prelude::*;
use postio_core::CommandId;
use postio_ui::reader::document::RenderModeWords;

use crate::widgets::KeycapButton;

pub struct RenderModeLine {
    root: gtk::Box,
    title: gtk::Label,
    detail: gtk::Label,
    always: gtk::Button,
    switch: Rc<KeycapButton>,
    fell_back: std::cell::Cell<bool>,
}

impl RenderModeLine {
    pub fn new() -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S2);
        root.add_css_class("postio-render-mode");
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

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

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

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

    /// In place of a treatment, because neither is what is drawn. The line offers no control.
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

    pub fn is_fallback(&self) -> bool {
        self.fell_back.get()
    }

    pub fn set_key(&self, key: Option<&str>) {
        self.switch.set_key(key);
    }

    pub fn connect_switch(&self, handler: impl Fn() + 'static) {
        self.switch.connect_clicked(handler);
    }

    pub fn connect_always(&self, handler: impl Fn() + 'static) {
        self.always.connect_clicked(move |_| handler());
    }

    pub fn is_shown(&self) -> bool {
        self.root.is_visible()
    }

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

    pub fn offers_always(&self) -> bool {
        self.root.is_visible() && self.always.is_visible()
    }

    pub fn press_always(&self) {
        self.always.emit_clicked();
    }
}
