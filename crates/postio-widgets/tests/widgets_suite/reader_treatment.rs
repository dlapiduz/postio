//! The reader says how the body on screen is drawn -- in the app's colours
//! or on paper -- so a surface can size its column by it (specs/007 T207).
//! The answer is the renderer's own decision, not a value someone sets: a
//! reader that draws treatments reports what it drew, tells its listeners on
//! every treated draw, and tells them again after `O` switches it; a reader
//! that does not draw treatments always answers app colours.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use postio_body::treatment::Treatment;
use postio_model::message::MessageBody;
use postio_model::test_corpus;
use postio_widgets::reader::{Reader, RemoteImageAllowList};

use crate::support;

fn newsletter() -> MessageBody {
    postio_model::mime::parse(test_corpus::load("html-newsletter-own-page").bytes()).body
}

fn reader(name: &str) -> (gtk::Window, Reader) {
    let reader = Reader::with_allowlist(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        std::env::temp_dir().join(format!(
            "postio-treatment-{name}-{}.json",
            std::process::id()
        )),
    );
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&reader.widget()));
    window.present();
    (window, reader)
}

pub fn a_reader_reports_its_treatment_and_tells_when_it_changes() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = reader("treated");
    reader.use_treatments();
    assert_eq!(
        reader.treatment(),
        Treatment::AppColours,
        "nothing drawn yet is app colours"
    );
    let told: Rc<RefCell<Vec<Treatment>>> = Rc::default();
    reader.connect_treatment_changed({
        let told = Rc::clone(&told);
        move |treatment| told.borrow_mut().push(treatment)
    });

    reader.render(&newsletter(), Some("news@treatment.example.com"));
    assert!(
        support::until(|| reader.treated().is_some()),
        "the newsletter was never drawn under a treatment"
    );
    assert_eq!(reader.treatment(), Treatment::Paper);
    assert_eq!(told.borrow().last(), Some(&Treatment::Paper));

    told.borrow_mut().clear();
    assert!(reader.switch_treatment(), "an HTML body can be switched");
    assert!(
        support::until(|| told.borrow().last() == Some(&Treatment::AppColours)),
        "switching never told the listeners: {:?}",
        told.borrow()
    );
    assert_eq!(reader.treatment(), Treatment::AppColours);
    window.close();
}

pub fn a_reader_that_draws_no_treatments_answers_app_colours() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let (window, reader) = reader("classic");
    let told: Rc<RefCell<Vec<Treatment>>> = Rc::default();
    reader.connect_treatment_changed({
        let told = Rc::clone(&told);
        move |treatment| told.borrow_mut().push(treatment)
    });
    reader.render(&newsletter(), Some("news@treatment.example.com"));
    assert!(
        support::until(|| reader.view().document().is_some()),
        "the newsletter never reached the screen"
    );
    assert_eq!(reader.treatment(), Treatment::AppColours);
    assert_eq!(reader.treated(), None);
    assert!(told.borrow().is_empty(), "told {:?}", told.borrow());
    window.close();
}
