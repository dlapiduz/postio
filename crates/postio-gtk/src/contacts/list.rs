//! The Contacts list's model: a position is a person, asked for only when the list view is about to
//! draw it.
//! The controller holds the pages; this holds only the row count, so a thousand-person address book
//! costs what a dozen does.

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
    pub struct ContactsList(ObjectSubclass<imp::ContactsList>)
        @implements gio::ListModel;
}

impl Default for ContactsList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ContactsList {
    pub fn set_total(&self, total: u32) {
        let before = self.imp().total.replace(total);
        self.items_changed(0, before, total);
    }

    pub fn refresh(&self) {
        let total = self.imp().total.get();
        self.items_changed(0, total, total);
    }
}

pub fn position_of(item: &glib::Object) -> Option<u32> {
    item.downcast_ref::<gtk::StringObject>()?
        .string()
        .parse()
        .ok()
}
