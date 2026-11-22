//! A button that says which key does the same thing. The key comes from the keymap, never a
//! literal; a command with no binding hides its cap.

use std::cell::RefCell;

use adw::prelude::*;
use postio_core::CommandId;

type ClickHandler = Box<dyn Fn()>;

pub struct KeycapButton {
    button: gtk::Button,
    hint: gtk::Label,
    /// `None` for a button whose action is local to its widget and not in the command vocabulary.
    command: Option<CommandId>,
    handlers: RefCell<Vec<ClickHandler>>,
}

impl KeycapButton {
    /// `primary` gives the suggested-action treatment: one per bar.
    pub fn new(command: Option<CommandId>, label: &str, class: &str, primary: bool) -> Self {
        Self::build(command, label, None, class, primary)
    }

    /// `label` is still the accessible name and the tooltip.
    pub fn with_icon(
        command: Option<CommandId>,
        label: &str,
        icon: &str,
        class: &str,
        primary: bool,
    ) -> Self {
        Self::build(command, label, Some(icon), class, primary)
    }

    fn build(
        command: Option<CommandId>,
        label: &str,
        icon: Option<&str>,
        class: &str,
        primary: bool,
    ) -> Self {
        let hint = super::keyhint::cap("");
        // A class of its own so a test can tell the button from the label showing its key.
        hint.add_css_class(&format!("{class}-hint"));
        hint.set_visible(false);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        match icon {
            Some(name) => {
                let image = gtk::Image::from_icon_name(name);
                image.set_accessible_role(gtk::AccessibleRole::Presentation);
                content.append(&image);
                // No key hint on an icon button: the letter is for the labelled primary only;
                // tooltip and accessible name carry the key.
                hint.set_visible(false);
            }
            None => {
                content.append(&gtk::Label::new(Some(label)));
                content.append(&hint);
            }
        }

        let button = gtk::Button::new();
        // As tall as its own words: a taller bar centres the button rather than stretching it.
        button.set_valign(gtk::Align::Center);
        button.set_child(Some(&content));
        button.add_css_class(class);
        button.add_css_class("postio-keycap-button");
        button.update_property(&[gtk::accessible::Property::Label(label)]);
        let kind = if primary {
            super::button::Kind::Primary
        } else {
            super::button::Kind::Ghost
        };
        super::button::style(&button, kind, super::button::Size::Regular);

        Self {
            button,
            hint,
            command,
            handlers: RefCell::new(Vec::new()),
        }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    pub fn command(&self) -> Option<CommandId> {
        self.command
    }

    /// `None` hides the cap: a blank one reads as a key that exists and does nothing.
    pub fn set_key(&self, key: Option<&str>) {
        match key {
            Some(key) => {
                self.hint.set_label(key);
                self.hint.set_visible(true);
            }
            None => self.hint.set_visible(false),
        }
    }

    pub fn key(&self) -> String {
        if self.hint.is_visible() {
            self.hint.label().to_string()
        } else {
            String::new()
        }
    }

    pub fn connect_clicked(&self, handler: impl Fn() + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }

    /// Separate from `new` because the `clicked` closure holds the button; owning its own cycle
    /// would never drop.
    pub fn arm(this: &std::rc::Rc<Self>) {
        let weak = std::rc::Rc::downgrade(this);
        this.button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.press();
            }
        });
    }

    pub fn press(&self) {
        for handler in self.handlers.borrow().iter() {
            handler();
        }
    }

    pub fn set_sensitive(&self, sensitive: bool) {
        self.button.set_sensitive(sensitive);
    }

    pub fn is_sensitive(&self) -> bool {
        self.button.is_sensitive()
    }

    pub fn set_label(&self, label: &str) {
        if let Some(content) = self.button.child().and_downcast::<gtk::Box>()
            && let Some(text) = content.first_child().and_downcast::<gtk::Label>()
        {
            text.set_label(label);
        }
        self.button
            .update_property(&[gtk::accessible::Property::Label(label)]);
    }

    pub fn label(&self) -> String {
        self.button
            .child()
            .and_downcast::<gtk::Box>()
            .and_then(|content| content.first_child())
            .and_downcast::<gtk::Label>()
            .map(|text| text.label().to_string())
            .unwrap_or_default()
    }
}
