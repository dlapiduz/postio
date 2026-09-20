//! The first composition must not pay for the editing surface.
//!
//! The composer's editing surface is ADR 0003's `WebView`, and the first
//! `load_html` on a fresh one starts a WebKit web process. Measured on this
//! workstation, splitting `Composer::open` into its parts: the first open
//! spent 28.7ms in `fill` and 11.2ms putting the keyboard somewhere, against
//! 0.2ms and 1.3ms for every open after it. So the whole of that cost falls on
//! the one composition a person actually notices -- the first one, the one
//! where they have just decided to write to somebody (#1216).
//!
//! Warming it is the same answer the reader got: do the first load early, on
//! an idle turn of the main loop, when nobody is waiting. What this file
//! asserts is that the mechanism exists and reports honestly; that the *app*
//! actually calls it is `postio-app`'s job to prove, because a warm-up wired
//! to nothing is exactly the shape of #327.
//!
//! Skips without a display. Nothing here touches the network -- the editor's
//! `WebView` has network off, and the seed is Postio's own markup.

use crate::settle;
use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn the_editing_surface_can_be_warmed_before_anyone_composes() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = window.composer();
    assert!(
        !composer.is_warm(),
        "a composer nobody has warmed cannot already be warm, or this proves nothing"
    );
    assert!(
        !composer.is_open(),
        "warming is not opening: the pane must stay where it is"
    );

    composer.warm();
    settle();

    assert!(composer.is_warm(), "warming it did not warm it");
    assert!(
        !composer.is_open(),
        "warming opened the composer over the reading pane"
    );

    // Idempotent, because whatever schedules it must not have to know whether
    // it has run -- and because a second load would throw away a draft that
    // had been typed into it in the meantime.
    composer.warm();
    settle();
    assert!(composer.is_warm());

    // And it is still a composer: warming seeds an empty document, so opening
    // one afterwards has to put the real draft in rather than find the seed.
    window.composer().open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();
    assert!(
        composer.is_open(),
        "the composer would not open after warming"
    );
    assert!(composer.is_warm(), "opening it un-warmed it");
}
