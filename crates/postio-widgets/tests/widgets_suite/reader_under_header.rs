//! What a surface puts under the reader's header: Focus's marker card sits
//! between the header and the body, on the widget tree after a message is
//! drawn.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_model::message::MessageBody;
use postio_widgets::reader::{Reader, RemoteImageAllowList};

fn drawn() -> (gtk::Window, Reader) {
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        std::env::temp_dir().join(format!("postio-under-header-{}.json", std::process::id())),
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

/// Focus's marker card sits under the message header and over the body
/// (contracts/focus-surface.md, the open-email dialog; research R2): the
/// reader holds a place there for whatever a surface puts in it, which
/// stays with the header while the body scrolls.
pub fn a_card_placed_under_the_header_sits_between_it_and_the_body() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = drawn();
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
