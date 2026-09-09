//! The conversation rail (#1374): the column that says where you are in a
//! thread.
//!
//! What needs a display is what a person would see — that there is a row per
//! message, that the row carries its number and sender, that a length appears
//! only where one is worth saying, and that the marked row is drawn as marked
//! rather than merely recorded as marked. The rule that decides *which* row is
//! marked is arithmetic and lives in `postio_ui::reader::rail`, where #1359
//! and #1372 prove it without a display.
//!
//! Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::conversation::ConversationView;
use postio_gtk::list::Row as ListRow;
use postio_gtk::reader::Reader;
use postio_gtk::reader::rail::{FULL_WIDTH, NARROW_WIDTH, RailColumn};
use postio_gtk::{fonts, style};
use postio_model::EmailAddress;
use postio_model::ids::{MessageId, ThreadId};
use postio_ui::reader::rail::{LENGTH_THRESHOLD, rows};

/// A realized rail, or `None` with no display.
fn rail() -> Option<(gtk::Window, RailColumn)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let rail = RailColumn::new();
    let window = gtk::Window::new();
    window.set_child(Some(rail.widget()));
    window.present();
    Some((window, rail))
}

/// Every label in the tree, in order, so a case can assert on what is on
/// screen rather than on what the widget was handed.
fn labels(root: &gtk::Widget) -> Vec<String> {
    of_class(root, None)
}

/// The labels carrying `class`, in order.
///
/// By class rather than by text, because a row number and a line count are
/// both small integers: the first version of this file asserted that no label
/// read `"4"` to prove a four-line message shows no length, and message *four*
/// made it fail. Position and length are different things that happen to look
/// alike, and only the class says which is which.
fn of_class(root: &gtk::Widget, class: Option<&str>) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = root.first_child();
    while let Some(child) = next {
        if let Some(label) = child.downcast_ref::<gtk::Label>()
            && class.is_none_or(|class| label.has_css_class(class))
        {
            found.push(label.label().to_string());
        }
        found.extend(of_class(&child, class));
        next = child.next_sibling();
    }
    found
}

pub fn the_rail_lists_a_thread_and_marks_what_is_on_screen() {
    let Some((window, rail)) = rail() else {
        return;
    };

    let senders: Vec<String> = ["Tessa Vaughn", "me", "Tessa Vaughn", "me"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    // The third message is the essay; the rest are short, and the last has no
    // body yet -- three different reasons for a row to carry no number.
    let lengths = [Some(4), Some(LENGTH_THRESHOLD - 1), Some(84), None];
    let initials = vec![
        "TV".to_owned(),
        "ME".to_owned(),
        "TV".to_owned(),
        "ME".to_owned(),
    ];
    rail.show_thread(&rows(&senders, &initials, &lengths));
    rail.set_marked(Some(2));

    let seen = labels(rail.widget());
    let joined = seen.join(" | ");

    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-number")),
        vec!["1", "2", "3", "4"],
        "a row per message, numbered as a person counts: {joined}"
    );
    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-sender")),
        senders,
        "every row says who wrote it: {joined}"
    );
    assert_eq!(
        of_class(rail.widget(), Some("postio-rail-length")),
        vec!["84"],
        "only the essay is long enough to be worth a number -- the four-line \
         message, the one just under the threshold, and the one whose body \
         has not arrived all show nothing: {joined}"
    );
    assert!(
        seen.iter().any(|text| text.contains("In this thread")),
        "the rail names itself: {joined}"
    );
    assert!(
        seen.iter().any(|text| text.contains("3 of 4")),
        "the footer states the position: {joined}"
    );

    assert_eq!(
        rail.marked_position(),
        Some(3),
        "the marked row is the one the rule chose, counted as a person counts"
    );
    assert!(
        rail.row_is_marked(2),
        "the marked row must be drawn as marked, not merely recorded"
    );
    assert!(
        !rail.row_is_marked(0),
        "exactly one row is marked at a time"
    );

    window.close();
}

pub fn activating_a_row_reports_the_message_it_names() {
    let Some((window, rail)) = rail() else {
        return;
    };

    rail.show_thread(&rows(
        &["Ada".to_owned(), "Grace".to_owned(), "Katherine".to_owned()],
        &["AD".to_owned(), "GR".to_owned(), "KA".to_owned()],
        &[None, None, None],
    ));

    let seen = Rc::new(RefCell::new(Vec::new()));
    rail.connect_activated({
        let seen = Rc::clone(&seen);
        move |index| seen.borrow_mut().push(index)
    });

    rail.activate_row(1);

    assert_eq!(
        seen.borrow().as_slice(),
        &[1],
        "activating a row reports the message it names, once"
    );

    window.close();
}

/// One message of a conversation.
fn message(id: i64) -> ListRow {
    ListRow {
        id: MessageId::new(id),
        thread: Some(ThreadId::new(1)),
        from: Some(EmailAddress::new(Some("Ada Norwood"), "ada@example.com")),
        subject: Some("Tide gate interlock".to_owned()),
        preview: Some("Snippet".to_owned()),
        received_at: chrono::Utc::now() - chrono::Duration::minutes(100 - id),
        seen: true,
        flagged: false,
        answered: false,
        draft: false,
        has_attachments: false,
        thread_count: 6,
        participants: Vec::new(),
    }
}

/// A pane in a window, with a reader factory that builds nothing worth
/// naming — no case here expands anything.
fn pane() -> Option<(gtk::Window, ConversationView)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let pane = ConversationView::new();
    pane.set_reader_factory(|_message| Some(Reader::new(Rc::new(|_content_id: &str| None))));
    let window = gtk::Window::new();
    window.set_child(Some(&pane));
    window.present();
    Some((window, pane))
}

/// The labels of `class` that are actually visible.
fn shown(root: &gtk::Widget, class: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut next = root.first_child();
    while let Some(child) = next {
        if let Some(label) = child.downcast_ref::<gtk::Label>()
            && label.has_css_class(class)
            && label.is_visible()
        {
            found.push(label.label().to_string());
        }
        found.extend(shown(&child, class));
        next = child.next_sibling();
    }
    found
}

pub fn the_rail_takes_its_step_on_the_ladder() {
    let Some((window, pane)) = pane() else {
        return;
    };

    pane.open((1..=6).map(message).collect());
    let rail = pane.rail().widget();

    pane.set_window_width(1400);
    assert!(rail.is_visible(), "a wide window shows the rail in full");
    assert_eq!(
        rail.width_request(),
        FULL_WIDTH,
        "the full step is the brief's 150px column"
    );
    assert!(
        !rail.has_css_class("postio-rail-narrow"),
        "a wide window is not the narrow step"
    );
    assert_eq!(
        shown(rail, "postio-rail-sender").len(),
        6,
        "the full step shows names"
    );
    assert!(
        shown(rail, "postio-rail-initials").is_empty(),
        "and not initials as well -- one or the other, never both"
    );

    pane.set_window_width(1150);
    assert!(rail.is_visible(), "the middle step still draws a column");
    assert_eq!(
        rail.width_request(),
        NARROW_WIDTH,
        "the middle step narrows rather than unmounting"
    );
    assert!(
        rail.has_css_class("postio-rail-narrow"),
        "the narrow step drops the senders, which is a class not a rebuild"
    );
    assert!(
        shown(rail, "postio-rail-sender").is_empty(),
        "no names at 118px -- a name cut to four characters is not a name"
    );
    assert_eq!(
        shown(rail, "postio-rail-initials").len(),
        6,
        "screen 29's middle step is numbers *and initials*, not numbers alone"
    );

    pane.set_window_width(1000);
    assert!(
        !rail.is_visible(),
        "below the floor the rail unmounts -- the body never narrows to fund it"
    );

    window.close();
}

pub fn hiding_the_rail_outlasts_the_conversation_and_the_width() {
    let Some((window, pane)) = pane() else {
        return;
    };

    pane.open((1..=6).map(message).collect());
    pane.set_window_width(1400);
    assert!(pane.rail().widget().is_visible());

    pane.toggle_rail();
    assert!(pane.rail_hidden(), "shift-R puts the rail away");
    assert!(!pane.rail().widget().is_visible());

    // FR-047: the choice belongs to the window. Neither widening it nor
    // opening another conversation is a request to undo it.
    pane.set_window_width(1600);
    assert!(
        !pane.rail().widget().is_visible(),
        "widening the window does not bring back a rail that was put away"
    );
    pane.open((1..=4).map(message).collect());
    assert!(
        !pane.rail().widget().is_visible(),
        "a new conversation does not bring it back either"
    );

    pane.toggle_rail();
    assert!(
        pane.rail().widget().is_visible(),
        "and shift-R brings it back"
    );

    window.close();
}

pub fn a_single_message_conversation_has_no_rail() {
    let Some((window, pane)) = pane() else {
        return;
    };

    // FR-045. Not at any width: there is nothing for an index to index, and
    // most of the mail a person reads is one message.
    pane.open(vec![message(1)]);
    for width in [1000, 1150, 1400, 1900] {
        pane.set_window_width(width);
        assert!(
            !pane.rail().widget().is_visible(),
            "one message needs no rail, and none is drawn at {width}px"
        );
    }

    window.close();
}
