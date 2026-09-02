//! Issue #339: the composer's formatting toolbar, on a real display.
//!
//! The toolbar is the visible half of the formatting commands #338 landed:
//! every button dispatches the same registry command the keyboard reaches,
//! and the toggles reflect where the caret sits — bold shows active when the
//! selection is inside `Strong`. That reflection crosses the bridge in the
//! other direction from an edit, which is why it needs a display: the state
//! report comes from the editing WebView, not from the document.
//!
//! Its own file: GTK is single-threaded and initialised once, so one
//! `#[test]` per integration binary. See `gtk_composer.rs`.

use crate::settle as pump;
use std::time::{Duration, Instant};

use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Inline};
use postio_gtk::composer;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};

/// Issue #741: a gate run once burned 120.59s here before failing, on a box
/// that was building elsewhere at the time; the same test passed in 0.60s
/// run alone minutes later. 120s was sized to never be reached, which made
/// it free to spend in full whenever something really was wrong. Twenty
/// seconds is still 20x what a bridge round trip costs in practice (see
/// `bridge_latency`), so a genuine stall still has generous room, but a gate
/// run can no longer lose two minutes finding out.
const SETTLE_DEADLINE: Duration = Duration::from_secs(20);

/// The wall-clock cost, measured now, of a trivial round trip through the
/// same WebKit bridge every reflection in this test depends on.
///
/// This is what turns a timeout into a diagnosis instead of a shrug: slow
/// here means the machine was contended and the wait above was racing load,
/// not a real regression; fast here means the report `settle` was waiting on
/// was never going to arrive at all, which is the failure this test exists
/// to catch.
fn bridge_latency(composer: &composer::Composer) -> Duration {
    let start = Instant::now();
    composer.test_body_eval("'ping'");
    start.elapsed()
}

fn settle(composer: &composer::Composer, what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + SETTLE_DEADLINE;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if done() {
        return;
    }
    let bridge = bridge_latency(composer);
    if bridge > Duration::from_secs(1) {
        panic!(
            "timed out waiting for {what} after {SETTLE_DEADLINE:?} -- but a trivial \
             bridge round trip alone just took {bridge:?}, so this machine is \
             contended and the wait above was racing load, not a real regression; \
             re-run somewhere quieter"
        );
    }
    panic!(
        "timed out waiting for {what} after {SETTLE_DEADLINE:?} -- the bridge itself \
         just answered a trivial round trip in {bridge:?}, so the report this test \
         was waiting on was never going to arrive"
    );
}

/// Depth-first search of a widget tree for the first one carrying `class`.
fn find(widget: &gtk::Widget, class: &str) -> Option<gtk::Widget> {
    if widget.has_css_class(class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(found) = find(&current, class) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}

fn toggle(composer: &composer::Composer, class: &str) -> gtk::ToggleButton {
    find(composer.upcast_ref::<gtk::Widget>(), class)
        .unwrap_or_else(|| panic!("the toolbar carries a {class} button"))
        .downcast()
        .expect("the formatting toggles are ToggleButtons")
}

pub fn the_toolbar_reaches_the_registry_commands_and_reflects_the_caret() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (run under the headless runner to exercise this)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);
    app::install_icons(&display);

    let window = Window::default();
    window.present();
    pump();

    let composer = composer::install(&window);
    window.handle_key(
        gdk::Key::from_name("c").unwrap(),
        gdk::ModifierType::empty(),
    );
    pump();
    assert!(composer.is_open());

    // ── the toolbar exists, between the fields and the body ───────────────
    let toolbar = find(
        composer.upcast_ref::<gtk::Widget>(),
        "postio-compose-toolbar",
    )
    .expect("the composer builds the toolbar unconditionally");
    assert!(toolbar.is_visible());

    let bold = toggle(&composer, "postio-toolbar-bold");
    let italic = toggle(&composer, "postio-toolbar-italic");
    let bullet = toggle(&composer, "postio-toolbar-bullet-list");
    let numbered = toggle(&composer, "postio-toolbar-numbered-list");
    let quote = toggle(&composer, "postio-toolbar-quote-block");
    let link: gtk::Button = find(composer.upcast_ref::<gtk::Widget>(), "postio-toolbar-link")
        .expect("the toolbar carries the link button")
        .downcast()
        .expect("the link button is a plain Button — a dialog, not a state");

    // Every button names itself and its key, from the registry, and none
    // steals the focus a click needs to keep in the editor.
    for button in [
        bold.clone().upcast::<gtk::Widget>(),
        italic.clone().upcast(),
        bullet.clone().upcast(),
        numbered.clone().upcast(),
        quote.clone().upcast(),
        link.clone().upcast(),
    ] {
        let tip = button.tooltip_text().expect("every button explains itself");
        assert!(
            tip.contains("Ctrl+") || tip.contains("ctrl+"),
            "the tooltip teaches the key: {tip}"
        );
        assert!(
            !button.property::<bool>("focus-on-click"),
            "a click must not steal the caret"
        );
    }

    // ── a click lands as the registry command does: canonical structure ───
    // Focus the body first, as a hand would have: the click acts on the
    // editor's selection, and WebKit only arms editing commands in a
    // focused frame.
    composer.test_focus_field(composer::Field::Body);
    pump();
    composer.test_set_body("make this strong");
    composer.test_select_body(0, 5, 9);
    bold.emit_clicked();
    settle(&composer, "bold from the toolbar to land as Strong", || {
        matches!(
            composer.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
        )
    });
    assert!(!italic.is_active(), "italic was never applied");
    assert!(!bullet.is_active());
    assert!(!numbered.is_active());
    assert!(!quote.is_active());

    // ── the toggles reflect the caret, not the last click ─────────────────
    // A click flips a ToggleButton on its own, so clicking proves nothing
    // about reflection. Moving the caret does: no click happens here, and
    // only the editor's format report can change the button.
    composer.test_select_body(0, 0, 2);
    settle(
        &composer,
        "the bold toggle to clear once the caret leaves Strong",
        || !bold.is_active(),
    );
    // Bolding split the paragraph's text into three nodes; the Strong run
    // is the second. Back inside it, the report lights the toggle again.
    composer.test_select_body(1, 1, 3);
    settle(&composer, "the bold toggle to light inside Strong", || {
        bold.is_active()
    });

    // ── clicking again toggles it off, in the document and the button ─────
    composer.test_select_body(1, 0, 4);
    bold.emit_clicked();
    settle(&composer, "bold to toggle back off", || {
        matches!(
            composer.document().blocks.first(),
            Some(Block::Paragraph(inlines))
                if !inlines.iter().any(|inline| matches!(inline, Inline::Strong(_)))
        )
    });
    settle(&composer, "the toggle to follow", || !bold.is_active());

    // ── the quote button walks the same path as its command ──────────────
    quote.emit_clicked();
    settle(&composer, "the quote block to land", || {
        matches!(composer.document().blocks.first(), Some(Block::Quote(_)))
    });
}
