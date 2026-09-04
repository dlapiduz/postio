//! A button that says which key does the same thing.
//!
//! Every action surface in Postio draws the same pair — a label and, beside
//! it in mono, the key that runs it — and every one of them built it by hand:
//! the reader's action bar, the unavailable panel's retry, the reader's
//! remote-image banner. Three spellings of one control, and only one of them
//! read the key out of the live keymap, so the other two said whatever was
//! typed into them.
//!
//! This is that control. **The key comes from the keymap, never from a
//! literal** — a `[keys]` rebind changes what a button says the same moment
//! it changes what the keyboard does — and a command with no binding at all
//! hides its cap rather than showing a blank one.

use std::cell::RefCell;

use adw::prelude::*;
use postio_core::CommandId;

type ClickHandler = Box<dyn Fn()>;

/// A label, its key, and what pressing it means.
pub struct KeycapButton {
    button: gtk::Button,
    hint: gtk::Label,
    /// The command this runs, when it runs one. `None` for a button whose
    /// action is local to the widget that built it — "Show images" acts on
    /// one message's blocked references and has no place in the command
    /// vocabulary.
    command: Option<CommandId>,
    handlers: RefCell<Vec<ClickHandler>>,
}

impl KeycapButton {
    /// A button carrying `label`, findable by `class`, running `command`.
    ///
    /// `primary` gives it the suggested-action treatment: one per bar, the
    /// verb the bar exists for.
    pub fn new(command: Option<CommandId>, label: &str, class: &str, primary: bool) -> Self {
        let hint = gtk::Label::new(None);
        hint.add_css_class("postio-keyhint");
        // A class of its own, distinct from the button's: a test finding
        // widgets by class needs to tell "the button" and "the label showing
        // its key" apart.
        hint.add_css_class(&format!("{class}-hint"));
        hint.set_accessible_role(gtk::AccessibleRole::Presentation);
        hint.set_visible(false);

        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.append(&gtk::Label::new(Some(label)));
        content.append(&hint);

        let button = gtk::Button::new();
        button.set_child(Some(&content));
        button.add_css_class(class);
        button.add_css_class("postio-keycap-button");
        button.update_property(&[gtk::accessible::Property::Label(label)]);
        if primary {
            button.add_css_class("suggested-action");
        } else {
            button.add_css_class("flat");
            button.add_css_class("postio-ghost");
        }

        Self {
            button,
            hint,
            command,
            handlers: RefCell::new(Vec::new()),
        }
    }

    /// The widget to place in a bar.
    pub fn widget(&self) -> gtk::Widget {
        self.button.clone().upcast()
    }

    /// The command this button runs, if it runs one.
    pub fn command(&self) -> Option<CommandId> {
        self.command
    }

    /// Show `key` as this button's cap, or hide the cap when there is none.
    ///
    /// `None` is the case where the user cleared a binding, or lost it to a
    /// collision with another command. A blank cap would read as a key that
    /// exists and does nothing.
    pub fn set_key(&self, key: Option<&str>) {
        match key {
            Some(key) => {
                self.hint.set_label(key);
                self.hint.set_visible(true);
            }
            None => self.hint.set_visible(false),
        }
    }

    /// The cap currently shown, empty when hidden. Test-facing: production
    /// code has no reason to read a label back.
    pub fn key(&self) -> String {
        if self.hint.is_visible() {
            self.hint.label().to_string()
        } else {
            String::new()
        }
    }

    /// Called when the button is pressed, however it was pressed.
    pub fn connect_clicked(&self, handler: impl Fn() + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }

    /// Wire the GTK click into the handler list.
    ///
    /// Separate from [`new`](Self::new) because a `clicked` closure has to
    /// hold the button, and a `KeycapButton` that owned its own cycle would
    /// never drop. Whoever puts this in a bar calls this once with the
    /// shared handle it keeps.
    pub fn arm(this: &std::rc::Rc<Self>) {
        let weak = std::rc::Rc::downgrade(this);
        this.button.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.press();
            }
        });
    }

    /// Run this button's handlers — what a click does, and what a test uses
    /// in place of a synthesized pointer event.
    pub fn press(&self) {
        for handler in self.handlers.borrow().iter() {
            handler();
        }
    }

    /// Grey the button out without hiding it, so a bar keeps its shape.
    pub fn set_sensitive(&self, sensitive: bool) {
        self.button.set_sensitive(sensitive);
    }

    /// Whether the button can be pressed.
    pub fn is_sensitive(&self) -> bool {
        self.button.is_sensitive()
    }

    /// Change the words on the button. The cap is untouched — the key that
    /// runs a verb does not change because the verb was renamed for one
    /// message.
    pub fn set_label(&self, label: &str) {
        if let Some(content) = self.button.child().and_downcast::<gtk::Box>()
            && let Some(text) = content.first_child().and_downcast::<gtk::Label>()
        {
            text.set_label(label);
        }
        self.button
            .update_property(&[gtk::accessible::Property::Label(label)]);
    }

    /// The words currently on the button. Test-facing.
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
