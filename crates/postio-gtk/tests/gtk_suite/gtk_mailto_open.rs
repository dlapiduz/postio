//! The application's `open` signal — what `postio mailto:…` becomes — reaches
//! the window as a delivered link.
//!
//! `gtk_mailto_seam` proves the window holds a link and hands it over;
//! `app_suite::mailto_uri` proves a delivered link becomes a composer. This
//! is the join above both: the desktop passes a URI, GApplication turns it
//! into `open`, and the application object has to have asked for that
//! (`HANDLES_OPEN`), opened its window, and delivered the link to it. It is
//! the one step that was missing for three releases, with every other layer
//! green.
//!
//! Registered `NON_UNIQUE`, so the test never touches the session bus name a
//! running Postio holds — a unique registration would make this process a
//! remote of that one and forward the link into somebody's real composer.
//!
//! One test function: GTK is single-threaded and initialised once per
//! process. Skips without a display. Nothing here touches the network.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio};
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

pub fn opening_a_mailto_uri_delivers_the_link_to_the_window() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let application = app::build();
    application.set_flags(application.flags() | gio::ApplicationFlags::NON_UNIQUE);
    application
        .register(None::<&gio::Cancellable>)
        .expect("a non-unique registration needs no bus");

    // What `Exec=postio %U` hands over for a clicked link.
    let link = gio::File::for_uri("mailto:ada@example.com?subject=Lunch");
    application.open(&[link], "");
    crate::pump();

    let window = application
        .active_window()
        .and_downcast::<Window>()
        .expect("`open` did not open the application's window");

    // Nobody has an account yet, so the link is waiting in the window; the
    // listener that connects now is what a fed composition root would be.
    let received: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
    window.connect_mailto({
        let received = Rc::clone(&received);
        move |mailto| {
            received.borrow_mut().push((
                mailto.to[0].address.clone(),
                mailto.subject.clone().unwrap_or_default(),
            ));
        }
    });
    assert_eq!(
        received.borrow().as_slice(),
        [("ada@example.com".to_owned(), "Lunch".to_owned())],
        "the URI the application was asked to open never reached the window as a \
         mailto link"
    );

    window.close();
    crate::pump();
}
