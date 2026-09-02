//! The parts panel on a real display: walk it, act on it, and never fetch.
//!
//! Reading the tree out of the part ids is pure and unit-tested in `parts.rs`.
//! What needs a display is the panel's behaviour: that every key does what its
//! own footer says, that acting on a part *asks* rather than fetches, that a
//! container offers nothing to save, and that `Esc` closes it the way `Esc`
//! closes everything else.
//!
//! Every key here goes in through [`Window::handle_key`] rather than calling
//! the panel directly — `postio-14b` (this suite's own bead) found that the
//! panel's keys used to be its own, unreachable through the window's real
//! resolver: `j` moved the message selection instead of walking the tree,
//! because nothing told the resolver the keyboard was in the panel at all.
//! Driving the fix through the same door a keystroke actually uses is the
//! only way this suite could have caught that.
//!
//! Skips without a display. Nothing here touches the network — and the point
//! of the panel is that it has no way to.
//!
//! One test function, for the reason `gtk_style.rs` gives.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_core::Context;
use postio_gtk::parts::Node;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::Attachment;
use postio_model::ids::{AttachmentId, MessageId};

/// Presses `key` exactly as the window's own top-level controller would,
/// and says whether it was taken. `GTK4` gives no supported way to
/// synthesize a real key event, so this drives the same entry point one
/// would deliver to — see [`Window::handle_key`].
fn press(window: &Window, key: gdk::Key) -> bool {
    window.handle_key(key, gdk::ModifierType::empty()) == glib::Propagation::Stop
}

pub fn the_parts_panel_walks_a_message_without_fetching_any_of_it() {
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

    let panel = window.parts();
    let asked: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let record = |asked: &Rc<RefCell<Vec<String>>>, verb: &'static str| {
        let asked = asked.clone();
        move |node: &Node| {
            asked
                .borrow_mut()
                .push(format!("{verb} {}", node.part_id.clone()))
        }
    };
    panel.connect_open(record(&asked, "open"));
    panel.connect_save({
        let asked = asked.clone();
        move |node: &Node, _| asked.borrow_mut().push(format!("save {}", node.part_id))
    });
    panel.connect_external(record(&asked, "external"));
    panel.connect_render_once(record(&asked, "render"));
    panel.connect_save_all({
        let asked = asked.clone();
        move |_| asked.borrow_mut().push("save-all".to_owned())
    });

    // -- opening it shows the structure, and starts on a part --------------

    assert!(!panel.is_visible());
    window.open_parts("multipart/mixed", &message());
    pump();

    assert!(panel.is_visible());
    let nodes = panel.nodes();
    assert_eq!(nodes.len(), 6, "five parts and the message itself");
    assert_eq!(
        panel.cursor().map(|node| node.part_id),
        Some("1".to_owned()),
        "the cursor starts on the first part, not on the container above it"
    );
    assert!(
        asked.borrow().is_empty(),
        "opening the panel asked for nothing: {:?}",
        asked.borrow()
    );
    assert_eq!(
        window.context(),
        Context::Parts,
        "the panel is up, so the keyboard belongs to it"
    );

    // -- a key that means something elsewhere does nothing here -------------
    //
    // `a` archives in `Context::List`. Before `postio-14b` there was nothing
    // to stop the window's own resolver from claiming it first: the panel's
    // key controller never got a turn, and pressing `a` here would have
    // archived the message underneath rather than doing nothing.
    assert!(
        !press(&window, gdk::Key::a),
        "`a` is not one of the panel's own keys and must not reach `archive`"
    );

    // -- j and k walk it, and stop at the ends -----------------------------

    assert!(press(&window, gdk::Key::j));
    pump();
    assert_eq!(
        panel.cursor().map(|node| node.part_id),
        Some("1.1".to_owned()),
        "the walk goes into the branch, not over it"
    );

    assert!(press(&window, gdk::Key::k));
    assert!(press(&window, gdk::Key::k));
    pump();
    assert_eq!(
        panel.cursor().map(|node| node.part_id),
        Some(String::new()),
        "`k` reached the message itself and stopped there"
    );
    assert!(press(&window, gdk::Key::k));
    assert_eq!(
        panel.cursor().map(|node| node.part_id),
        Some(String::new()),
        "and stays there rather than wrapping round"
    );

    // -- a container has nothing to save -----------------------------------

    let root = panel.cursor().expect("a cursor");
    assert!(!root.is_leaf());
    assert!(
        !save_button(&window).is_sensitive(),
        "the message is not a part, so there is nothing to save"
    );

    let container = nodes
        .iter()
        .find(|node| node.mime == "multipart/alternative")
        .expect("the fixture nests");
    assert!(
        !container.is_leaf(),
        "a container holds parts, it is not one"
    );

    // -- every key does what the footer says --------------------------------

    while panel.cursor().map(|node| node.part_id) != Some("3".to_owned()) {
        assert!(press(&window, gdk::Key::j));
    }
    pump();
    assert!(save_button(&window).is_sensitive());

    for (key, expected) in [(gdk::Key::Return, "open 3"), (gdk::Key::x, "external 3")] {
        assert!(press(&window, key));
        assert_eq!(
            asked.borrow().last().map(String::as_str),
            Some(expected),
            "after {key:?}"
        );
    }

    // `s` and `S` put a real GtkFileDialog up, which nothing in a test can
    // answer — so what is checked here is that the key is taken and that the
    // panel still asked for nothing by itself. What the dialog hands back is
    // `connect_save`'s business, and its shape is checked by the compiler.
    //
    // They must also be closed again. An unanswered dialog outlives the test:
    // it was landing on the maintainer's desktop on every run and staying
    // there, because a test process exiting does not dismiss a window the
    // display server is already showing.
    let before = asked.borrow().len();
    assert!(press(&window, gdk::Key::s));
    assert!(press(&window, gdk::Key::S));
    assert_eq!(
        asked.borrow().len(),
        before,
        "nothing is saved until the user has said where"
    );
    assert!(
        dismiss_dialogs(&window) > 0,
        "pressing s should have opened a dialog to dismiss"
    );

    // -- and every one of them only *asked* ---------------------------------

    assert!(
        panel.nodes().iter().all(|node| !node.downloaded),
        "the panel has no way to fetch, so nothing became downloaded"
    );

    // -- the held-back part says what it is holding back --------------------

    panel.set_held_back(3, 1);
    pump();
    assert!(
        blocked_tag(&window).is_visible(),
        "the header says the reader held something back"
    );

    while panel.cursor().map(|node| node.mime) != Some("text/html".to_owned()) {
        assert!(press(&window, gdk::Key::k));
    }
    pump();
    let note = note_text(&window);
    assert!(
        note.contains("3 remote images") && note.contains("1 likely tracker"),
        "the note names what would load, and hedges the tracker count \
         honestly -- the heuristic reads declared size and can be wrong \
         about a picture (#174): {note:?}"
    );

    assert!(press(&window, gdk::Key::H));
    assert!(
        asked
            .borrow()
            .last()
            .map(String::as_str)
            .unwrap_or("")
            .starts_with("render"),
        "and `H` is how you ask for it anyway"
    );

    // -- a part that references nothing is not held back --------------------

    while panel.cursor().map(|node| node.mime) != Some("image/png".to_owned()) {
        assert!(press(&window, gdk::Key::j));
    }
    pump();
    assert!(
        !note_text(&window).contains("held back"),
        "an image references nothing, so holding it back would be theatre"
    );

    // -- Esc closes it, through the registry's own Back ---------------------

    assert!(press(&window, gdk::Key::Escape));
    pump();
    assert!(!panel.is_visible());
    assert_ne!(
        window.context(),
        Context::Parts,
        "closing the panel gives the keyboard back"
    );

    window.destroy();
}

/// A message with a nested alternative, a patch and an image — canvas 3g's
/// own shape, with one extra level so the tree really is a tree.
fn message() -> Vec<Attachment> {
    let message = MessageId::new(1);
    let part = |id: i64, path: &str, mime: &str, size: u64, filename: Option<&str>| {
        let mut part = Attachment::new(message, mime, size);
        part.id = AttachmentId::new(id);
        part.part_id = Some(path.to_owned());
        part.filename = filename.map(str::to_owned);
        part
    };
    vec![
        part(1, "1", "multipart/alternative", 0, None),
        part(2, "1.1", "text/plain", 2_100, None),
        part(3, "1.2", "text/html", 6 * 1024, None),
        part(4, "2", "text/x-diff", 11 * 1024, Some("0001-index.patch")),
        part(5, "3", "image/png", 1_100 * 1024, Some("cold.png")),
    ]
}

fn save_button(window: &Window) -> gtk::Button {
    labelled_button(window, "Save part")
}

fn labelled_button(window: &Window, label: &str) -> gtk::Button {
    find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-parts-action")
            && find(widget, &|inner| {
                inner
                    .downcast_ref::<gtk::Label>()
                    .is_some_and(|inner| inner.text() == label)
            })
            .is_some()
    })
    .and_then(|widget| widget.downcast::<gtk::Button>().ok())
    .unwrap_or_else(|| panic!("a button labelled {label}"))
}

fn blocked_tag(window: &Window) -> gtk::Widget {
    find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-parts-blocked")
    })
    .expect("the header has a blocked tag")
}

fn note_text(window: &Window) -> String {
    find(&window.clone().upcast(), &|widget| {
        widget.has_css_class("postio-parts-note")
    })
    .and_then(|widget| widget.downcast::<gtk::Label>().ok())
    .map(|label| label.text().to_string())
    .unwrap_or_default()
}

/// Close every toplevel except the test's own window, and say how many.
///
/// `save_part` and `save_all` construct a `GtkFileDialog` and hand it to the
/// display server. A test has no reference to it and cannot answer it, and
/// the process ending does not take it away — so without this, running the
/// suite leaves a "Save cold.png" dialog on the developer's desktop, once per
/// run, forever.
///
/// Closing every other toplevel rather than naming the dialog is deliberate:
/// the test never constructed it and has no handle on it, and any *other*
/// stray toplevel is equally something this test should not leave behind.
fn dismiss_dialogs(keep: &Window) -> usize {
    let mut closed = 0;
    let toplevels = gtk::Window::toplevels();
    let keep_window: gtk::Window = keep.clone().upcast();
    for item in toplevels.into_iter().flatten() {
        if let Ok(window) = item.downcast::<gtk::Window>()
            && window != keep_window
        {
            window.destroy();
            closed += 1;
        }
    }
    pump();
    closed
}

fn pump() {
    let context = glib::MainContext::default();
    while context.iteration(false) {}
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
