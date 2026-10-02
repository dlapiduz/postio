//! Startup over an account whose password the keyring does not have.
//!
//! An account row and no credential for it: the state a keyring write that
//! failed after the row was saved leaves behind. Such an account opens, and
//! cannot authenticate. This drives Focus's own startup over it
//! (`startup::open`, then `Session::start_syncing`, which is what `app.rs`
//! does after the first frame) and asserts what the window then offers: a
//! sign-in banner with "Update password...", which opens the credential form
//! for that account, filled in from its row, rather than a window that
//! looks like a working account and is a dead end.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_focus::startup::{Opener, Session};
use postio_focus::window::FocusWindow;
use postio_storage::key::{Purpose, StoreKey};

use crate::support;

/// A keyring that has the store's key and nothing for any account.
fn keyring(key: &StoreKey) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(key.to_hex().as_str()),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

pub fn an_account_with_no_credential_offers_the_repair() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a scratch directory");
        let store = directory.path().join("postio.db");
        let key = StoreKey::generate();
        let account = {
            let seeded = postio_storage::Store::open(&store, &key.derive(Purpose::Database))
                .await
                .expect("a store");
            let connection = seeded.connect().await.expect("a connection");
            postio_storage::test_support::account(&connection).await
        };

        let window = FocusWindow::new(None);
        window.present();
        let opened: Rc<RefCell<Option<Session>>> = Rc::default();
        let opener = Opener::at(None, store, keyring(&key));
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
            crate::settle_until(async || opened.borrow().is_some()).await,
            "the store never opened"
        );
        // What `app.rs` does once the stored mail is drawn.
        opened
            .borrow()
            .as_ref()
            .expect("the session")
            .start_syncing();

        assert!(
            crate::settle_until(async || window.banner_showing().is_some()).await,
            "an account with no password in the keyring opened as though it \
             were a working account: the window offers no way to repair it"
        );
        let (title, button, _) = window.banner_showing().expect("a banner");
        assert!(
            title.contains("sign in") && button.as_deref() == Some("Update password\u{2026}"),
            "the banner offers something other than the credential repair: \
             {title:?} / {button:?}"
        );

        let banner = support::only(&window, "focus-banner");
        let press = support::descendants(&banner)
            .into_iter()
            .find(|widget| widget.is::<gtk::Button>() && widget.is_mapped())
            .expect("the banner's button is on screen");
        support::click(&window, &press, 1);
        let form = || {
            support::descendants(&window)
                .into_iter()
                .find_map(|widget| {
                    widget
                        .downcast::<postio_widgets::onboarding::Onboarding>()
                        .ok()
                })
                .filter(|form| form.is_mapped())
        };
        assert!(
            crate::settle_until(async || form().is_some()).await,
            "Update password opened no credential form"
        );
        let form = form().expect("the form");
        assert!(
            matches!(
                form.status(),
                postio_widgets::onboarding::Status::Reauthenticate(_)
            ),
            "the repair arrived looking like a first run: {:?}",
            form.status()
        );
        assert_eq!(
            form.address(),
            account.address.address,
            "the form made the user retype an address the store already had"
        );
        if let Some(session) = opened.take() {
            session.stop();
            support::keep(session);
        }
    });
}
