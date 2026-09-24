//! The Contacts list's model: a `GListModel` windowed over paged storage.
//!
//! The same shape as the message list's (`crate::list`), for the same
//! reasons, at a fraction of the size: the paging lives in
//! `postio_ui::contacts::ContactsWindow`, and what stays here is what GTK's
//! own contract demands -- one [`ContactItem`] per position, filled in place
//! when its page lands so the view never rebuilds a widget for it, and the
//! rule that `GListModel::item()` is never answered by a model that changes
//! mid-call (see [`ContactsModel::hold`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ContactListRow;
use postio_ui::contacts::{ContactsWindow, Delivered, Slot};

/// Where the rows come from. The model never blocks on it: `request` starts
/// the read, and the answer arrives through [`ContactsModel::deliver`] --
/// carrying the `generation` it was asked for, so an answer for a list that
/// has since been reset is dropped.
pub trait ContactPageSource {
    /// Start reading `page` of the list as it stood at `generation`.
    fn request(&self, generation: u64, page: u32);
}

mod item_imp {
    use super::*;

    #[derive(Default)]
    pub struct ContactItem {
        pub row: RefCell<Option<ContactListRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactItem {
        const NAME: &'static str = "PostioContactItem";
        type Type = super::ContactItem;
    }

    impl ObjectImpl for ContactItem {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
    }
}

glib::wrapper! {
    /// One position in the Contacts list: a loaded row, or a placeholder
    /// whose page has not arrived.
    pub struct ContactItem(ObjectSubclass<item_imp::ContactItem>);
}

impl Default for ContactItem {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl ContactItem {
    /// Whether this position's page has arrived.
    pub fn is_loaded(&self) -> bool {
        self.imp().row.borrow().is_some()
    }

    /// The row, once it has arrived.
    pub fn row(&self) -> Option<ContactListRow> {
        self.imp().row.borrow().clone()
    }

    /// Fills the row in place and says so, quietly when nothing moved.
    pub fn set_row(&self, row: ContactListRow) {
        {
            let mut held = self.imp().row.borrow_mut();
            if held.as_ref() == Some(&row) {
                return;
            }
            *held = Some(row);
        }
        self.emit_by_name::<()>("changed", &[]);
    }

    /// Calls `on_change` whenever the row is filled or replaced. The handler
    /// is the caller's to disconnect when a recycled list item moves on.
    /// Asks whatever draws this item to draw it again: for a change the
    /// row's data does not carry, like being marked.
    pub fn touch(&self) {
        self.emit_by_name::<()>("changed", &[]);
    }

    pub fn connect_changed(&self, on_change: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("changed", false, move |values| {
            if let Some(item) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_change(&item);
            }
            None
        })
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ContactsModel {
        pub source: RefCell<Option<Rc<dyn ContactPageSource>>>,
        pub window: RefCell<ContactsWindow>,
        /// Whether the model is part-way through answering `item()`.
        pub reading: Cell<bool>,
        /// The item each position answers with, for as long as the position
        /// means the same person.
        pub handed: RefCell<HashMap<u32, ContactItem>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ContactsModel {
        const NAME: &'static str = "PostioContactsModel";
        type Type = super::ContactsModel;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for ContactsModel {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            // A delivery gave rows to positions that already existed -- for
            // whoever is not a `GtkListView` and wants to know, the detail
            // column following the cursor onto a row that just became real.
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("filled").build()])
        }
    }

    impl ListModelImpl for ContactsModel {
        fn item_type(&self) -> glib::Type {
            ContactItem::static_type()
        }

        fn n_items(&self) -> u32 {
            self.window.borrow().total()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            let outer = self.reading.replace(true);
            let item = self.obj().item_at(position);
            self.reading.set(outer);
            item.map(|item| item.upcast())
        }
    }
}

glib::wrapper! {
    /// A `GListModel` over one Contacts view, windowed rather than loaded.
    pub struct ContactsModel(ObjectSubclass<imp::ContactsModel>)
        @implements gio::ListModel;
}

impl ContactsModel {
    /// An empty list reading from `source`.
    pub fn new(source: Rc<dyn ContactPageSource>) -> Self {
        let model: Self = glib::Object::new();
        model.imp().source.replace(Some(source));
        model
    }

    /// The generation requests are being made for.
    pub fn generation(&self) -> u64 {
        self.imp().window.borrow().generation()
    }

    /// Starts the list over at `total` rows, returning the new generation.
    ///
    /// Not held while the model is answering `item()`: a reset is a change of
    /// view or filter, made by the user, never by a page source mid-read.
    pub fn reset(&self, total: u32) -> u64 {
        let removed = self.imp().window.borrow().total();
        let generation = self.imp().window.borrow_mut().reset(total);
        self.imp().handed.borrow_mut().clear();
        self.items_changed(0, removed, total);
        generation
    }

    /// A page arrived for `generation`.
    pub fn deliver(&self, generation: u64, page: u32, rows: Vec<ContactListRow>, total: u32) {
        if self.imp().reading.get() {
            self.hold(move |model| model.deliver(generation, page, rows, total));
            return;
        }
        let start = page * postio_ui::list::PAGE_SIZE;
        let filling: Vec<(ContactItem, ContactListRow)> = {
            let handed = self.imp().handed.borrow();
            rows.iter()
                .enumerate()
                .filter_map(|(offset, row)| {
                    handed
                        .get(&(start + offset as u32))
                        .map(|held| (held.clone(), row.clone()))
                })
                .collect()
        };
        let before = self.imp().window.borrow().total();
        let delivered = self
            .imp()
            .window
            .borrow_mut()
            .deliver(generation, page, rows, total);
        let Delivered::Filled {
            total_changed,
            evicted,
            ..
        } = delivered
        else {
            return;
        };
        for (held, row) in filling {
            held.set_row(row);
        }
        self.emit_by_name::<()>("filled", &[]);
        {
            let mut handed = self.imp().handed.borrow_mut();
            for page in evicted {
                let first = page * postio_ui::list::PAGE_SIZE;
                for position in first..first + postio_ui::list::PAGE_SIZE {
                    handed.remove(&position);
                }
            }
        }
        if total_changed {
            // The list grew or shrank at its end while this page was read.
            self.imp().handed.borrow_mut().retain(|p, _| *p < total);
            if total > before {
                self.items_changed(before, 0, total - before);
            } else {
                self.items_changed(total, before - total, 0);
            }
        }
    }

    /// Calls `on_filled` whenever a delivery lands -- after the rows are in,
    /// which a delivery held past an `item()` call may be a turn later.
    pub fn connect_filled(&self, on_filled: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("filled", false, move |values| {
            if let Some(model) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_filled(&model);
            }
            None
        })
    }

    /// Gives up on a page whose read failed, so the view can ask again.
    pub fn abandon(&self, generation: u64, page: u32) {
        if self.imp().reading.get() {
            self.hold(move |model| model.abandon(generation, page));
            return;
        }
        self.imp().window.borrow_mut().abandon(generation, page);
    }

    /// The row at `position`, if its page is resident.
    pub fn row(&self, position: u32) -> Option<ContactListRow> {
        match self.imp().window.borrow_mut().row_at(position) {
            Slot::Row(row) => Some(row.clone()),
            Slot::Loading { .. } => None,
        }
    }

    /// Where a person sits, if their page is resident.
    pub fn position_of(&self, id: postio_model::ContactId) -> Option<u32> {
        self.imp().window.borrow().position_of(id)
    }

    fn item_at(&self, position: u32) -> Option<ContactItem> {
        if position >= self.imp().window.borrow().total() {
            return None;
        }
        let (row, request) = match self.imp().window.borrow_mut().row_at(position) {
            Slot::Row(row) => (Some(row.clone()), None),
            Slot::Loading { request } => (None, request),
        };
        let item = self
            .imp()
            .handed
            .borrow_mut()
            .entry(position)
            .or_default()
            .clone();
        if let Some(row) = row {
            item.set_row(row);
        }
        if let Some(page) = request {
            let generation = self.generation();
            let source = self.imp().source.borrow().clone();
            if let Some(source) = source {
                source.request(generation, page);
            }
        }
        Some(item)
    }

    /// Re-runs `action` on the next turn of the main loop, above GDK's redraw
    /// band -- the message list's `hold` gives the reasons: a source may
    /// answer while `item()` is still answering, `GtkListView` does not
    /// survive the model changing under it, and a deferral a repaint can
    /// postpone indefinitely is not a deferral.
    fn hold(&self, action: impl FnOnce(&ContactsModel) + 'static) {
        let model = self.clone();
        let mut action = Some(action);
        glib::idle_add_local_full(glib::Priority::HIGH_IDLE, move || {
            if let Some(action) = action.take() {
                action(&model);
            }
            glib::ControlFlow::Break
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::{ContactId, ContactListRow, ContactSource, ContactState};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct Recorded(RefCell<Vec<(u64, u32)>>);

    impl ContactPageSource for Recorded {
        fn request(&self, generation: u64, page: u32) {
            self.0.borrow_mut().push((generation, page));
        }
    }

    fn rows(page: u32, len: u32) -> Vec<ContactListRow> {
        (0..len)
            .map(|i| {
                let n = page * postio_ui::list::PAGE_SIZE + i;
                ContactListRow {
                    id: ContactId::new(i64::from(n) + 1),
                    name: format!("Person {n}"),
                    preferred: Some(format!("p{n}@example.com")),
                    address_count: 1,
                    last_seen_at: None,
                    source: ContactSource::Mail,
                    state: ContactState::Live,
                    sort_key: format!("person {n:06}"),
                }
            })
            .collect()
    }

    fn item(model: &ContactsModel, position: u32) -> ContactItem {
        model
            .item(position)
            .and_downcast::<ContactItem>()
            .expect("an item at every position")
    }

    #[test]
    fn a_new_list_is_as_long_as_the_view_and_asks_for_the_page_it_is_read_at() {
        let source = Rc::new(Recorded::default());
        let model = ContactsModel::new(source.clone());
        let generation = model.reset(120);

        assert_eq!(model.n_items(), 120);
        let placeholder = item(&model, 60);
        assert!(
            !placeholder.is_loaded(),
            "a skeleton until its page arrives"
        );
        let _ = item(&model, 61);
        assert_eq!(*source.0.borrow(), [(generation, 1)], "one request a page");
    }

    #[test]
    fn a_delivered_page_fills_the_objects_the_view_already_holds() {
        let source = Rc::new(Recorded::default());
        let model = ContactsModel::new(source.clone());
        let generation = model.reset(120);
        let held = item(&model, 60);

        model.deliver(generation, 1, rows(1, 50), 120);

        assert_eq!(
            held.row().map(|row| row.name),
            Some("Person 60".to_owned()),
            "the same object, filled in -- not a new one the view never sees"
        );
        assert_eq!(item(&model, 60), held);
    }

    #[test]
    fn a_page_for_a_list_that_has_moved_on_is_dropped() {
        let source = Rc::new(Recorded::default());
        let model = ContactsModel::new(source.clone());
        let old = model.reset(120);
        let _ = item(&model, 0);
        model.reset(3);

        model.deliver(old, 0, rows(0, 50), 120);

        assert_eq!(model.n_items(), 3);
        assert!(!item(&model, 0).is_loaded());
    }

    #[test]
    fn a_resetting_list_announces_its_new_length() {
        let source = Rc::new(Recorded::default());
        let model = ContactsModel::new(source);
        let changes = Rc::new(RefCell::new(Vec::new()));
        model.connect_items_changed({
            let changes = changes.clone();
            move |_, position, removed, added| changes.borrow_mut().push((position, removed, added))
        });

        model.reset(10);
        model.reset(4);

        assert_eq!(*changes.borrow(), [(0, 0, 10), (0, 10, 4)]);
    }

    #[test]
    fn a_failed_page_is_asked_for_again() {
        let source = Rc::new(Recorded::default());
        let model = ContactsModel::new(source.clone());
        let generation = model.reset(120);
        let _ = item(&model, 0);
        model.abandon(generation, 0);
        let _ = item(&model, 1);
        assert_eq!(source.0.borrow().len(), 2);
    }
}
