//! The composer's recipient chips (specs/007-postio-focus T079, US3
//! scenario 6): an opt-in presentation of `To`, `Cc` and `Bcc`, which Focus
//! turns on. Choosing a suggestion adds a chip with the name and the
//! address; the × on a chip takes that recipient off the message.
//!
//! Asserted on the widget tree -- the chip's words, the field's text -- and
//! on the draft the composer would hand to a send.

use gtk::prelude::*;
use postio_model::EmailAddress;
use postio_widgets::composer::{Composer, RecipientCandidate};

use crate::support;

/// Every widget under `root` wearing `class`, in tree order.
fn with_class(root: &gtk::Widget, class: &str) -> Vec<gtk::Widget> {
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if widget.has_css_class(class) {
            found.push(widget.clone());
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    found
}

/// The words a label-bearing widget shows, in tree order.
fn words(root: &gtk::Widget) -> Vec<String> {
    let mut said = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(widget) = stack.pop() {
        if let Some(label) = widget.downcast_ref::<gtk::Label>()
            && !label.text().is_empty()
        {
            said.push(label.text().to_string());
        }
        let mut children = Vec::new();
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            children.push(next);
        }
        stack.extend(children.into_iter().rev());
    }
    said
}

fn grace() -> EmailAddress {
    EmailAddress::new(Some("Grace Oyelaran"), "grace@example.org")
}

/// A composer with chips on, suggesting Grace and Graham, in a window.
fn composing() -> (gtk::Window, Composer) {
    let composer = Composer::new();
    composer.set_recipient_chips(true);
    composer.connect_recipient_suggestions(|prefix| {
        [
            grace(),
            EmailAddress::new(Some("Graham Ellis"), "graham@example.net"),
        ]
        .into_iter()
        .filter(|address| {
            address
                .name
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .starts_with(&prefix.to_lowercase())
        })
        .map(RecipientCandidate::Contact)
        .collect()
    });
    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&composer));
    window.present();
    composer.set_visible(true);
    (window, composer)
}

pub fn choosing_a_suggestion_adds_a_chip_with_the_name_and_the_address() {
    if adw::init().is_err() {
        return;
    }
    let (_window, composer) = composing();
    // Four characters: the one completion rule opens at four (spec C23).
    composer.test_set_to("Grac");
    assert!(
        support::until(|| composer.test_recipient_popover_visible()),
        "no suggestions for \"Grac\""
    );
    assert!(
        composer.test_accept_recipient_suggestion(),
        "nothing chosen"
    );
    let root: gtk::Widget = composer.clone().upcast();
    assert!(
        support::until(|| with_class(&root, "postio-recipient-chip").len() == 1),
        "choosing a suggestion drew no chip"
    );
    let chip = with_class(&root, "postio-recipient-chip").remove(0);
    assert_eq!(
        words(&chip),
        ["Grace Oyelaran", "grace@example.org"],
        "the chip shows the name and the address"
    );
    assert_eq!(
        composer.draft().to,
        [grace()],
        "and the message goes to her"
    );
    assert!(
        !composer.test_recipient_popover_visible(),
        "the suggestions close once one is chosen"
    );
}

pub fn a_chip_s_remove_button_takes_the_recipient_off() {
    if adw::init().is_err() {
        return;
    }
    let (_window, composer) = composing();
    composer.test_set_to("Grace Oyelaran <grace@example.org>, ");
    let root: gtk::Widget = composer.clone().upcast();
    assert!(
        support::until(|| with_class(&root, "postio-recipient-chip").len() == 1),
        "a typed, finished address became no chip"
    );
    let chip = with_class(&root, "postio-recipient-chip").remove(0);
    let remove = with_class(&chip, "postio-recipient-chip-remove")
        .into_iter()
        .next()
        .and_downcast::<gtk::Button>()
        .expect("the chip's ×");
    remove.emit_clicked();
    assert!(
        support::until(|| with_class(&root, "postio-recipient-chip").is_empty()),
        "the chip stayed"
    );
    assert!(composer.draft().to.is_empty(), "and so did the recipient");
}
