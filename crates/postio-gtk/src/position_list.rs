//! A list model of positions only: the controller holds the rows, and the view asks for one when it
//! is about to draw it, so a long list costs what a short one does. Contacts and Agenda use it.

use std::cell::Cell;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct PositionList {
        pub total: Cell<u32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for PositionList {
        const NAME: &'static str = "PostioPositionList";
        type Type = super::PositionList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for PositionList {}

    impl ListModelImpl for PositionList {
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
    pub struct PositionList(ObjectSubclass<imp::PositionList>)
        @implements gio::ListModel;
}

impl Default for PositionList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl PositionList {
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
