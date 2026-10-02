//! The empty inbox (screen 16; contracts/focus-surface.md, "States"): a tray,
//! "Inbox is empty" in bold (or, until a first sync has finished, what the
//! inbox is waiting on: T220), when the next digest comes if there are
//! digests, and shortcuts to only what exists.
//!
//! What it says is `postio_ui::focus_state::empty_inbox`'s; this draws it.
//! Every shortcut is a button that runs its command, and wears the key the
//! keymap in force gives that command.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use postio_core::CommandId;
use postio_widgets::widgets::keyhint;
use postio_widgets::widgets::space::{S1, S2, S3};
use postio_widgets::widgets::{Kind, Size};

/// What a shortcut asks the window to do.
type Handler = Rc<dyn Fn(CommandId)>;

/// The empty page.
pub struct EmptyInbox {
    root: gtk::Box,
    heading: gtk::Label,
    detail: gtk::Label,
    next_digest: gtk::Label,
    shortcuts: gtk::Box,
    handler: RefCell<Option<Handler>>,
}

impl EmptyInbox {
    /// The page, saying nothing beyond its heading until it is told.
    pub fn new() -> Rc<Self> {
        let tray = gtk::Image::from_gicon(&gio::ThemedIcon::from_names(&[
            "mail-inbox-symbolic",
            "folder-symbolic",
        ]));
        tray.set_pixel_size(48);
        tray.add_css_class("dim-label");
        tray.set_accessible_role(gtk::AccessibleRole::Presentation);
        let heading = gtk::Label::new(Some("Inbox is empty"));
        heading.add_css_class("focus-empty-heading");
        let detail = gtk::Label::new(None);
        detail.add_css_class("dim-label");
        detail.set_visible(false);
        detail.set_wrap(true);
        detail.set_justify(gtk::Justification::Center);
        let next_digest = gtk::Label::new(None);
        next_digest.add_css_class("dim-label");
        next_digest.set_visible(false);
        let shortcuts = gtk::Box::new(gtk::Orientation::Horizontal, S3);
        shortcuts.set_halign(gtk::Align::Center);
        let root = gtk::Box::new(gtk::Orientation::Vertical, S2);
        root.add_css_class("focus-empty");
        root.set_valign(gtk::Align::Center);
        root.set_halign(gtk::Align::Center);
        root.set_vexpand(true);
        root.append(&tray);
        root.append(&heading);
        root.append(&detail);
        root.append(&next_digest);
        root.append(&shortcuts);
        Rc::new(EmptyInbox {
            root,
            heading,
            detail,
            next_digest,
            shortcuts,
            handler: RefCell::default(),
        })
    }

    /// The page, to place where the list goes.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Run `handler` with a shortcut's command when it is pressed.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.handler.replace(Some(Rc::new(handler)));
    }

    /// Say `said`.
    pub fn show(self: &Rc<Self>, said: &postio_ui::focus_state::EmptyInbox) {
        self.heading.set_text(&said.heading);
        match &said.detail {
            Some(line) => {
                self.detail.set_text(line);
                self.detail.set_visible(true);
            }
            None => self.detail.set_visible(false),
        }
        match &said.next_digest {
            Some(line) => {
                self.next_digest.set_text(line);
                self.next_digest.set_visible(true);
            }
            None => self.next_digest.set_visible(false),
        }
        while let Some(child) = self.shortcuts.first_child() {
            self.shortcuts.remove(&child);
        }
        for (key, what, command) in &said.shortcuts {
            let button = gtk::Button::new();
            postio_widgets::widgets::button::style(&button, Kind::Ghost, Size::Regular);
            button.add_css_class("focus-empty-shortcut");
            let row = gtk::Box::new(gtk::Orientation::Horizontal, S1);
            if let Some(key) = key {
                let cap = keyhint::cap(key);
                cap.set_valign(gtk::Align::Center);
                row.append(&cap);
            }
            row.append(&gtk::Label::new(Some(what)));
            button.set_child(Some(&row));
            let weak = Rc::downgrade(self);
            let command = *command;
            button.connect_clicked(move |_| {
                let handler = weak
                    .upgrade()
                    .and_then(|page| page.handler.borrow().clone());
                if let Some(handler) = handler {
                    handler(command);
                }
            });
            self.shortcuts.append(&button);
        }
    }
}
