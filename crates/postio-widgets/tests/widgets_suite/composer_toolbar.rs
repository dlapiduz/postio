use crate::support_compose::settle as pump;
use std::time::{Duration, Instant};

use crate::support_compose::Window;
use crate::support_compose::composer;
use crate::support_compose::{app, fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_body::{Block, Inline};

/// 20 s is still 20x a bridge round trip, so a real stall has room but a gate run cannot lose two
/// minutes (#741).
const SETTLE_DEADLINE: Duration = Duration::from_secs(20);

/// Turns a timeout into a diagnosis: slow means a contended machine, fast means the awaited report
/// was never going to arrive.
fn bridge_latency(composer: &composer::Composer) -> Duration {
    let start = Instant::now();
    composer.test_body_eval("'ping'");
    start.elapsed()
}

fn settle(composer: &composer::Composer, what: &str, done: impl Fn() -> bool) {
    let limit = postio_test_support::scaled(SETTLE_DEADLINE);
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        while glib::MainContext::default().iteration(false) {}
        if done() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if done() {
        return;
    }
    let bridge = bridge_latency(composer);
    if bridge > Duration::from_secs(1) {
        panic!(
            "timed out waiting for {what} after {limit:?} -- but a trivial \
             bridge round trip alone just took {bridge:?}, so this machine is \
             contended and the wait above was racing load, not a real regression; \
             re-run somewhere quieter"
        );
    }
    panic!(
        "timed out waiting for {what} after {limit:?} -- the bridge answered a \
         trivial round trip in {bridge:?} once the wait was over, so it is alive \
         now. That is one sample taken after the fact and it does not establish \
         that the report never came: a starved web process can be idle and \
         responsive the moment it is finally asked. Raise POSTIO_TEST_PATIENCE \
         and see whether it still fails before concluding anything (#957)"
    );
}

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

    let toolbar = find(
        composer.upcast_ref::<gtk::Widget>(),
        "postio-compose-toolbar",
    )
    .expect("the composer builds the toolbar unconditionally");
    assert!(toolbar.is_visible());

    let bold = toggle(&composer, "postio-toolbar-bold");
    let italic = toggle(&composer, "postio-toolbar-italic");
    let heading = toggle(&composer, "postio-toolbar-heading");
    // Postio's own glyph (#1485): Adwaita draws no heading, and a name the theme cannot find shows
    // the missing-image icon.
    let glyph = heading.icon_name().expect("the Heading toggle has an icon");
    assert!(
        gtk::IconTheme::for_display(&display).has_icon(&glyph),
        "the icon theme cannot find {glyph}"
    );
    let bullet = toggle(&composer, "postio-toolbar-bullet-list");
    let numbered = toggle(&composer, "postio-toolbar-numbered-list");
    let quote = toggle(&composer, "postio-toolbar-quote-block");
    let link: gtk::Button = find(composer.upcast_ref::<gtk::Widget>(), "postio-toolbar-link")
        .expect("the toolbar carries the link button")
        .downcast()
        .expect("the link button is a plain Button — a dialog, not a state");

    for button in [
        bold.clone().upcast::<gtk::Widget>(),
        italic.clone().upcast(),
        heading.clone().upcast(),
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

    // Focus the body first: WebKit only arms editing commands in a focused frame.
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
    assert!(!heading.is_active());
    assert!(!bullet.is_active());
    assert!(!numbered.is_active());
    assert!(!quote.is_active());

    // A click flips a ToggleButton itself, so clicking proves nothing about reflection; moving the
    // caret does.
    composer.test_select_body(0, 0, 2);
    settle(
        &composer,
        "the bold toggle to clear once the caret leaves Strong",
        || !bold.is_active(),
    );
    composer.test_select_body(1, 1, 3);
    settle(&composer, "the bold toggle to light inside Strong", || {
        bold.is_active()
    });

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

    quote.emit_clicked();
    settle(&composer, "the quote block to land", || {
        matches!(composer.document().blocks.first(), Some(Block::Quote(_)))
    });
}

pub fn the_toolbar_offers_attaching_a_file() {
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

    let attach = find(&composer.clone().upcast(), "postio-toolbar-attach")
        .expect("the composer toolbar has an attach control");
    assert!(
        attach.is_visible(),
        "and it is on screen with the rest of them"
    );

    let tooltip = attach
        .tooltip_text()
        .map(|text| text.to_string())
        .unwrap_or_default();
    assert!(
        tooltip.starts_with(postio_core::registry::get(postio_core::CommandId::AttachFile).title),
        "the tooltip names the command from the registry, so the button and \
         the key cannot come to say different things: {tooltip:?}"
    );
    assert!(
        tooltip.contains('+'),
        "and it says which key, which is the whole point of adding it: \
         {tooltip:?}"
    );
}
