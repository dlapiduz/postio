use std::time::{Duration, Instant};

use crate::support_compose::Window;
use crate::support_compose::composer::{self, Field};
use crate::support_compose::{fonts, style};
use gtk::gdk;
use postio_model::{AccountId, Draft, DraftKind, EmailAddress, MessageBody};

fn started(kind: DraftKind) -> Draft {
    let mut draft = Draft::new(AccountId::UNASSIGNED);
    draft.kind = kind;
    draft.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    draft.subject = "the mbox importer".to_owned();
    draft.body = MessageBody {
        text: Some("Looking now.".to_owned()),
        html: None,
    };
    draft
}

fn quiet_for(span: Duration) {
    let context = glib::MainContext::default();
    let deadline = Instant::now() + postio_test_support::scaled(span);
    while Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn settle_until(done: impl Fn() -> bool) {
    let context = glib::MainContext::default();
    let heartbeat =
        glib::timeout_add_local(Duration::from_millis(5), || glib::ControlFlow::Continue);
    let deadline = Instant::now() + postio_test_support::scaled(Duration::from_millis(3000));
    while !done() && Instant::now() < deadline {
        context.iteration(true);
    }
    heartbeat.remove();
}

pub fn focus_lands_when_the_composer_opens_before_the_window_is_ever_mapped() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = composer::install(&window);

    // Nothing presented yet, so nothing is realized and the first `grab_focus` cannot succeed.
    // New mail with nobody to send to starts in the body, as any addressed draft does.
    let mut fresh = started(DraftKind::New);
    fresh.to.clear();
    composer.open(fresh);

    window.present();
    settle_until(|| composer.focused_field().is_some());

    assert_eq!(
        composer.focused_field(),
        Some(Field::To),
        "new mail starts in To, once the retry has had real frames to land in"
    );

    let window = Window::default();
    let composer = composer::install(&window);
    composer.open(started(DraftKind::Reply));
    window.present();
    settle_until(|| composer.focused_field().is_some());

    assert_eq!(
        composer.focused_field(),
        Some(Field::Body),
        "a reply starts in the body, once the retry has had real frames to land in"
    );
}

/// Driven with the window's `move-focus` signal, which a Tab press is bound to: the order under
/// test is the toolkit's, not a helper's.
///
/// POSTIO-MEASUREMENT: it walks every control of the composer and settles after each,
/// 9 s on a CI runner, so it runs on the nightly timer.
pub fn the_keyboard_walks_the_composer_in_a_defined_order() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let composer = composer::install(&window);
    composer.open(started(DraftKind::New));
    window.present();
    settle_until(|| composer.focused_field().is_some());

    // Cc and Bcc join the order only once showing. `show_copy_fields` leaves the keyboard in Cc, so
    // the walk restarts from To.
    composer.show_copy_fields();
    settle_until(|| composer.focused_field() == Some(Field::Cc));

    let tab = |direction| {
        quiet_for(Duration::from_millis(400));
        gtk::glib::prelude::ObjectExt::emit_by_name::<()>(
            window.gtk(),
            "move-focus",
            &[&direction],
        );
    };

    tab(gtk::DirectionType::TabBackward);
    settle_until(|| composer.focused_field() == Some(Field::To));
    assert_eq!(
        composer.focused_field(),
        Some(Field::To),
        "shift-tabbing out of Cc reaches To, so To is the field above it"
    );

    for expected in [Field::Cc, Field::Bcc, Field::Subject] {
        tab(gtk::DirectionType::TabForward);
        settle_until(|| composer.focused_field() == Some(expected));
        assert_eq!(
            composer.focused_field(),
            Some(expected),
            "tabbing forward should reach {expected:?}"
        );
    }

    for expected in [Field::Bcc, Field::Cc, Field::To] {
        tab(gtk::DirectionType::TabBackward);
        settle_until(|| composer.focused_field() == Some(expected));
        assert_eq!(
            composer.focused_field(),
            Some(expected),
            "shift-tabbing back should reach {expected:?}"
        );
    }
}
