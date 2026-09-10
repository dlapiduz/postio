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
    let whens = vec![
        "22 Aug".to_owned(),
        "24 Aug".to_owned(),
        "24 Aug".to_owned(),
        "25 Aug".to_owned(),
    ];
    rail.show_thread(&rows(&senders, &initials, &whens, &lengths));
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
        &["1 Sep".to_owned(), "2 Sep".to_owned(), "3 Sep".to_owned()],
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
    assert!(pane.rail_hidden(), "shift-I puts the rail away");
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
        "and shift-I brings it back"
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

pub fn below_the_floor_the_header_carries_the_index() {
    let Some((window, pane)) = pane() else {
        return;
    };

    pane.open((1..=6).map(message).collect());
    pane.set_window_width(1400);
    let counter = pane.header().counter();
    assert!(
        !counter.is_visible(),
        "a window with room for the column does not also carry a counter -- \
         the rail already says the position"
    );

    let wide_meta = pane.header().meta();
    assert!(
        wide_meta.contains("Ada Norwood"),
        "a wide header names the participants: {wide_meta}"
    );

    pane.set_window_width(1000);
    assert!(
        counter.is_visible(),
        "below the floor the header takes over saying where you are"
    );
    // Screen 29's narrow header is `6 messages · 22-25 Aug` and nothing else.
    // With the names still in it the line ellipsised to a single letter once
    // the counter took the trailing edge, which says less than leaving them
    // out -- and the avatar chips still say who is here.
    let narrow_meta = pane.header().meta();
    assert!(
        !narrow_meta.contains("Ada Norwood"),
        "the names are the first thing to go when the header is short of \
         room: {narrow_meta}"
    );
    assert!(
        narrow_meta.contains("6 messages"),
        "but the count stays, because nothing else says it: {narrow_meta}"
    );
    // The names going was not enough on its own: with the chips and the
    // scoping note still there, the *dates* then ellipsised to one character.
    // Screen 29's narrow row is the count, the dates and the counter.
    assert!(
        !pane.header().scoping_visible(),
        "the scoping note stands down with the names"
    );
    assert!(
        !pane.header().participants_visible(),
        "and so do the avatar chips"
    );
    // Against the rail's own mark rather than a literal. The counter is the
    // rail in another shape, so what matters is that the two cannot say
    // different things -- and the pane opens on the newest message, not the
    // first, which is what the literal got wrong.
    let marked = pane
        .rail()
        .marked_position()
        .expect("opening a conversation marks the message it lands on");
    assert_eq!(
        counter.label().unwrap_or_default(),
        format!("{marked}/6"),
        "the counter and the rail must not be able to disagree"
    );

    // The same widget, moved -- not a second one built to look the same.
    // Two rails would be two marked rows, and the second would be wrong
    // exactly when someone scrolled with the index open.
    assert!(
        pane.rail()
            .widget()
            .ancestor(gtk::Popover::static_type())
            .is_some(),
        "at the popover step the index lives in the popover, not the pane"
    );
    assert_eq!(
        pane.header().index().child().map(|child| child.type_()),
        Some(pane.rail().widget().type_()),
        "and the popover's child is that rail"
    );

    // Back up the ladder, and it goes back to being a column.
    pane.set_window_width(1400);
    assert!(
        pane.rail()
            .widget()
            .ancestor(gtk::Popover::static_type())
            .is_none(),
        "widening puts the rail back beside the body"
    );
    assert!(pane.rail().widget().is_visible());
    assert!(
        !counter.is_visible(),
        "and the counter stands down again -- exactly one of the two speaks"
    );
    assert_eq!(
        pane.header().meta(),
        wide_meta,
        "and the participants come back with the room for them"
    );
    assert!(pane.header().scoping_visible());
    assert!(pane.header().participants_visible());

    window.close();
}

pub fn a_conversation_with_no_rail_has_no_counter_either() {
    let Some((window, pane)) = pane() else {
        return;
    };

    // FR-045: one message has no position worth stating, at any width.
    pane.open(vec![message(1)]);
    pane.set_window_width(1000);
    assert!(!pane.header().counter().is_visible());

    // FR-047: nor does a conversation whose rail was put away. The counter
    // is the rail in another shape, so hiding one hides the other -- a
    // counter that survived would be the index the reader just dismissed.
    pane.open((1..=6).map(message).collect());
    pane.toggle_rail();
    pane.set_window_width(1000);
    assert!(!pane.header().counter().is_visible());

    window.close();
}

pub fn opening_a_conversation_narrow_keeps_the_header_short() {
    let Some((window, pane)) = pane() else {
        return;
    };

    // The application's order, and the opposite of every other case in this
    // file: the ladder runs *inside* `open`, before the header has been given
    // its conversation. Setting the width afterwards -- which is what the
    // other cases do -- re-runs the ladder and hides the bug, and did: the
    // header put the long line back and nothing ran again to correct it.
    // Only a screenshot noticed.
    pane.set_window_width(1000);
    pane.open((1..=6).map(message).collect());

    let meta = pane.header().meta();
    assert!(
        !meta.contains("Ada Norwood"),
        "opening a conversation must not undo the ladder's step: {meta}"
    );
    assert!(
        pane.header().counter().is_visible(),
        "and the counter is drawn on open, not only on the next resize"
    );

    window.close();
}

pub fn a_conversation_opens_on_its_most_recent_message() {
    let Some((window, pane)) = pane() else {
        return;
    };
    pane.set_window_width(1400);

    // FR-015, and the maintainer's own words when the spec was clarified:
    // the pane opens on the last message of the thread, *whether or not*
    // earlier ones are unread. Landing on the oldest means the message you
    // were notified about is below eleven others.
    for one_document in [false, true] {
        pane.set_one_document(one_document);
        let mut messages: Vec<ListRow> = (1..=6).map(message).collect();
        // Two unread ones early in the thread, because "whether or not
        // earlier messages are unread" is the half of FR-015 that a
        // first-unread rule would satisfy the assertion by accident.
        messages[1].seen = false;
        messages[2].seen = false;
        pane.open(messages);
        crate::pump();

        let which = if one_document {
            "one document"
        } else {
            "stacked"
        };
        assert_eq!(
            pane.focused_index(),
            Some(5),
            "the {which} pane must open on the most recent message, not on \
             the first unread and not on the oldest"
        );
        assert_eq!(
            pane.rail().marked_position(),
            Some(6),
            "and the rail must agree with it, or the two say different \
             things about where you are in the {which} pane"
        );
    }

    // FR-015's second half: reopening does not restore where you stopped.
    // Moved with the keyboard rather than by reopening, because `J` is the
    // gesture the requirement is about -- and in the stacked pane, which is
    // where focus can currently be moved at all (see #1386).
    pane.set_one_document(false);
    pane.open((1..=6).map(message).collect());
    crate::pump();
    assert!(pane.focus_previous(), "K moves back from the newest");
    assert_eq!(pane.focused_index(), Some(4));

    pane.open((1..=6).map(message).collect());
    crate::pump();
    assert_eq!(
        pane.focused_index(),
        Some(5),
        "reopening a conversation lands on the most recent message again -- \
         the pane does not restore where the reader stopped"
    );

    window.close();
}

pub fn the_rail_moves_the_focus_in_the_one_document_pane() {
    let Some((window, pane)) = pane() else {
        return;
    };
    pane.set_window_width(1400);
    pane.set_one_document(true);
    pane.open((1..=6).map(message).collect());
    crate::pump();

    assert_eq!(pane.focused_index(), Some(5), "opens on the newest");

    // A rail row is the pointer's way to move, and it did nothing here: the
    // pane has no `Entry` per message -- the whole thread is one document --
    // so every route into focus returned early. The rail could be drawn,
    // marked and clicked in the one pane it was designed for, and clicking
    // it moved nothing.
    pane.rail().activate_row(1);
    crate::pump();
    assert_eq!(
        pane.focused_index(),
        Some(1),
        "activating a rail row moves the focus"
    );
    assert_eq!(
        pane.rail().marked_position(),
        Some(2),
        "and the mark follows it, through the one entry point"
    );

    // `J` and `K`, which reach the same place.
    assert!(pane.focus_next(), "J moves down");
    assert_eq!(pane.focused_index(), Some(2));
    assert!(pane.focus_previous(), "K moves back");
    assert_eq!(pane.focused_index(), Some(1));

    // The ends still stop it.
    pane.rail().activate_row(5);
    crate::pump();
    assert!(!pane.focus_next(), "there is nothing past the newest");
    assert_eq!(pane.focused_index(), Some(5));

    window.close();
}

pub fn the_one_document_pane_offers_nothing_to_expand() {
    let Some((window, pane)) = pane() else {
        return;
    };
    pane.set_window_width(1400);

    // The stacked pane still offers it: there, read messages are collapsed to
    // keep a thirty-message thread from opening thirty `WebView`s.
    pane.set_one_document(false);
    pane.open((1..=6).map(message).collect());
    crate::pump();
    assert!(
        pane.header().offers_expand_all(),
        "the stacked pane collapses read messages, so expanding them is a \
         thing a reader can want"
    );

    // FR-013: in the one-document pane there is nothing for the user to
    // expand in order to read the conversation, so a control that would do
    // nothing must not be drawn -- it says there is something unseen.
    pane.set_one_document(true);
    pane.open((1..=6).map(message).collect());
    crate::pump();
    assert!(
        !pane.header().offers_expand_all(),
        "every body is already open, so Expand all has nothing to do"
    );

    window.close();
}

pub fn marking_a_message_read_does_not_redraw_the_conversation() {
    let Some((window, pane)) = pane() else {
        return;
    };
    pane.set_window_width(1400);
    pane.set_one_document(true);

    let unread: Vec<ListRow> = (1..=3)
        .map(|id| {
            let mut row = message(id);
            row.seen = false;
            row
        })
        .collect();
    pane.open(unread.clone());
    crate::pump();

    // Every body first. The pane holds a redraw until the thread is whole or
    // the deadline passes, so a pane with no bodies has simply not drawn yet
    // -- and "it did not redraw" would be true of it for the wrong reason.
    // The control at the foot of this case is what caught that.
    for id in 1..=3 {
        pane.set_thread_body(
            MessageId::new(id),
            postio_model::MessageBody {
                text: Some(format!("the body of message {id}")),
                html: None,
            },
        );
    }
    crate::settle_until("the conversation drew its thread", || {
        pane.thread_renders() > 0
    });

    // A redraw here is not a repaint. It is a full WebKit teardown and reload
    // of the whole thread, a frame of unpainted view, and the scroll position
    // discarded -- #749's black flash, which is the complaint ADR 0032 came
    // from. Marking read happens on a *dwell*, so a pane that redrew for it
    // would flash whenever someone rested on a message long enough to have
    // read it, which is continuously.
    let before = pane.thread_renders();

    // What the application does when a dwell marks one read: the list reloads
    // and the pane is opened again with the same thread, one row's `seen`
    // flipped.
    let mut after_read = unread.clone();
    after_read[1].seen = true;
    pane.open(after_read);
    crate::pump();
    crate::settle();

    assert_eq!(
        pane.thread_renders(),
        before,
        "marking a message read tore the conversation down and rebuilt it"
    );

    // The control, and the reason the assertion above means anything: a
    // change that genuinely alters the document still renders. Without it
    // this case passed while the pane had never drawn at all.
    pane.set_thread_body(
        MessageId::new(2),
        postio_model::MessageBody {
            text: Some("a body that has just arrived, and is different".to_owned()),
            html: None,
        },
    );
    crate::settle_until("the arriving body redrew the thread", || {
        pane.thread_renders() > before
    });
    assert!(
        pane.thread_renders() > before,
        "a body arriving did not render either, so the assertion above was \
         satisfied by a pane that never draws"
    );

    window.close();
}

pub fn the_conversation_header_is_pinned_and_stays_two_rows() {
    let Some((window, pane)) = pane() else {
        return;
    };
    pane.set_window_width(1400);

    // A subject far longer than any pane, so "it fits" cannot be why the
    // header stays one line.
    let mut messages: Vec<ListRow> = (1..=6).map(message).collect();
    messages[0].subject = Some("Re: ".to_owned() + &"a very long subject ".repeat(40));
    pane.open(messages);
    crate::pump();

    let header = pane.header().widget();

    // ── pinned ───────────────────────────────────────────────────────────
    // Structurally true today -- the header is appended to the pane's root,
    // not to the scroller -- and that is exactly the kind of truth a refactor
    // removes without noticing. Moving it inside the scroller is a one-line
    // change that looks tidier and compiles. Then a long conversation scrolls
    // and the header goes with it, so the thread stops saying what it is and
    // the `latest · all 6` line that explains the verbs' scoping (FR-008a)
    // scrolls away from the verbs.
    assert!(
        header
            .ancestor(gtk::ScrolledWindow::static_type())
            .is_none(),
        "the header is inside a scroller, so it scrolls away from the \
         conversation it names"
    );

    // ── two rows, whatever the subject ───────────────────────────────────
    let rows = {
        let mut count = 0;
        let mut child = header.first_child();
        while let Some(row) = child {
            count += 1;
            child = row.next_sibling();
        }
        count
    };
    assert_eq!(
        rows, 2,
        "screen 30's header is two rows: the subject and its verbs, then the \
         conversation's own line"
    );

    // ── and the subject truncates rather than wrapping ───────────────────
    // The label, not the height: this display lays nothing out, so a measured
    // height would be zero and prove nothing (#1307). Ellipsizing is the
    // property that keeps the header two rows, and it is a DOM-style fact
    // about the widget rather than about the layout.
    let subject = find_label(&header, "conversation-subject").expect("the header draws a subject");
    assert_eq!(
        subject.ellipsize(),
        gtk::pango::EllipsizeMode::End,
        "a subject longer than the pane would wrap the header taller instead \
         of truncating"
    );
    assert!(
        !subject.wraps(),
        "a wrapping subject grows the header however it is ellipsized"
    );

    window.close();
}

/// The first label carrying `class` anywhere under `root`.
fn find_label(root: &gtk::Widget, class: &str) -> Option<gtk::Label> {
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && label.has_css_class(class)
        {
            return Some(label.clone());
        }
        if let Some(found) = find_label(&widget, class) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
