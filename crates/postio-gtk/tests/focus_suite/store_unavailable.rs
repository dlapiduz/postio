use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_gtk::startup::{Opener, Session};
use postio_gtk::window::FocusWindow;
use postio_storage::key::{Purpose, StoreKey};

use crate::support;

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

/// There was no top bar until the inbox had opened, so the only way out was the keyboard, which did
/// nothing either.
pub fn the_close_button_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
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

pub fn ctrl_q_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
        let window = refused();
        support::deliver_with(&window, "q", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "Ctrl+Q did nothing on the page that says why there is no mail"
        );
    });
}

pub fn ctrl_w_closes_the_window_when_the_store_will_not_open() {
    crate::gtk_case(async {
        let window = refused();
        support::deliver_with(&window, "w", gtk::gdk::ModifierType::CONTROL_MASK);
        assert!(
            crate::settle_until(async || !window.is_visible()).await,
            "Ctrl+W did nothing on the page that says why there is no mail"
        );
    });
}

fn keyring(key: &StoreKey) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(key.to_hex().as_str()),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

pub fn a_store_no_migration_reaches_offers_a_fresh_store_that_keeps_the_account() {
    crate::gtk_case(async {
        let directory = tempfile::tempdir().expect("a scratch directory");
        let store = directory.path().join("postio.db");
        let key = StoreKey::generate();
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
        postio_gtk::startup::open(
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

pub fn a_store_under_another_key_offers_a_fresh_store_and_keeps_the_old_one() {
    crate::gtk_case(async {
        let directory = tempfile::tempdir().expect("a scratch directory");
        let store = directory.path().join("postio.db");
        {
            let other = StoreKey::generate();
            let earlier = postio_storage::Store::open(&store, &other.derive(Purpose::Database))
                .await
                .expect("a store under another installation's key");
            let connection = earlier.connect().await.expect("a connection");
            postio_storage::test_support::account(&connection).await;
        }

        let window = FocusWindow::new(None);
        window.present();
        let opened: Rc<RefCell<Option<Session>>> = Rc::default();
        let opener = Opener::at(None, store.clone(), keyring(&StoreKey::generate()));
        postio_gtk::startup::open(
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
            "a store under another key opened, or the window said nothing"
        );
        let said = support::texts(&window);
        assert!(
            !said.contains(&"Try again".to_owned()),
            "trying again meets the same key: {said:?}"
        );
        let reason = window.unavailable_reason().expect("the page says why");
        for promise in ["key", "accounts again", "set aside, not deleted"] {
            assert!(
                reason.contains(promise),
                "the page has to say why, what a fresh store costs and where \
                 the old one goes before it is chosen; no {promise:?} in {reason:?}"
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
        session.stop();
    });
}
