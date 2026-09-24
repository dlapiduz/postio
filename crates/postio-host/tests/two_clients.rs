//! Two frontends on one daemon, end to end (US6).
//!
//! The daemon as it runs: a host serving a real socket in a temporary
//! runtime directory, its sync engine started, and two clients connected
//! over that socket as the desktop app and the terminal connect. What is
//! not real is the network: mail is read and filed through `MockBackend`
//! and submitted through a scripted SMTP server, so every remote effect can
//! be counted (CLAUDE.md: nothing in the default suite dials).

use std::sync::Arc;
use std::time::Duration;

use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_client::socket::{Endpoint, connect};
use postio_core::{Event, EventEnvelope};
use postio_host::Host;
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::{ListScope, MailboxId};
use postio_smtp::transport::{ScriptedConnector, SmtpScript};
use postio_storage::test_support;

/// How long anything here may take to happen before it has not.
const PATIENCE: Duration = Duration::from_secs(20);

/// A message the mock server holds from the start.
fn a_message(subject: &str, id: &str) -> MockMessage {
    MockMessage::from(
        format!(
            "Message-ID: <{id}@example.com>\r\nFrom: Ada <ada@example.com>\r\nTo: Test User \
         <test@example.com>\r\nSubject: {subject}\r\nDate: Tue, 22 Sep 2026 09:00:00 +0000\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{subject}, in full.\r\n"
        )
        .into_bytes(),
    )
}

/// An SMTP server that takes whatever it is sent.
fn accepting() -> SmtpScript {
    SmtpScript::new("220 smtp.example.com ESMTP ready")
        .on("EHLO", "250-smtp.example.com\r\n250 AUTH PLAIN")
        .on("AUTH PLAIN", "235 authenticated")
        .on("MAIL FROM", "250 ok")
        .on("RCPT TO", "250 ok")
        .on("DATA", "354 go ahead")
        .on("QUIT", "221 bye")
}

/// The daemon, its mock servers, and where to reach it.
pub struct Daemon {
    /// For the test's own awaiting.
    pub rt: tokio::runtime::Runtime,
    host: Arc<Host>,
    endpoint: Endpoint,
    serving: Option<std::thread::JoinHandle<()>>,
    /// The mail server.
    pub mock: MockBackend,
    /// The submission server.
    pub smtp: ScriptedConnector,
    /// The account's inbox, as the store knows it.
    pub inbox: MailboxId,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

impl Daemon {
    /// A daemon over a store with one account, its server holding `inbox`.
    pub fn start(inbox: Vec<MockMessage>) -> Daemon {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        let mut mailbox = MockMailbox::new("INBOX");
        for message in inbox {
            mailbox = mailbox.message(message);
        }
        let mock = MockBackend::builder()
            .mailbox(mailbox)
            .mailbox(MockMailbox::new("Archive"))
            .mailbox(MockMailbox::new("Sent"))
            .mailbox(MockMailbox::new("Drafts"))
            .build();
        let smtp = ScriptedConnector::new(accepting());
        let secrets = Arc::new(MemorySecretStore::new());
        let (database, inbox_id) = rt.block_on(async {
            let database = test_support::memory().await;
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            for path in ["Archive", "Sent", "Drafts"] {
                test_support::mailbox(&connection, &account, path).await;
            }
            secrets
                .store(
                    &AccountKey::new(account.address.address.clone()),
                    &Password::new("password"),
                )
                .await
                .expect("the password");
            (database, inbox)
        });
        let blobs_dir = tempfile::tempdir().expect("a blob directory");
        let blobs = postio_storage::BlobStore::open(
            blobs_dir.path().to_path_buf(),
            &test_support::blob_keys(),
        )
        .expect("a blob store");
        let mail = postio_session::MailOverride {
            backend: Arc::new(mock.clone()),
            smtp: Arc::new(smtp.clone()),
        };
        let host = Host::start(database, blobs, move |wiring| {
            wiring.with_secrets(secrets).with_mail(mail)
        })
        .expect("a host");
        host.start_syncing();
        let host = Arc::new(host);

        let runtime_dir = tempfile::tempdir().expect("a runtime directory");
        let endpoint = Endpoint::at(runtime_dir.path().join("postio"));
        let listener = postio_host::serve::bind(&endpoint).expect("the socket binds");
        let serving = std::thread::spawn({
            let host = Arc::clone(&host);
            move || host.serve(listener, Duration::from_millis(500))
        });
        Daemon {
            rt,
            host,
            endpoint,
            serving: Some(serving),
            mock,
            smtp,
            inbox: inbox_id,
            _dirs: (blobs_dir, runtime_dir),
        }
    }

    /// A frontend of `kind`, connected over the socket.
    pub fn connect(&self, kind: ClientKind) -> (Client, async_channel::Receiver<EventEnvelope>) {
        let client = connect(&self.endpoint, kind).expect("connects");
        let events = client.events();
        (client, events)
    }

    /// The inbox as `client` pages it.
    pub fn inbox_rows(&self, client: &Client) -> u32 {
        let page = self
            .rt
            .block_on(client.list_page(PageRequest {
                scope: ListScope::Mailbox(self.inbox),
                offset: 0,
                limit: 50,
            }))
            .expect("a page");
        match page {
            ListPage::Messages(page) => page.total,
            ListPage::Threads(page) => page.total,
        }
    }

    /// Wait until `done` holds, or fail naming `what`.
    pub fn until(&self, what: &str, mut done: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + PATIENCE;
        while !done() {
            assert!(
                std::time::Instant::now() < deadline,
                "{what} did not happen"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Wait for an event matching `wanted` on `events`.
    pub fn hear(
        &self,
        events: &async_channel::Receiver<EventEnvelope>,
        wanted: impl Fn(&Event) -> bool,
    ) -> Event {
        self.rt.block_on(async {
            tokio::time::timeout(PATIENCE, async {
                loop {
                    let envelope = events.recv().await.expect("the daemon is running");
                    if wanted(&envelope.event) {
                        return envelope.event;
                    }
                }
            })
            .await
            .expect("the event arrived")
        })
    }

    /// How many messages the mock server holds in `path`.
    pub fn on_server(&self, path: &str) -> u32 {
        self.rt
            .block_on(self.mock.status(path))
            .expect("the mock answers")
            .exists
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Every client is gone by now; the daemon leaves after its grace.
        if let Some(serving) = self.serving.take() {
            let _ = serving.join();
        }
        let _ = &self.host;
    }
}

#[test]
fn both_frontends_see_the_same_mailbox_from_one_daemon() {
    // T077: the harness itself -- one sync, two frontends, one store.
    let daemon = Daemon::start(vec![a_message("Tide gate", "tide")]);
    let (desktop, _) = daemon.connect(ClientKind::Gtk);
    let (terminal, _) = daemon.connect(ClientKind::Tui);
    daemon.until("the first sync", || daemon.inbox_rows(&desktop) == 1);
    assert_eq!(daemon.inbox_rows(&terminal), 1);
    assert_eq!(
        daemon
            .mock
            .header_fetches()
            .iter()
            .filter(|path| *path == "INBOX")
            .count(),
        1,
        "fetched once for both"
    );
}

/// `client`, looking at the inbox with the cursor on the message listed
/// `position`th, as a frontend's own state would be.
fn looking_at(daemon: &Daemon, client: Client, position: usize) -> Client {
    let page = daemon
        .rt
        .block_on(client.list_page(PageRequest {
            scope: ListScope::Mailbox(daemon.inbox),
            offset: 0,
            limit: 50,
        }))
        .expect("a page");
    let message = match page {
        ListPage::Messages(page) => page.rows[position].id,
        ListPage::Threads(page) => page.rows[position].representative.id,
    };
    let state = postio_core::SharedState::default();
    let (quiet, _) = postio_core::bridge::event_channel();
    state.update(&quiet, |app| {
        let mut events = app.open_mailbox(daemon.inbox);
        events.extend(app.select(Vec::new(), Some(message)));
        events
    });
    client.with_state(state)
}

#[test]
fn an_archive_in_one_frontend_is_gone_from_the_other() {
    // US6 scenario 1.
    let daemon = Daemon::start(vec![a_message("Tide gate", "tide")]);
    let (desktop, _) = daemon.connect(ClientKind::Gtk);
    let (terminal, heard) = daemon.connect(ClientKind::Tui);
    daemon.until("the first sync", || daemon.inbox_rows(&desktop) == 1);
    let desktop = looking_at(&daemon, desktop, 0);

    daemon
        .rt
        .block_on(desktop.send(postio_core::Command::Archive {
            target: postio_core::MessageTarget::Selection,
        }))
        .expect("sent");
    daemon.hear(&heard, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
    assert_eq!(
        daemon.inbox_rows(&terminal),
        0,
        "gone from the other's next page"
    );

    daemon.until("the move on the server", || {
        daemon.on_server("Archive") == 1
    });
    assert_eq!(daemon.on_server("INBOX"), 0);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        daemon.on_server("Archive"),
        1,
        "moved once, not once per frontend"
    );
}
