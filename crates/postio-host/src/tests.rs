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
        let host = Host::start(database, blobs, |wiring| {
            wiring.with_secrets(std::sync::Arc::new(MemorySecretStore::new()))
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
