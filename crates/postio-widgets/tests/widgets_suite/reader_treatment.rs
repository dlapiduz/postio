//! The reader says how the body on screen is shown -- in the app's colours
//! or on paper -- so a surface can size its column by it (specs/007 T207).
//! Until something classifies the body, it is the app's colours.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use postio_body::treatment::Treatment;
use postio_widgets::reader::{Reader, RemoteImageAllowList, Verbs};

pub fn a_reader_reports_its_treatment_and_tells_when_it_changes() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let reader = Reader::with_verbs(
        Rc::new(|_content_id: &str| None),
        RemoteImageAllowList::default(),
        std::env::temp_dir().join(format!("postio-treatment-{}.json", std::process::id())),
        Verbs::NONE,
    );
    assert_eq!(reader.treatment(), Treatment::AppColours);
    let told: Rc<RefCell<Vec<Treatment>>> = Rc::default();
    reader.connect_treatment_changed({
        let told = Rc::clone(&told);
        move |treatment| told.borrow_mut().push(treatment)
    });
    reader.set_treatment(Treatment::Paper);
    reader.set_treatment(Treatment::Paper);
    assert_eq!(reader.treatment(), Treatment::Paper);
    reader.set_treatment(Treatment::AppColours);
    assert_eq!(
        *told.borrow(),
        [Treatment::Paper, Treatment::AppColours],
        "told once per change, and not when nothing changed"
    );
}
