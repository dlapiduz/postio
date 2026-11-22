//! What the list view draws: the paged rows with an expanded thread's children spliced in under
//! their row. The paged model never holds a child, since putting a thread's messages into the paged
//! query would shift every page's offsets. Children appear and go in one change, with no
//! transition.

use std::cell::{Cell, RefCell};

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ids::MessageId;
use postio_ui::thread_list::{ChildRowView, Slot, Splice};
use postio_widgets::list_model::WindowedModel;

use super::model::{FocusList, RowObject};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SplicedList {
        pub rows: RefCell<Option<FocusList>>,
        pub splice: Cell<Option<Splice>>,
        pub children: RefCell<Vec<RowObject>>,
        pub anchor: RefCell<Option<RowObject>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SplicedList {
        const NAME: &'static str = "PostioFocusSplicedList";
        type Type = super::SplicedList;
        type Interfaces = (gio::ListModel, gtk::SectionModel);
    }

    impl ObjectImpl for SplicedList {}

    impl ListModelImpl for SplicedList {
        fn item_type(&self) -> glib::Type {
            RowObject::static_type()
        }

        fn n_items(&self) -> u32 {
            self.obj().len()
        }

        fn item(&self, position: u32) -> Option<glib::Object> {
            self.obj().object_at(position)
        }
    }

    impl SectionModelImpl for SplicedList {
        fn section(&self, position: u32) -> (u32, u32) {
            self.obj().section_at(position)
        }
    }
}

glib::wrapper! {
    pub struct SplicedList(ObjectSubclass<imp::SplicedList>)
        @implements gio::ListModel, gtk::SectionModel;
}

impl SplicedList {
    pub fn new(rows: &FocusList) -> Self {
        let list: Self = glib::Object::new();
        list.imp().rows.replace(Some(rows.clone()));
        rows.connect_items_changed({
            let list = list.downgrade();
            move |_, position, removed, added| {
                if let Some(list) = list.upgrade() {
                    list.rows_changed(position, removed, added);
                }
            }
        });
        rows.connect_sections_changed({
            let list = list.downgrade();
            move |_, _, _| {
                if let Some(list) = list.upgrade() {
                    let total = list.len();
                    if total > 0 {
                        list.sections_changed(0, total);
                    }
                }
            }
        });
        list
    }

    fn rows(&self) -> FocusList {
        self.imp()
            .rows
            .borrow()
            .clone()
            .expect("a spliced list is made over its rows")
    }

    fn effective(&self) -> Option<Splice> {
        let splice = self.imp().splice.get()?;
        (splice.anchor < self.rows().n_items() && splice.children > 0).then_some(splice)
    }

    fn len(&self) -> u32 {
        postio_ui::thread_list::len(self.imp().splice.get(), self.rows().n_items())
    }

    pub fn slot(&self, position: u32) -> Slot {
        postio_ui::thread_list::slot(self.effective(), position)
    }

    pub fn display_of(&self, row: u32) -> u32 {
        postio_ui::thread_list::display(self.effective(), Slot::Row(row))
    }

    pub fn position_of(&self, message: MessageId) -> Option<u32> {
        self.rows()
            .position_of(message)
            .map(|row| self.display_of(row))
    }

    pub fn open_thread(&self) -> Option<Splice> {
        self.effective()
    }

    pub fn child_at(&self, position: u32) -> Option<ChildRowView> {
        self.object_at(position)
            .and_downcast::<RowObject>()
            .and_then(|object| object.child_view())
    }

    fn object_at(&self, position: u32) -> Option<glib::Object> {
        match self.slot(position) {
            Slot::Row(row) => self.rows().item(row),
            Slot::Child { index, .. } => self
                .imp()
                .children
                .borrow()
                .get(index as usize)
                .map(|object| object.clone().upcast()),
        }
    }

    fn section_at(&self, position: u32) -> (u32, u32) {
        let total = self.len();
        if position >= total {
            return (total, u32::MAX);
        }
        let rows = self.rows();
        let Some(splice) = self.effective() else {
            return rows.day_section(position);
        };
        let row = match splice.slot(position) {
            Slot::Row(row) | Slot::Child { row, .. } => row,
        };
        let (start, end) = rows.day_section(row);
        let shift = |row: u32| {
            if row > splice.anchor {
                row + splice.children
            } else {
                row
            }
        };
        let end = if end > splice.anchor {
            end.saturating_add(splice.children).min(total)
        } else {
            end
        };
        (shift(start), end)
    }

    fn rows_changed(&self, position: u32, removed: u32, added: u32) {
        let Some(splice) = self.imp().splice.get() else {
            self.items_changed(position, removed, added);
            return;
        };
        let now = self.rows().n_items();
        let before = now + removed - added;
        let was = splice.anchor < before && splice.children > 0;
        let is = splice.anchor < now && splice.children > 0;
        if was != is || (position <= splice.anchor && position + removed > splice.anchor) {
            // The anchor's own row, or whether it is there at all, moved: the rest is told as one
            // change over everything until the controller says where the thread went.
            let drawn_before = before + if was { splice.children } else { 0 };
            let drawn_now = now + if is { splice.children } else { 0 };
            self.items_changed(0, drawn_before, drawn_now);
        } else if is && position > splice.anchor {
            self.items_changed(position + splice.children, removed, added);
        } else {
            if position + removed <= splice.anchor {
                // Rows above the thread's (new mail at the top): its row moves, and its children go
                // with it. Left where it was, the anchor would hang them from another row and the
                // view would keep the old ones too.
                let anchor = splice.anchor - removed + added;
                self.imp().splice.set(Some(Splice { anchor, ..splice }));
                if is && let Some(row) = self.rows().item(anchor).and_downcast::<RowObject>() {
                    self.mark_anchor(Some(row));
                }
            }
            self.items_changed(position, removed, added);
        }
    }

    pub fn splice(&self, anchor: u32, children: Vec<ChildRowView>) {
        let count = children.len() as u32;
        if let Some(open) = self.imp().splice.get()
            && open.anchor == anchor
            && open.children == count
        {
            let held = self.imp().children.borrow().clone();
            let last = children.len().saturating_sub(1);
            for (index, (object, view)) in held.iter().zip(children).enumerate() {
                object.set_child(view, index == last);
            }
            return;
        }
        self.unsplice();
        let last = children.len().saturating_sub(1);
        let objects: Vec<RowObject> = children
            .into_iter()
            .enumerate()
            .map(|(index, view)| RowObject::child(view, index == last))
            .collect();
        let rows = self.rows();
        let shown = anchor < rows.n_items() && count > 0;
        self.imp().children.replace(objects);
        self.imp().splice.set(Some(Splice {
            anchor,
            children: count,
        }));
        if shown {
            self.mark_anchor(rows.item(anchor).and_downcast::<RowObject>());
            self.items_changed(anchor + 1, 0, count);
        }
    }

    pub fn unsplice(&self) {
        let Some(open) = self.imp().splice.take() else {
            return;
        };
        let shown = open.anchor < self.rows().n_items() && open.children > 0;
        self.imp().children.borrow_mut().clear();
        self.mark_anchor(None);
        if shown {
            self.items_changed(open.anchor + 1, open.children, 0);
        }
    }

    fn mark_anchor(&self, anchor: Option<RowObject>) {
        let before = self.imp().anchor.replace(anchor.clone());
        if let Some(before) = before {
            before.set_expanded(false);
        }
        if let Some(anchor) = anchor {
            anchor.set_expanded(true);
        }
    }
}
