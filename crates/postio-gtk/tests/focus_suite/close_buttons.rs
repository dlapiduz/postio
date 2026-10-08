//! T192: one rule for every close control. Each surface that closes has
//! the one X icon button the shared constructor draws, at the right end of
//! its header, and nothing else in that header is further right or says
//! "Close".

use adw::prelude::AdwDialogExt;
use gtk::prelude::*;

use crate::support::{self, Fixture};

/// How tall a header band is: anything whose top is above this is "the
/// header" of its surface.
const BAND: f32 = 72.0;

/// The surface's one close control: it is the shared X, and it is the
/// right-most button in the top band; no other button there is worded
/// "Close".
fn assert_one_close_at_the_right(name: &str, surface: &impl IsA<gtk::Widget>) {
    let root: gtk::Widget = surface.as_ref().clone();
    let closes = support::with_class(&root, "postio-close-button");
    assert_eq!(closes.len(), 1, "{name}: exactly one close control");
    let close = closes[0].downcast_ref::<gtk::Button>().expect("a button");
    assert_eq!(
        close.icon_name().as_deref(),
        Some("window-close-symbolic"),
        "{name}: the close is an X icon"
    );
    assert_eq!(close.tooltip_text().as_deref(), Some("Close"), "{name}");
    let bounds = |widget: &gtk::Widget| widget.compute_bounds(&root).expect("laid out");
    let at = bounds(close.upcast_ref());
    assert!(at.width() > 0.0, "{name}: the close was never laid out");
    for widget in support::descendants(&root) {
        let Some(button) = widget.downcast_ref::<gtk::Button>() else {
            continue;
        };
        if button == close || !button.is_mapped() {
            continue;
        }
        let there = bounds(button.upcast_ref());
        if there.y() > BAND || there.width() <= 0.0 {
            continue;
        }
        assert!(
            there.x() < at.x(),
            "{name}: a header button ({:?}) is right of Close",
            button.icon_name()
        );
        let said = support::texts(button);
        assert!(
            !said.iter().any(|text| text == "Close"),
            "{name}: a second, worded Close: {said:?}"
        );
    }
}

pub fn every_closable_surface_has_the_same_x_at_the_right() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        let (message, _) = fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "x", 10)
            .await;
        fixture.write_body(message, "A body.").await;
        let (window, _client) = fixture.open().await;
        assert!(crate::settle_until(async || support::subjects(&window).len() == 1).await);
        window.set_default_size(1000, 640);
        crate::settle();

        // The window's own top bar.
        assert_one_close_at_the_right("the window", &window);

        // The key map.
        let keys = postio_gtk::keymap_dialog::build(&window.keymap());
        keys.present(Some(&window));
        assert!(
            crate::settle_until(async || support::with_class(&keys, "postio-close-button")
                .first()
                .is_some_and(|close| close.width() > 0))
            .await
        );
        assert_one_close_at_the_right("the key map", &keys);
        keys.close();
        crate::settle();

        // The raw source.
        let source = postio_gtk::source::dialog(b"From: a@example.com\r\n\r\nx", &window.keymap());
        source.present(Some(&window));
        assert!(
            crate::settle_until(async || support::with_class(&source, "postio-close-button")
                .first()
                .is_some_and(|close| close.width() > 0))
            .await
        );
        assert_one_close_at_the_right("the raw source", &source);
        source.close();
        crate::settle();

        // The open message.
        support::keys(&window, &["j"]);
        support::press(&window, "Return", gtk::gdk::ModifierType::empty());
        let reading = window.reading().expect("Enter opened the message");
        let dialog = reading.dialog();
        assert!(
            crate::settle_until(async || support::with_class(&dialog, "postio-close-button")
                .first()
                .is_some_and(|close| close.width() > 0))
            .await
        );
        assert_one_close_at_the_right("the open message", &dialog);
        reading.close();
        crate::settle();

        // The composer.
        support::keys(&window, &["c"]);
        assert!(crate::settle_until(async || window.compose_dialog().is_some()).await);
        let compose = window.compose_dialog().expect("the compose dialog");
        assert!(
            crate::settle_until(
                async || support::with_class(&compose, "postio-close-button")
                    .first()
                    .is_some_and(|close| close.width() > 0)
            )
            .await
        );
        assert_one_close_at_the_right("the composer", &compose);
    });
}

pub fn the_digest_has_the_same_x_at_the_right() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (fixture, _, _) = crate::digest::delivered_holding().await;
        let (window, _client) = fixture.open().await;
        let digest = crate::digest::open_digest(&window).await;
        let dialog = digest.dialog().clone();
        assert!(
            crate::settle_until(async || support::with_class(&dialog, "postio-close-button")
                .first()
                .is_some_and(|close| close.width() > 0))
            .await
        );
        assert_one_close_at_the_right("the digest", &dialog);
    });
}

pub fn settings_has_the_same_x_at_the_right() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, _path) = crate::settings::one_message_under("").await;
        support::deliver_with(&window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
        let dialog = crate::settings::settings_shown(&window)
            .await
            .expect("Settings opened");
        assert!(
            crate::settle_until(async || support::with_class(&dialog, "postio-close-button")
                .first()
                .is_some_and(|close| close.width() > 0))
            .await
        );
        assert_one_close_at_the_right("Settings", &dialog);
    });
}

/// T216, again for the form that adds an account: each of its three steps
/// ends its header in the shared X, and pressing it closes the dialog.
pub fn add_account_has_the_same_x_at_the_right_on_every_step() {
    use postio_widgets::onboarding::{Onboarding, Status};

    fn form_in(widget: &gtk::Widget) -> Option<Onboarding> {
        support::descendants(widget)
            .into_iter()
            .find_map(|widget| widget.downcast::<Onboarding>().ok())
    }

    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::deliver_with(
            &window,
            "N",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(crate::settle_until(async || window.add_account_dialog().is_some()).await);
        let dialog = window.add_account_dialog().expect("the dialog");
        let form = form_in(dialog.upcast_ref()).expect("the account form");
        for (step, status) in [
            ("1 / 3", Status::Idle),
            ("2 / 3", Status::WaitingForBrowser),
            ("3 / 3", Status::SyncWindow),
        ] {
            form.set_status(status);
            assert!(
                crate::settle_until(async || support::with_class(&dialog, "postio-close-button")
                    .first()
                    .is_some_and(|close| close.width() > 0))
                .await,
                "step {step} has no close button"
            );
            assert_one_close_at_the_right(&format!("add account, step {step}"), &dialog);
        }

        let close = support::with_class(&dialog, "postio-close-button")
            .into_iter()
            .next()
            .and_downcast::<gtk::Button>()
            .expect("the close button");
        close.emit_clicked();
        assert!(
            crate::settle_until(async || window.add_account_dialog().is_none()).await,
            "the close button left the add-account dialog open"
        );
    });
}

/// The add-account form opens with the keyboard in its first field, not on
/// its close X: the X closes with the mouse, and Escape is its key. Putting
/// the X first in the header put it first in the dialog's focus order, so the
/// form opened with the keyboard on the one control that throws it away.
pub fn add_account_opens_with_the_keyboard_in_its_first_field() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::deliver_with(
            &window,
            "N",
            gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK,
        );
        assert!(crate::settle_until(async || window.add_account_dialog().is_some()).await);
        let in_a_field =
            || gtk::prelude::RootExt::focus(&window).is_some_and(|focus| focus.is::<gtk::Text>());
        assert!(
            crate::settle_until(async || in_a_field()).await,
            "the form opened with the keyboard on {:?}",
            gtk::prelude::RootExt::focus(&window).map(|focus| focus.type_().name())
        );

        // Waiting for the browser there is nothing to type: the keyboard
        // goes to Cancel sign-in, still not to the X.
        let dialog = window.add_account_dialog().expect("the dialog");
        let form = support::descendants(dialog.upcast_ref::<gtk::Widget>())
            .into_iter()
            .find_map(|widget| {
                widget
                    .downcast::<postio_widgets::onboarding::Onboarding>()
                    .ok()
            })
            .expect("the account form");
        form.set_status(postio_widgets::onboarding::Status::WaitingForBrowser);
        let on_cancel = || {
            gtk::prelude::RootExt::focus(&window).is_some_and(|focus| {
                let button = if focus.is::<gtk::Button>() {
                    Some(focus)
                } else {
                    focus.ancestor(gtk::Button::static_type())
                };
                button.is_some_and(|button| {
                    support::texts(&button)
                        .iter()
                        .any(|text| text == "Cancel sign-in")
                })
            })
        };
        assert!(
            crate::settle_until(async || on_cancel()).await,
            "waiting for the browser, the keyboard is on {:?}",
            gtk::prelude::RootExt::focus(&window).map(|focus| focus.type_().name())
        );
    });
}
