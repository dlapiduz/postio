//! The host over a real store, driven through clients the way a frontend
//! drives it. Asserted on what a frontend would see: its events and its list.

use std::time::Duration;

use chrono::Utc;
use postio_account::secret::MemorySecretStore;
use postio_client::Client;
use postio_client::protocol::ClientKind;
use postio_core::bridge::event_channel;
use postio_core::{Command, Event, EventEnvelope, MessageTarget, SharedState};
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::{ListScope, MailboxId, Message, MessageId};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

use super::Host;

/// A store with an inbox, an archive and a trash, one message in the inbox,
/// and a host over it.
pub(crate) struct World {
    pub(crate) rt: tokio::runtime::Runtime,
    host: Option<Host>,
    inbox: MailboxId,
    message: MessageId,
    database: postio_storage::Store,
    blob_dir: std::path::PathBuf,
    _blobs: tempfile::TempDir,
}

impl World {
    pub(crate) fn new() -> World {
        World::configured(|wiring| wiring)
    }

    /// The same world, its host wired further by `configure`.
    pub(crate) fn configured(
        configure: impl FnOnce(postio_session::Wiring) -> postio_session::Wiring + Send + 'static,
    ) -> World {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime");
        let (database, inbox, message) = rt.block_on(async {
            let database = test_support::memory().await;
            let connection = database.connect().await.expect("a connection");
            let (account, inbox) = test_support::account_with_inbox(&connection).await;
            test_support::mailbox(&connection, &account, "Archive").await;
            test_support::mailbox(&connection, &account, "Trash").await;
            let mut message = Message::new(account.id, inbox, Utc::now());
            let message = MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            (database, inbox, message)
        });
        let directory = tempfile::tempdir().expect("a blob directory");
        let blobs = postio_storage::BlobStore::open(
            directory.path().to_path_buf(),
            &test_support::blob_keys(),
        )
        .expect("a blob store");
        let kept = database.clone();
        let host = Host::start(database, blobs, move |wiring| {
            configure(wiring.with_secrets(std::sync::Arc::new(MemorySecretStore::new())))
        })
        .expect("a host");
        World {
            rt,
            host: Some(host),
            inbox,
            message,
            database: kept,
            blob_dir: directory.path().to_path_buf(),
            _blobs: directory,
        }
    }

    /// The host.
    pub(crate) fn host(&self) -> &Host {
        self.host.as_ref().expect("running")
    }

    /// A frontend's state: looking at the inbox, the cursor on the message.
    pub(crate) fn looking_at_the_message(&self) -> SharedState {
        let state = SharedState::default();
        let (quiet, _) = event_channel();
        state.update(&quiet, |app| {
            let mut events = app.open_mailbox(self.inbox);
            events.extend(app.select(Vec::new(), Some(self.message)));
            events
        });
        state
    }

    /// A frontend looking at the inbox with the cursor on the message.
    fn frontend(&self, kind: ClientKind) -> (Client, async_channel::Receiver<EventEnvelope>) {
        let state = self.looking_at_the_message();
        let client = self
            .host
            .as_ref()
            .expect("running")
            .connect(kind)
            .with_state(state);
        let events = client.events();
        (client, events)
    }

    pub(crate) fn inbox_rows(&self, client: &Client) -> u32 {
        let page = self
            .rt
            .block_on(client.list_page(PageRequest {
                scope: ListScope::Mailbox(self.inbox),
                offset: 0,
                limit: 20,
            }))
            .expect("a page");
        match page {
            ListPage::Messages(page) => page.total,
            ListPage::Threads(page) => page.total,
        }
    }

    /// Wait for an event matching `wanted`, failing after a while.
    pub(crate) fn hear(
        &self,
        events: &async_channel::Receiver<EventEnvelope>,
        wanted: impl Fn(&Event) -> bool,
    ) -> Event {
        self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    let envelope = events.recv().await.expect("the host is running");
                    if wanted(&envelope.event) {
                        return envelope.event;
                    }
                }
            })
            .await
            .expect("the event arrived")
        })
    }

    /// Everything that arrives within a short quiet period.
    fn drain(&self, events: &async_channel::Receiver<EventEnvelope>) -> Vec<Event> {
        self.rt.block_on(async {
            let mut heard = Vec::new();
            while let Ok(Ok(envelope)) =
                tokio::time::timeout(Duration::from_millis(300), events.recv()).await
            {
                heard.push(envelope.event);
            }
            heard
        })
    }

    fn send(&self, client: &Client, command: Command) {
        self.rt.block_on(client.send(command)).expect("sent");
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // The host's runtime cannot be dropped from inside another runtime's
        // `block_on`, and this is outside every one of them.
        self.host.take();
    }
}

fn archive() -> Command {
    Command::Archive {
        target: MessageTarget::Selection,
    }
}

#[test]
fn a_frontend_archives_through_the_host_and_its_list_empties() {
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Test);
    assert_eq!(world.inbox_rows(&client), 1);

    world.send(&client, archive());

    world.hear(&events, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
    assert_eq!(world.inbox_rows(&client), 0);
}

#[test]
fn what_one_frontend_changed_reaches_the_other() {
    let world = World::new();
    let (terminal, _) = world.frontend(ClientKind::Tui);
    let (_desktop, desktop_events) = world.frontend(ClientKind::Gtk);

    world.send(&terminal, archive());

    world.hear(&desktop_events, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
}

#[test]
fn an_undo_offer_is_only_for_the_frontend_that_can_take_it() {
    let world = World::new();
    let (terminal, terminal_events) = world.frontend(ClientKind::Tui);
    let (_desktop, desktop_events) = world.frontend(ClientKind::Gtk);

    world.send(&terminal, archive());

    world.hear(&terminal_events, |event| {
        matches!(event, Event::ActionCompleted { undoable: true, .. })
    });
    let desktop_heard = world.drain(&desktop_events);
    assert!(
        desktop_heard
            .iter()
            .any(|event| matches!(event, Event::MessagesRemoved { .. })),
        "the desktop hears what changed: {desktop_heard:?}"
    );
    assert!(
        !desktop_heard
            .iter()
            .any(|event| matches!(event, Event::ActionCompleted { .. })),
        "but not the terminal's undo offer: {desktop_heard:?}"
    );
}

#[test]
fn undo_takes_back_only_what_this_frontend_did() {
    let world = World::new();
    let (terminal, terminal_events) = world.frontend(ClientKind::Tui);
    let (desktop, desktop_events) = world.frontend(ClientKind::Gtk);

    world.send(&terminal, archive());
    world.hear(&terminal_events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });

    // The desktop did nothing, so it has nothing to undo.
    world.send(&desktop, Command::Undo);
    world.hear(&desktop_events, |event| {
        matches!(event, Event::CommandRejected { .. })
    });
    assert_eq!(world.inbox_rows(&desktop), 0, "the archive stands");

    // The terminal does.
    world.send(&terminal, Command::Undo);
    world.hear(&terminal_events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(world.inbox_rows(&desktop), 1, "and the message is back");
}

#[test]
fn two_commands_sent_back_to_back_run_in_the_order_they_were_sent() {
    // Archive, then undo, without waiting between them. In order, the
    // message ends where it started; reversed, the undo finds nothing to
    // take back and the archive stands. Answering each request on a spawned
    // task made that a race; commands are answered in order now.
    let world = World::new();
    let (client, events) = world.frontend(ClientKind::Tui);

    world.rt.block_on(async {
        let archiving = client.send(archive());
        let undoing = client.send(Command::Undo);
        let (archived, undone) = tokio::join!(archiving, undoing);
        archived.expect("sent");
        undone.expect("sent");
    });

    world.hear(&events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(world.inbox_rows(&client), 1);
}

#[test]
fn a_frontend_can_list_the_accounts_for_its_sidebar() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let accounts = world.rt.block_on(client.accounts()).expect("the accounts");
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].enabled);
}

#[test]
fn a_frontend_reads_a_body_or_hears_why_there_is_none() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    // The fixture's message has headers and no body yet: the ordinary state
    // of a mailbox mid-backfill, not a fault.
    let body = world
        .rt
        .block_on(client.body(world.message))
        .expect("an answer");
    assert_eq!(body, postio_client::protocol::Body::Partial);
}

#[test]
fn a_frontend_reads_a_conversations_messages_oldest_first() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    // A second, later message joined to the first one's thread.
    let (thread, later) = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let first = MessageRepository::new(&connection)
            .get(world.message)
            .await
            .expect("read")
            .expect("there");
        let mut reply = Message::new(
            first.account_id,
            world.inbox,
            Utc::now() + chrono::Duration::minutes(5),
        );
        let reply = MessageRepository::new(&connection)
            .create(&mut reply)
            .await
            .expect("a reply");
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let mut thread = postio_model::Thread::new(first.account_id);
        threads.create(&mut thread).await.expect("a thread");
        for member in [world.message, reply] {
            threads
                .add_message(thread.id, member)
                .await
                .expect("membership");
        }
        (thread.id, reply)
    });
    let members = world
        .rt
        .block_on(client.conversation(thread))
        .expect("the conversation");
    let ids: Vec<MessageId> = members.iter().map(|row| row.id).collect();
    assert_eq!(ids, vec![world.message, later]);
}

#[test]
fn unsubscribing_records_the_activation_against_the_messages_list() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    // The fixture message has no List-Id and no sender, so it names no list:
    // a refusal, said as a sentence, and nothing recorded.
    assert!(
        world
            .rt
            .block_on(client.unsubscribe(world.message))
            .is_err()
    );

    let (account, message) = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let first = MessageRepository::new(&connection)
            .get(world.message)
            .await
            .expect("read")
            .expect("there");
        let mut listed = Message::new(first.account_id, world.inbox, Utc::now());
        listed.list_id = Some("weekly.news.example.org".into());
        let id = MessageRepository::new(&connection)
            .create(&mut listed)
            .await
            .expect("a message");
        (first.account_id, id)
    });
    let list = world
        .rt
        .block_on(client.unsubscribe(message))
        .expect("unsubscribed");
    assert_eq!(list, "weekly.news.example.org");
    let recorded = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::UnsubscribeRepository::new(&connection)
            .for_account(account)
            .await
            .expect("the activations")
    });
    assert_eq!(recorded.len(), 1);
}

#[test]
fn a_frontend_lists_a_messages_parts_and_saves_one() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("the same blob store");
    let blob = blobs.put(b"%PDF-1.7 the report").expect("stored");
    let message = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let first = MessageRepository::new(&connection)
            .get(world.message)
            .await
            .expect("read")
            .expect("there");
        let mut message = Message::new(first.account_id, world.inbox, Utc::now());
        let mut report =
            postio_model::Attachment::new(MessageId::UNASSIGNED, "application/pdf", 19);
        report.filename = Some("report.pdf".into());
        report.part_id = Some("2".into());
        report.blob_id = Some(blob);
        message.attachments.push(report);
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message")
    });

    let parts = world.rt.block_on(client.parts(message)).expect("the parts");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].filename.as_deref(), Some("report.pdf"));

    let out = tempfile::tempdir().unwrap();
    let to = out.path().join("report.pdf");
    let written = world
        .rt
        .block_on(client.save_part(message, parts[0].id, to.clone()))
        .expect("saved");
    assert_eq!(written, to);
    assert_eq!(std::fs::read(&to).unwrap(), b"%PDF-1.7 the report");
}

/// A draft to `account`, saying `words`.
fn a_draft(account: postio_model::AccountId, words: &str) -> postio_model::Draft {
    let mut draft = postio_model::Draft::new(account);
    draft.to = vec![postio_model::EmailAddress::new(
        None::<String>,
        "grace@example.net",
    )];
    draft.subject = "Tide gate".to_owned();
    draft.body.text = Some(words.to_owned());
    draft.body_markdown = Some(words.to_owned());
    draft
}

fn drafts_in(world: &World, account: postio_model::AccountId) -> Vec<postio_model::Draft> {
    world
        .rt
        .block_on(crate::compose::drafts_of(&world.database, account))
        .expect("the drafts")
}

#[test]
fn two_saves_made_back_to_back_write_one_draft() {
    // An autosave tick and the next keystroke's tick, the second made before
    // the first landed: the second carries no id yet, and must still update
    // the row the first made rather than start another.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;

    let (first, second) = world.rt.block_on(async {
        let first = client.save_draft(1, a_draft(account, "Half a"));
        let second = client.save_draft(1, a_draft(account, "Half a **sentence**"));
        tokio::join!(first, second)
    });

    let first = first.expect("saved");
    assert_eq!(second.expect("saved"), first);
    let drafts = drafts_in(&world, account);
    assert_eq!(drafts.len(), 1, "one composition, one row");
    assert_eq!(
        drafts[0].body_markdown.as_deref(),
        Some("Half a **sentence**")
    );
}

#[test]
fn a_draft_sent_from_a_frontend_is_queued_and_a_discard_after_it_keeps_it() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;

    world.rt.block_on(async {
        client
            .save_draft(7, a_draft(account, "Ready"))
            .await
            .expect("saved");
        client
            .queue_send(7, a_draft(account, "Ready."), None)
            .await
            .expect("queued");
        // Closing the composer after a send is a discard of that composition;
        // it must not take the queued draft with it.
        client.discard_draft(7, None).await.expect("answered");
    });

    let drafts = drafts_in(&world, account);
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].state, postio_model::DraftState::Queued);
    assert_eq!(drafts[0].body.text.as_deref(), Some("Ready."));
}

#[test]
fn a_composition_closed_empty_leaves_no_draft() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;

    world.rt.block_on(async {
        client
            .save_draft(3, a_draft(account, "x"))
            .await
            .expect("saved");
        client.discard_draft(3, None).await.expect("answered");
    });

    assert!(drafts_in(&world, account).is_empty());
}

#[test]
fn a_frontend_searches_and_hears_which_messages_matched() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    let matching = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
        let mut message = Message::new(account, world.inbox, Utc::now());
        message.subject = Some("Tide gate interlock".into());
        message.sync.body_state = postio_model::BodyState::Full;
        let messages = MessageRepository::new(&connection);
        let id = messages.create(&mut message).await.expect("a message");
        postio_index::index::index_body(&connection, id.get(), Some("the interlock report"))
            .await
            .expect("indexed");
        id
    });

    let found = world
        .rt
        .block_on(client.search(postio_client::protocol::Search {
            account: postio_model::AccountScope::Account(account),
            query: "interlock".into(),
            newest_first: false,
        }))
        .expect("an answer")
        .expect("the store was read");
    assert_eq!(found.ids, vec![matching]);
    assert_eq!(found.hits, 1);
    assert!(!found.capped);
}

#[test]
fn postio_diag_asks_the_daemon_for_its_reports() {
    // T083: the daemon owns the store while it runs, so the diagnostic
    // reports are its to run.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Test);
    let census = world
        .rt
        .block_on(client.diagnose("census".into()))
        .expect("a report");
    assert!(census.contains("messages"), "{census}");
    assert!(
        census
            .lines()
            .any(|line| line.starts_with("messages") && line.trim_end().ends_with('1')),
        "the fixture's one message is counted: {census}"
    );
    assert!(
        world
            .rt
            .block_on(client.diagnose("nonsense".into()))
            .is_err()
    );
}

/// A network that answers nothing: discovery falls back to the provider
/// table, and nothing dials.
struct Offline;

#[async_trait::async_trait]
impl postio_account::discovery::DiscoveryTransport for Offline {
    async fn autoconfig(
        &self,
        _endpoint: postio_account::discovery::AutoconfigEndpoint<'_>,
        _cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoveryAutoconfig,
        postio_account::discovery::TransportError,
    > {
        Err(postio_account::discovery::TransportError::new("offline"))
    }

    async fn srv(
        &self,
        _domain: &str,
        _cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoverySrvReport,
        postio_account::discovery::TransportError,
    > {
        Err(postio_account::discovery::TransportError::new("offline"))
    }

    async fn mx(
        &self,
        _domain: &str,
        _cancel: &postio_account::discovery::CancelToken,
    ) -> Result<Vec<String>, postio_account::discovery::TransportError> {
        Err(postio_account::discovery::TransportError::new("offline"))
    }
}

/// A network whose one answer is `example.test`'s autoconfig document, as a
/// provider publishes it; nothing else answers and nothing dials.
struct Publishing;

const AUTOCONFIG: &str = r#"<clientConfig version="1.1">
  <emailProvider id="example.test">
    <domain>example.test</domain>
    <displayName>Example Mail</displayName>
    <incomingServer type="imap">
      <hostname>mail.example.test</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </incomingServer>
    <outgoingServer type="smtp">
      <hostname>send.example.test</hostname>
      <port>465</port>
      <socketType>SSL</socketType>
      <username>%EMAILADDRESS%</username>
      <authentication>password-cleartext</authentication>
    </outgoingServer>
  </emailProvider>
</clientConfig>"#;

#[async_trait::async_trait]
impl postio_account::discovery::DiscoveryTransport for Publishing {
    async fn autoconfig(
        &self,
        _endpoint: postio_account::discovery::AutoconfigEndpoint<'_>,
        _cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoveryAutoconfig,
        postio_account::discovery::TransportError,
    > {
        Ok(serde_xml_rs::from_str(AUTOCONFIG).expect("the document parses"))
    }

    async fn srv(
        &self,
        domain: &str,
        cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoverySrvReport,
        postio_account::discovery::TransportError,
    > {
        Offline.srv(domain, cancel).await
    }

    async fn mx(
        &self,
        domain: &str,
        cancel: &postio_account::discovery::CancelToken,
    ) -> Result<Vec<String>, postio_account::discovery::TransportError> {
        Offline.mx(domain, cancel).await
    }
}

/// A host whose onboarding reaches no network: discovery reads
/// [`Publishing`]'s document, and the proof signs in to `mock`.
fn onboarding_world(mock: postio_account::backend::MockBackend) -> World {
    World::configured(move |wiring| {
        wiring
            .with_discovery(std::sync::Arc::new(Publishing))
            .with_mail(postio_session::MailOverride {
                backend: std::sync::Arc::new(mock),
                smtp: std::sync::Arc::new(postio_smtp::transport::ScriptedConnector::new(
                    postio_smtp::transport::SmtpScript::new("220 ready"),
                )),
            })
    })
}

#[test]
fn a_frontend_finds_a_domains_servers_through_the_daemon() {
    // US7: discovery, from the terminal, is the desktop's.
    let world = onboarding_world(postio_account::backend::MockBackend::new());
    let (client, _) = world.frontend(ClientKind::Tui);
    let status = world
        .rt
        .block_on(client.discover("ada@example.test".into()))
        .expect("an answer");
    match status {
        postio_ui::onboarding::Status::Found(settings) => {
            assert_eq!(settings.imap.host, "mail.example.test");
        }
        other => panic!("{other:?}"),
    }
}

fn submission(password: &str) -> postio_ui::onboarding::Submission {
    postio_ui::onboarding::Submission {
        address: "grace@example.test".into(),
        name: "Grace".into(),
        password: password.into(),
        settings: postio_ui::onboarding::Settings {
            imap: postio_ui::onboarding::Server {
                host: "mail.example.test".into(),
                port: 993,
                security: postio_model::TransportSecurity::Tls,
            },
            smtp: postio_ui::onboarding::Server {
                host: "send.example.test".into(),
                port: 465,
                security: postio_model::TransportSecurity::Tls,
            },
            login: "grace@example.test".into(),
            ..Default::default()
        },
        oauth_client: None,
    }
}

#[test]
fn a_frontend_adds_an_account_through_the_daemon() {
    let world = onboarding_world(postio_account::backend::MockBackend::new());
    let (client, _) = world.frontend(ClientKind::Tui);
    world
        .rt
        .block_on(client.add_account(submission("correct horse")))
        .expect("added");
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    assert!(
        accounts
            .iter()
            .any(|account| account.address.address == "grace@example.test")
    );
}

#[test]
fn a_refused_password_is_said_as_the_desktop_says_it() {
    // T085: the same sentence, from the same `explain`.
    let mock = postio_account::backend::MockBackend::new();
    mock.fail_all(postio_account::backend::Fault::AuthFailed);
    let world = onboarding_world(mock);
    let (client, _) = world.frontend(ClientKind::Tui);
    let error = world
        .rt
        .block_on(client.add_account(submission("wrong")))
        .expect_err("refused");
    let desktop =
        postio_session::onboarding::explain(&postio_account::backend::BackendError::Auth {
            account: "grace@example.test".into(),
            reason: "refused".into(),
        });
    assert_eq!(error.message(), desktop);
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    assert!(
        !accounts
            .iter()
            .any(|account| account.address.address == "grace@example.test"),
        "nothing is written for a refused sign-in"
    );
}

#[test]
fn a_submission_never_shows_its_password() {
    let shown = format!("{:?}", submission("correct horse"));
    assert!(!shown.contains("correct horse"), "{shown}");
}

#[test]
fn a_frontend_disables_removes_and_restores_an_account_through_the_daemon() {
    // T087: the settings' account commands, from the terminal.
    use postio_client::protocol::AccountOp;
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    let accounts = || world.rt.block_on(client.accounts()).expect("accounts");

    world
        .rt
        .block_on(client.account(AccountOp::SetEnabled {
            account,
            enabled: false,
        }))
        .expect("disabled");
    assert!(!accounts()[0].enabled);

    world
        .rt
        .block_on(client.account(AccountOp::Remove(account)))
        .expect("removed");
    assert!(
        accounts().is_empty(),
        "a removed account is gone from the list"
    );
    world
        .rt
        .block_on(client.account(AccountOp::Restore(account)))
        .expect("restored");
    assert_eq!(accounts().len(), 1, "and comes back on undo");

    world
        .rt
        .block_on(client.account(AccountOp::SetDefault(account)))
        .expect("made the default");
}

#[test]
fn the_finder_is_told_the_accounts_correspondents_and_labels() {
    // T066: `@` and `+` offer what the store knows, read through the daemon.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::ContactRepository::new(&connection)
            .record(
                Some(account),
                &postio_model::EmailAddress::new(None::<String>, "grace@example.test"),
                Utc::now(),
            )
            .await
            .expect("a correspondent");
        postio_storage::repository::LabelRepository::new(&connection)
            .create(&mut postio_model::Label::new(account, "Receipts"))
            .await
            .expect("a label");
    });

    let correspondents = world
        .rt
        .block_on(client.correspondents(account))
        .expect("correspondents");
    assert_eq!(
        correspondents
            .iter()
            .map(|contact| contact.address.address.as_str())
            .collect::<Vec<_>>(),
        ["grace@example.test"]
    );
    let labels = world.rt.block_on(client.labels(account)).expect("labels");
    assert_eq!(
        labels
            .iter()
            .map(|label| label.name.as_str())
            .collect::<Vec<_>>(),
        ["Receipts"]
    );
}

#[test]
fn the_host_says_which_verbs_a_frontend_can_send_it() {
    // A frontend filters its gestures by what the bus answers, so the ones
    // another consumer owns do not come back "not wired up in this build".
    let world = World::new();
    let wired = world.host.as_ref().expect("a host").wired();
    for verb in [
        postio_core::CommandId::Archive,
        postio_core::CommandId::Undo,
        postio_core::CommandId::Refresh,
    ] {
        assert!(wired.contains(&verb), "{verb:?} in {wired:?}");
    }
    assert!(!wired.contains(&postio_core::CommandId::Compose));
}

#[test]
fn a_host_over_a_wiring_built_elsewhere_serves_its_store_and_its_news() {
    // T018: the desktop's surfaces and its integration suites build a
    // `Wiring` of their own; a host adopts it rather than opening another.
    let world = World::new();
    let hub = postio_core::bridge::EventHub::new();
    let (commands, _queued) = postio_core::bridge::command_channel();
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("a blob store");
    let wiring = postio_session::Wiring::new(
        world.database.clone(),
        blobs,
        world.rt.handle().clone(),
        hub.sink(),
        commands,
    );
    let host = Host::over(wiring);
    let client = host.connect(ClientKind::Gtk);
    let events = client.events();

    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    assert_eq!(accounts.len(), 1);

    let account = accounts[0].id;
    hub.sink().emit(Event::MailboxesChanged { account });
    let heard = world
        .rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(5), events.recv()).await })
        .expect("in time")
        .expect("an event");
    assert_eq!(heard.event, Event::MailboxesChanged { account });
}

#[test]
fn a_host_over_a_wiring_with_one_event_reader_still_serves_its_store() {
    // Most of the desktop's integration suites wire their events to one
    // stream they read themselves; a client over such a wiring can still
    // read.
    let world = World::new();
    let (sink, _events) = event_channel();
    let (commands, _queued) = postio_core::bridge::command_channel();
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("a blob store");
    let wiring = postio_session::Wiring::new(
        world.database.clone(),
        blobs,
        world.rt.handle().clone(),
        sink,
        commands,
    );
    let client = Host::over(wiring).connect(ClientKind::Gtk);
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    assert_eq!(accounts.len(), 1);
}

/// A second message in the fixture's account and inbox, shaped by `shape`.
fn another_message(world: &World, shape: impl FnOnce(&mut Message)) -> MessageId {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let first = MessageRepository::new(&connection)
            .get(world.message)
            .await
            .expect("read")
            .expect("there");
        let mut message = Message::new(first.account_id, world.inbox, Utc::now());
        shape(&mut message);
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message")
    })
}

#[test]
fn a_reading_pane_reads_everything_it_draws_of_several_messages_in_one_call() {
    // The desktop's pane draws a header, a parts row and a body from one
    // read; a conversation is every member's in one read (#1609).
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let listed = another_message(&world, |message| {
        message.subject = Some("Minutes".into());
        message.list_id = Some("minutes.example.org".into());
    });

    let readings = world
        .rt
        .block_on(client.readings(vec![world.message, listed], false))
        .expect("the readings");
    assert_eq!(client.counts().of("Readings"), 1, "one call for both");
    assert_eq!(
        readings
            .iter()
            .map(|reading| reading.message)
            .collect::<Vec<_>>(),
        vec![world.message, listed],
        "in the order asked"
    );
    assert_eq!(readings[0].body, postio_client::protocol::Body::Partial);
    let row = readings[1].row.as_deref().expect("the row came with it");
    assert_eq!(row.subject.as_deref(), Some("Minutes"));
    assert_eq!(row.list_id.as_deref(), Some("minutes.example.org"));
    assert_eq!(readings[1].send_state, None, "ordinary mail is not sending");

    // Offline is the frontend's to say: the host has no reachability of its
    // own, and "downloading" about a body nothing is fetching is untrue.
    let offline = world
        .rt
        .block_on(client.readings(vec![world.message], true))
        .expect("the reading");
    assert_eq!(offline[0].body, postio_client::protocol::Body::Offline);
}

#[test]
fn the_conversation_after_the_cursor_is_read_ahead_in_one_call() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let later = another_message(&world, |message| {
        message.received_at = Utc::now() + chrono::Duration::minutes(5);
    });
    let thread = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let account = MessageRepository::new(&connection)
            .get(world.message)
            .await
            .expect("read")
            .expect("there")
            .account_id;
        let threads = postio_storage::repository::ThreadRepository::new(&connection);
        let mut thread = postio_model::Thread::new(account);
        threads.create(&mut thread).await.expect("a thread");
        for member in [world.message, later] {
            threads
                .add_message(thread.id, member)
                .await
                .expect("membership");
        }
        thread.id
    });

    let all = world
        .rt
        .block_on(client.thread_readings(thread, 50, false))
        .expect("the conversation");
    assert_eq!(
        all.iter()
            .map(|reading| reading.message)
            .collect::<Vec<_>>(),
        vec![world.message, later],
        "oldest first"
    );
    let first = world
        .rt
        .block_on(client.thread_readings(thread, 1, false))
        .expect("the conversation");
    assert_eq!(first.len(), 1, "no more than the limit is read");
    assert_eq!(client.counts().of("ThreadReadings"), 2);
}

#[test]
fn an_inline_image_resolves_only_inside_the_message_that_declares_it() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("the same blob store");
    let blob = blobs.put(b"\x89PNG a logo").expect("stored");
    let declaring = another_message(&world, |message| {
        let mut logo = postio_model::Attachment::new(MessageId::UNASSIGNED, "image/png", 11);
        logo.content_id = Some("logo@example.com".into());
        logo.blob_id = Some(blob);
        message.attachments.push(logo);
    });

    let found = world
        .rt
        .block_on(client.inline_part(declaring, "logo@example.com".into()))
        .expect("an answer")
        .expect("the part is here");
    assert_eq!(found.0, b"\x89PNG a logo");
    assert_eq!(found.1, "image/png");

    // Another sender's message cannot address this one's parts.
    let elsewhere = world
        .rt
        .block_on(client.inline_part(world.message, "logo@example.com".into()))
        .expect("an answer");
    assert_eq!(elsewhere, None);
}

#[test]
fn saving_every_part_writes_each_and_counts_what_could_not_be() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("the same blob store");
    let blob = blobs.put(b"one,two").expect("stored");
    let message = another_message(&world, |message| {
        let mut figures = postio_model::Attachment::new(MessageId::UNASSIGNED, "text/csv", 7);
        figures.filename = Some("figures.csv".into());
        figures.part_id = Some("2".into());
        figures.blob_id = Some(blob);
        message.attachments.push(figures);
    });
    let part = world.rt.block_on(client.parts(message)).expect("the parts")[0].id;
    let out = tempfile::tempdir().unwrap();

    let failed = world
        .rt
        .block_on(client.save_parts(
            message,
            vec![
                // A part this message does not have: refused, and the rest
                // still saved.
                (
                    postio_model::ids::AttachmentId::new(987_654),
                    out.path().join("nothing"),
                ),
                (part, out.path().join("figures.csv")),
            ],
        ))
        .expect("an answer");
    assert_eq!(failed, 1);
    assert_eq!(
        std::fs::read(out.path().join("figures.csv")).unwrap(),
        b"one,two"
    );
    assert!(!out.path().join("nothing").exists());
    assert_eq!(client.counts().of("SaveParts"), 1, "one call for the batch");
}
