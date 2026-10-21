//! The Contacts list's model: a position is a person, asked for when the
//! list view is about to draw it and never before. The controller holds the
//! pages (`Feed::contacts_row`); this holds only how many rows there are, so a
//! thousand-person address book costs what a dozen does (docs/PRODUCT.md
//! section 18).

use std::cell::Cell;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ContactsList {
        pub total: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactsList {
        const NAME: &'static str = "PostioContactsList";
        type Type = super::ContactsList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ContactsList {}

    impl ListModelImpl for ContactsList {
        fn item_type(&self) -> glib::Type {
            gtk::StringObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.total.get()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            (position < self.total.get())
                .then(|| gtk::StringObject::new(&position.to_string()).upcast())
        }
    }
}

glib::wrapper! {
    /// How many people the list holds; each position is made when asked for.
    pub struct ContactsList(ObjectSubclass<imp::ContactsList>)
        @implements gio::ListModel;
}

impl Default for ContactsList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ContactsList {
    /// The list now holds `total` people: every row drawn is asked for again.
    pub fn set_total(&self, total: u32) {
        let before = self.imp().total.replace(total);
        self.items_changed(0, before, total);
    }

    /// Rows landed: the ones drawn are asked for again, the count unchanged.
    pub fn refresh(&self) {
        let total = self.imp().total.get();
        self.items_changed(0, total, total);
    }
}

/// The position a list item stands for.
pub fn position_of(item: &glib::Object) -> Option<u32> {
    item.downcast_ref::<gtk::StringObject>()?
        .string()
        .parse()
        .ok()
}
