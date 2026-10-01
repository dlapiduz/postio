//! The attachments of an open message, as a row of chips (canvas 1b).
//!
//! Moved from postio-gtk's `parts.rs` with the reader that draws them
//! (ADR 0043; specs/007-postio-focus T019). The tree they are drawn from is
//! `postio_ui::reader::parts`; the panel they open is the classic app's.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::pango;
use gtk::prelude::*;
use postio_model::Attachment;
use postio_ui::format::human_size;
use postio_ui::reader::parts::{Node, detail, spoken, tree};

/// Called with the part a chip stands for.
type NodeHandler = Box<dyn Fn(&Node)>;

/// The attachments of an open message, as a row of chips.
///
/// Canvas 1b draws these under the body: what came with the message, named
/// and sized, before anything is downloaded. They are the way into the
/// classic app's parts panel (`postio_gtk::parts::PartsPanel`) — a message's
/// structure is a thing you go and look at, and this is the affordance that
/// says there is something to look at.
///
/// Only parts that hold bytes get a chip. A `multipart/alternative` is real
/// and appears in the tree, but nobody wants a chip for it.
#[derive(Clone)]
pub struct Chips {
    row: gtk::Box,
    handlers: Rc<RefCell<Vec<NodeHandler>>>,
    /// Drawn as cards -- an icon, the name, the size in mono, in a row --
    /// rather than as pills ([`Chips::set_cards`]).
    cards: Rc<Cell<bool>>,
}

impl Default for Chips {
    fn default() -> Self {
        Self::new()
    }
}

impl Chips {
    /// An empty, hidden row.
    pub fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("postio-attachments");
        row.set_visible(false);
        row.update_property(&[gtk::accessible::Property::Label("Attachments")]);
        Chips {
            row,
            handlers: Rc::new(RefCell::new(Vec::new())),
            cards: Rc::new(Cell::new(false)),
        }
    }

    /// Draw each attachment as a card, as Focus's open message does (the
    /// message dialog handoff, T209): a 40px chip holding a file icon, the
    /// name and the size in mono, in one row. Read when the parts are next
    /// set.
    pub fn set_cards(&self, cards: bool) {
        self.cards.set(cards);
    }

    /// The widget to place under a message body.
    pub fn widget(&self) -> gtk::Widget {
        self.row.clone().upcast()
    }

    /// Draw a chip for every part of `parts` that holds bytes.
    ///
    /// `root` is the message's own content type, so the nodes handed to
    /// [`Chips::connect_activated`] are the same nodes the parts panel walks.
    pub fn set_parts(&self, root: &str, parts: &[Attachment]) {
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        let mut any = false;
        for node in tree(root, parts).into_iter().filter(Node::is_leaf) {
            // The body parts came with the message and are already on screen;
            // a chip for the text you are reading is noise.
            if node.mime.starts_with("text/") && node.filename.is_none() {
                continue;
            }
            self.row.append(&self.chip(node));
            any = true;
        }
        self.row.set_visible(any);
    }

    /// Called when a chip is activated, with the part it stands for.
    pub fn connect_activated(&self, handler: impl Fn(&Node) + 'static) {
        self.handlers.borrow_mut().push(Box::new(handler));
    }

    fn chip(&self, node: Node) -> gtk::Button {
        let name = gtk::Label::new(Some(node.label()));
        name.add_css_class("postio-attachment-name");
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        name.set_max_width_chars(24);
        name.set_accessible_role(gtk::AccessibleRole::Presentation);

        let size = gtk::Label::new(Some(&human_size(node.size)));
        size.add_css_class("postio-attachment-size");
        size.set_accessible_role(gtk::AccessibleRole::Presentation);

        let button = gtk::Button::new();
        button.add_css_class("postio-attachment");
        if self.cards.get() {
            button.add_css_class("postio-attachment-card");
            name.set_xalign(0.0);
            size.set_xalign(0.0);
            let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
            icon.add_css_class("postio-attachment-icon");
            icon.set_accessible_role(gtk::AccessibleRole::Presentation);
            let card = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S3);
            card.append(&icon);
            card.append(&name);
            card.append(&size);
            button.set_child(Some(&card));
        } else {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            line.append(&name);
            line.append(&size);
            button.set_child(Some(&line));
        }
        button.update_property(&[gtk::accessible::Property::Label(&spoken(&node))]);
        // What it *is*, not what activating it will do: activating opens the
        // parts panel, which is where the verbs live. A chip that promised to
        // save would be a second place the same verb was implemented.
        button.set_tooltip_text(Some(&format!(
            "{} — show the message's parts",
            detail(&node)
        )));

        let handlers = Rc::clone(&self.handlers);
        button.connect_clicked(move |_| {
            for handler in handlers.borrow().iter() {
                handler(&node);
            }
        });
        button
    }
}
