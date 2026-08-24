//! The windowed message list, without a database, a display or a runtime.
//!
//! The model asks for pages through a [`PageSource`] and waits, so a fake
//! source that records what it was asked for is enough to check the whole of
//! it — including the two things that are otherwise expensive to be sure
//! about: that scrolling a huge folder keeps memory flat, and that a page is
//! never fetched twice.

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{TimeZone, Utc};
use gtk::prelude::*;
use postio_gtk::list::{CACHE_PAGES, MessageList, MessageRow, PAGE_SIZE, PageSource, Row};
use postio_model::ids::MessageId;

/// A folder big enough that loading it would be the bug.
const HUGE: u32 = 100_000;

/// Records what the model asked for, and answers only when told to.
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

    /// Every page asked for since the last drain, in order.
    fn drain(&self) -> Vec<u32> {
        self.requested.borrow_mut().drain(..).collect()
    }

    /// How many requests in total, across every drain.
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

/// A row whose id is its position, so a test can say where it came from.
fn row(position: u32) -> Row {
    Row {
        id: MessageId::new(position as i64 + 1),
        thread: None,
        from: None,
        subject: Some(format!("message {position}")),
        preview: None,
        received_at: Utc
            .timestamp_opt(1_700_000_000 - position as i64, 0)
            .unwrap(),
        seen: false,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 1,
    }
}

fn page_rows(page: u32, total: u32) -> Vec<Row> {
    let start = page * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    (start..end).map(row).collect()
}

/// Answer everything the fake has been asked for.
fn settle(model: &MessageList, source: &Fake) {
    for page in source.drain() {
        model.deliver(page, page_rows(page, source.total()));
    }
}

fn item(model: &MessageList, position: u32) -> Option<MessageRow> {
    model.item(position).and_then(|o| o.downcast().ok())
}

/// Every `items_changed` the model emitted, as `(position, removed, added)`.
fn watch(model: &MessageList) -> Rc<RefCell<Vec<(u32, u32, u32)>>> {
    let log = Rc::new(RefCell::new(Vec::new()));
    model.connect_items_changed({
        let log = log.clone();
        move |_, position, removed, added| log.borrow_mut().push((position, removed, added))
    });
    log
}

#[test]
fn an_empty_list_has_nothing_in_it() {
    let model = MessageList::new();
    assert_eq!(model.n_items(), 0);
    assert!(model.item(0).is_none());
    assert_eq!(model.resident_rows(), 0);
}

#[test]
fn pointing_at_a_folder_costs_nothing_until_a_row_is_read() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    assert_eq!(model.n_items(), HUGE, "the list knows how long it is");
    assert_eq!(
        source.outstanding(),
        0,
        "opening a 100k folder must not read a single row"
    );
    assert_eq!(model.resident_rows(), 0);
}

#[test]
fn a_row_that_is_not_here_yet_is_a_placeholder_and_a_request() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    let first = item(&model, 0).expect("position 0 is in range");
    assert!(!first.is_loaded(), "it has to draw something meanwhile");
    assert_eq!(first.row(), None);
    assert_eq!(
        source.drain(),
        [0, 1],
        "the page it needs, and the one it is about to need"
    );

    let log = watch(&model);
    model.deliver(0, page_rows(0, HUGE));

    let first = item(&model, 0).expect("position 0 is in range");
    assert!(first.is_loaded());
    assert_eq!(first.row().unwrap().subject.as_deref(), Some("message 0"));
    assert_eq!(
        *log.borrow(),
        [(0, PAGE_SIZE, PAGE_SIZE)],
        "the positions did not move; what they answer with changed"
    );
}

#[test]
fn a_page_is_never_asked_for_twice() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    item(&model, 0);
    assert_eq!(source.drain(), [0, 1]);

    // Still outstanding: asking again would be a second query for the same
    // rows, which is how a list ends up hammering SQLite while it scrolls.
    for position in 0..PAGE_SIZE {
        item(&model, position);
    }
    assert!(source.drain().is_empty(), "no page was asked for twice");

    // And once delivered, still not.
    model.deliver(0, page_rows(0, HUGE));
    model.deliver(1, page_rows(1, HUGE));
    item(&model, 10);
    assert!(source.drain().is_empty());
}

#[test]
fn scrolling_a_hundred_thousand_messages_keeps_memory_flat() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    let ceiling = CACHE_PAGES * PAGE_SIZE as usize;
    let mut high_water = 0;

    // A tenth of the way down the folder, a screenful at a time.
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

#[test]
fn the_pages_that_go_are_the_ones_nobody_is_looking_at() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    // Walk far enough to overflow the cache several times over.
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

#[test]
fn a_redelivered_page_keeps_the_row_objects_it_already_had() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
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

#[test]
fn a_flag_change_touches_one_row_and_nothing_else() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
    model.set_source(source.clone());

    item(&model, 0);
    model.deliver(0, page_rows(0, HUGE));
    let before = item(&model, 7).unwrap();

    let log = watch(&model);
    let mut changed = row(7);
    changed.seen = true;
    changed.flagged = true;
    assert!(model.update_row(changed), "row 7 is resident");

    assert_eq!(
        *log.borrow(),
        [(7, 1, 1)],
        "one row changed, so one row is announced — not a reload"
    );
    let after = item(&model, 7).unwrap();
    assert_eq!(before, after, "and it is the same object it always was");
    assert!(after.row().unwrap().seen);
    assert!(after.row().unwrap().flagged);
}

#[test]
fn a_message_that_is_not_on_screen_needs_no_update() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
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

#[test]
fn new_mail_arrives_as_an_insertion_at_the_top() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
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

#[test]
fn nothing_happens_when_no_mail_arrives() {
    let source = Fake::new(HUGE);
    let model = MessageList::new();
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

#[test]
fn a_shrinking_folder_drops_the_rows_that_went() {
    let source = Fake::new(500);
    let model = MessageList::new();
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

#[test]
fn switching_folders_forgets_the_one_before() {
    let inbox = Fake::new(HUGE);
    let model = MessageList::new();
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

#[test]
fn a_reordered_list_is_asked_for_again_from_the_top() {
    let source = Fake::new(500);
    let model = MessageList::new();
    model.set_source(source.clone());
    item(&model, 0);
    settle(&model, &source);

    let log = watch(&model);
    model.invalidate();

    assert_eq!(model.n_items(), 500, "the folder is the same length");
    assert_eq!(*log.borrow(), [(0, 500, 500)]);
    assert_eq!(model.resident_rows(), 0);
}

#[test]
fn there_is_nothing_past_the_end() {
    let source = Fake::new(120);
    let model = MessageList::new();
    model.set_source(source.clone());

    assert!(model.item(119).is_some());
    assert!(model.item(120).is_none());
    assert!(model.item(u32::MAX).is_none());

    // And the model does not ask for a page that lies wholly past the end.
    item(&model, 119);
    assert_eq!(
        source.drain(),
        [2, 1],
        "the last page and the one before it, and no page 3"
    );
}

#[test]
fn the_model_can_say_which_page_holds_a_message() {
    // What makes an in-place update cheap: a changed message costs a refetch
    // of the one page it is on, not of the folder.
    let source = Fake::new(HUGE);
    let list = MessageList::new();
    list.set_source(source.clone());

    list.item(0);
    list.item(PAGE_SIZE * 4);
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

#[test]
fn the_model_can_say_where_a_resident_message_sits() {
    // What a notification's click needs: not just which page a message is
    // on, but the exact position to put the cursor on.
    let source = Fake::new(HUGE);
    let list = MessageList::new();
    list.set_source(source.clone());

    list.item(0);
    list.item(PAGE_SIZE * 4);
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

/// A source that answers inside `request`, which the contract forbids and
/// which a real one cannot do — but a test double, a bench or a second view
/// written in a hurry can, and used to take the process down with it.
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

#[test]
fn a_source_that_answers_too_soon_is_held_until_it_is_safe() {
    // `request` is called from inside the model answering `item()`, so a
    // delivery made there would emit `items_changed` while a view is
    // mid-read. GtkListView does not survive that — it segfaults, with no
    // message and a long way from the mistake. So the model holds the
    // delivery until the read is over rather than trusting the contract.
    let source = Rc::new(Impatient {
        total: 120,
        list: RefCell::new(None),
    });
    let list = MessageList::new();
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

    // The held delivery lands on the next turn of the main loop, and the
    // rows are the rows that were asked for.
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
            .and_then(|item| item.row())
            .and_then(|row| row.subject),
        Some("message 3".to_string())
    );
}
