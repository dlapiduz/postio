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
use postio_widgets::list_model::{ModelRow, Windowed, WindowedModel};

use super::item::FocusRow;

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
            // bound row widget redraws, and the model stays quiet (the
            // classic row's rule, #1216).
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
    }

    #[derive(Default)]
    pub struct FocusList {
        pub core: Windowed<super::RowObject>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FocusList {
        const NAME: &'static str = "PostioFocusList";
        type Type = super::FocusList;
        type Interfaces = (gio::ListModel,);
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
    /// Focus's list: a window over the paged store, never the mailbox.
    pub struct FocusList(ObjectSubclass<imp::FocusList>) @implements gio::ListModel;
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

impl WindowedModel for FocusList {
    type Row = RowObject;

    fn windowed(&self) -> &Windowed<RowObject> {
        &self.imp().core
    }
}
