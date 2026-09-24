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
    pub(crate) account: postio_model::AccountId,
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
        let (database, inbox, message, account) = rt.block_on(async {
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
            let account = account.id;
            (database, inbox, message, account)
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
            account,
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

    /// The inbox.
    pub(crate) fn inbox(&self) -> MailboxId {
        self.inbox
    }

    /// The message in it.
    pub(crate) fn message(&self) -> MessageId {
        self.message
    }

    /// A frontend looking at the inbox with the cursor on the message.
    pub(crate) fn frontend(
        &self,
        kind: ClientKind,
    ) -> (Client, async_channel::Receiver<EventEnvelope>) {
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
fn a_crashed_session_hands_back_the_draft_being_written_and_a_clean_one_does_not() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;

    // A store this binary never opened is not a crash.
    let first = world.rt.block_on(client.recover_draft(account));
    assert_eq!(first, Ok(None));

    // Something written, and a composition holding only whitespace.
    world.rt.block_on(async {
        client
            .save_draft(1, a_draft(account, "Half a sentence"))
            .await
            .expect("saved");
        let mut untouched = postio_model::Draft::new(account);
        untouched.body.text = Some("  \n".to_owned());
        client.save_draft(2, untouched).await.expect("saved");
    });

    // The session never ended: the next start is a crash, and gets back the
    // draft with words in it, not the empty one.
    let recovered = world
        .rt
        .block_on(client.recover_draft(account))
        .expect("an answer")
        .expect("the draft being written");
    assert_eq!(recovered.body.text.as_deref(), Some("Half a sentence"));

    // A clean end, and the next start leaves the draft parked.
    world
        .rt
        .block_on(postio_session::end_session(&world.database));
    assert_eq!(world.rt.block_on(client.recover_draft(account)), Ok(None));
    assert_eq!(client.counts().of("RecoverDraft"), 3);
}

#[test]
fn a_file_is_attached_as_the_type_the_frontend_sniffed_and_its_bytes_read_back() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let directory = tempfile::tempdir().expect("a directory");
    let path = directory.path().join("minutes");
    std::fs::write(&path, b"the minutes, unlabelled").expect("written");

    let attached = world
        .rt
        .block_on(client.attach_as(path.clone(), Some("text/x-minutes".into())))
        .expect("an answer")
        .expect("stored");
    assert_eq!(attached.mime_type, "text/x-minutes");
    assert_eq!(attached.filename.as_deref(), Some("minutes"));

    let blob = attached.blob_id.expect("a blob");
    let bytes = world
        .rt
        .block_on(client.attachment_bytes(blob))
        .expect("an answer");
    assert_eq!(bytes.as_deref(), Some(&b"the minutes, unlabelled"[..]));

    // With no type, the host guesses, as it does for the terminal.
    let guessed = world
        .rt
        .block_on(client.attach(path))
        .expect("an answer")
        .expect("stored");
    assert_eq!(guessed.mime_type, "application/octet-stream");

    let missing = world
        .rt
        .block_on(client.attachment_bytes(postio_model::ids::BlobId::new("0".repeat(64))))
        .expect("an answer");
    assert_eq!(missing, None);
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
fn a_frontend_in_another_process_is_told_the_same_verbs() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let told = world.rt.block_on(client.wired()).expect("an answer");
    assert_eq!(told, world.host().wired());
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

/// A message in the fixture's inbox whose body says `body`, indexed the way
/// the backfill indexes it.
fn an_indexed_message(world: &World, subject: &str, body: &str) -> MessageId {
    let message = another_message(world, |message| {
        message.subject = Some(subject.into());
        message.sync.body_state = postio_model::BodyState::Full;
    });
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
        MessageRepository::new(&connection)
            .set_body(
                message,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("the body");
        postio_index::index::index_body(&connection, message.get(), Some(body))
            .await
            .expect("indexed");
    });
    message
}

#[test]
fn the_desktop_search_answers_its_hits_with_an_excerpt_then_its_columns() {
    // The desktop's bar draws more than the terminal's list: the focused
    // hit's excerpt, the sender and folder from the index, and a second
    // read for the scope counts beside the results.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    let matching = an_indexed_message(
        &world,
        "Tide gate",
        "The interlock on the tide gate was tested on Thursday.",
    );
    let query = postio_search::parse("interlock", Utc::now().date_naive());
    let scope = postio_model::AccountScope::Account(account);

    let results = world
        .rt
        .block_on(client.search_hits(
            scope,
            query.clone(),
            postio_search::facets::Scope::AllMail,
            postio_search::ResultOrder::Relevance,
            1,
        ))
        .expect("an answer")
        .expect("the store was read");
    assert_eq!(
        results
            .hits
            .iter()
            .map(|hit| hit.message_id)
            .collect::<Vec<_>>(),
        vec![matching]
    );
    assert_eq!(results.total_hits, 1);
    let marked = postio_search::highlight::from_snippet(&results.hits[0].snippet);
    assert_eq!(
        marked
            .matches
            .iter()
            .map(|range| &marked.text[range.clone()])
            .collect::<Vec<_>>(),
        vec!["interlock"],
        "the focused hit is excerpted where it matched"
    );

    let facets = world
        .rt
        .block_on(client.facets(scope, query, postio_search::facets::Scope::AllMail))
        .expect("an answer")
        .expect("the counts ran");
    assert_eq!(facets.hits(postio_search::facets::Scope::AllMail), 1);
    assert_eq!(facets.hits(postio_search::facets::Scope::Inbox), 1);
    assert_eq!(client.counts().of("SearchHits"), 1);
    assert_eq!(client.counts().of("Facets"), 1);
}

#[test]
fn a_search_preview_reads_the_stored_words_or_nothing() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let with_words = an_indexed_message(&world, "Minutes", "Half past twelve?");

    let body = world
        .rt
        .block_on(client.stored_body(with_words))
        .expect("an answer");
    assert_eq!(body.text.as_deref(), Some("Half past twelve?"));

    // Headers only: the preview keeps the excerpt it already drew.
    let bare = world
        .rt
        .block_on(client.stored_body(world.message))
        .expect("an answer");
    assert_eq!(bare, postio_model::MessageBody::default());
}

#[test]
fn messages_dragged_out_are_written_as_the_bytes_the_server_sent_in_one_call() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let blobs = postio_storage::BlobStore::open(world.blob_dir.clone(), &test_support::blob_keys())
        .expect("the same blob store");
    let one = blobs.put(b"Subject: one\r\n\r\nOne.\r\n").expect("stored");
    let two = blobs.put(b"Subject: two\r\n\r\nTwo.\r\n").expect("stored");
    let first = another_message(&world, |message| message.raw_blob_id = Some(one));
    let second = another_message(&world, |message| message.raw_blob_id = Some(two));
    let out = tempfile::tempdir().unwrap();

    let written = world
        .rt
        .block_on(client.export_messages(vec![
            (second, out.path().join("Two.eml")),
            (first, out.path().join("One.eml")),
        ]))
        .expect("exported");
    assert_eq!(
        written,
        vec![out.path().join("Two.eml"), out.path().join("One.eml")],
        "in the order asked"
    );
    assert_eq!(
        std::fs::read(out.path().join("One.eml")).unwrap(),
        b"Subject: one\r\n\r\nOne.\r\n"
    );
    assert_eq!(client.counts().of("ExportMessages"), 1);
}

#[test]
fn the_settings_panel_reads_every_account_with_its_folders_in_one_call() {
    // One call for the whole panel, where the desktop read each account's
    // folders, roles and weight in turn.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let shown = world
        .rt
        .block_on(client.account_settings(true))
        .expect("the settings");
    assert_eq!(shown.len(), 1);
    let account = &shown[0];
    for folder in ["Archive", "Trash"] {
        assert!(
            account.folders.iter().any(|path| path == folder),
            "{folder} is offered: {:?}",
            account.folders
        );
    }
    assert!(
        account.weight.is_some(),
        "the weight is measured when asked for"
    );
    assert_eq!(client.counts().of("AccountSettings"), 1);

    let unweighed = world
        .rt
        .block_on(client.account_settings(false))
        .expect("the settings");
    assert_eq!(
        unweighed[0].weight, None,
        "and only then: it scans every message"
    );
}

#[test]
fn an_account_field_edited_in_the_settings_reaches_its_row() {
    use postio_client::protocol::AccountField;
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    world
        .rt
        .block_on(client.edit_account(account, AccountField::DisplayName("Work".into())))
        .expect("edited");
    world
        .rt
        .block_on(client.edit_account(account, AccountField::ImapPort(1993)))
        .expect("edited");
    let row = world.rt.block_on(client.accounts()).expect("accounts")[0].clone();
    assert_eq!(row.display_name, "Work");
    assert_eq!(row.incoming.port, 1993);
}

fn signatures_of(world: &World, account: postio_model::AccountId) -> Vec<postio_model::Signature> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::SignatureRepository::new(&connection)
            .list_for_account(account)
            .await
            .expect("the signatures")
    })
}

#[test]
fn a_signature_is_written_refused_by_name_edited_and_removed() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    world
        .rt
        .block_on(client.save_signature(account, None, "Work".into(), "Ada".into()))
        .expect("written");
    let written = signatures_of(&world, account);
    assert_eq!(written.len(), 1);

    let refused = world
        .rt
        .block_on(client.save_signature(account, None, "Work".into(), "Again".into()))
        .expect_err("a second of the same name");
    assert_eq!(
        refused.message(),
        "This account already has a signature called “Work”",
        "said as the person who typed it needs it"
    );

    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut rich = written[0].clone();
        rich.html = Some("<b>Ada</b>".into());
        postio_storage::repository::SignatureRepository::new(&connection)
            .update(&rich)
            .await
            .expect("a rich variant");
    });
    world
        .rt
        .block_on(client.save_signature(
            account,
            Some(written[0].id),
            "Work".into(),
            "Ada L.".into(),
        ))
        .expect("edited");
    let edited = signatures_of(&world, account);
    assert_eq!(edited[0].text, "Ada L.");
    assert_eq!(
        edited[0].html.as_deref(),
        Some("<b>Ada</b>"),
        "the rich variant this form does not show is kept"
    );

    world
        .rt
        .block_on(client.delete_signature(written[0].id))
        .expect("removed");
    assert!(signatures_of(&world, account).is_empty());
}

#[test]
fn a_rebuild_asked_for_from_the_settings_answers_when_it_is_over() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
    });
    world
        .rt
        .block_on(client.rebuild_index(account))
        .expect("rebuilt");
    assert_eq!(client.counts().of("RebuildIndex"), 1);
}

#[test]
fn the_egress_log_is_read_newest_first() {
    use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let log = postio_storage::repository::EgressLogRepository::new(&connection);
        for (seconds, host) in [(1, "mail.example.test"), (2, "send.example.test")] {
            log.record(&EgressEvent {
                at: chrono::DateTime::from_timestamp(1_790_000_000 + seconds, 0).expect("a time"),
                subsystem: EgressSubsystem::Imap,
                account: None,
                host: host.into(),
                port: 993,
                outcome: EgressOutcome::Connected,
            })
            .await
            .expect("recorded");
        }
    });
    let entries = world.rt.block_on(client.egress_log(1)).expect("the log");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.host.as_str())
            .collect::<Vec<_>>(),
        ["send.example.test"]
    );
}

#[test]
fn the_privacy_pane_reads_its_log_and_its_count_in_one_call() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    another_message(&world, |message| message.read_receipt_requested = true);
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let log = postio_storage::repository::UnsubscribeRepository::new(&connection);
        for (seconds, list) in [(1, "older.example.test"), (2, "newer.example.test")] {
            let at = chrono::DateTime::from_timestamp(1_790_000_000 + seconds, 0).expect("a time");
            log.record(&mut postio_model::UnsubscribeActivation::new(
                account, list, at,
            ))
            .await
            .expect("recorded");
        }
    });
    let log = world.rt.block_on(client.privacy_log()).expect("the log");
    assert_eq!(
        log.activations
            .iter()
            .map(|activation| activation.list_identifier.as_str())
            .collect::<Vec<_>>(),
        ["newer.example.test", "older.example.test"]
    );
    assert_eq!(log.read_receipts, 1);
    assert_eq!(client.counts().of("PrivacyLog"), 1);
}

#[test]
fn skipping_a_folders_backfill_answers_the_folders_as_they_now_stand() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let folders = world
        .rt
        .block_on(client.set_backfill_excluded(world.inbox, true))
        .expect("the folders");
    let inbox = folders
        .iter()
        .find(|mailbox| mailbox.id == world.inbox)
        .expect("the inbox is among them");
    assert!(inbox.backfill_excluded);
    assert!(
        folders.len() >= 3,
        "every folder of its account: {folders:?}"
    );
}

#[test]
fn the_orientation_is_unseen_until_it_is_retired() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    assert!(!world.rt.block_on(client.orientation_seen()).expect("asked"));
    world
        .rt
        .block_on(client.retire_orientation())
        .expect("retired");
    let (later, _) = world.frontend(ClientKind::Gtk);
    assert!(
        world.rt.block_on(later.orientation_seen()).expect("asked"),
        "every later run, whichever frontend asks"
    );
}

#[test]
fn an_account_a_frontend_proved_is_saved_by_the_host() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    world
        .rt
        .block_on(client.save_account(
            submission("correct horse"),
            postio_model::account::Backend::Imap,
        ))
        .expect("saved");
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    let saved = accounts
        .iter()
        .find(|account| account.address.address == "grace@example.test")
        .expect("the row");
    assert_eq!(saved.incoming.host, "mail.example.test");
}

#[test]
fn a_browser_sign_in_a_frontend_completed_is_saved_with_its_endpoints() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let mut signed_in = submission("");
    signed_in.oauth_client = Some(postio_ui::onboarding::OAuthClientSubmission {
        client_id: "postio-test".into(),
        client_secret: None,
    });
    let grant = postio_client::protocol::OAuthGrant {
        submission: signed_in,
        authorize_url: "https://auth.example.test/authorize".into(),
        token_url: "https://auth.example.test/token".into(),
        scopes: vec!["mail".into()],
        refresh_token_lifetime_days: Some(7),
        access_token: "access-sentinel".into(),
        refresh_token: Some("refresh-sentinel".into()),
        expires_in: Some(Duration::from_secs(3600)),
        token_type: "Bearer".into(),
        scope: None,
    };
    assert!(
        !format!("{grant:?}").contains("sentinel"),
        "a grant never shows its tokens"
    );
    world
        .rt
        .block_on(client.save_oauth_account(grant))
        .expect("saved");
    let accounts = world.rt.block_on(client.accounts()).expect("accounts");
    let saved = accounts
        .iter()
        .find(|account| account.address.address == "grace@example.test")
        .expect("the row");
    assert_eq!(saved.auth, postio_model::account::AuthMethod::XOAuth2);
    let oauth = saved.oauth.as_ref().expect("its sign-in");
    assert_eq!(oauth.token_url, "https://auth.example.test/token");
    assert_eq!(oauth.scopes, "mail");
}

#[test]
fn a_window_is_told_to_repair_an_account_the_keyring_has_no_password_for() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let route = world
        .rt
        .block_on(client.startup_route())
        .expect("an answer");
    match route {
        postio_client::protocol::StartupRoute::Onboard(Some(account)) => {
            assert_eq!(account.address.address, "test@example.com");
        }
        other => panic!("a row with no credential is not an account to open: {other:?}"),
    }
}

#[test]
fn a_window_opens_on_an_account_whose_password_the_keyring_has() {
    use postio_account::secret::{AccountKey, Password, SecretStore};
    let secrets = std::sync::Arc::new(MemorySecretStore::new());
    let world = World::configured({
        let secrets = secrets.clone();
        move |wiring| wiring.with_secrets(secrets)
    });
    world
        .rt
        .block_on(secrets.store(
            &AccountKey::new("test@example.com".to_owned()),
            &Password::new("app-specific"),
        ))
        .expect("stored");
    let (client, _) = world.frontend(ClientKind::Gtk);
    let route = world
        .rt
        .block_on(client.startup_route())
        .expect("an answer");
    assert!(
        matches!(route, postio_client::protocol::StartupRoute::Ready(_)),
        "{route:?}"
    );
}

/// Ask `read` until it answers `Some`, failing after a while: for work the
/// host does on its own time.
fn eventually<T>(world: &World, mut read: impl FnMut() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(found) = read() {
            return found;
        }
        assert!(std::time::Instant::now() < deadline, "it never happened");
        world.rt.block_on(async {
            tokio::time::sleep(Duration::from_millis(50)).await;
        });
    }
}

#[test]
fn a_connection_a_frontend_made_itself_reaches_the_egress_log() {
    use postio_model::egress::{EgressEvent, EgressOutcome, EgressSubsystem};
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    client.egress().record(EgressEvent {
        at: chrono::DateTime::from_timestamp(1_790_000_000, 0).expect("a time"),
        subsystem: EgressSubsystem::Discovery,
        account: None,
        host: "autoconfig.example.test".into(),
        port: 443,
        outcome: EgressOutcome::Connected,
    });
    let host = eventually(&world, || {
        let entries = world.rt.block_on(client.egress_log(10)).expect("the log");
        entries.first().map(|entry| entry.host.clone())
    });
    assert_eq!(host, "autoconfig.example.test");
}

#[test]
fn the_host_makes_bodies_already_on_disk_searchable_once_it_catches_up() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    // A body on disk the index never heard of: stored, not indexed.
    let message = another_message(&world, |message| {
        message.subject = Some("Minutes".into());
        message.sync.body_state = postio_model::BodyState::Full;
    });
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        // The index exists, as it does in every store opened for real.
        postio_index::index::ensure_schema(&connection)
            .await
            .expect("the index");
        MessageRepository::new(&connection)
            .set_body(
                message,
                &postio_storage::repository::StoredBody {
                    text: Some("Quarterly figures attached".to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("the body");
    });

    let search = || {
        world
            .rt
            .block_on(client.search(postio_client::protocol::Search {
                account: postio_model::AccountScope::Unified,
                query: "quarterly".into(),
                newest_first: true,
            }))
            .expect("an answer")
            .filter(|found| found.ids.contains(&message))
    };
    assert_eq!(search(), None, "not searchable until the index catches up");

    world.host().start_idle_passes();

    let found = eventually(&world, search);
    assert_eq!(found.ids, vec![message]);
}

/// A mail server holding one message in `INBOX`, as a synced account's
/// would.
fn server_with_one_message() -> postio_account::backend::MockBackend {
    use postio_account::backend::{MockBackend, MockMailbox, MockMessage};
    let raw = "Message-ID: <tide@example.com>\r\nFrom: Ada <ada@example.com>\r\nTo: Test User \
               <test@example.com>\r\nSubject: Tide gate\r\nDate: Tue, 22 Sep 2026 09:00:00 \
               +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe tide gate \
               opens at nine.\r\n";
    MockBackend::builder()
        .mailbox(MockMailbox::new("INBOX").message(MockMessage::from(raw.as_bytes().to_vec())))
        .mailbox(MockMailbox::new("Archive"))
        .mailbox(MockMailbox::new("Trash"))
        .build()
}

/// A host whose account can sync from `mock`, with no background body
/// fetching: a body arrives only because somebody asked for it.
fn syncing_world(mock: postio_account::backend::MockBackend) -> World {
    use postio_account::secret::{AccountKey, Password, SecretStore};
    let secrets = std::sync::Arc::new(MemorySecretStore::new());
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime for the keyring")
        .block_on(secrets.store(
            &AccountKey::new("test@example.com".to_owned()),
            &Password::new("password"),
        ))
        .expect("the password");
    World::configured(move |wiring| {
        wiring
            .with_secrets(secrets)
            .with_backfill(postio_runtime::BackfillPolicy {
                background: false,
                ..postio_runtime::BackfillPolicy::default()
            })
            .with_mail(postio_session::MailOverride {
                backend: std::sync::Arc::new(mock),
                smtp: std::sync::Arc::new(postio_smtp::transport::ScriptedConnector::new(
                    postio_smtp::transport::SmtpScript::new("220 ready"),
                )),
            })
    })
}

/// The inbox row with `subject`, once there is one.
fn row_titled(world: &World, client: &Client, subject: &str) -> Option<MessageId> {
    let page = world
        .rt
        .block_on(client.list_page(PageRequest {
            scope: ListScope::Mailbox(world.inbox),
            offset: 0,
            limit: 20,
        }))
        .ok()?;
    match page {
        ListPage::Messages(page) => page
            .rows
            .into_iter()
            .find(|row| row.subject.as_deref() == Some(subject))
            .map(|row| row.id),
        ListPage::Threads(page) => page
            .rows
            .into_iter()
            .find(|row| row.representative.subject.as_deref() == Some(subject))
            .map(|row| row.representative.id),
    }
}

#[test]
fn a_posted_fetch_body_brings_the_opened_messages_body_and_says_so() {
    // `Req::FetchBody`: a person opened a message whose body was never
    // fetched. With the background lane off, nothing else fetches it.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let (client, events) = world.frontend(ClientKind::Tui);
    world.host().start_syncing();
    let message = eventually(&world, || row_titled(&world, &client, "Tide gate"));
    assert_eq!(
        world.rt.block_on(client.body(message)).expect("an answer"),
        postio_client::protocol::Body::Partial,
        "headers only, before anybody asks"
    );
    assert!(mock.body_fetches().is_empty(), "nothing fetched a body yet");

    client.fetch_body(message);

    world.hear(
        &events,
        |event| matches!(event, Event::BodyLoaded { message: loaded, .. } if *loaded == message),
    );
    match world.rt.block_on(client.body(message)).expect("an answer") {
        postio_client::protocol::Body::Ready { body, .. } => {
            let text = body.text.expect("a text part");
            assert!(text.contains("The tide gate opens at nine."), "{text}");
        }
        other => panic!("the fetched body is not what `Body` answers: {other:?}"),
    }
    assert_eq!(
        mock.body_fetches(),
        vec!["INBOX".to_owned()],
        "fetched once"
    );
}

/// `count` messages in the inbox, oldest first, each with a raw-source blob
/// of `size` bytes of its own, as `reclaim.rs` shapes a store to evict from.
fn messages_with_blobs(world: &World, count: u8, size: usize) -> Vec<postio_model::ids::BlobId> {
    let blobs = world.host().wiring().blobs.clone();
    (0..count)
        .map(|index| {
            let blob = blobs.put(&vec![b'a' + index; size]).expect("a blob");
            let stored = blob.clone();
            another_message(world, move |message| {
                message.received_at =
                    chrono::DateTime::from_timestamp(1_000 + i64::from(index), 0).expect("a time");
                message.server.uid = Some(postio_model::ids::Uid::new(u32::from(index) + 1));
                message.server.uid_validity = Some(postio_model::ids::UidValidity::new(1));
                message.raw_blob_id = Some(stored);
            });
            blob
        })
        .collect()
}

#[test]
fn a_storage_ceiling_evicts_the_oldest_blobs_over_it_and_keeps_what_fits() {
    // `Req::StorageCeiling`: `[storage] max_bytes` changed in a frontend.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Gtk);
    let written = messages_with_blobs(&world, 3, 40_000);
    let blobs = world.host().wiring().blobs.clone();

    // No ceiling is no eviction.
    client.storage_ceiling(None);
    world
        .rt
        .block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });
    assert!(
        written.iter().all(|blob| blobs.contains(blob)),
        "none taken"
    );

    let budget = blobs.len_of(&written[2]).expect("its size") + 16;
    client.storage_ceiling(Some(budget));

    eventually(&world, || {
        (!blobs.contains(&written[0]) && !blobs.contains(&written[1])).then_some(())
    });
    assert!(
        blobs.contains(&written[2]),
        "the newest fits the ceiling and is kept"
    );
}

#[test]
fn start_sync_asked_twice_gives_the_account_one_engine() {
    // `Req::StartSync`: every window asks once its first frame is up, and
    // the daemon has usually asked already.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let (client, _) = world.frontend(ClientKind::Gtk);
    let (other, _) = world.frontend(ClientKind::Tui);
    let running = || {
        world
            .host()
            .inner
            .engines
            .running
            .lock()
            .expect("never poisoned")
            .keys()
            .copied()
            .collect::<Vec<_>>()
    };
    world
        .rt
        .block_on(async { tokio::time::sleep(Duration::from_millis(300)).await });
    assert!(running().is_empty(), "nothing syncs until it is asked to");
    assert!(mock.header_fetches().is_empty());

    // Two frontends at once, then one again once it is running.
    client.start_sync();
    other.start_sync();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));
    client.start_sync();
    world
        .rt
        .block_on(async { tokio::time::sleep(Duration::from_millis(500)).await });

    assert_eq!(running(), vec![world.account], "one engine, the account's");
    let inbox_syncs = mock
        .header_fetches()
        .iter()
        .filter(|path| *path == "INBOX")
        .count();
    assert_eq!(inbox_syncs, 1, "the inbox was synced once, by one engine");
}
