//! The reading pane: where an open message is drawn when read beside the list. It holds the message
//! dialog's one message view,
//! the composer's surface when a reply takes the pane over, and else the empty page. Which shows is
//! the window's to compute, never each occupant's.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Empty,
    Message,
    Compose,
}

impl Page {
    fn name(self) -> &'static str {
        match self {
            Page::Empty => "empty",
            Page::Message => "message",
            Page::Compose => "compose",
        }
    }
}

pub struct ReadingPane {
    root: gtk::Stack,
    message: gtk::Box,
    compose: gtk::Box,
    empty: Rc<crate::empty::EmptyInbox>,
}

impl ReadingPane {
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let slot = || {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.set_hexpand(true);
            slot.set_vexpand(true);
            slot
        };
        let (message, compose) = (slot(), slot());
        let empty = crate::empty::EmptyInbox::new();
        empty.widget().remove_css_class("focus-empty");
        empty.hide_picture();
        empty.widget().add_css_class("focus-reading-pane-empty");
        empty.show(&postio_ui::focus_state::empty_pane(keymap));
        let root = gtk::Stack::new();
        root.add_css_class("focus-reading-pane");
        root.set_transition_type(gtk::StackTransitionType::None);
        root.set_hhomogeneous(false);
        root.set_vexpand(true);
        root.add_named(empty.widget(), Some(Page::Empty.name()));
        root.add_named(&message, Some(Page::Message.name()));
        root.add_named(&compose, Some(Page::Compose.name()));
        root.set_visible_child_name(Page::Empty.name());
        Rc::new(ReadingPane {
            root,
            message,
            compose,
            empty,
        })
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    pub fn message_slot(&self) -> &gtk::Box {
        &self.message
    }

    pub fn compose_slot(&self) -> &gtk::Box {
        &self.compose
    }

    pub fn show(&self, page: Page) {
        if self.root.visible_child_name().as_deref() != Some(page.name()) {
            self.root.set_visible_child_name(page.name());
        }
    }

    pub fn page(&self) -> Page {
        match self.root.visible_child_name().as_deref() {
            Some("message") => Page::Message,
            Some("compose") => Page::Compose,
            _ => Page::Empty,
        }
    }

    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.empty.connect_command(handler);
    }

    pub fn set_keymap(&self, keymap: &Keymap) {
        self.empty.show(&postio_ui::focus_state::empty_pane(keymap));
    }
}
