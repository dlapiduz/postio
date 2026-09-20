//! A composer built after a rebind starts on the rebound key (#828).
//!
//! One `#[test]`, alone in its binary, on purpose. It calls
//! `Window::composer`, which installs a WebKit editor, and
//! `tests/gtk_suite/` deliberately keeps WebKit out of the shared-process
//! suite — `gtk_reader` is excluded for the same reason (#272). Putting it
//! there destabilised the suite: `gtk_display_required`, which runs later and
//! asserts CI has a display at all, began failing.
//!
//! Its sibling case — that applying a keymap does *not* build a composer —
//! needs no WebKit and lives in `tests/gtk_suite/gtk_keymap_lazy.rs`.
//!
//! Skips without a display.

use gtk::gdk;
use postio_config::KeyBindings;
use postio_core::Keymap;
use postio_gtk::window::Window;
use postio_gtk::{fonts, style};

pub fn a_composer_built_after_a_rebind_starts_on_the_rebound_key() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    // The other half of keeping it lazy: `apply_keymap` cannot reach a
    // composer that does not exist, so the composer has to pick the keymap up
    // when it is finally built -- or a rebind made before anyone composed
    // would be invisible until the next edit of `config.toml`.
    let window = Window::default();
    let mut overrides = KeyBindings::default();
    overrides
        .overrides_mut()
        .insert("save_draft".to_string(), "mod+w".to_string());
    window.apply_keymap(Keymap::resolve(&overrides));

    let composer = window.composer();
    assert!(window.has_composer(), "asking for it builds it");
    assert_eq!(
        composer.test_save_hint(),
        Some("ctrl+w".to_string()),
        "the composer was built after the rebind and missed it"
    );
}

/// Typing wins: a single-key binding does not fire inside a text field.
///
/// FR-004, and #602 is what it costs when it does — `e` ran *reply* instead of
/// typing an `e`, so a half-written reply answered itself. The rule is two
/// halves (`Window::is_typing` and the resolver) and neither had a test.
///
/// Asserted on both kinds of field, because they are answered by different
/// code. A subject is a `GtkText`, caught by a type test; the body is a
/// `WebView` over a `contenteditable` document, which that test calls "not
/// typing" — so the body is the case #602 actually was, and the one a type
/// test alone would miss.
pub fn a_single_key_binding_does_not_fire_while_typing() {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display");
        return;
    }
    let display = gdk::Display::default().expect("a display");
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    let fired: std::rc::Rc<std::cell::RefCell<Vec<postio_core::CommandId>>> =
        std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    window.connect_command({
        let fired = std::rc::Rc::clone(&fired);
        move |id| fired.borrow_mut().push(id)
    });

    // `Window::composer`, not `composer::install`: only the former records the
    // composer in the window's slot, and `is_typing` asks that slot. Installed
    // the other way the window cannot see its own composer, and every key
    // would look like a command -- which is this test failing for its own
    // reasons rather than the app's.
    let composer = window.composer();
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    window.present();
    settle();

    // `e` is Reply, `a` is Archive, `c` is Compose: three verbs a person
    // writing prose types constantly.
    let typed = ["e", "a", "c"];

    for field in [
        postio_gtk::composer::Field::Subject,
        postio_gtk::composer::Field::Body,
    ] {
        assert!(
            composer.test_focus_field(field),
            "the test needs the keyboard in {field:?} to say anything"
        );
        settle();
        assert_eq!(
            composer.focused_field(),
            Some(field),
            "the keyboard did not land in {field:?}"
        );

        fired.borrow_mut().clear();
        for key in typed {
            window.handle_key(
                gdk::Key::from_name(key).unwrap(),
                gdk::ModifierType::empty(),
            );
        }
        settle();

        assert!(
            fired.borrow().is_empty(),
            "typing {typed:?} into {field:?} ran {:?}. #602 is this exact \
             shape: the letter becomes a verb and a half-written reply \
             answers itself",
            fired.borrow()
        );
    }
}

/// Drives idle and frame-clock sources so a grab and a key both land.
fn settle() {
    let context = gtk::glib::MainContext::default();
    let heartbeat = gtk::glib::timeout_add_local(std::time::Duration::from_millis(5), || {
        gtk::glib::ControlFlow::Continue
    });
    let deadline = std::time::Instant::now()
        + postio_test_support::scaled(std::time::Duration::from_millis(600));
    while std::time::Instant::now() < deadline {
        context.iteration(false);
    }
    heartbeat.remove();
}
