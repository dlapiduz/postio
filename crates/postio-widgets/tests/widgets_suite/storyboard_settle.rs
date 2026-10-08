//! Settling: when has a step finished drawing (spec 008, research R4)?
//!
//! The window is watched on its own frame clock. It is settled when several
//! samples in a row are identical, and then watched a while longer, because
//! a frame that changes after "settling" is a jump a reviewer must see.

use std::time::Duration;

use gtk::prelude::*;
use postio_widgets::storyboard::settle::{Settings, Verdict, settle};

use super::storyboard_support::{display, show};

fn window_with(label: &gtk::Label) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_child(Some(label));
    window.set_default_size(300, 120);
    show(&window);
    window
}

/// Settings with a shorter wait, stretched by `POSTIO_TEST_PATIENCE` on a
/// loaded machine, where six identical frames can take longer than 1.2 s.
fn quick() -> Settings {
    Settings {
        max: postio_test_support::scaled(Duration::from_millis(1200)),
        ..Settings::default()
    }
}

pub fn a_static_window_settles() {
    if !display() {
        return;
    }
    let label = gtk::Label::new(Some("nothing moves here"));
    let window = window_with(&label);

    let settled = settle(&window, &quick());

    assert!(
        matches!(settled.verdict, Verdict::Settled { .. }),
        "a window that never changes was {:?}",
        settled.verdict
    );
    assert!(settled.hashes.len() >= quick().identical);
    assert_eq!(settled.hash.len(), 64, "a hex blake3");
    assert!(settled.extra.is_empty());
}

pub fn a_change_after_settling_is_a_jump_with_both_frames() {
    if !display() {
        return;
    }
    let label = gtk::Label::new(Some("before"));
    let window = window_with(&label);
    // Sampling settles after six identical frames; the change comes ten
    // frames later, inside the watch. Counted on the frame clock, not
    // timed: a wall-clock 250 ms landed before the settle on a loaded
    // machine, which drew one frame in the time it had six.
    let ticks = std::cell::Cell::new(0u32);
    label.add_tick_callback(move |label, _| {
        ticks.set(ticks.get() + 1);
        if ticks.get() == 16 {
            label.set_text("a rather different after");
            return gtk::glib::ControlFlow::Break;
        }
        gtk::glib::ControlFlow::Continue
    });

    let settled = settle(
        &window,
        &Settings {
            watch: postio_test_support::scaled(Duration::from_millis(3000)),
            ..quick()
        },
    );

    assert!(
        matches!(settled.verdict, Verdict::Jumped { frames: 2 }),
        "expected a jump, got {:?}",
        settled.verdict
    );
    assert_eq!(settled.extra.len(), 2, "both frames are kept");
    assert_ne!(
        settled.extra[0].width() * settled.extra[0].height(),
        0,
        "the kept frames are pictures"
    );
}

pub fn an_empty_window_is_blank() {
    if !display() {
        return;
    }
    let label = gtk::Label::new(Some("never realised"));
    label.set_visible(false);
    // No title bar either: a bar is something drawn, and this window has
    // to have nothing in it at all.
    let window = gtk::Window::new();
    window.set_decorated(false);
    window.set_child(Some(&label));
    window.set_default_size(300, 120);
    show(&window);

    let settled = settle(&window, &quick());

    assert!(
        matches!(settled.verdict, Verdict::Blanked { .. }),
        "a window with nothing drawn was {:?}",
        settled.verdict
    );
    assert!(!settled.extra.is_empty(), "the blank frame is kept");
}

pub fn a_window_that_never_stops_changing_is_unsettled() {
    if !display() {
        return;
    }
    let label = gtk::Label::new(Some("0"));
    let window = window_with(&label);
    let mut count = 0u32;
    let ticking = label.clone();
    let timer = gtk::glib::timeout_add_local(Duration::from_millis(50), move || {
        count += 1;
        ticking.set_text(&"#".repeat(1 + (count % 17) as usize));
        gtk::glib::ControlFlow::Continue
    });

    let settled = settle(&window, &quick());
    timer.remove();

    assert!(
        matches!(settled.verdict, Verdict::Unsettled { ms } if ms >= 1200),
        "a window changing every 50 ms was {:?}",
        settled.verdict
    );
}
