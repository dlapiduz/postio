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
