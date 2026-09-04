//! One line of chrome that reports on the content below it.
//!
//! "Remote images are blocked", "parts of this message could not be decoded",
//! "reader view is on" — three of these existed, each built by hand, each
//! with its own spacing, and one of them wrapping to three lines because its
//! button spelled out a 70-character relay address inline (canvas turn 7).
//!
//! A notice is **native chrome, never drawn inside the document it reports
//! on**: the pane it sits above is exactly the thing being reported on, and
//! it has to keep working when every remote image in the message stays
//! blocked forever.
//!
//! # It is one line, and it never wraps
//!
//! That is the whole shape. The label ellipsises, the action keeps its
//! keycap, and anything long enough to need a sentence — an address, a list
//! of what was blocked — goes in the overflow menu rather than into the bar.
//! A notice that grows vertically pushes the mail down the pane, which is
//! the opposite of what a notice is for.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use super::keycap::KeycapButton;

/// One entry in a notice's overflow menu.
pub struct NoticeMenuItem {
    pub label: String,
    pub handler: Rc<dyn Fn()>,
}

impl NoticeMenuItem {
    /// An entry that runs `handler` when chosen.
    pub fn new(label: impl Into<String>, handler: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            handler: Rc::new(handler),
        }
    }
}

/// A one-line notice: icon, text, an optional action, an optional overflow.
pub struct NoticeBar {
    root: gtk::Box,
    icon: gtk::Image,
    label: gtk::Label,
    action: Rc<KeycapButton>,
    menu: gtk::MenuButton,
    model: gio::Menu,
    group: gio::SimpleActionGroup,
    /// Kept alive because a `gio::SimpleAction`'s `activate` closure is what
    /// runs them, and the group only holds the actions.
    handlers: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl NoticeBar {
    /// A notice with `icon_name`, findable by `class`, hidden.
    ///
    /// The action and the overflow start hidden too: a notice that has
    /// nothing to offer is still worth showing, and reserving space for
    /// buttons it will never grow would leave a gap the eye reads as
    /// missing.
    pub fn new(icon_name: &str, class: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("postio-notice");
        root.add_css_class(class);
        root.set_visible(false);
        root.set_accessible_role(gtk::AccessibleRole::Group);

        let icon = gtk::Image::from_icon_name(icon_name);
        icon.add_css_class("postio-notice-icon");
        root.append(&icon);

        let label = gtk::Label::new(None);
        label.set_hexpand(true);
        label.set_xalign(0.0);
        // The whole point of the widget. `set_wrap(false)` is the default,
        // but saying it here is what stops a caller reaching in to turn it
        // on — a notice that wraps is a notice that pushes the mail down.
        label.set_wrap(false);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.add_css_class("postio-notice-label");
        root.append(&label);

        let action = Rc::new(KeycapButton::new(
            None,
            "",
            &format!("{class}-action"),
            false,
        ));
        KeycapButton::arm(&action);
        action.widget().set_visible(false);
        root.append(&action.widget());

        let model = gio::Menu::new();
        let menu = gtk::MenuButton::new();
        menu.set_icon_name("view-more-symbolic");
        menu.add_css_class("flat");
        menu.add_css_class(&format!("{class}-menu"));
        menu.set_menu_model(Some(&model));
        menu.set_visible(false);
        menu.update_property(&[gtk::accessible::Property::Label("More about this notice")]);
        root.append(&menu);

        let group = gio::SimpleActionGroup::new();
        root.insert_action_group("notice", Some(&group));

        Rc::new(Self {
            root,
            icon,
            label,
            action,
            menu,
            model,
            group,
            handlers: RefCell::new(Vec::new()),
        })
    }

    /// The widget to place above the content it reports on.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Show or hide the whole notice.
    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    /// Whether the notice is on screen.
    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    /// What the notice says. One line; anything longer ellipsises.
    pub fn set_text(&self, text: &str) {
        self.label.set_label(text);
        // The label ellipsises, so the full sentence has to reach a screen
        // reader some other way.
        self.root
            .update_property(&[gtk::accessible::Property::Description(text)]);
    }

    /// What the notice currently says. Test-facing.
    pub fn text(&self) -> String {
        self.label.label().to_string()
    }

    /// Swap the icon — the same notice reporting a different state.
    pub fn set_icon(&self, icon_name: &str) {
        self.icon.set_icon_name(Some(icon_name));
    }

    /// Give the notice its one action, or `None` to take it away.
    pub fn set_action(&self, label: Option<&str>) {
        match label {
            Some(label) => {
                self.action.set_label(label);
                self.action.widget().set_visible(true);
            }
            None => self.action.widget().set_visible(false),
        }
    }

    /// The key shown on the action, from the keymap rather than a literal.
    pub fn set_action_key(&self, key: Option<&str>) {
        self.action.set_key(key);
    }

    /// Whether the action can be pressed. A notice keeps its shape when its
    /// action is unavailable rather than losing a button.
    pub fn set_action_sensitive(&self, sensitive: bool) {
        self.action.set_sensitive(sensitive);
    }

    /// The action button, for a test that wants to read its label or cap.
    pub fn action(&self) -> &Rc<KeycapButton> {
        &self.action
    }

    /// Called when the action is pressed.
    pub fn connect_action(&self, handler: impl Fn() + 'static) {
        self.action.connect_clicked(handler);
    }

    /// Press the action without a pointer, for a test.
    pub fn press_action(&self) {
        self.action.press();
    }

    /// Fill the overflow menu. An empty list hides the button.
    ///
    /// Replaces whatever was there: a notice's menu describes the message it
    /// is currently reporting on, and leaving the previous message's entries
    /// behind would offer to always-allow the wrong sender.
    pub fn set_menu(&self, items: Vec<NoticeMenuItem>) {
        self.model.remove_all();
        for name in self.group.list_actions() {
            self.group.remove_action(&name);
        }
        self.handlers.borrow_mut().clear();

        if items.is_empty() {
            self.menu.set_visible(false);
            return;
        }

        for (index, item) in items.into_iter().enumerate() {
            let name = format!("item{index}");
            let action = gio::SimpleAction::new(&name, None);
            let handler = item.handler.clone();
            action.connect_activate(move |_, _| handler());
            self.group.add_action(&action);
            self.model
                .append(Some(&item.label), Some(&format!("notice.{name}")));
            self.handlers.borrow_mut().push(item.handler);
        }
        self.menu.set_visible(true);
    }

    /// The overflow's entries, in order. Test-facing.
    pub fn menu_labels(&self) -> Vec<String> {
        (0..self.model.n_items())
            .filter_map(|index| {
                self.model
                    .item_attribute_value(index, "label", Some(glib_string()))
                    .and_then(|value| value.str().map(str::to_owned))
            })
            .collect()
    }

    /// Choose an overflow entry without opening the menu, for a test.
    pub fn press_menu_item(&self, index: usize) {
        if let Some(handler) = self.handlers.borrow().get(index) {
            handler();
        }
    }
}

/// `glib`'s string variant type, spelled once.
fn glib_string() -> &'static gtk::glib::VariantTy {
    gtk::glib::VariantTy::STRING
}
