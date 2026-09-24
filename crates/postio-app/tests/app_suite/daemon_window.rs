//! The desktop app's own start, over a store another process owns (ADR 0041).
//!
//! `run()` opens no store. It reaches `postio-daemon` over its socket and
//! builds the window over that one client (`postio_app::remote`). This drives
//! exactly that path against a host serving a real socket in a temporary
//! runtime directory, as the daemon does, and asserts on what a person would
//! see: the daemon's mail in the window's list, and an archive pressed in the
//! window reaching a second frontend on the same store -- the reason the
//! daemon exists.
//!
//! Nothing dials out: the host's engine syncs from `MockBackend`.

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
use postio_gtk::window::Window;
use postio_gtk::{app, fonts, style};
use postio_host::Host;
use postio_model::ListScope;
use postio_model::listing::{ListPage, MailStore, PageRequest};
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

pub fn a_window_over_the_daemons_socket_shows_its_mail_and_acts_on_it() {
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

        // ── the daemon: a store with one account, its server holding two ────
        let mock = MockBackend::builder()
            .mailbox(
                MockMailbox::new("INBOX")
                    .message(a_message("Quarterly report", "one"))
                    .message(a_message("Lunch on Friday", "two")),
            )
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
        let host = Arc::new(
            Host::start(database, blobs, move |wiring| {
                wiring.with_secrets(secrets).with_mail(mail)
            })
            .expect("a host"),
        );
        // As `postio-daemon` does once its store is open; the window's own
        // ask after its first frame then finds the account already syncing.
        host.start_syncing();
        let runtime_dir = tempfile::tempdir().expect("a runtime directory");
        let endpoint = Endpoint::at(runtime_dir.path().join("postio"));
        let listener = postio_host::serve::bind(&endpoint).expect("the socket binds");
        let serving = std::thread::spawn({
            let host = Arc::clone(&host);
            move || host.serve(listener, Duration::from_millis(500))
        });

        // ── the desktop app's start, as `run` makes it ──────────────────────
        let window = Window::default();
        window.present();
        while glib::MainContext::default().iteration(false) {}

        let state = SharedState::default();
        let reached = Rc::new(RefCell::new(None));
        remote::follow(
            &window,
            // Nothing to start: the daemon is already listening.
            remote::reach(endpoint.clone(), "/nonexistent/postio-daemon".into()),
            Rc::new(remote::Following {
                runtime: tokio::runtime::Handle::current(),
                state: state.clone(),
                attachments_eager: false,
                reached: Rc::clone(&reached),
                fed: Rc::new(Cell::new(false)),
                again: Box::new({
                    let endpoint = endpoint.clone();
                    move || remote::reach(endpoint.clone(), "/nonexistent/postio-daemon".into())
                }),
                on_connected: Box::new(|| {}),
                previous: RefCell::new(None),
            }),
        );

        // ── what a person sees: the daemon's mail, in this window ───────────
        let list = window.list();
        assert!(
            settle_until(async || list.model().n_items() == 2).await,
            "the window over the daemon's socket shows {} rows, not the two \
             messages its server holds",
            list.model().n_items()
        );

        // ── an archive pressed here reaches another frontend ────────────────
        let terminal = connect(&endpoint, ClientKind::Tui).expect("a second frontend");
        let inbox_rows = |client: &postio_client::Client| {
            let client = client.clone();
            async move {
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
        };
        assert_eq!(inbox_rows(&terminal).await, 2);

        let first = list.model().peek(0).expect("a first row");
        list.select_message(first);
        settle_until(async || list.cursor_id() == Some(first)).await;
        window.act(Command::Archive {
            target: MessageTarget::Selection,
        });
        assert!(
            settle_until(async || inbox_rows(&terminal).await == 1).await,
            "the archive pressed in the window never reached the other frontend"
        );
        assert!(
            settle_until(async || list.model().n_items() == 1).await,
            "and the window's own list still shows the archived message"
        );

        drop(terminal);
        drop(reached);
        drop(window);
        // The daemon stops once nothing has been connected for its grace.
        let _ = serving;
    });
}
