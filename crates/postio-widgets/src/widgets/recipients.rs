//! The composer's own address entry, moved inside and drawn as one chip per finished recipient,
//! followed by the address being typed.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use postio_model::EmailAddress;
use postio_model::address::{current_entry, parse_list};

use crate::widgets::space::S1;

type Changed = Box<dyn Fn()>;

pub struct RecipientChips {
    root: gtk::Box,
    chips: gtk::Box,
    entry: gtk::Entry,
    addresses: RefCell<Vec<EmailAddress>>,
    changed: RefCell<Vec<Changed>>,
    /// Set while the entry's text is rewritten here, so the rewrite is not
    /// read back as typing.
    committing: Cell<bool>,
}

impl RecipientChips {
    pub fn around(entry: &gtk::Entry) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        root.add_css_class("postio-recipient-chips");
        root.set_hexpand(true);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        let parent = entry.parent().and_downcast::<gtk::Box>();
        let before = entry.prev_sibling();
        if let Some(parent) = &parent {
            parent.remove(entry);
        }
        root.append(&chips);
        root.append(entry);
        if let Some(parent) = &parent {
            parent.insert_child_after(&root, before.as_ref());
        }
        let this = Rc::new(RecipientChips {
            root,
            chips,
            entry: entry.clone(),
            addresses: RefCell::default(),
            changed: RefCell::default(),
            committing: Cell::new(false),
        });
        let weak = Rc::downgrade(&this);
        entry.connect_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.commit();
            }
        });
        this.commit();
        this
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn addresses(&self) -> Vec<EmailAddress> {
        let mut addresses = self.addresses.borrow().clone();
        addresses.extend(parse_list(&self.entry.text()));
        addresses
    }

    pub fn set_addresses(self: &Rc<Self>, addresses: &[EmailAddress]) {
        self.committing.set(true);
        self.entry.set_text("");
        self.committing.set(false);
        self.addresses.replace(addresses.to_vec());
        self.draw();
    }

    pub fn connect_changed(&self, handler: impl Fn() + 'static) {
        self.changed.borrow_mut().push(Box::new(handler));
    }

    fn commit(self: &Rc<Self>) {
        if self.committing.get() {
            return;
        }
        let text = self.entry.text().to_string();
        let (start, _) = current_entry(&text);
        let finished = parse_list(&text[..start]);
        if finished.is_empty() {
            return;
        }
        self.addresses.borrow_mut().extend(finished);
        self.committing.set(true);
        self.entry.set_text(&text[start..]);
        self.entry.set_position(-1);
        self.committing.set(false);
        self.draw();
    }

    fn remove(self: &Rc<Self>, index: usize) {
        {
            let mut addresses = self.addresses.borrow_mut();
            if index >= addresses.len() {
                return;
            }
            addresses.remove(index);
        }
        self.draw();
    }

    fn draw(self: &Rc<Self>) {
        while let Some(child) = self.chips.first_child() {
            self.chips.remove(&child);
        }
        for (index, address) in self.addresses.borrow().iter().enumerate() {
            self.chips.append(&self.chip(index, address));
        }
        for handler in self.changed.borrow().iter() {
            handler();
        }
    }

    fn chip(self: &Rc<Self>, index: usize, address: &EmailAddress) -> gtk::Box {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, S1);
        chip.add_css_class("postio-recipient-chip");
        if let Some(name) = address
            .name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
        {
            let label = gtk::Label::new(Some(name));
            label.add_css_class("postio-recipient-chip-name");
            chip.append(&label);
        }
        let label = gtk::Label::new(Some(&address.address));
        label.add_css_class("postio-recipient-chip-address");
        chip.append(&label);
        let spoken = format!("Remove {}", address.address);
        let remove = super::icon_button("window-close-symbolic", &spoken);
        remove.add_css_class("postio-recipient-chip-remove");
        let weak = Rc::downgrade(self);
        remove.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.remove(index);
            }
        });
        chip.append(&remove);
        chip
    }
}
