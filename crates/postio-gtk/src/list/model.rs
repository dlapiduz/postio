//! Focus's list model: `postio_widgets::list_model`'s window over the paged
//! store, holding [`FocusRow`]s.
//!
//! The windowing, the page cache, the fill-in-place and the refresh are the
//! shared model's (T021, ADR 0043). What is here is the two `GObject`s a
//! `GtkListView` needs -- the item for one position, and the list -- in the
//! two-line shape `WindowedModel` asks for.

use std::cell::RefCell;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::list::ListRow;
use postio_ui::list::PAGE_SIZE;
use postio_widgets::list_model::{ModelRow, Windowed, WindowedModel};

use postio_ui::focus_list::{FocusRow, day_of};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct RowObject {
        pub item: RefCell<Option<FocusRow>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for RowObject {
        const NAME: &'static str = "PostioFocusRowObject";
        type Type = super::RowObject;
    }

    impl ObjectImpl for RowObject {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            // What the row says has changed while it stayed where it is: a
            // bound row widget redraws, and the model stays quiet (#1216).
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
    }

    #[derive(Default)]
    pub struct FocusList {
        pub core: Windowed<super::RowObject>,
        /// One heading over the whole list instead of a day's, while a
        /// filter is on: "Has action · 7" (screen 03).
        pub single: RefCell<Option<String>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FocusList {
        const NAME: &'static str = "PostioFocusList";
        type Type = super::FocusList;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }

    impl SectionModelImpl for FocusList {
        fn section(&self, position: u32) -> (u32, u32) {
            self.obj().day_section(position)
        }
    }

    impl ObjectImpl for FocusList {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(postio_widgets::list_model::signals)
        }
    }

    impl ListModelImpl for FocusList {
        fn item_type(&self) -> glib::Type {
            super::RowObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.core.n_items()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.obj().list_item(position)
        }
    }
}

glib::wrapper! {
    /// One position of Focus's list: a loaded [`FocusRow`], or a
    /// placeholder until its page arrives.
    pub struct RowObject(ObjectSubclass<imp::RowObject>);
}

glib::wrapper! {
    /// Focus's list: a window over the paged store, never the mailbox. Its
    /// sections are days, for the day headings (FR-010).
    pub struct FocusList(ObjectSubclass<imp::FocusList>)
        @implements gio::ListModel, gtk::SectionModel;
}

impl Default for FocusList {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl RowObject {
    /// What the row stands for, once its page has arrived.
    pub fn item(&self) -> Option<FocusRow> {
        self.imp().item.borrow().clone()
    }

    /// The local day the row's mail arrived on, once its page has.
    pub fn day(&self) -> Option<chrono::NaiveDate> {
        self.imp().item.borrow().as_ref().map(day_of)
    }

    /// Replace what the row says, keeping the object, and tell a bound
    /// widget -- quietly when nothing changed.
    pub fn set_item(&self, item: FocusRow) {
        {
            let mut held = self.imp().item.borrow_mut();
            if held.as_ref() == Some(&item) {
                return;
            }
            *held = Some(item);
        }
        self.emit_by_name::<()>("changed", &[]);
    }

    /// Call `on_change` whenever what the row says is replaced. The handler
    /// is the caller's to disconnect: a row widget is recycled across rows.
    pub fn connect_changed(&self, on_change: impl Fn(&Self) + 'static) -> glib::SignalHandlerId {
        self.connect_local("changed", false, move |values| {
            if let Some(row) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_change(&row);
            }
            None
        })
    }
}

impl ListRow for RowObject {
    fn id(&self) -> Option<MessageId> {
        self.imp().item.borrow().as_ref().map(FocusRow::id)
    }

    fn thread(&self) -> Option<ThreadId> {
        self.imp().item.borrow().as_ref().and_then(FocusRow::thread)
    }

    fn reconcile(existing: &Self, incoming: Self) -> Self {
        if let Some(item) = incoming.item() {
            existing.set_item(item);
        }
        existing.clone()
    }
}

impl ModelRow for RowObject {
    type Data = FocusRow;

    fn placeholder() -> Self {
        glib::Object::new()
    }

    fn with_contents(data: FocusRow) -> Self {
        let row = Self::placeholder();
        row.set_item(data);
        row
    }

    fn contents(&self) -> Option<FocusRow> {
        self.item()
    }

    fn fill(&self, data: FocusRow) {
        self.set_item(data);
    }

    fn id_of(data: &FocusRow) -> MessageId {
        data.id()
    }
}

impl FocusList {
    /// The run of positions around `position` that share its day: one day
    /// heading's section. Read from the rows already here and nothing else:
    /// a run whose page has not landed is its own section, bounded by its
    /// page, until it does -- and a landed page says so
    /// ([`Self::days_moved`]).
    pub fn day_section(&self, position: u32) -> (u32, u32) {
        let total = self.n_items();
        if position >= total {
            return (total, u32::MAX);
        }
        if self.imp().single.borrow().is_some() {
            return (0, total);
        }
        let window = self.windowed().window();
        let day_at = |at: u32| window.resident_at(at).and_then(RowObject::day);
        let day = day_at(position);
        let page = PAGE_SIZE;
        let (low, high) = match day {
            // A day can run across pages: only what is here bounds it.
            Some(_) => (0, total),
            // Nothing here: the unloaded run, no wider than its page.
            None => (
                position / page * page,
                ((position / page + 1) * page).min(total),
            ),
        };
        let mut start = position;
        while start > low && day_at(start - 1) == day {
            start -= 1;
        }
        let mut end = position + 1;
        while end < high && day_at(end) == day {
            end += 1;
        }
        (start, end)
    }

    /// Put one heading over the whole list -- a filter's, "Has action · 7"
    /// -- or, with `None`, go back to a heading a day.
    pub fn set_single_heading(&self, heading: Option<String>) {
        self.imp().single.replace(heading);
        self.days_moved();
    }

    /// The one heading over the whole list, while there is one.
    pub fn single_heading(&self) -> Option<String> {
        self.imp().single.borrow().clone()
    }

    /// Rows have landed or moved: the day headings may have moved with
    /// them, so the view asks the sections again.
    pub fn days_moved(&self) {
        let total = self.n_items();
        if total > 0 {
            self.sections_changed(0, total);
        }
    }
}

impl WindowedModel for FocusList {
    type Row = RowObject;

    fn windowed(&self) -> &Windowed<RowObject> {
        &self.imp().core
    }
}
