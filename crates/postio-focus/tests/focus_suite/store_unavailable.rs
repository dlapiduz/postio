//! The page that says why there is no mail (T215, T216): it can always be
//! closed, and it offers the way forward that can work -- "Try again" for
//! what can pass, a fresh store for a store no migration reaches, which the
//! same file will refuse however often it is tried.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_focus::startup::{Opener, Session};
use postio_focus::window::FocusWindow;
use postio_storage::key::{Purpose, StoreKey};

use crate::support;

/// A window showing why there is no mail, as `startup::open` leaves it when
/// another Postio has the store.
fn refused() -> FocusWindow {
    let window = FocusWindow::new(None);
    window.present();
    window.show_unavailable(
        "Postio is already open in another window. Close it to open Postio here.",
        || {},
    );
    crate::settle();
    window
}

/// T216: the window's close button is there on the page that says why there
/// is no mail, and a click on it -- delivered as a person's is -- closes the
/// window. There was no top bar at all until the inbox had opened, so the
/// one way out was the keyboard, and that did nothing either.
pub fn the_close_button_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = refused();
        let close = support::only(&window, "focus-close");
        assert!(
            close.is_drawable(),
            "the close button is on the page that says why there is no mail"
        );
        let bounds = close
            .compute_bounds(&window)
            .expect("the close button has a place in the window");
        support::click_at(
            &window,
            f64::from(bounds.x() + bounds.width() / 2.0),
            f64::from(bounds.y() + bounds.height() / 2.0),
            1,
        );
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "the close button did not close a window that has no mail to show"
        );
    });
}

/// T216: `Ctrl+Q` quits from the page that says why there is no mail, as it
/// does from the inbox: the registry's Quit, with no inbox behind it.
pub fn ctrl_q_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = refused();
        support::deliver_with(&window, "q", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "Ctrl+Q did nothing on the page that says why there is no mail"
        );
    });
}

/// T216: `Ctrl+W` closes the window, as it does in every GNOME app; Focus has
/// one window, so closing it is quitting.
pub fn ctrl_w_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let window = refused();
        support::deliver_with(&window, "w", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "Ctrl+W did nothing on the page that says why there is no mail"
        );
    });
}

/// A keyring holding `key` as the store key.
fn keyring(key: &StoreKey) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(key.to_hex().as_str()),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

/// T215: a store whose schema no migration reaches is not offered "Try
/// again", which would meet the same file every time. The page says what
/// happened, what a fresh store keeps and what stays behind, and "Start a
/// fresh store" sets the old one aside -- not deleted -- and opens a fresh
/// one with the account still in it.
pub fn a_store_no_migration_reaches_offers_a_fresh_store_that_keeps_the_account() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a scratch directory");
        let store = directory.path().join("postio.db");
        let key = StoreKey::generate();
        // A store some build wrote whose schema this one has no record of:
        // this build's tables, under a stamp no migration leads from.
        let address = {
            let earlier = postio_storage::Store::open(&store, &key.derive(Purpose::Database))
                .await
                .expect("a store");
            let connection = earlier.connect().await.expect("a connection");
            let account = postio_storage::test_support::account(&connection).await;
            connection
                .execute("PRAGMA user_version = 1", ())
                .await
                .expect("the stamp is writable");
            account.address.address
        };

        let window = FocusWindow::new(None);
        window.present();
        let opened: Rc<RefCell<Option<Session>>> = Rc::default();
        let opener = Opener::at(None, store.clone(), keyring(&key));
        postio_focus::startup::open(
            &window,
            opener.open_on_a_thread(),
            Rc::new(postio_config::Config::default()),
            opener,
            {
                let opened = Rc::clone(&opened);
                Rc::new(move |session| {
                    opened.replace(Some(session));
                })
            },
        );
        assert!(
            crate::settle_until(async || window.unavailable_reason().is_some()).await,
            "a store no migration reaches opened, or the window said nothing"
        );
        let said = support::texts(&window);
        assert!(
            !said.contains(&"Try again".to_owned()),
            "trying again meets the same file: {said:?}"
        );
        let reason = window.unavailable_reason().expect("the page says why");
        for promise in ["accounts", "settings", "set aside", "Snoozes", "sync"] {
            assert!(
                reason.contains(promise),
                "the page has to say what a fresh store keeps, what stays \
                 behind and where it goes before it is chosen; no {promise:?} in \
                 {reason:?}"
            );
        }

        let fresh = support::button_labelled(&window, "Start a fresh store");
        support::click(&window, &fresh, 1);
        assert!(
            crate::settle_until(async || opened.borrow().is_some()).await,
            "a fresh store did not open: {:?}",
            window.unavailable_reason()
        );

        let set_aside: Vec<_> = std::fs::read_dir(directory.path().join("set-aside"))
            .expect("the old store was set aside, not deleted")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        assert_eq!(set_aside.len(), 1, "{set_aside:?}");
        assert!(
            set_aside[0].join("postio.db").exists(),
            "the old database is in {}",
            set_aside[0].display()
        );

        let session = opened.take().expect("the session");
        let accounts = session
            .client()
            .accounts()
            .await
            .expect("the accounts read");
        assert_eq!(
            accounts
                .iter()
                .map(|account| account.address.address.clone())
                .collect::<Vec<_>>(),
            vec![address],
            "the fresh store keeps the account, so it syncs rather than asking"
        );
        session.stop();
    });
}
