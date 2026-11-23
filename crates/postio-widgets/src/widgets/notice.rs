//! One line that reports on the content below it: native chrome, never drawn inside the document it
//! reports on.
//! It never wraps: the label ellipsises and anything long goes in the overflow menu, since a
//! growing notice pushes the mail down.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;

use super::keycap::KeycapButton;

pub struct NoticeMenuItem {
    pub label: String,
    pub handler: Rc<dyn Fn()>,
}

impl NoticeMenuItem {
    pub fn new(label: impl Into<String>, handler: impl Fn() + 'static) -> Self {
        Self {
            label: label.into(),
            handler: Rc::new(handler),
        }
    }
}

pub struct NoticeBar {
    root: gtk::Box,
    label: gtk::Label,
    action: Rc<KeycapButton>,
    menu: gtk::MenuButton,
    model: gio::Menu,
    group: gio::SimpleActionGroup,
    /// Kept alive because a `gio::SimpleAction`'s `activate` closure runs them and the group only
    /// holds the actions.
    handlers: RefCell<Vec<Rc<dyn Fn()>>>,
}

impl NoticeBar {
    /// The action and overflow start hidden too, so no gap is reserved for buttons it never grows.
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
        // Never wraps: a wrapping notice pushes the mail down.
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
        let menu = super::icon_menu_button(
            "view-more-symbolic",
            "More about this notice",
            model.upcast_ref(),
        );
        menu.add_css_class(&format!("{class}-menu"));
        menu.set_visible(false);
        root.append(&menu);

        let group = gio::SimpleActionGroup::new();
        root.insert_action_group("notice", Some(&group));

        Rc::new(Self {
            root,
            label,
            action,
            menu,
            model,
            group,
            handlers: RefCell::new(Vec::new()),
        })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn set_visible(&self, visible: bool) {
        self.root.set_visible(visible);
    }

    pub fn is_visible(&self) -> bool {
        self.root.is_visible()
    }

    pub fn set_text(&self, text: &str) {
        self.label.set_label(text);
        // The label ellipsises, so the full sentence has to reach a screen
        // reader some other way.
        self.root
            .update_property(&[gtk::accessible::Property::Description(text)]);
    }

    pub fn text(&self) -> String {
        self.label.label().to_string()
    }

    pub fn set_action(&self, label: Option<&str>) {
        match label {
            Some(label) => {
                self.action.set_label(label);
                self.action.widget().set_visible(true);
            }
            None => self.action.widget().set_visible(false),
        }
    }

    pub fn set_action_key(&self, key: Option<&str>) {
        self.action.set_key(key);
    }

    pub fn action(&self) -> &Rc<KeycapButton> {
        &self.action
    }

    pub fn connect_action(&self, handler: impl Fn() + 'static) {
        self.action.connect_clicked(handler);
    }

    pub fn press_action(&self) {
        self.action.press();
    }

    /// An empty list hides the button. Replaces the old entries: leaving the previous message's
    /// would offer to always-allow the wrong sender.
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

    pub fn menu_labels(&self) -> Vec<String> {
        (0..self.model.n_items())
            .filter_map(|index| {
                self.model
                    .item_attribute_value(index, "label", Some(glib_string()))
                    .and_then(|value| value.str().map(str::to_owned))
            })
            .collect()
    }

    /// The handler is cloned out and the borrow dropped: acting on a notice often rebuilds this
    /// menu, so a handler run under the borrow would panic.
    pub fn press_menu_item(&self, index: usize) {
        let handler = self.handlers.borrow().get(index).cloned();
        if let Some(handler) = handler {
            handler();
        }
    }
}

fn glib_string() -> &'static gtk::glib::VariantTy {
    gtk::glib::VariantTy::STRING
}
