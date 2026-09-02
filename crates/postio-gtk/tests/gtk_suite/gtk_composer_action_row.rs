//! The composer's action row must not overflow its window at a narrow
//! allocation — on a real display, since this is a layout property GTK only
//! resolves once the row is actually allocated (#692).
//!
//! Reported as "the Schedule send button's label is cropped with an
//! ellipsis". Rendered at several widths (900px down to 520px, the detached
//! composer, and 200% text scale) turned up no such thing: `labelled()`
//! never sets `ellipsize` on a button's own label, so none of Send,
//! Schedule… or Save draft can lose a word — "Schedule…" already ends in an
//! ellipsis by design, which is what the report actually saw. What genuinely
//! broke at a narrow width was the *trailing* "Esc keeps the draft" hint:
//! nothing in the row could shrink below its natural size, so the whole row
//! overflowed the window instead, clipping that hint against the window's
//! own edge with no ellipsis at all -- the row is the size checked here.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use gtk::gdk;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_model::{AccountId, Draft};

fn settle() {
    while gtk::glib::MainContext::default().iteration(false) {}
}

pub fn the_escape_hint_is_the_one_allowed_to_shrink() {
    let state_dir =
        std::env::temp_dir().join(format!("postio-composer-action-row-{}", std::process::id()));
    std::fs::create_dir_all(&state_dir).unwrap();
    // SAFETY: first statement of a single-threaded test.
    #[allow(unsafe_code)]
    unsafe {
        std::env::set_var("XDG_STATE_HOME", &state_dir)
    };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = window.composer();
    composer.open(Draft::new(AccountId::UNASSIGNED));
    settle();

    assert!(
        composer.test_escape_hint_ellipsizes(),
        "the escape hint must be allowed to shrink, or nothing in the row \
         can give way under a narrow allocation and the whole row overflows \
         the window instead"
    );
}
