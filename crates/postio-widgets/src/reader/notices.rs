//! The one slot above a message where the reader says something about it: always the same height,
//! showing the most important
//! notice that applies, so the body does not move between messages raising different numbers of
//! notices. Order: decode
//! caveat, reader view (a rewrite is never silent, #1009), remote images, unsubscribe.

use std::cell::Cell;

use adw::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Notice {
    Decode = 0,
    ReaderView = 1,
    RemoteImages = 2,
    Unsubscribe = 3,
}

const ORDER: [Notice; 4] = [
    Notice::Decode,
    Notice::ReaderView,
    Notice::RemoteImages,
    Notice::Unsubscribe,
];

const NOTHING: &str = "nothing";

fn page_name(notice: Notice) -> &'static str {
    match notice {
        Notice::Decode => "decode",
        Notice::ReaderView => "reader-view",
        Notice::RemoteImages => "remote-images",
        Notice::Unsubscribe => "unsubscribe",
    }
}

pub(crate) struct NoticeSlot {
    stack: gtk::Stack,
    wanted: [Cell<bool>; 4],
    collapsing: Cell<bool>,
}

impl NoticeSlot {
    /// A `Stack` sized to its tallest page, with no transition: a notice sliding in is the body
    /// moving by another name.
    pub(crate) fn new(widgets: [gtk::Widget; 4]) -> Self {
        let stack = gtk::Stack::new();
        stack.add_css_class("postio-notice-slot");
        stack.set_vhomogeneous(true);
        stack.set_hhomogeneous(true);
        stack.set_transition_type(gtk::StackTransitionType::None);
        let nothing = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        stack.add_named(&nothing, Some(NOTHING));
        for (notice, widget) in ORDER.iter().zip(widgets) {
            // Visible as far as the stack is concerned: a hidden page would drop out of the height
            // the slot keeps.
            widget.set_visible(true);
            stack.add_named(&widget, Some(page_name(*notice)));
        }
        stack.set_visible_child_name(NOTHING);
        NoticeSlot {
            stack,
            wanted: Default::default(),
            collapsing: Cell::new(false),
        }
    }

    /// For a reader flowing in its owner's column, where an empty slot would be a gap. An empty
    /// collapsing slot is hidden, not
    /// zero tall, so a margin set on [`widget`](Self::widget) goes with it.
    pub(crate) fn set_collapsing(&self, collapsing: bool) {
        self.collapsing.set(collapsing);
        self.stack.set_vhomogeneous(!collapsing);
        self.show(self.page());
    }

    pub(crate) fn widget(&self) -> gtk::Widget {
        self.stack.clone().upcast()
    }

    pub(crate) fn want(&self, notice: Notice, wanted: bool) {
        self.wanted[notice as usize].set(wanted);
        self.show(self.page());
    }

    pub(crate) fn clear(&self) {
        for notice in ORDER {
            self.wanted[notice as usize].set(false);
        }
        self.show(NOTHING);
    }

    fn page(&self) -> &'static str {
        ORDER
            .iter()
            .find(|notice| self.wanted[**notice as usize].get())
            .map_or(NOTHING, |notice| page_name(*notice))
    }

    fn show(&self, page: &str) {
        self.stack.set_visible_child_name(page);
        if self.collapsing.get() {
            self.stack.set_visible(page != NOTHING);
        }
    }

    pub(crate) fn wanted(&self, notice: Notice) -> bool {
        self.wanted[notice as usize].get()
    }

    pub(crate) fn shows(&self, notice: Notice) -> bool {
        self.stack.is_visible()
            && self.stack.visible_child_name().as_deref() == Some(page_name(notice))
    }
}
