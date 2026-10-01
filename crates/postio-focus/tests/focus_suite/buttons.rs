//! T175, T177: a button is as tall as the design draws it, and a keycap is a
//! small hint inside its button -- not a second box that fills the button's
//! height and doubles it. Measured on the screen: what a person sees is the
//! allocated size, so that is what is read.

use adw::prelude::*;
use gtk::gdk;

use crate::support::{self, Fixture};

/// The design's action buttons (screens 04 and 05) are 26px tall; a
/// regular button 30. Nothing in Focus is taller than a regular button.
const REGULAR: i32 = 30;
const TOOLBAR: i32 = 26;
/// A keycap is 16px in the design, and never as tall as its button.
const KEYCAP: i32 = 18;

/// Every visible keycap under `button`.
fn keycaps(button: &gtk::Widget) -> Vec<gtk::Widget> {
    ["postio-keyhint", "postio-key"]
        .iter()
        .flat_map(|class| support::with_class(button, class))
        .filter(|cap| cap.is_mapped())
        .collect()
}

/// `button` is a compact button carrying a compact keycap.
fn assert_compact(what: &str, button: &gtk::Widget, at_most: i32) {
    assert!(
        button.height() <= at_most,
        "{what} is {}px tall, over the design's {at_most}px",
        button.height()
    );
    for cap in keycaps(button) {
        assert!(
            cap.height() <= KEYCAP,
            "{what}'s keycap is {}px tall: a box inside the button, not a hint",
            cap.height()
        );
        assert!(
            cap.height() < button.height(),
            "{what}'s keycap fills the button ({}px)",
            cap.height()
        );
    }
}

pub fn the_open_message_toolbar_is_compact_with_its_keycaps_inside() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        fixture.write_body(message, "The body").await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || support::subjects(&window).len() == 1).await);
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        assert!(
            crate::settle_until(async || reading.body_text().contains("The body")).await,
            "the body never arrived"
        );
        let dialog = reading.dialog();
        let shown = |dialog: &adw::Dialog| -> Vec<gtk::Widget> {
            support::with_class(dialog, "postio-keycap-button")
                .into_iter()
                .filter(|button| button.is_mapped() && button.height() > 1)
                .collect()
        };
        assert!(
            crate::settle_until(async || shown(&dialog).len() >= 6).await,
            "the toolbar was never drawn"
        );
        let buttons = shown(&dialog);
        assert!(
            buttons.len() >= 6,
            "the toolbar's buttons: {}",
            buttons.len()
        );
        for button in &buttons {
            assert!(
                !keycaps(button).is_empty(),
                "a toolbar button without its key"
            );
            assert_compact("a toolbar button", button, TOOLBAR);
        }
        // T179: Delete is there beside the rest, with its key.
        let delete = support::only(&dialog, "focus-open-delete");
        assert!(!keycaps(&delete).is_empty(), "Delete shows no key");
        // T189: Close is an X icon now, and carries no keycap.
        for close in support::with_class(&dialog, "focus-open-close") {
            assert_compact("Close", &close, REGULAR);
        }
    });
}

pub fn the_compose_dialog_draws_every_button_one_way() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        let (window, _client) = fixture.open().await;
        assert!(
            crate::settle_until(async || window.composer().is_some()).await,
            "no composer was mounted"
        );
        support::keys(&window, &["c"]);
        assert!(
            crate::settle_until(async || window.compose_dialog().is_some()).await,
            "c opened no composer"
        );
        let dialog = window.compose_dialog().expect("the compose dialog");
        crate::settle();
        // T192: Close is the shared X icon, with no keycap.
        assert_compact(
            "focus-compose-close",
            &support::only(&dialog, "focus-compose-close"),
            REGULAR,
        );
        for class in [
            "focus-compose-send-later",
            "focus-compose-send",
            "focus-compose-attach",
            "focus-compose-remind",
        ] {
            let button = support::only(&dialog, class);
            crate::settle_until(async || !keycaps(&button).is_empty()).await;
            assert!(
                !keycaps(&button).is_empty(),
                "{class} has no keycap inside it"
            );
            assert_compact(class, &button, REGULAR);
        }
    });
}
