//! Startup with the window already on screen (#1114, #1604).
//!
//! Two cases, which are the two ways the store's open and the window's
//! arrival can be ordered: the store lands behind a window that is already
//! up and the window fills, and the store starts opening before there is a
//! window at all and the window takes that open over rather than making a
//! second. Both run Focus's own `startup::open`, over `Opener::at` a scratch
//! store with an in-memory keyring -- everything `app.rs`'s `activate`
//! handler does except the one line that calls it.
//!
//! The store is a fresh one, so its open is the slowest thing a case here
//! waits on: the app answers over a channel and depends on no clock, and
//! the cases wait for the answer under [`crate::STORE_OPEN`] rather than
//! the ordinary ten seconds.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gtk::prelude::*;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretError, SecretStore};
use postio_focus::startup::{Opener, Session};
use postio_focus::window::FocusWindow;
use postio_widgets::startup::{Phase, Timeline};

use crate::support;

/// A secret store that counts how often it is asked for anything.
#[derive(Debug, Default)]
struct Counting {
    inner: MemorySecretStore,
    asked: AtomicUsize,
}

#[async_trait::async_trait]
impl SecretStore for Counting {
    fn describe(&self) -> &'static str {
        "counting"
    }
    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        self.inner.store(key, password).await
    }
    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.inner.retrieve(key).await
    }
    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        self.inner.delete(key).await
    }
}

/// The store opens behind a window that is already up, and the window fills
/// when it lands: the thread, the channel, the stages it reports, the
/// assembly on the main context and the feed at the end.
pub fn the_store_opens_behind_a_window_that_is_already_up() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a store directory");
        let store = directory.path().join("postio.db");

        // A window on screen with nothing behind it.
        let timeline = Timeline::start();
        let window = FocusWindow::new(None);
        postio_focus::startup::time(&window, timeline.clone());
        window.present();
        crate::settle();
        assert!(window.is_mapped(), "the window is up before the store is");
        assert!(
            window.rows_on_screen().is_empty() && window.pane().is_none(),
            "a window offering mail before anything has been read"
        );

        let opened: Rc<RefCell<Option<Session>>> = Rc::default();
        let opener = Opener::at(None, store, Arc::new(MemorySecretStore::default()));
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
            crate::settle_until_within(crate::STORE_OPEN, async || opened.borrow().is_some()).await,
            "the store never landed, so the thread, the channel or the assembly \
             on the main context is not joined up"
        );
        assert!(
            crate::settle_until(async || window.pane().is_some()).await,
            "the store opened and the window was never told, so it is still \
             withholding the inbox from a window that has one"
        );
        assert!(
            window.waiting_text().is_empty(),
            "and nothing is still being waited for: {:?}",
            window.waiting_text()
        );
        assert!(
            window.unavailable_reason().is_none(),
            "an open store left the window saying it is unavailable"
        );
        assert!(
            timeline.at(Phase::Store).is_some(),
            "the phase that measures the wait was never marked, so a trace would \
             attribute it to whatever phase came next"
        );
        if let Some(session) = opened.take() {
            session.stop();
            support::keep(session);
        }
    });
}

/// #1604: the store open -- the keyring, then the engine's open of an
/// encrypted file -- must not wait for the window to be built and presented.
/// The two chains need nothing from each other, so the open starts first and
/// the window takes it over when it exists. What must not happen is a second
/// open: the window taking the early one is the whole point, and a second
/// would read the keyring and open the file again.
pub fn the_store_starts_opening_before_there_is_a_window() {
    crate::gtk_case(async {
        if !support::display() {
            return;
        }
        let directory = tempfile::tempdir().expect("a store directory");
        let secrets = Arc::new(Counting::default());
        let opener = Opener::at(
            None,
            directory.path().join("postio.db"),
            Arc::clone(&secrets) as Arc<dyn SecretStore>,
        );
        // No window yet: only the open.
        let progress = opener.open_on_a_thread();
        assert!(
            crate::settle_until(async || secrets.asked.load(Ordering::SeqCst) > 0).await,
            "starting the open before any window never reached the keyring"
        );
        let asked_before_the_window = secrets.asked.load(Ordering::SeqCst);

        let window = FocusWindow::new(None);
        window.present();
        let opened: Rc<RefCell<Option<Session>>> = Rc::default();
        postio_focus::startup::open(
            &window,
            progress,
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
            crate::settle_until_within(crate::STORE_OPEN, async || opened.borrow().is_some()).await,
            "the window never received the store the early open was making"
        );
        assert_eq!(
            secrets.asked.load(Ordering::SeqCst),
            asked_before_the_window,
            "the window opened the store a second time instead of taking the \
             open that had already started"
        );
        if let Some(session) = opened.take() {
            session.stop();
            support::keep(session);
        }
    });
}
