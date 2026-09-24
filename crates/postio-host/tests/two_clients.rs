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
        .on(
            "EHLO",
            "250-smtp.example.com\r\n250-STARTTLS\r\n250 AUTH PLAIN",
        )
        .on("STARTTLS", "220 ready to start TLS")
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
    /// The store, for what a test reads directly.
    pub store: postio_storage::Store,
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
            // Someone to send as, as every real account has.
            let mut identity = postio_model::Identity::new(account.id, account.address.clone());
            identity.is_default = true;
            postio_storage::repository::IdentityRepository::new(&connection)
                .create(&mut identity)
                .await
                .expect("an identity");
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
        let store = database.clone();
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
            store,
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

impl Daemon {
    /// Whether the daemon has stopped serving, waiting at most `within`.
    pub fn stopped_within(&self, within: Duration) -> bool {
        let deadline = std::time::Instant::now() + postio_test_support::scaled(within);
        loop {
            if self
                .serving
                .as_ref()
                .is_none_or(|serving| serving.is_finished())
            {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
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

#[test]
fn new_mail_is_fetched_once_and_reaches_both() {
    // US6 scenario 2.
    use postio_account::backend::{AppendMessage, MailboxEvent};
    let daemon = Daemon::start(vec![a_message("Tide gate", "tide")]);
    let (desktop, desktop_heard) = daemon.connect(ClientKind::Gtk);
    let (terminal, terminal_heard) = daemon.connect(ClientKind::Tui);
    daemon.until("the first sync", || daemon.inbox_rows(&desktop) == 1);
    let fetched_before = daemon.mock.header_fetches().len();

    // Mail arrives at the server, and the server says so to whoever idles.
    let raw = "Message-ID: <later@example.com>\r\nFrom: Grace <grace@example.net>\r\nTo: \
         test@example.com\r\nSubject: Interlock report\r\nDate: Tue, 22 Sep 2026 10:00:00 \
         +0000\r\n\r\nAttached.\r\n"
        .to_owned();
    daemon
        .rt
        .block_on(
            daemon
                .mock
                .append("INBOX", &AppendMessage::new(raw.into_bytes())),
        )
        .expect("delivered");
    daemon
        .mock
        .push_event("INBOX", MailboxEvent::Exists { count: 2 });

    let arrived = |event: &Event| {
        matches!(
            event,
            Event::MessageListChanged { .. } | Event::NewMail { .. }
        )
    };
    daemon.hear(&desktop_heard, arrived);
    daemon.hear(&terminal_heard, arrived);
    daemon.until("the new mail in both", || {
        daemon.inbox_rows(&desktop) == 2 && daemon.inbox_rows(&terminal) == 2
    });
    let fetched = daemon.mock.header_fetches().len() - fetched_before;
    assert!(fetched <= 1, "fetched {fetched} times for two frontends");
}

#[test]
fn a_mixed_session_from_both_reaches_the_servers_exactly_once() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("postio_session=error,postio_storage=warn")
        .with_test_writer()
        .try_init();
    // US6 scenario 3 / SC-007: archive, flag, move and send, from both.
    let daemon = Daemon::start(vec![
        a_message("First", "first"),
        a_message("Second", "second"),
        a_message("Third", "third"),
    ]);
    let (desktop, desktop_heard) = daemon.connect(ClientKind::Gtk);
    let (terminal, _) = daemon.connect(ClientKind::Tui);
    daemon.until("the first sync", || daemon.inbox_rows(&desktop) == 3);
    let accounts = daemon.rt.block_on(terminal.accounts()).expect("accounts");
    let account = accounts[0].id;
    let archive = daemon
        .rt
        .block_on(terminal.mailboxes(account))
        .expect("folders")
        .into_iter()
        .find(|folder| folder.path == "Archive")
        .expect("an archive")
        .id;
    let selection = || postio_core::MessageTarget::Selection;

    // The desktop archives the first row, the terminal flags the next one,
    // and the desktop moves what is then first to the archive by name.
    let desktop_on_first = looking_at(&daemon, desktop, 0);
    daemon
        .rt
        .block_on(desktop_on_first.send(postio_core::Command::Archive {
            target: selection(),
        }))
        .expect("archived");
    daemon.until("the archive", || daemon.inbox_rows(&terminal) == 2);
    let terminal_on_first = looking_at(&daemon, terminal, 0);
    daemon
        .rt
        .block_on(terminal_on_first.send(postio_core::Command::Flag {
            target: selection(),
            flagged: Some(true),
        }))
        .expect("flagged");
    let desktop_on_second = looking_at(&daemon, desktop_on_first, 1);
    daemon
        .rt
        .block_on(desktop_on_second.send(postio_core::Command::Move {
            target: selection(),
            to: Some(archive),
        }))
        .expect("moved");

    // And the terminal sends one.
    let mut draft = postio_model::Draft::new(account);
    draft.to = vec![postio_model::EmailAddress::new(
        None::<String>,
        "grace@example.net",
    )];
    draft.subject = "Tide gate".into();
    draft.body_markdown = Some("Looking **now**.".into());
    draft.body.text = Some("Looking **now**.".into());
    daemon
        .rt
        .block_on(terminal_on_first.queue_send(1, draft, None))
        .expect("queued");

    let submissions = || {
        daemon
            .smtp
            .log()
            .commands()
            .iter()
            .filter(|command| command.as_str() == "DATA")
            .count()
    };
    if !(0..250).any(|_| {
        std::thread::sleep(Duration::from_millis(40));
        daemon.on_server("Archive") == 2
    }) {
        let rows: Vec<(i64, String)> = daemon.rt.block_on(async {
            let connection = daemon.store.connect().await.expect("a connection");
            let mut rows = connection
                .query("SELECT id, op_type || ' ' || state || ' ' || coalesce(last_error, '') FROM operation_queue", ())
                .await
                .expect("the queue");
            let mut out = Vec::new();
            while let Ok(Some(row)) = rows.next().await {
                out.push((row.get::<i64>(0).unwrap(), row.get::<String>(1).unwrap()));
            }
            out
        });
        let said: Vec<String> = std::iter::from_fn(|| desktop_heard.try_recv().ok())
            .map(|envelope| format!("{:?}", envelope.event))
            .filter(|event| event.contains("Rejected") || event.contains("Error"))
            .collect();
        panic!(
            "moves: archive {} inbox {}; queue {rows:?}; desktop heard {said:?}",
            daemon.on_server("Archive"),
            daemon.on_server("INBOX"),
        );
    }
    if !(0..250).any(|_| {
        std::thread::sleep(Duration::from_millis(40));
        submissions() == 1
    }) {
        let why = daemon.rt.block_on(async {
            let connection = daemon.store.connect().await.expect("a connection");
            let drafts = postio_storage::repository::DraftRepository::new(&connection)
                .list_for_account(account)
                .await
                .expect("drafts");
            let mut why = Vec::new();
            for draft in drafts {
                let failure =
                    postio_storage::repository::OperationQueueRepository::new(&connection)
                        .last_failure_for(postio_model::OperationTarget::Draft(draft.id))
                        .await
                        .expect("the queue");
                why.push(format!("{:?}: {failure:?}", draft.state));
            }
            why
        });
        panic!(
            "no submission: {why:?}; smtp said {:?}",
            daemon.smtp.log().commands()
        );
    }
    // Long enough for any second attempt, from either side, to have shown.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(daemon.on_server("Archive"), 2, "each move made once");
    assert_eq!(daemon.on_server("INBOX"), 1);
    assert_eq!(submissions(), 1, "one submission for one send");
    let flagged = daemon
        .rt
        .block_on(desktop_on_second.list_page(PageRequest {
            scope: ListScope::Mailbox(daemon.inbox),
            offset: 0,
            limit: 50,
        }))
        .expect("a page");
    let flagged = match flagged {
        ListPage::Messages(page) => page.rows.iter().any(|row| row.flagged),
        ListPage::Threads(page) => page.rows.iter().any(|row| row.representative.flagged),
    };
    assert!(flagged, "the terminal's flag is what the desktop sees");
}

#[test]
fn one_frontend_leaving_mid_sync_leaves_the_other_whole_and_the_last_to_leave_ends_it() {
    // US6 scenario 4 and FR-043.
    let daemon = Daemon::start(vec![
        a_message("Tide gate", "tide"),
        a_message("Second", "second"),
    ]);
    let (desktop, _) = daemon.connect(ClientKind::Gtk);
    let (terminal, heard) = daemon.connect(ClientKind::Tui);
    // The desktop goes while the first sync is still running.
    drop(desktop);

    daemon.until("the first sync", || daemon.inbox_rows(&terminal) == 2);
    let terminal = looking_at(&daemon, terminal, 0);
    daemon
        .rt
        .block_on(terminal.send(postio_core::Command::Archive {
            target: postio_core::MessageTarget::Selection,
        }))
        .expect("archived");
    daemon.hear(&heard, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
    daemon.until("the queue drained", || daemon.on_server("Archive") == 1);
    assert!(
        !daemon.stopped_within(Duration::from_millis(100)),
        "a frontend is still here, so the daemon is too"
    );

    drop(heard);
    drop(terminal);
    assert!(
        daemon.stopped_within(Duration::from_secs(5)),
        "the last frontend gone, the daemon leaves after its grace"
    );
}

/// The local draft behind the only row in Drafts, as `client` reopens it.
fn draft_in_drafts(
    daemon: &Daemon,
    client: &Client,
    account: postio_model::AccountId,
) -> postio_model::Draft {
    let drafts = daemon
        .rt
        .block_on(client.mailboxes(account))
        .expect("folders")
        .into_iter()
        .find(|folder| folder.path == "Drafts")
        .expect("a Drafts folder")
        .id;
    let mut found = None;
    daemon.until("the draft's row in Drafts", || {
        let page = daemon
            .rt
            .block_on(client.list_page(PageRequest {
                scope: ListScope::Mailbox(drafts),
                offset: 0,
                limit: 10,
            }))
            .expect("a page");
        let rows: Vec<_> = match page {
            ListPage::Messages(page) => page.rows.into_iter().map(|row| row.id).collect(),
            ListPage::Threads(page) => page
                .rows
                .into_iter()
                .map(|row| row.representative.id)
                .collect(),
        };
        found = rows
            .into_iter()
            .find_map(|row| daemon.rt.block_on(client.draft_behind(row)).ok().flatten());
        found.is_some()
    });
    found.expect("found")
}

#[test]
fn a_draft_crosses_between_the_frontends_with_its_formatting() {
    // FR-023: written on the desktop, reopened in the terminal as Markdown.
    let daemon = Daemon::start(vec![]);
    let (desktop, _) = daemon.connect(ClientKind::Gtk);
    let (terminal, _) = daemon.connect(ClientKind::Tui);
    let account = daemon.rt.block_on(desktop.accounts()).expect("accounts")[0].id;

    let mut written = postio_model::Draft::new(account);
    written.subject = "Tide gate".into();
    written.body.text = Some("Some bold words".into());
    written.body.html = Some("<p>Some <strong>bold</strong> words</p>".into());
    daemon
        .rt
        .block_on(desktop.save_draft(1, written))
        .expect("saved on the desktop");

    let reopened = draft_in_drafts(&daemon, &terminal, account);
    assert_eq!(reopened.body_markdown, None, "the desktop wrote HTML");
    let html = reopened.body.html.as_deref().expect("its HTML travelled");
    let markdown = postio_body::markdown::from_document(&postio_body::parse(html));
    assert_eq!(markdown.trim_end(), "Some **bold** words");
}

#[test]
fn a_terminal_draft_opens_on_the_desktop_with_its_formatting() {
    // FR-023, the other way: written in Markdown, reopened as HTML with the
    // formatting in it, and the Markdown kept for the terminal to reopen.
    let daemon = Daemon::start(vec![]);
    let (desktop, _) = daemon.connect(ClientKind::Gtk);
    let (terminal, _) = daemon.connect(ClientKind::Tui);
    let account = daemon.rt.block_on(terminal.accounts()).expect("accounts")[0].id;

    let typed = "Some **bold** words";
    let document = postio_body::markdown::to_document(typed);
    let mut written = postio_model::Draft::new(account);
    written.subject = "Tide gate".into();
    written.body.text = Some(typed.into());
    written.body.html = Some(postio_body::render(&document).1);
    written.body_markdown = Some(typed.into());
    daemon
        .rt
        .block_on(terminal.save_draft(1, written))
        .expect("saved in the terminal");

    let reopened = draft_in_drafts(&daemon, &desktop, account);
    let html = reopened.body.html.as_deref().expect("its HTML travelled");
    assert!(html.contains("<strong>bold</strong>"), "{html}");
    assert_eq!(reopened.body_markdown.as_deref(), Some(typed));
}
