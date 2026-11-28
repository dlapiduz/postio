use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_model::ids::MessageId;
use postio_widgets::list_model::{CACHE_PAGES, ModelRow, PAGE_SIZE, PageSource, WindowedModel};

use crate::list_model_generic::{Note, NoteList as MessageList, NoteRow as MessageRow};

const HUGE: u32 = 100_000;

struct Fake {
    total: u32,
    requested: RefCell<Vec<u32>>,
}

impl Fake {
    fn new(total: u32) -> Rc<Self> {
        Rc::new(Fake {
            total,
            requested: RefCell::new(Vec::new()),
        })
    }

    fn drain(&self) -> Vec<u32> {
        self.requested.borrow_mut().drain(..).collect()
    }

    fn outstanding(&self) -> usize {
        self.requested.borrow().len()
    }
}

impl PageSource for Fake {
    fn total(&self) -> u32 {
        self.total
    }

    fn request(&self, page: u32) {
        self.requested.borrow_mut().push(page);
    }
}

fn row(position: u32) -> Note {
    Note::numbered(position as i64 + 1, &format!("message {position}"))
}

fn page_rows(page: u32, total: u32) -> Vec<Note> {
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    (start..end).map(row).collect()
}

fn settle(model: &MessageList, source: &Fake) {
    for page in source.drain() {
        model.deliver(page, page_rows(page, source.total()));
    }
}

fn new_list() -> MessageList {
    glib::Object::new()
}

fn item(model: &MessageList, position: u32) -> Option<MessageRow> {
    model.item(position).and_then(|o| o.downcast().ok())
}

/// Since #1216 the model announces structure and a row announces its own contents; a page landing
/// on existing positions is the latter, and saying it as the former costs `GtkListView` a rebuilt
/// widget per row.
fn announcements(row: &MessageRow) -> Rc<std::cell::Cell<u32>> {
    let count = Rc::new(std::cell::Cell::new(0));
    row.connect_changed({
        let count = count.clone();
        move |_| count.set(count.get() + 1)
    });
    count
}

fn watch(model: &MessageList) -> Rc<RefCell<Vec<(u32, u32, u32)>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed({
        let log = log.clone();
        move |_, position, removed, added| log.borrow_mut().push((position, removed, added))
    });
    log
}

pub fn an_empty_list_has_nothing_in_it() {
    let model = new_list();
    assert_eq!(model.n_items(), 0);
    assert!(model.item(0).is_none());
    assert_eq!(model.resident_rows(), 0);
}

pub fn pointing_at_a_folder_costs_nothing_until_a_row_is_read() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    assert_eq!(model.n_items(), HUGE, "the list knows how long it is");
    assert_eq!(
        source.outstanding(),
        0,
        "opening a 100k folder must not read a single row"
    );
    assert_eq!(model.resident_rows(), 0);
}

pub fn a_row_that_is_not_here_yet_is_a_placeholder_and_a_request() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    let first = item(&model, 0).expect("position 0 is in range");
    assert!(!first.is_loaded(), "it has to draw something meanwhile");
    assert_eq!(first.contents(), None);
    assert_eq!(
        source.drain(),
        [0, 1],
        "the page it needs, and the one it is about to need"
    );

    let log = watch(&model);
    let announced = announcements(&first);
    model.deliver(0, page_rows(0, HUGE));

    assert!(
        first.is_loaded(),
        "the object the view is holding is not the one the page filled in"
    );
    assert_eq!(first.contents().unwrap().text, "message 0");
    assert_eq!(
        item(&model, 0).as_ref(),
        Some(&first),
        "position 0 started answering with a different object"
    );
    assert_eq!(
        announced.get(),
        1,
        "and it has to say so, or nothing redraws"
    );
    assert_eq!(
        *log.borrow(),
        [],
        "the positions did not move, so the model has nothing structural to \
         say -- saying it anyway costs a rebuilt widget per row in range"
    );
}

pub fn a_page_is_never_asked_for_twice() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    item(&model, 0);
    assert_eq!(source.drain(), [0, 1]);

    // Still outstanding: asking again would be a second query for the same rows.
    for position in 0..PAGE_SIZE {
        item(&model, position);
    }
    assert!(source.drain().is_empty(), "no page was asked for twice");

    model.deliver(0, page_rows(0, HUGE));
    model.deliver(1, page_rows(1, HUGE));
    item(&model, 10);
    assert!(source.drain().is_empty());
}

pub fn scrolling_a_hundred_thousand_messages_keeps_memory_flat() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    let ceiling = CACHE_PAGES * PAGE_SIZE as usize;
    let mut high_water = 0;

    let mut position = 0;
    while position < HUGE {
        for offset in 0..15 {
            item(&model, (position + offset).min(HUGE - 1));
        }
        settle(&model, &source);
        high_water = high_water.max(model.resident_rows());
        assert!(
            model.resident_rows() <= ceiling,
            "at position {position} the model held {} rows, over the {ceiling} \
             the cache is allowed",
            model.resident_rows()
        );
        position += 500;
    }

    assert!(
        high_water > PAGE_SIZE as usize,
        "the test should have exercised the cache, not just one page"
    );
    assert_eq!(
        model.n_items(),
        HUGE,
        "and the list is still as long as it was"
    );
}

pub fn the_pages_that_go_are_the_ones_nobody_is_looking_at() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    for page in 0..(CACHE_PAGES as u32 * 3) {
        item(&model, page * PAGE_SIZE);
        settle(&model, &source);
    }

    let resident = model.resident_pages();
    assert!(
        resident.len() <= CACHE_PAGES,
        "{} pages resident, cache holds {CACHE_PAGES}",
        resident.len()
    );
    assert!(
        !resident.contains(&0),
        "the first page is the least recently used and should be long gone"
    );
    assert!(
        resident.contains(&(CACHE_PAGES as u32 * 3 - 1)),
        "the page just read must still be there: {resident:?}"
    );
}

pub fn a_redelivered_page_keeps_the_row_objects_it_already_had() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    item(&model, 0);
    model.deliver(0, page_rows(0, HUGE));
    let before = item(&model, 3).unwrap();

    model.deliver(0, page_rows(0, HUGE));
    let after = item(&model, 3).unwrap();

    assert_eq!(
        before, after,
        "same message, same GObject — anything holding the row keeps working"
    );
}

pub fn a_flag_change_touches_one_row_and_nothing_else() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());

    item(&model, 0);
    model.deliver(0, page_rows(0, HUGE));
    let before = item(&model, 7).unwrap();

    let log = watch(&model);
    let announced = announcements(&before);
    let changed = Note::numbered(8, "message 7, flagged");
    assert!(model.update_row(changed), "row 7 is resident");

    assert_eq!(
        announced.get(),
        1,
        "one row changed, so one row announced it"
    );
    assert_eq!(
        *log.borrow(),
        [],
        "a flag is not a structural change — not a reload, and not a \
         one-row replacement either, which `GtkListView` answers by \
         rebuilding that row's widget"
    );
    let after = item(&model, 7).unwrap();
    assert_eq!(before, after, "and it is the same object it always was");
    assert_eq!(after.contents().unwrap().text, "message 7, flagged");
}

pub fn a_message_that_is_not_on_screen_needs_no_update() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());
    item(&model, 0);
    model.deliver(0, page_rows(0, HUGE));

    let log = watch(&model);
    assert!(
        !model.update_row(row(90_000)),
        "its page is not resident, so there is nothing to update"
    );
    assert!(
        log.borrow().is_empty(),
        "and nothing to announce: the page will be read fresh when it is needed"
    );
}

pub fn new_mail_arrives_as_an_insertion_at_the_top() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());
    item(&model, 0);
    settle(&model, &source);
    assert!(model.resident_rows() > 0);

    let log = watch(&model);
    model.inserted_at_top(3);

    assert_eq!(model.n_items(), HUGE + 3);
    assert_eq!(
        *log.borrow(),
        [(0, 0, 3)],
        "an insertion, not a reset: a selection model moves the selection \
         down with the row it is on, and the view keeps its scroll anchor"
    );
    assert_eq!(
        model.resident_rows(),
        0,
        "every cached page is now misaligned against its positions"
    );
}

/// A second refresh asked while the top read is out may predate that read, so the top is read again
/// once it lands (#2152).
pub fn a_refresh_asked_while_the_top_is_on_its_way_reads_it_again() {
    let source = Fake::new(0);
    let model = new_list();
    model.set_source(source.clone());
    assert!(source.drain().is_empty(), "an empty list reads nothing");

    model.refresh();
    assert_eq!(source.drain(), [0]);
    model.note_pending(0);
    model.refresh();
    model.deliver(0, Vec::new());
    assert_eq!(
        source.drain(),
        [0],
        "the second change was folded into a read that may not have seen it"
    );
}

pub fn nothing_happens_when_no_mail_arrives() {
    let source = Fake::new(HUGE);
    let model = new_list();
    model.set_source(source.clone());
    item(&model, 0);
    settle(&model, &source);

    let resident = model.resident_rows();
    let log = watch(&model);
    model.inserted_at_top(0);

    assert!(log.borrow().is_empty());
    assert_eq!(
        model.resident_rows(),
        resident,
        "and nothing was thrown away"
    );
}

pub fn a_shrinking_folder_drops_the_rows_that_went() {
    let source = Fake::new(500);
    let model = new_list();
    model.set_source(source.clone());
    for page in 0..10 {
        item(&model, page * PAGE_SIZE);
        settle(&model, &source);
    }
    assert!(model.resident_pages().contains(&7));

    let log = watch(&model);
    model.set_total(120);

    assert_eq!(model.n_items(), 120);
    assert_eq!(*log.borrow(), [(120, 380, 0)]);
    assert!(
        model.resident_pages().iter().all(|page| *page < 3),
        "pages wholly past the end are stale: {:?}",
        model.resident_pages()
    );
    assert!(
        model.item(120).is_none(),
        "and there is nothing past the end"
    );
}

pub fn switching_folders_forgets_the_one_before() {
    let inbox = Fake::new(HUGE);
    let model = new_list();
    model.set_source(inbox.clone());
    item(&model, 0);
    settle(&model, &inbox);
    assert!(model.resident_rows() > 0);

    let log = watch(&model);
    let archive = Fake::new(40);
    model.set_source(archive.clone());

    assert_eq!(model.n_items(), 40);
    assert_eq!(
        *log.borrow(),
        [(0, HUGE, 40)],
        "the whole list was replaced, because it answers a different question"
    );
    assert_eq!(
        model.resident_rows(),
        0,
        "and none of the old folder's rows are still held"
    );
}

pub fn a_reordered_list_is_asked_for_again_from_the_top() {
    let source = Fake::new(500);
    let model = new_list();
    model.set_source(source.clone());
    item(&model, 0);
    settle(&model, &source);

    let log = watch(&model);
    model.invalidate();

    assert_eq!(model.n_items(), 500, "the folder is the same length");
    assert_eq!(*log.borrow(), [(0, 500, 500)]);
    assert_eq!(model.resident_rows(), 0);
}

pub fn there_is_nothing_past_the_end() {
    let source = Fake::new(120);
    let model = new_list();
    model.set_source(source.clone());

    assert!(model.item(119).is_some());
    assert!(model.item(120).is_none());
    assert!(model.item(u32::MAX).is_none());

    item(&model, 119);
    assert_eq!(
        source.drain(),
        [2, 1],
        "the last page and the one before it, and no page 3"
    );
}

pub fn the_model_can_say_which_page_holds_a_message() {
    let source = Fake::new(HUGE);
    let list = new_list();
    list.set_source(source.clone());

    item(&list, 0);
    item(&list, PAGE_SIZE * 4);
    source.drain();
    list.deliver(0, (0..PAGE_SIZE).map(row).collect());
    list.deliver(4, (PAGE_SIZE * 4..PAGE_SIZE * 5).map(row).collect());

    assert_eq!(list.page_of(MessageId::new(1)), Some(0));
    assert_eq!(
        list.page_of(MessageId::new(PAGE_SIZE as i64 * 4 + 1)),
        Some(4)
    );
    assert_eq!(
        list.page_of(MessageId::new(PAGE_SIZE as i64 * 2 + 1)),
        None,
        "a message whose page is not resident has no page to refetch"
    );
}

pub fn the_model_can_say_where_a_resident_message_sits() {
    let source = Fake::new(HUGE);
    let list = new_list();
    list.set_source(source.clone());

    item(&list, 0);
    item(&list, PAGE_SIZE * 4);
    source.drain();
    list.deliver(0, (0..PAGE_SIZE).map(row).collect());
    list.deliver(4, (PAGE_SIZE * 4..PAGE_SIZE * 5).map(row).collect());

    assert_eq!(list.position_of(MessageId::new(1)), Some(0));
    assert_eq!(
        list.position_of(MessageId::new(7)),
        Some(6),
        "position, not just page — the row's own offset within it"
    );
    assert_eq!(
        list.position_of(MessageId::new(PAGE_SIZE as i64 * 4 + 1)),
        Some(PAGE_SIZE * 4)
    );
    assert_eq!(
        list.position_of(MessageId::new(PAGE_SIZE as i64 * 2 + 1)),
        None,
        "a message whose page is not resident has no position to give"
    );
}

/// A source that answers inside `request`, which the contract forbids; it used to take the process
/// down.
struct Impatient {
    total: u32,
    list: RefCell<Option<MessageList>>,
}

impl PageSource for Impatient {
    fn total(&self) -> u32 {
        self.total
    }

    fn request(&self, page: u32) {
        let Some(list) = self.list.borrow().clone() else {
            return;
        };
        let start = page * PAGE_SIZE;
        let end = (start + PAGE_SIZE).min(self.total);
        list.deliver(page, (start..end).map(row).collect());
    }
}

pub fn a_source_that_answers_too_soon_is_held_until_it_is_safe() {
    // A delivery inside the model's `item()` would emit `items_changed` mid-read, and GtkListView
    // segfaults on that; the model holds it until the read is over.
    let source = Rc::new(Impatient {
        total: 120,
        list: RefCell::new(None),
    });
    let list = new_list();
    *source.list.borrow_mut() = Some(list.clone());
    list.set_source(source.clone());

    let reading = Rc::new(std::cell::Cell::new(false));
    let during = Rc::new(std::cell::Cell::new(false));
    list.connect_items_changed({
        let reading = reading.clone();
        let during = during.clone();
        move |_, _, _, _| {
            if reading.get() {
                during.set(true);
            }
        }
    });

    reading.set(true);
    let first = list.item(0).and_downcast::<MessageRow>().unwrap();
    reading.set(false);

    assert!(
        !during.get(),
        "the model told the world its rows changed while it was handing one out"
    );
    assert!(
        !first.is_loaded(),
        "a position answered with data the view had not been told about yet"
    );

    for _ in 0..8 {
        while glib::MainContext::default().iteration(false) {}
    }
    assert!(
        list.item(0)
            .and_downcast::<MessageRow>()
            .is_some_and(|item| item.is_loaded()),
        "an impatient source's rows never arrived at all"
    );
    assert_eq!(
        list.item(3)
            .and_downcast::<MessageRow>()
            .and_then(|item| item.contents())
            .map(|note| note.text),
        Some("message 3".to_string())
    );
}

/// GDK paints at `GDK_PRIORITY_REDRAW`, `G_PRIORITY_HIGH_IDLE + 20`.
const GDK_PRIORITY_REDRAW: i32 = 120;

pub fn a_held_delivery_is_not_postponed_by_a_window_that_is_repainting() {
    // A held change at `G_PRIORITY_DEFAULT_IDLE` (200) never runs while a window with something to
    // paint is ready at 120 (#1015).
    // A source always ready at the redraw priority stands in for a window drawing flat out.
    let source = Rc::new(Impatient {
        total: 120,
        list: RefCell::new(None),
    });
    let list = new_list();
    *source.list.borrow_mut() = Some(list.clone());
    list.set_source(source.clone());

    assert!(
        !list
            .item(0)
            .and_downcast::<MessageRow>()
            .expect("a position inside the folder answers")
            .is_loaded(),
        "the delivery was applied during the read instead of being held"
    );

    let painting = glib::idle_add_local_full(glib::Priority::from(GDK_PRIORITY_REDRAW), || {
        glib::ControlFlow::Continue
    });
    // Generous: at 200 the change never runs however long this goes on, and above the redraw it
    // runs on the first turn.
    const TURNS: usize = 2_000;
    for _ in 0..TURNS {
        glib::MainContext::default().iteration(false);
    }
    painting.remove();

    assert!(
        list.item(0)
            .and_downcast::<MessageRow>()
            .is_some_and(|item| item.is_loaded()),
        "{TURNS} turns of the main loop and the held delivery never ran: a \
         window that is repainting postponed it indefinitely"
    );
}

/// A search with no hits has no page 0; a list that was empty stayed "loading" for good and the
/// list-state pane showed nothing.
pub fn giving_up_a_first_page_there_is_none_of_ends_the_wait() {
    let model = new_list();
    let filled = Rc::new(std::cell::Cell::new(0));
    model.connect_local("filled", false, {
        let filled = filled.clone();
        move |_| {
            filled.set(filled.get() + 1);
            None
        }
    });

    let nothing = Fake::new(0);
    model.set_source(nothing.clone());
    assert!(model.is_loading(), "a new source is waited for");

    model.give_up(model.generation(), 0);
    assert!(
        !model.is_loading(),
        "a source with no first page to give is still waited for"
    );
    assert_eq!(
        filled.get(),
        1,
        "whoever draws the empty state was never told the wait was over"
    );
}
