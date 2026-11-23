use crate::support_compose::settle;
use std::cell::Cell;
use std::rc::Rc;

use crate::support_compose::Window;
use crate::support_compose::composer::{self, Composer, RecipientCandidate};
use crate::support_compose::{fonts, style};
use gtk::gdk;
use gtk::prelude::*;
use postio_model::EmailAddress;

pub fn destroying_a_composer_releases_recipient_completion() {
    fn completion(widget: &gtk::Widget) -> Option<gtk::Widget> {
        if widget.has_css_class("postio-recipient-completion") {
            return Some(widget.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            if let Some(found) = completion(&current) {
                return Some(found);
            }
            child = current.next_sibling();
        }
        None
    }

    let window = Window::default();
    let weak_window = window.downgrade();
    let composer = window.composer();
    let weak_composer = composer.downgrade();
    let popover = completion(composer.upcast_ref()).expect("recipient completion is mounted");
    let weak_popover = popover.downgrade();
    let weak_list = popover
        .first_child()
        .expect("completion has a list")
        .downgrade();
    let weak_entry = popover.parent().expect("popover has an entry").downgrade();

    window.destroy();
    drop(popover);
    drop(composer);
    drop(window);
    settle();

    assert!(weak_window.upgrade().is_none(), "window stayed alive");
    assert!(weak_composer.upgrade().is_none(), "composer stayed alive");
    assert!(weak_entry.upgrade().is_none(), "entry stayed alive");
    assert!(weak_popover.upgrade().is_none(), "popover stayed alive");
    assert!(
        weak_list.upgrade().is_none(),
        "completion list stayed alive"
    );
}

fn grace() -> EmailAddress {
    EmailAddress::new(Some("Grace Hopper"), "grace@example.com")
}

fn graham() -> EmailAddress {
    EmailAddress::new(Some("Graham Bell"), "graham@example.net")
}

fn a_composer_offering_two() -> Option<(Window, Composer, Rc<Cell<usize>>)> {
    if adw::init().is_err() || gdk::Display::default().is_none() {
        eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
        return None;
    }
    let display = gdk::Display::default().unwrap();
    fonts::install().expect("the embedded fonts should install");
    style::install(&display);

    let window = Window::default();
    window.present();
    settle();

    let composer = composer::install(&window);
    composer.open(postio_model::Draft::new(
        postio_model::AccountId::UNASSIGNED,
    ));
    settle();

    let asked = Rc::new(Cell::new(0usize));
    composer.connect_recipient_suggestions({
        let asked = Rc::clone(&asked);
        move |prefix| {
            asked.set(asked.get() + 1);
            // Answers every prefix of "graham", so anything not offered is the gate's doing.
            if "graham".starts_with(prefix) {
                vec![
                    RecipientCandidate::Contact(grace()),
                    RecipientCandidate::Contact(graham()),
                ]
            } else {
                Vec::new()
            }
        }
    });

    Some((window, composer, asked))
}

/// Row 1, not row 0: row 0 is the default selection, so a handler ignoring the click would produce
/// it too.
pub fn clicking_a_suggestion_puts_that_one_in_the_field() {
    let Some((_window, composer, _asked)) = a_composer_offering_two() else {
        return;
    };

    composer.test_set_to("grah");
    settle();
    assert!(
        composer.test_recipient_popover_visible(),
        "four characters should have offered Grace and Graham"
    );
    assert_eq!(composer.test_recipient_suggestion_count(), 2);

    assert!(
        composer.test_click_recipient_suggestion(1),
        "there should be a second row to click"
    );
    settle();

    assert_eq!(
        composer.draft().to,
        vec![graham()],
        "clicking the second suggestion should commit the second suggestion"
    );
    assert!(
        !composer.test_recipient_popover_visible(),
        "committing a suggestion closes the popover"
    );
}

pub fn return_commits_the_suggestion_the_popover_has_selected() {
    let Some((_window, composer, _asked)) = a_composer_offering_two() else {
        return;
    };

    composer.test_set_to("grah");
    settle();
    assert!(composer.test_recipient_popover_visible());

    // Down first, so this cannot pass by accepting the default row.
    assert!(
        composer.test_press_recipient_key(gdk::Key::Down),
        "Down belongs to the popover while it is open"
    );
    assert!(
        composer.test_press_recipient_key(gdk::Key::Return),
        "Return belongs to the popover while it is open"
    );
    settle();

    assert_eq!(
        composer.draft().to,
        vec![graham()],
        "Return should commit the row Down moved to"
    );
    assert!(!composer.test_recipient_popover_visible());
}
