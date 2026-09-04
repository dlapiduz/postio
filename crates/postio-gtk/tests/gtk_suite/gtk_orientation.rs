//! The first-run orientation strip, with a real widget around it.
//!
//! `orientation.rs`'s own unit tests prove what the strip *says* for a given
//! keymap, with no display. What needs a widget is that the saying reaches
//! the screen — a chip per hint, drawn from the keymap it was handed — and
//! the case the pure functions can only describe: a keymap that binds none
//! of the three keys leaves nothing to teach, and a strip with nothing on it
//! must not take a strip's worth of the mail column.
//!
//! Reachability inside the running application is `postio-app`'s
//! `app_suite/orientation.rs`; this is the widget alone.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the widget under test exists,
// which is the one moment it is sound. The crate's library code forbids
// `unsafe`.

use crate::settle;
use gtk::gdk;
use gtk::prelude::*;
use postio_config::KeyBindings;
use postio_config::paths::Platform;
use postio_core::Keymap;
use postio_gtk::orientation::OrientationStrip;
use postio_gtk::{app, fonts, style};

/// How narrow `Shell` lets the message column get (`shell.rs`'s own size
/// request), and therefore the width this strip has to stay inside.
const COLUMN: i32 = 280;

/// Every label on the strip, in the order it draws them.
fn labels(strip: &OrientationStrip) -> Vec<String> {
    fn walk(widget: &gtk::Widget, found: &mut Vec<String>) {
        if let Some(label) = widget.downcast_ref::<gtk::Label>() {
            found.push(label.text().to_string());
        }
        let mut child = widget.first_child();
        while let Some(node) = child {
            walk(&node, found);
            child = node.next_sibling();
        }
    }
    let mut found = Vec::new();
    walk(&strip.widget(), &mut found);
    found
}

pub fn the_strip_teaches_the_keys_in_force_and_nothing_when_there_are_none() {
    let state_dir = tempfile::tempdir().expect("a state directory");
    // SAFETY: first statement of a single-threaded test.
    unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    // ── the keys the user actually has ──────────────────────────────────
    let strip = OrientationStrip::new();
    strip.set_keymap(&Keymap::resolve_on(
        &KeyBindings::default(),
        Platform::Freedesktop,
    ));
    strip.set_visible(true);
    settle();

    assert!(strip.is_visible(), "asked to show, with three keys to teach");
    let drawn = labels(&strip);
    for expected in ["Postio is keyboard-first", "ctrl+k", "?", "j/k", "Got it"] {
        assert!(
            drawn.iter().any(|label| label == expected),
            "the strip never draws {expected:?}; it draws {drawn:?}"
        );
    }

    // ── it has to fit the column it sits in ─────────────────────────────
    // "Fully dismissible" is an acceptance criterion, and a horizontal box
    // hands every child its minimum and clips the rest — so a strip wider
    // than the column pushes its last child, "Got it", off the end, and the
    // one control that ends this becomes unreachable. `Shell` lets the
    // message column go down to 280px, so that is the width this has to
    // survive, not the window's.
    let (minimum, _natural, _, _) = strip.widget().measure(gtk::Orientation::Horizontal, -1);
    assert!(
        minimum <= COLUMN,
        "the strip demands {minimum}px before it will give any ground, and \
         the message column is allowed down to {COLUMN}px: everything past \
         that width -- \"Got it\" last of all -- is cut off"
    );

    // ── and a build of it that has none ─────────────────────────────────
    // Not a contrived state: a `[keys]` that takes `?`, the palette key and
    // `j`/`k` for other commands leaves this strip with nothing to say, and
    // `Keymap::default()` is that keymap without depending on which
    // override happens to win a collision this release. An empty box with a
    // "Got it" button in it is worse than no strip at all, so it must
    // refuse to appear.
    let keymap = Keymap::default();
    assert!(
        postio_gtk::orientation::hints(&keymap).is_empty(),
        "the fixture was supposed to leave nothing bound, so this test \
         could not fail: {:?}",
        postio_gtk::orientation::hints(&keymap)
    );

    let empty = OrientationStrip::new();
    empty.set_keymap(&keymap);
    empty.set_visible(true);
    settle();

    assert!(
        !empty.is_visible(),
        "a strip with no keys to teach put itself over the mail anyway: \
         it draws {:?}",
        labels(&empty)
    );
}
