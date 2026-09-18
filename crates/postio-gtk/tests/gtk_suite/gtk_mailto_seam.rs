//! A `mailto:` link the window is handed before anyone can act on it waits,
//! and is handed over the moment someone can.
//!
//! The window does not know which account a new message is from; the
//! composition root does, and it connects late — after the store opens,
//! which on a cold launch from a browser is after the link arrived. The seam
//! has to hold the link across that gap and deliver it in order, and deliver
//! later links at once. `app_suite::mailto_uri` proves the link reaches a
//! composer; this proves the seam's own two promises with nothing behind it.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};
use postio_model::mailto::Mailto;

pub fn a_mailto_delivered_before_anyone_listens_is_handed_over_in_order() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let link = |address: &str| Mailto::parse(&format!("mailto:{address}")).expect("a mailto uri");

    // Two links before there is a listener: a browser that was asked twice
    // while Postio was still opening its store.
    window.deliver_mailto(link("ada@example.com"));
    window.deliver_mailto(link("grace@example.net"));

    let received: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_mailto({
        let received = Rc::clone(&received);
        move |mailto| {
            received.borrow_mut().push(mailto.to[0].address.clone());
        }
    });
    assert_eq!(
        received.borrow().as_slice(),
        ["ada@example.com", "grace@example.net"],
        "the links that arrived before the listener were not handed over, in order, \
         when it connected"
    );

    // And one after: straight through, nothing parked.
    window.deliver_mailto(link("alan@example.org"));
    assert_eq!(
        received.borrow().as_slice(),
        ["ada@example.com", "grace@example.net", "alan@example.org"],
        "a link delivered to a listening window was not handed over at once"
    );
}
