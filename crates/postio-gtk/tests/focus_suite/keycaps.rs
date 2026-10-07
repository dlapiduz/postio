//! US7: every button that runs a command carries its keycap -- the toast's
//! Undo, the banner's Retry now, the page that says why there is no mail,
//! Settings' Add account -- so the app teaches the keyboard on the control
//! a person is looking at. The key is the keymap's, never a literal.

use gtk::prelude::*;
use postio_core::{CommandId, ConnectionState, Event};

use crate::support::{self, Fixture};

/// The mapped keycaps (`postio-keyhint`) under `button`, as they read.
fn caps(button: &gtk::Widget) -> Vec<String> {
    support::with_class(button, "postio-keyhint")
        .into_iter()
        .filter(|cap| cap.is_mapped())
        .filter_map(|cap| cap.downcast::<gtk::Label>().ok())
        .map(|cap| cap.label().to_string())
        .collect()
}

/// The key the window's keymap gives `command`.
fn bound(window: &postio_gtk::window::FocusWindow, command: CommandId) -> String {
    window
        .keymap()
        .binding(command)
        .unwrap_or_else(|| panic!("{command:?} has no binding"))
        .to_owned()
}

pub fn the_undo_toast_names_the_key_that_undoes() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window) = support::three_in_the_inbox().await;
        support::keys(&window, &["j", "a"]);
        assert!(
            crate::settle_until(async || window.toast_showing().is_some()).await,
            "archiving raised no toast"
        );
        let undo = support::button_labelled(&window, "Undo");
        assert!(
            crate::settle_until(async || caps(&undo) == [bound(&window, CommandId::Undo)]).await,
            "the toast's Undo shows {:?}, not the key that undoes",
            caps(&undo)
        );
    });
}

pub fn the_offline_banner_names_the_key_that_retries() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let fixture = Fixture::empty().await;
        fixture
            .file(("Ada Moreno", "ada@example.com"), "Budget", "Numbers.", 5)
            .await;
        let (host, sink) = fixture.host_telling();
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        support::keep(postio_gtk::startup::adopt(
            &window,
            host,
            &postio_config::Config::default(),
        ));
        assert!(
            crate::settle_until(async || !window.rows_on_screen().is_empty()).await,
            "the inbox never reached the screen"
        );
        assert!(sink.emit(Event::ConnectionChanged {
            account: fixture.account.id,
            state: ConnectionState::Offline,
        }));
        assert!(
            crate::settle_until(async || window.banner_showing().is_some()).await,
            "no offline banner"
        );
        let retry = support::button_labelled(&window, "Retry now");
        assert!(
            crate::settle_until(async || caps(&retry) == [bound(&window, CommandId::Refresh)])
                .await,
            "Retry now shows {:?}, not the key that retries",
            caps(&retry)
        );
    });
}

pub fn try_again_names_the_key_that_runs_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = postio_gtk::window::FocusWindow::new(None);
        window.present();
        window.show_unavailable("Another Postio has the store open.", || {});
        crate::settle();
        let retry = support::button_labelled(&window, "Try again");
        assert_eq!(
            caps(&retry),
            ["Return"],
            "Try again holds the keyboard, and Return answers it"
        );
    });
}

pub fn settings_add_account_names_its_key() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let (_fixture, window, _directory, _path) =
            crate::settings::one_message_under("").await;
        support::deliver_with(&window, "comma", gtk::gdk::ModifierType::CONTROL_MASK);
        let dialog = crate::settings::settings_shown(&window)
            .await
            .expect("mod+comma opened Settings");
        let add = support::button_labelled(&dialog, "Add account");
        assert!(
            crate::settle_until(async || caps(&add) == [bound(&window, CommandId::AddAccount)])
                .await,
            "Add account shows {:?}, not its key",
            caps(&add)
        );
    });
}
