//! The one place above a message where the reader says something about it.
//!
//! A message can raise several notices at once -- the words may not be the
//! sender's, the layout was reduced, images were held back, it came from a
//! list -- and each used to be a bar of its own, stacked above the body. So
//! the body's first line sat one bar lower for every notice the message
//! raised, and moving between two messages that raised different numbers of
//! them moved the text a person was about to read: 116px, 150px, 218px down
//! over three messages in the test that now holds it still.
//!
//! One slot instead, always the same height: the most important notice that
//! applies, or nothing drawn in the same space. Dealing with the one on show
//! -- showing the images, leaving reader view -- brings up the next.
//!
//! # The order
//!
//! What a person most needs to know about the words in front of them first:
//!
//! 1. **Decode** -- parts of this may not be what was sent. A caveat about
//!    the text itself outranks everything about its presentation.
//! 2. **Reader view** -- what is on screen is Postio's reduction, not the
//!    sender's layout. A rewrite must never be silent (#1009).
//! 3. **Remote images** -- something was held back, and here is the consent.
//! 4. **Unsubscribe** -- an offer about the list, not about this message.

use std::cell::Cell;

use adw::prelude::*;

/// Which notice, in the order the slot prefers them.
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

/// The page shown when no notice applies.
const NOTHING: &str = "nothing";

fn page_name(notice: Notice) -> &'static str {
    match notice {
        Notice::Decode => "decode",
        Notice::ReaderView => "reader-view",
        Notice::RemoteImages => "remote-images",
        Notice::Unsubscribe => "unsubscribe",
    }
}

/// The slot: every notice, one of them showing, always one bar tall.
pub(crate) struct NoticeSlot {
    stack: gtk::Stack,
    wanted: [Cell<bool>; 4],
}

impl NoticeSlot {
    /// A slot holding these four notice widgets, in [`Notice`] order.
    ///
    /// A `Stack` sized to its tallest page whichever is showing, so the space
    /// is the same with one notice, three, or none -- and no transition,
    /// because a notice sliding in is the body moving by another name.
    pub(crate) fn new(widgets: [gtk::Widget; 4]) -> Self {
        let stack = gtk::Stack::new();
        stack.add_css_class("postio-notice-slot");
        stack.set_vhomogeneous(true);
        stack.set_hhomogeneous(true);
        stack.set_transition_type(gtk::StackTransitionType::None);
        let nothing = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        stack.add_named(&nothing, Some(NOTHING));
        for (notice, widget) in ORDER.iter().zip(widgets) {
            // Visible as far as the stack is concerned, always: which one is
            // on screen is the stack's page, and a hidden page would drop out
            // of the height the slot keeps.
            widget.set_visible(true);
            stack.add_named(&widget, Some(page_name(*notice)));
        }
        stack.set_visible_child_name(NOTHING);
        NoticeSlot {
            stack,
            wanted: Default::default(),
        }
    }

    pub(crate) fn widget(&self) -> gtk::Widget {
        self.stack.clone().upcast()
    }

    /// Say whether `notice` applies to the message on screen.
    pub(crate) fn want(&self, notice: Notice, wanted: bool) {
        self.wanted[notice as usize].set(wanted);
        let page = ORDER
            .iter()
            .find(|notice| self.wanted[**notice as usize].get())
            .map_or(NOTHING, |notice| page_name(*notice));
        self.stack.set_visible_child_name(page);
    }

    /// Nothing applies: a message was replaced by something the notices
    /// were not about.
    pub(crate) fn clear(&self) {
        for notice in ORDER {
            self.wanted[notice as usize].set(false);
        }
        self.stack.set_visible_child_name(NOTHING);
    }

    /// Whether `notice` applies to the message on screen, drawn or not: the
    /// slot shows one notice at a time, and the others still apply.
    pub(crate) fn wanted(&self, notice: Notice) -> bool {
        self.wanted[notice as usize].get()
    }

    /// Whether `notice` is the one on screen.
    pub(crate) fn shows(&self, notice: Notice) -> bool {
        self.stack.is_visible()
            && self.stack.visible_child_name().as_deref() == Some(page_name(notice))
    }
}
