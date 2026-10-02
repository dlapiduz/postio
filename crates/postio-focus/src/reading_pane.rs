//! The reading pane (T232; screens.md, "Reading beside the list"): where an
//! open message is drawn when the person reads beside the list rather than
//! over it.
//!
//! The pane draws nothing of its own. It holds the message dialog's one
//! message view (`open::OpenMessage`), the composer's surface when a reply
//! takes the pane over, and, with neither, the empty inbox's page saying
//! nothing is open and how to open something. Which of the three shows is
//! the window's to compute from what is open ([`Page`]), never something
//! each occupant toggles for itself.

use std::rc::Rc;

use gtk::prelude::*;
use postio_core::{CommandId, Keymap};

/// What the pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Nothing is open: what opens a message, and the way back to the dialog.
    Empty,
    /// The open message.
    Message,
    /// The composer, which takes the place of the open message (T221).
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

/// The pane: one stack, a page for each occupant.
pub struct ReadingPane {
    root: gtk::Stack,
    message: gtk::Box,
    compose: gtk::Box,
    empty: Rc<crate::empty::EmptyInbox>,
}

impl ReadingPane {
    /// An empty pane, its keys from `keymap`.
    pub fn new(keymap: &Keymap) -> Rc<Self> {
        let slot = || {
            let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
            slot.set_hexpand(true);
            slot.set_vexpand(true);
            slot
        };
        let (message, compose) = (slot(), slot());
        let empty = crate::empty::EmptyInbox::new();
        // The empty inbox's page, drawn as it is; named for the pane, so the
        // inbox's own empty page is still the one `focus-empty`.
        empty.widget().remove_css_class("focus-empty");
        empty.hide_picture();
        empty.widget().add_css_class("focus-reading-pane-empty");
        empty.show(&postio_ui::focus_state::empty_pane(keymap));
        let root = gtk::Stack::new();
        root.add_css_class("focus-reading-pane");
        // A pane switch is instant (the motion budget): no transition.
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

    /// The pane, to place beside the list.
    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// Where the open message's view goes.
    pub fn message_slot(&self) -> &gtk::Box {
        &self.message
    }

    /// Where the composer's surface goes.
    pub fn compose_slot(&self) -> &gtk::Box {
        &self.compose
    }

    /// Show `page`.
    pub fn show(&self, page: Page) {
        if self.root.visible_child_name().as_deref() != Some(page.name()) {
            self.root.set_visible_child_name(page.name());
        }
    }

    /// What the pane shows.
    pub fn page(&self) -> Page {
        match self.root.visible_child_name().as_deref() {
            Some("message") => Page::Message,
            Some("compose") => Page::Compose,
            _ => Page::Empty,
        }
    }

    /// Run `handler` with the command an empty pane's shortcut stands for.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.empty.connect_command(handler);
    }

    /// Draw every key the empty pane shows from `keymap`.
    pub fn set_keymap(&self, keymap: &Keymap) {
        self.empty.show(&postio_ui::focus_state::empty_pane(keymap));
    }
}
