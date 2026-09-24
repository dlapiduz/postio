//! The daemon going away under an open window, and coming back.
//!
//! `daemon_window` is the window reaching `postio-daemon` over its socket.
//! This stops that daemon while the window is open -- as a crash or a `kill`
//! stops a real one -- and asserts on what a person sees: the unavailable
//! screen saying the background service stopped, and after "Try again" with
//! a new daemon listening, the window's list back, showing that daemon's
//! mail, with a gesture reaching it.
//!
//! Nothing dials out: each host's engine syncs from its own `MockBackend`.

#![allow(unsafe_code)]
// Rust 2024 made `std::env::set_var` unsafe: it races any other thread reading
// the environment. This test sets it before the app under test starts, which
// is the one moment it is sound.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::settle_until;
use gtk::prelude::*;
use gtk::{gdk, glib};
use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_app::remote;
use postio_client::protocol::ClientKind;
use postio_client::socket::{Endpoint, connect};
use postio_core::state::SharedState;
use postio_core::{Command, MessageTarget};
use postio_gtk::unavailable::Unavailable;
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_host::Host;
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::{ListScope, MailboxId};
use postio_storage::test_support;

/// A message the mock server holds from the start.
fn a_message(subject: &str, id: &str) -> MockMessage {
    MockMessage::from(
        format!(
            "Message-ID: <{id}@example.com>\r\nFrom: Ada <ada@example.com>\r\nTo: Test User \
             <test@example.com>\r\nSubject: {subject}\r\nDate: Tue, 22 Sep 2026 09:00:00 \
             +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{subject}, in full.\r\n"
        )
        .into_bytes(),
    )
}

/// A running daemon: the thread serving it, and how to stop it.
struct Daemon {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    serving: Option<std::thread::JoinHandle<()>>,
    inbox: MailboxId,
    _blobs: tempfile::TempDir,
}

impl Daemon {
    /// A daemon at `endpoint` over a store of its own, whose server holds
    /// `subjects` in its inbox.
    async fn start(endpoint: &Endpoint, subjects: &[&str]) -> Daemon {
        let mut inbox = MockMailbox::new("INBOX");
        for (index, subject) in subjects.iter().enumerate() {
            inbox = inbox.message(a_message(subject, &format!("m{index}")));
        }
        let mock = MockBackend::builder()
            .mailbox(inbox)
            .mailbox(MockMailbox::new("Archive"))
            .build();
        let secrets = Arc::new(MemorySecretStore::new());
        let database = test_support::memory().await;
        let inbox = {
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            test_support::mailbox(&connection, &account, "Archive").await;
            secrets
                .store(
                    &AccountKey::new(account.address.address.clone()),
                    &Password::new("password"),
                )
                .await
                .expect("the password");
            inbox
        };
        let blobs_dir = tempfile::tempdir().expect("a blob directory");
        let blobs = postio_storage::BlobStore::open(
            blobs_dir.path().to_path_buf(),
            &test_support::blob_keys(),
        )
        .expect("a blob store");
        let mail = postio_session::MailOverride {
            backend: Arc::new(mock),
            smtp: Arc::new(postio_smtp::transport::ScriptedConnector::new(
                postio_smtp::transport::SmtpScript::new("220 ready"),
            )),
        };
        let host = Host::start(database, blobs, move |wiring| {
            wiring.with_secrets(secrets).with_mail(mail)
        })
        .expect("a host");
        host.start_syncing();
        let listener = postio_host::serve::bind(endpoint).expect("the socket binds");
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let serving = std::thread::spawn(move || {
            host.serve_until(listener, Duration::from_millis(500), async move {
                let _ = stopped.await;
            });
            // As `postio-daemon` exits once serving returns: the host, its
            // runtime and every connection on it go.
            drop(host);
        });
        Daemon {
            stop: Some(stop),
            serving: Some(serving),
            inbox,
            _blobs: blobs_dir,
        }
    }

    /// Stop it with frontends still connected, and wait until it has gone.
    fn kill(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(serving) = self.serving.take() {
            serving.join().expect("the daemon stopped");
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn inbox_rows(client: &postio_client::Client, inbox: MailboxId) -> u32 {
    match client
        .list_page(PageRequest {
            scope: ListScope::Mailbox(inbox),
            offset: 0,
            limit: 50,
        })
        .await
        .expect("a page")
    {
        ListPage::Messages(page) => page.total,
        ListPage::Threads(page) => page.total,
    }
}

pub fn a_window_whose_daemon_stops_says_so_and_retry_reaches_a_new_one() {
    crate::gtk_case(async {
        let state_dir = tempfile::tempdir().expect("a state directory");
        // SAFETY: first statement of a single-threaded test.
        unsafe { std::env::set_var("XDG_STATE_HOME", state_dir.path()) };

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (run under `scripts/test-headless.sh`)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        fonts::install().expect("the embedded fonts should install");
        style::install(&display);
        app::install_icons(&display);

        let runtime_dir = tempfile::tempdir().expect("a runtime directory");
        let endpoint = Endpoint::at(runtime_dir.path().join("postio"));
        let mut first = Daemon::start(&endpoint, &["Quarterly report", "Lunch on Friday"]).await;

        // ── the desktop app's start, as `run` makes it ──────────────────────
        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}
        let reach = {
            let endpoint = endpoint.clone();
            move || remote::reach(endpoint.clone(), "/nonexistent/postio-daemon".into())
        };
        remote::follow(
            &window,
            reach(),
            Rc::new(remote::Following {
                runtime: tokio::runtime::Handle::current(),
                state: SharedState::default(),
                attachments_eager: false,
                reached: Rc::new(RefCell::new(None)),
                fed: Rc::new(Cell::new(false)),
                again: Box::new(reach),
                on_connected: Box::new(|| {}),
                previous: RefCell::new(None),
            }),
        );
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() == 2).await,
            "the window over the first daemon shows {} rows, not its two",
            list.model().n_items()
        );

        // ── the daemon goes, with the window open ───────────────────────────
        first.kill();
        assert!(
            settle_until(async || Unavailable::showing_in(&window).is_some()).await,
            "the daemon went away and the window still shows its panes, as if \
             nothing had happened"
        );
        let screen = Unavailable::showing_in(&window).expect("showing");
        assert!(
            screen.reason().contains("background service stopped"),
            "it says what happened: {:?}",
            screen.reason()
        );

        // ── a new daemon, and "Try again" reaches it ────────────────────────
        let second = Daemon::start(
            &endpoint,
            &["Quarterly report", "Lunch on Friday", "Tide gate"],
        )
        .await;
        screen.retry();
        assert!(
            settle_until(async || Unavailable::showing_in(&window).is_none()).await,
            "retry with a daemon listening left the unavailable screen up"
        );
        assert!(
            settle_until(async || list.model().n_items() == 3).await,
            "the list shows {} rows, not the new daemon's three: it was not \
             read again over the new connection",
            list.model().n_items()
        );

        // ── and a gesture in the window reaches the new daemon ──────────────
        let terminal = connect(&endpoint, ClientKind::Tui).expect("a second frontend");
        assert_eq!(inbox_rows(&terminal, second.inbox).await, 3);
        let row = list.model().peek(0).expect("a first row");
        list.select_message(row);
        settle_until(async || list.cursor_id() == Some(row)).await;
        window.act(Command::Archive {
            target: MessageTarget::Selection,
        });
        assert!(
            settle_until(async || inbox_rows(&terminal, second.inbox).await == 2).await,
            "the archive pressed after reconnecting never reached the new daemon"
        );

        drop(terminal);
        drop(window);
        drop(second);
    });
}
