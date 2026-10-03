//! Focus measures its start against the 500 ms budget (`docs/PRODUCT.md`
//! §18) with the same timeline the classic app used, `postio_widgets::startup`:
//! a window on screen, the store open, the inbox fed, and the frame with the
//! mail in it, each a phase, in that order.
//!
//! Through `startup::open` over a real store, as `app::run` opens it, so a
//! phase nothing marks is a phase this fails on.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_gtk::startup::{Opener, Session};
use postio_gtk::window::FocusWindow;
use postio_storage::key::{Purpose, StoreKey};
use postio_widgets::startup::{Phase, Timeline};

use crate::support;

fn keyring(key: &StoreKey) -> Arc<dyn SecretStore> {
    let secrets = MemorySecretStore::default();
    postio_session::blocking::now(secrets.store(
        &AccountKey::new(postio_session::STORE_KEY_ENTRY),
        &Password::new(key.to_hex().as_str()),
    ))
    .expect("the memory keyring takes it");
    Arc::new(secrets)
}

pub fn the_timeline_reaches_the_frame_with_mail_in_it() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a scratch directory");
        let store = directory.path().join("postio.db");
        let key = StoreKey::generate();
        {
            let seeded = postio_storage::Store::open(&store, &key.derive(Purpose::Database))
                .await
                .expect("a store");
            let connection = seeded.connect().await.expect("a connection");
            postio_storage::test_support::account(&connection).await;
        }

        let timeline = Timeline::start();
        let window = FocusWindow::new(None);
        postio_gtk::startup::time(&window, timeline.clone());
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
            crate::settle_until(async || timeline.total().is_some()).await,
            "the start never reached its first frame with mail: {}",
            timeline.report()
        );

        let marked: Vec<Phase> = Phase::ALL
            .into_iter()
            .filter(|phase| timeline.at(*phase).is_some())
            .collect();
        for phase in [
            Phase::Window,
            Phase::Shell,
            Phase::Store,
            Phase::Account,
            Phase::Feeds,
            Phase::FirstFrame,
        ] {
            assert!(
                marked.contains(&phase),
                "{} was never marked: {}",
                phase.label(),
                timeline.report()
            );
        }
        // Each phase after the one before it: the costs add up only then.
        assert!(
            timeline.at(Phase::Store) <= timeline.at(Phase::Account)
                && timeline.at(Phase::Account) <= timeline.at(Phase::Feeds)
                && timeline.at(Phase::Feeds) <= timeline.at(Phase::FirstFrame),
            "the phases are out of order: {}",
            timeline.report()
        );
        assert!(
            timeline.report().contains("budget 500.0ms"),
            "{}",
            timeline.report()
        );
        if let Some(session) = opened.take() {
            session.stop();
        }
    });
}
