//! The reader's verb bars are configuration (specs/007-postio-focus research
//! R1): the reading pane draws Reply, Reply all, Forward and Archive in the
//! message header, and a surface with a toolbar of its own -- Focus's
//! open-message dialog -- asks for none, rather than two rows of the same
//! verbs.
//!
//! Both configurations are asserted side by side, on the widget tree after a
//! message is drawn: what a person could see and press.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_widgets::reader::{Reader, RemoteImageAllowList, Verbs};

/// Every CSS class in `widget`'s subtree, the widget's own included.
fn classes(widget: &gtk::Widget) -> Vec<String> {
    let mut out: Vec<String> = widget.css_classes().iter().map(|c| c.to_string()).collect();
    let mut child = widget.first_child();
    while let Some(current) = child {
        out.extend(classes(&current));
        child = current.next_sibling();
    }
    out
}

fn drawn(verbs: Verbs) -> (gtk::Window, Reader) {
    let reader = Reader::with_verbs(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        std::env::temp_dir().join(format!("postio-verbs-{}.json", std::process::id())),
        verbs,
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    reader.render(
        &MessageBody {
            text: Some("The survey starts Monday.".to_owned()),
            html: None,
        },
        Some("ada@example.com"),
    );
    while gtk::glib::MainContext::default().iteration(false) {}
    (window, reader)
}

pub fn a_reader_draws_the_verbs_it_is_given_and_none_when_given_none() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }

    let (window, reader) = drawn(Verbs::STANDARD);
    assert_eq!(
        reader.visible_verbs(),
        ["Reply", "Reply all", "Forward", "Archive"],
        "the reading pane's verbs, in canvas order"
    );
    assert!(
        classes(&reader.header().widget()).contains(&"postio-reader-actions".to_owned()),
        "the reading pane's bar is not in its header"
    );
    window.destroy();

    let (window, reader) = drawn(Verbs::NONE);
    assert!(
        reader.visible_verbs().is_empty(),
        "a reader given no verbs offers some: {:?}",
        reader.visible_verbs()
    );
    assert!(
        !classes(&reader.widget()).contains(&"postio-reader-actions".to_owned()),
        "a reader given no verbs still mounts a bar for them"
    );
    window.destroy();
}

/// Focus's marker card sits under the message header and over the body
/// (contracts/focus-surface.md, the open-email dialog; research R2): the
/// reader holds a place there for whatever a surface puts in it, which
/// stays with the header while the body scrolls.
pub fn a_card_placed_under_the_header_sits_between_it_and_the_body() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = drawn(Verbs::NONE);
    let card = gtk::Label::new(Some("the marker card"));
    reader.set_under_header(Some(card.upcast_ref::<gtk::Widget>()));
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(card.is_mapped(), "the card is shown");
    let order: Vec<gtk::Widget> = {
        let mut out = Vec::new();
        let mut child = reader.widget().first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            out.push(current);
        }
        out
    };
    let place = |target: &gtk::Widget| {
        order
            .iter()
            .position(|child| child == target || target.is_ancestor(child))
            .expect("inside the reader")
    };
    let header = reader.header().widget();
    let body = reader.view().clone().upcast::<gtk::Widget>();
    assert!(
        place(&header) < place(card.upcast_ref()) && place(card.upcast_ref()) < place(&body),
        "the card is between the header and the body"
    );
    reader.set_under_header(None);
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(!card.is_mapped(), "taking it away takes it away");
    window.destroy();
}

/// A surface that heads its column with the subject itself -- Focus's
/// dialog -- hides the header's own subject line rather than draw it twice.
pub fn a_header_can_leave_the_subject_to_its_surface() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = drawn(Verbs::NONE);
    let header = reader.header();
    header.set_message(
        &[],
        &[],
        &[],
        Some("Harbor API draft v3"),
        chrono::Utc::now(),
    );
    while gtk::glib::MainContext::default().iteration(false) {}
    let shown = |header: &postio_widgets::reader::MessageHeader| {
        let mut stack = vec![header.widget()];
        let mut found = false;
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && label.label() == "Harbor API draft v3"
                && label.is_mapped()
            {
                found = true;
            }
            let mut child = widget.first_child();
            while let Some(next) = child {
                child = next.next_sibling();
                stack.push(next);
            }
        }
        found
    };
    assert!(shown(&header), "the subject line is drawn by default");
    header.set_subject_visible(false);
    while gtk::glib::MainContext::default().iteration(false) {}
    assert!(!shown(&header), "the subject line stayed");
    window.destroy();
}
