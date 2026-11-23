//! The attachments of an open message as a row of chips (canvas 1b), drawn from the tree in
//! `postio_ui::reader::parts`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::pango;
use gtk::prelude::*;
use postio_model::Attachment;
use postio_ui::format::human_size;
use postio_ui::reader::parts::{Node, detail, spoken, tree};

type NodeHandler = Box<dyn Fn(&Node)>;

/// Only parts that hold bytes get a chip: a `multipart/alternative` appears in the tree but gets
/// none.
#[derive(Clone)]
pub struct Chips {
    row: gtk::Box,
    handlers: Rc<RefCell<Vec<NodeHandler>>>,
    cards: Rc<Cell<bool>>,
}

impl Default for Chips {
    fn default() -> Self {
        Self::new()
    }
}

impl Chips {
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

    pub fn set_cards(&self, cards: bool) {
        self.cards.set(cards);
    }

    pub fn widget(&self) -> gtk::Widget {
        self.row.clone().upcast()
    }

    /// `root` is the message's own content type, so the nodes handed to
    /// [`Chips::connect_activated`] are the ones the parts panel walks.
    pub fn set_parts(&self, root: &str, parts: &[Attachment]) {
        while let Some(child) = self.row.first_child() {
            self.row.remove(&child);
        }
        let mut any = false;
        for node in tree(root, parts).into_iter().filter(Node::is_leaf) {
            // The body parts are already on screen; a chip for the text being read is noise.
            if node.mime.starts_with("text/") && node.filename.is_none() {
                continue;
            }
            self.row.append(&self.chip(node));
            any = true;
        }
        self.row.set_visible(any);
    }

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
        // What it is, not what activating does: activation opens the parts panel, where the verbs
        // live.
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
