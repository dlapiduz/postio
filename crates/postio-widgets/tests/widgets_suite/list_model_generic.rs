//! The windowed list model is generic over its row (specs/007-postio-focus
//! T021): a list of rows that are not the classic app's is windowed, filled in
//! place and refreshed by the same code, through a `GObject` of its own that
//! implements `WindowedModel` in two lines -- the shape Focus's list takes.
//!
//! What is asserted is what a `GtkListView` over it would see: the item each
//! position answers with, and whether it is the same object as before.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::list::ListRow;
use postio_widgets::list_model::{ModelRow, PAGE_SIZE, PageSource, Windowed, WindowedModel};

/// A row that is not the classic app's: a note with an id.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub(crate) id: MessageId,
    pub(crate) text: String,
}

impl Note {
    /// A note with an id and a text.
    pub(crate) fn numbered(id: i64, text: &str) -> Self {
        Note {
            id: MessageId::new(id),
            text: text.to_owned(),
        }
    }
}

mod note_imp {
    use super::*;

    #[derive(Default)]
    pub struct NoteRow {
        pub note: RefCell<Option<Note>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteRow {
        const NAME: &'static str = "PostioTestNoteRow";
        type Type = super::NoteRow;
    }

    impl ObjectImpl for NoteRow {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("changed").build()])
        }
    }

    #[derive(Default)]
    pub struct NoteList {
        pub core: Windowed<super::NoteRow>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for NoteList {
        const NAME: &'static str = "PostioTestNoteList";
        type Type = super::NoteList;
        type Interfaces = (gio::ListModel,);
    }

    impl ObjectImpl for NoteList {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> =
                std::sync::OnceLock::new();
            SIGNALS.get_or_init(postio_widgets::list_model::signals)
        }
    }

    impl ListModelImpl for NoteList {
        fn item_type(&self) -> glib::Type {
            super::NoteRow::static_type()
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
    pub struct NoteRow(ObjectSubclass<note_imp::NoteRow>);
}

glib::wrapper! {
    pub struct NoteList(ObjectSubclass<note_imp::NoteList>) @implements gio::ListModel;
}

impl NoteRow {
    /// Call `on_change` when the row's contents changed.
    pub(crate) fn connect_changed(
        &self,
        on_change: impl Fn(&Self) + 'static,
    ) -> glib::SignalHandlerId {
        self.connect_local("changed", false, move |values| {
            if let Some(row) = values.first().and_then(|value| value.get::<Self>().ok()) {
                on_change(&row);
            }
            None
        })
    }
}

impl ListRow for NoteRow {
    fn thread(&self) -> Option<ThreadId> {
        None
    }

    fn id(&self) -> Option<MessageId> {
        self.imp().note.borrow().as_ref().map(|note| note.id)
    }

    fn reconcile(existing: &Self, incoming: Self) -> Self {
        if let Some(note) = incoming.contents() {
            existing.fill(note);
        }
        existing.clone()
    }
}

impl ModelRow for NoteRow {
    type Data = Note;

    fn placeholder() -> Self {
        glib::Object::new()
    }

    fn with_contents(data: Note) -> Self {
        let row = Self::placeholder();
        row.fill(data);
        row
    }

    fn contents(&self) -> Option<Note> {
        self.imp().note.borrow().clone()
    }

    fn fill(&self, data: Note) {
        let before = self.imp().note.replace(Some(data.clone()));
        if before.as_ref() != Some(&data) {
            self.emit_by_name::<()>("changed", &[]);
        }
    }

    fn id_of(data: &Note) -> MessageId {
        data.id
    }
}

impl WindowedModel for NoteList {
    type Row = NoteRow;

    fn windowed(&self) -> &Windowed<NoteRow> {
        &self.imp().core
    }
}

/// A source of `total` notes that records what it was asked for and answers
/// only when told to.
struct Notes {
    total: u32,
    asked: RefCell<Vec<u32>>,
}

impl PageSource for Notes {
    fn total(&self) -> u32 {
        self.total
    }

    fn request(&self, page: u32) {
        self.asked.borrow_mut().push(page);
    }
}

fn note(id: i64, text: &str) -> Note {
    Note {
        id: MessageId::new(id),
        text: text.to_owned(),
    }
}

fn page_of_notes(first: i64, count: u32) -> Vec<Note> {
    (0..i64::from(count))
        .map(|n| note(first + n, &format!("note {}", first + n)))
        .collect()
}

fn text_at(list: &NoteList, position: u32) -> Option<String> {
    list.item(position)
        .and_downcast::<NoteRow>()
        .and_then(|row| row.contents())
        .map(|note| note.text)
}

pub fn a_list_of_another_row_type_is_windowed_filled_and_refreshed() {
    let source = Rc::new(Notes {
        total: PAGE_SIZE + 5,
        asked: RefCell::default(),
    });
    let list: NoteList = glib::Object::new();
    list.set_source(source.clone());
    assert_eq!(list.n_items(), PAGE_SIZE + 5, "the source's count");

    // Drawing position 0 asks for its page, and answers with a placeholder
    // the delivery then fills in place.
    let held = list.item(0).and_downcast::<NoteRow>().expect("a row");
    assert!(!held.is_loaded(), "nothing has arrived yet");
    assert!(
        source.asked.borrow().contains(&0),
        "page 0 was not asked for"
    );
    list.deliver(0, page_of_notes(1, PAGE_SIZE));
    assert_eq!(
        held.contents().map(|n| n.text).as_deref(),
        Some("note 1"),
        "the object the view holds was not filled in place"
    );
    assert_eq!(
        list.item(0).and_downcast::<NoteRow>(),
        Some(held.clone()),
        "position 0 answers with a different object after its page landed"
    );
    assert_eq!(list.position_of(MessageId::new(3)), Some(2));

    // A note changed in place keeps its object.
    assert!(list.update_row(note(1, "note 1, edited")));
    assert_eq!(text_at(&list, 0).as_deref(), Some("note 1, edited"));

    // A refresh that finds a note gone splices it out where it stood.
    list.refresh();
    let mut fresh = page_of_notes(1, PAGE_SIZE);
    fresh.remove(1);
    list.deliver(0, fresh);
    assert_eq!(
        text_at(&list, 1).as_deref(),
        Some("note 3"),
        "the refresh did not take the missing note out"
    );
    assert_eq!(
        list.item(0).and_downcast::<NoteRow>(),
        Some(held),
        "the note that stayed was rebuilt rather than kept"
    );
}
