//! The live hit count on a real display: what it says, and what it refuses
//! to say.
//!
//! The pure halves — the wording of `14 hits · 11 ms` and the supersession
//! rule — are unit-tested in `search.rs`. What needs a display is the part
//! that only exists once the box, the field and the timer are real: that a
//! burst of typing costs one search rather than one per character, that a
//! keystroke never waits for the search it schedules, and that an answer to a
//! query the user has already moved on from never reaches the field.
//!
//! Skips without a display. Nothing here touches the network.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use crate::settle as pump;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_gtk::finder::{Mode, Query};
use postio_gtk::search::{DEBOUNCE, Outcome};
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn the_readout_answers_the_query_on_screen_and_no_other() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    pump();

    let finder = window.finder();
    let live = finder
        .live()
        .expect("the box has a field, so it has a readout");

    // Every run the box asks for: the query it asked about, and the sequence
    // number its answer has to come back under.
    let asked: Rc<RefCell<Vec<(String, u64)>>> = Rc::new(RefCell::new(Vec::new()));
    live.connect_run({
        let asked = asked.clone();
        move |query, sequence| {
            asked
                .borrow_mut()
                .push((query.input().to_owned(), sequence))
        }
    });

    window.open_finder(Mode::Search);

    // -- an empty box has not searched, so it says nothing ----------------

    assert_eq!(readout(&window), "", "an empty box has no count to show");

    // -- a burst of typing costs one search -------------------------------

    for prefix in ["m", "ma", "mai", "mail", "maild", "maildi", "maildir"] {
        finder.set_query(Query {
            mode: Mode::Search,
            text: prefix.to_owned(),
        });
    }
    // The budget, asserted as its cause rather than as a stopwatch reading.
    //
    // A keystroke fits in 16ms because it schedules a search instead of
    // running one, and *that* is what this checks. The wall-clock assertion
    // that used to stand beside it — `typing < INTERACTION_BUDGET` — measured
    // the process it ran in, not the code: it failed at 16.17ms as one of 180
    // cases in a shared binary against an allocator the other 179 had already
    // warmed, and passed alone every time. #841 moved `gtk_composer` out of
    // this suite for exactly that, and CLAUDE.md's rule is the general form:
    // "a shared runner cannot defend 16ms, so what gates a PR is the *cause*
    // of each budget, counted".
    //
    // Nothing is lost. Zero searches for seven keystrokes is the strictly
    // stronger statement — a build that blew the budget by searching inline
    // fails here first, and fails the same way on every machine.
    assert!(
        asked.borrow().is_empty(),
        "typing must not run a search inline; it only reschedules one — which \
         is why a keystroke fits the 16ms interaction budget: {:?}",
        asked.borrow()
    );

    wait_out_the_debounce();
    assert_eq!(
        asked.borrow().len(),
        1,
        "a burst of seven characters is one question, not seven"
    );
    assert_eq!(asked.borrow()[0].0, "maildir");

    // -- the answer to that question reaches the field --------------------

    let sequence = asked.borrow()[0].1;
    assert!(live.deliver(sequence, outcome(14, 11)));
    pump();
    assert_eq!(readout(&window), "14 hits · 11 ms");

    // -- a redraw that changes nothing does not cancel a search -----------

    finder.set_query(Query {
        mode: Mode::Search,
        text: "maildir".to_owned(),
    });
    // Setting the workspace context redraws the box; it is not typing.
    finder.set_context(postio_core::Context::Reader);
    wait_out_the_debounce();
    assert_eq!(
        asked.borrow().len(),
        1,
        "nothing was typed, so nothing new should have been asked"
    );
    assert_eq!(
        readout(&window),
        "14 hits · 11 ms",
        "and the answer already on screen still stands"
    );

    // -- a superseded query is dropped, not drawn -------------------------

    finder.set_query(Query {
        mode: Mode::Search,
        text: "rebuild".to_owned(),
    });
    wait_out_the_debounce();
    assert_eq!(asked.borrow().len(), 2);
    let (_, current) = asked.borrow()[1];

    // The first search finally answers, long after the user moved on.
    assert!(
        !live.deliver(sequence, outcome(9_999, 400)),
        "an answer to a query nobody is asking any more is not an answer"
    );
    pump();
    assert_ne!(
        readout(&window),
        "9999 hits · 400 ms",
        "the readout must not go backwards through superseded answers"
    );

    assert!(live.deliver(current, outcome(3, 4)));
    pump();
    assert_eq!(readout(&window), "3 hits · 4 ms");

    // -- while one search runs, the next query waits its turn -------------
    //
    // One search in flight at a time (#500). On a fast store the debounce is
    // the only pacing anyone ever sees; on a store that has gone slow — a
    // cold cache, a backfill hammering the disk — every keystroke used to
    // launch another search into the pool while the last one was still out,
    // and the pile-up made the slow store slower. The rule: a new query
    // waits for the outstanding answer, and replaces any query already
    // waiting.

    finder.set_query(Query {
        mode: Mode::Search,
        text: "radon".to_owned(),
    });
    wait_out_the_debounce();
    assert_eq!(
        asked.borrow().len(),
        3,
        "nothing is in flight, so the query goes straight out"
    );
    let (_, running) = asked.borrow()[2];

    // The user keeps typing while that search is still out at the store.
    finder.set_query(Query {
        mode: Mode::Search,
        text: "radon report".to_owned(),
    });
    wait_out_the_debounce();
    assert_eq!(
        asked.borrow().len(),
        3,
        "a store still answering one query must not be asked a second"
    );

    // The running search answers — stale, so it is dropped — and the query
    // that was waiting goes out the moment the store is free.
    assert!(!live.deliver(running, outcome(112, 3788)));
    assert_eq!(
        asked.borrow().len(),
        4,
        "the waiting query runs as soon as the outstanding answer lands"
    );
    assert_eq!(asked.borrow()[3].0, "radon report");
    let (_, follow) = asked.borrow()[3];
    assert!(live.deliver(follow, outcome(7, 12)));
    pump();
    assert_eq!(readout(&window), "7 hits · 12 ms");

    // -- Enter does not wait out the debounce ------------------------------
    //
    // The debounce is sized to typing cadence, which makes it long enough to
    // feel if a person types a query and hits Enter inside the window. Enter
    // means "search now", so it flushes the queued query instead of waiting.

    finder.set_query(Query {
        mode: Mode::Search,
        text: "radon urgent".to_owned(),
    });
    assert_eq!(
        asked.borrow().len(),
        4,
        "the debounce has not fired; the query is still queued"
    );
    finder.activate();
    assert_eq!(
        asked.borrow().len(),
        5,
        "Enter runs the queued query immediately"
    );
    assert_eq!(asked.borrow()[4].0, "radon urgent");
    let (_, entered) = asked.borrow()[4];
    assert!(live.deliver(entered, outcome(2, 5)));
    pump();

    // -- emptying the box takes the count down ----------------------------

    finder.set_query(Query {
        mode: Mode::Search,
        text: String::new(),
    });
    pump();
    assert_eq!(
        readout(&window),
        "",
        "an empty box has not searched, so `no hits` would be a lie"
    );

    // -- closing the box stops it answering at all ------------------------

    finder.set_query(Query {
        mode: Mode::Search,
        text: "rebuild".to_owned(),
    });
    wait_out_the_debounce();
    let (_, last) = *asked.borrow().last().expect("a third run");
    window.close_finder();
    pump();
    assert!(
        !live.deliver(last, outcome(3, 4)),
        "a closed box is not waiting for an answer"
    );
    assert_eq!(readout(&window), "");

    window.destroy();
}

fn outcome(hits: u64, millis: u64) -> Outcome {
    Outcome {
        hits,
        capped: false,
        elapsed: Duration::from_millis(millis),
        // A settled account, which is what this file's assertions about the
        // readout's wording are written against (#352).
        corpus_complete: true,
        // And every account answering (#812), for the same reason.
        unreachable: Vec::new(),
    }
}

/// What the field's readout is currently showing, as a screen reader or a
/// screenshot would find it: by looking at the widget, not by asking the
/// model that drives it.
fn readout(window: &Window) -> String {
    let label = find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-readout")
    })
    .expect("the field has a readout");
    let label: gtk::Label = label.downcast().expect("the readout is a label");
    if label.is_visible() {
        label.text().to_string()
    } else {
        String::new()
    }
}

/// Runs the main loop past the debounce, so a scheduled search comes due.
fn wait_out_the_debounce() {
    let deadline = Instant::now() + DEBOUNCE + Duration::from_millis(60);
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    while Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

/// Depth-first search of a widget tree.
fn find(widget: &gtk::Widget, wanted: &dyn Fn(&gtk::Widget) -> bool) -> Option<gtk::Widget> {
    if wanted(widget) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, wanted) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
