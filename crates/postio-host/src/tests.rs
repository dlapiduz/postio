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
    pub(crate) database: postio_storage::Store,
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

    /// The store under the host, for a test to read what a verb wrote.
    pub(crate) fn database(&self) -> &postio_storage::Store {
        &self.database
    }

    /// Quit, as a frontend's last window closing stops its host, and launch
    /// again over the same store and blobs: the next session.
    pub(crate) fn relaunch(&mut self) {
        if let Some(host) = self.host.take() {
            host.close();
        }
        let blobs =
            postio_storage::BlobStore::open(self.blob_dir.clone(), &test_support::blob_keys())
                .expect("a blob store");
        self.host = Some(
            Host::start(self.database.clone(), blobs, |wiring| {
                wiring.with_secrets(std::sync::Arc::new(MemorySecretStore::new()))
            })
            .expect("a host"),
        );
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
    pub(crate) fn drain(&self, events: &async_channel::Receiver<EventEnvelope>) -> Vec<Event> {
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

    pub(crate) fn send(&self, client: &Client, command: Command) {
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

/// Files `rfc` into `inbox`, threaded as a sync pass would, and answers
/// its conversation.
fn file_threaded(
    world: &World,
    account: postio_model::AccountId,
    inbox: MailboxId,
    rfc: &str,
) -> postio_model::ThreadId {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut message = Message::new(account, inbox, Utc::now());
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(rfc));
        message.subject = Some("Launch".to_owned());
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        postio_storage::repository::ThreadingRepository::new(&connection, account)
            .thread(&message)
            .await
            .expect("threaded")
            .thread_id
    })
}

#[test]
fn archiving_a_focus_row_received_at_two_addresses_archives_both_copies() {
    // Spec 007, Edge Cases: one inbox across all accounts. The announcement
    // reached both of the person's addresses, so Focus shows it once -- and
    // archiving that one row has to archive both copies, as a unified row's
    // group does, or the other copy is still in its inbox and the row comes
    // straight back.
    let world = World::new();
    let (second, second_inbox, second_archive) = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("a second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
        let archive = test_support::mailbox(&connection, &account, "Archive").await;
        (account.id, inbox.id, archive.id)
    });
    let first_copy = file_threaded(&world, world.account, world.inbox(), "<launch@example.net>");
    let second_copy = file_threaded(&world, second, second_inbox, "<launch@example.net>");
    let (focus, events) = world.frontend(ClientKind::Focus);
    let scope = ListScope::Focus(postio_model::FocusScope::Inbox);
    let page = |client: &Client| match world
        .rt
        .block_on(client.list_page(PageRequest {
            scope,
            offset: 0,
            limit: 20,
        }))
        .expect("a Focus page")
    {
        ListPage::Threads(page) => page,
        ListPage::Messages(_) => panic!("Focus's inbox lists conversations"),
    };
    let before = page(&focus);
    let row = before
        .rows
        .iter()
        .find(|row| row.id == Some(second_copy) || row.id == Some(first_copy))
        .expect("the announcement is listed");
    let mut copies: Vec<_> = row
        .id
        .into_iter()
        .chain(row.copies.iter().copied())
        .collect();
    copies.sort();
    let mut both = vec![first_copy, second_copy];
    both.sort();
    assert_eq!(copies, both, "one row, naming both copies");

    world.send(
        &focus,
        Command::Archive {
            target: MessageTarget::Threads(copies),
        },
    );
    world.hear(&events, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });

    let after = page(&focus);
    assert_eq!(after.total, before.total - 1, "the row left Focus's inbox");
    assert!(
        after.rows.iter().all(|row| {
            row.id != Some(first_copy)
                && row.id != Some(second_copy)
                && !row.copies.contains(&first_copy)
                && !row.copies.contains(&second_copy)
        }),
        "and no copy of it came back as a row of its own"
    );
    let archived = |mailbox: MailboxId| match world
        .rt
        .block_on(focus.list_page(PageRequest {
            scope: ListScope::Mailbox(mailbox),
            offset: 0,
            limit: 20,
        }))
        .expect("a folder page")
    {
        ListPage::Threads(page) => page.total,
        ListPage::Messages(page) => page.total,
    };
    let first_archive = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::MailboxRepository::new(&connection)
            .by_role(world.account, postio_model::mailbox::MailboxRole::Archive)
            .await
            .expect("a read")
            .expect("an archive")
            .id
    });
    assert_eq!(archived(first_archive), 1, "the first copy is archived");
    assert_eq!(
        archived(second_archive),
        1,
        "and the second, in its own account's Archive"
    );
}

/// Files a message about `subject` into `inbox` as the start of its own
/// conversation, threaded as a sync pass would.
fn file_about(
    world: &World,
    account: postio_model::AccountId,
    inbox: MailboxId,
    rfc: &str,
    subject: &str,
) -> MessageId {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut message = Message::new(account, inbox, Utc::now());
        message.rfc_message_id = Some(postio_model::RfcMessageId::new(rfc));
        message.subject = Some(subject.to_owned());
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        postio_storage::repository::ThreadingRepository::new(&connection, account)
            .thread(&message)
            .await
            .expect("threaded");
        id
    })
}

/// A second account beside the world's, with an inbox and an archive.
fn second_account(world: &World) -> (postio_model::AccountId, MailboxId, MailboxId) {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut account = postio_model::Account::new(
            "Second",
            postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
        );
        postio_storage::repository::AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .expect("a second account");
        let inbox = test_support::mailbox(&connection, &account, "INBOX").await;
        let archive = test_support::mailbox(&connection, &account, "Archive").await;
        (account.id, inbox.id, archive.id)
    })
}

#[test]
fn a_focus_select_all_archives_what_focus_lists_and_never_held_mail() {
    // T167: `X` then `a` in Focus. The selection is a predicate over what
    // Focus's inbox lists -- not the unified inbox, which also holds mail
    // kept for a digest -- and a row taken back out of it keeps every copy
    // it stands for, in every account.
    let world = World::new();
    let (second, second_inbox, second_archive) = second_account(&world);
    file_threaded(&world, world.account, world.inbox(), "<launch@example.net>");
    file_threaded(&world, second, second_inbox, "<launch@example.net>");
    let budget = file_about(
        &world,
        world.account,
        world.inbox(),
        "<budget@example.net>",
        "Budget",
    );
    let minutes = file_about(
        &world,
        second,
        second_inbox,
        "<minutes@example.org>",
        "Minutes",
    );
    let held = file_about(
        &world,
        world.account,
        world.inbox(),
        "<ledger@example.com>",
        "The Ledger, this week",
    );
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::DigestRepository::new(&connection)
            .hold(held, "The Ledger", Utc::now())
            .await
            .expect("held for its digest");
    });

    let listed = |client: &Client| match world
        .rt
        .block_on(client.list_page(PageRequest {
            scope: ListScope::Focus(postio_model::FocusScope::Inbox),
            offset: 0,
            limit: 20,
        }))
        .expect("a Focus page")
    {
        ListPage::Threads(page) => page,
        ListPage::Messages(_) => panic!("Focus's inbox lists conversations"),
    };
    let (focus, events) = world.frontend(ClientKind::Focus);
    let before = listed(&focus);
    assert_eq!(
        before.total, 4,
        "the launch once, the budget, the minutes, and the world's own message"
    );
    let launch = before
        .rows
        .iter()
        .find(|row| row.subject.as_deref() == Some("Launch"))
        .expect("the launch is listed");
    assert_eq!(launch.copies.len(), 1, "folded from both inboxes");

    // As Focus aims `X`, and then `x` on the launch's row: its
    // representative, the one id the row carries.
    let state = SharedState::default();
    let (sink, _) = event_channel();
    state.update(&sink, |app| {
        let mut said = app.open_view(postio_core::state::ViewScope::Focus {
            accounts: vec![world.account, second],
        });
        said.extend(app.select_all());
        said.extend(app.toggle_selection(launch.representative.id));
        said
    });
    let aimed = world.host().connect(ClientKind::Focus).with_state(state);
    let heard = aimed.events();
    world.send(
        &aimed,
        Command::Archive {
            target: MessageTarget::Selection,
        },
    );
    let said = world.drain(&heard);
    assert!(
        said.iter()
            .any(|event| matches!(event, Event::ActionCompleted { .. })),
        "the archive was not done: {said:?}"
    );
    drop(events);

    let after = listed(&focus);
    assert_eq!(
        after
            .rows
            .iter()
            .map(|row| row.subject.clone().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["Launch".to_owned()],
        "only the row taken back out is still listed"
    );
    assert_eq!(after.rows[0].copies.len(), 1, "with both its copies");

    let in_folder = |mailbox: MailboxId| -> Vec<Option<String>> {
        match world
            .rt
            .block_on(focus.list_page(PageRequest {
                scope: ListScope::Mailbox(mailbox),
                offset: 0,
                limit: 20,
            }))
            .expect("a folder page")
        {
            ListPage::Threads(page) => page.rows.into_iter().map(|row| row.subject).collect(),
            ListPage::Messages(page) => page.rows.into_iter().map(|row| row.subject).collect(),
        }
    };
    let mut first_inbox = in_folder(world.inbox());
    first_inbox.sort();
    assert_eq!(
        first_inbox,
        vec![
            Some("Launch".to_owned()),
            Some("The Ledger, this week".to_owned())
        ],
        "the held newsletter never left its inbox: Focus never listed it"
    );
    assert_eq!(
        in_folder(second_inbox),
        vec![Some("Launch".to_owned())],
        "the launch's other copy stayed with it"
    );
    assert_eq!(
        in_folder(second_archive),
        vec![Some("Minutes".to_owned())],
        "what Focus listed in the second account was archived"
    );
    let _ = (budget, minutes);
}

#[test]
fn what_one_frontend_changed_reaches_the_other() {
    let world = World::new();
    let (terminal, _) = world.frontend(ClientKind::Tui);
    let (_desktop, desktop_events) = world.frontend(ClientKind::Focus);

    world.send(&terminal, archive());

    world.hear(&desktop_events, |event| {
        matches!(event, Event::MessagesRemoved { .. })
    });
}

#[test]
fn an_undo_offer_is_only_for_the_frontend_that_can_take_it() {
    let world = World::new();
    let (terminal, terminal_events) = world.frontend(ClientKind::Tui);
    let (_desktop, desktop_events) = world.frontend(ClientKind::Focus);

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
    let (desktop, desktop_events) = world.frontend(ClientKind::Focus);

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
fn each_frontend_reads_the_undo_it_would_take_back() {
    // What Edit > Undo names on the Mac (specs/009-focus-macos T044): this
    // frontend's own top, read without taking it back.
    let world = World::new();
    let (terminal, terminal_events) = world.frontend(ClientKind::Tui);
    let (desktop, _desktop_events) = world.frontend(ClientKind::Focus);
    let top = |client: &Client| world.rt.block_on(client.undo_top()).expect("the undo top");

    assert_eq!(top(&terminal), None, "nothing done, nothing to name");
    world.send(&terminal, archive());
    world.hear(&terminal_events, |event| {
        matches!(event, Event::ActionCompleted { .. })
    });
    assert_eq!(top(&terminal).as_deref(), Some("Archived 1 message"));
    assert_eq!(top(&desktop), None, "the desktop did nothing");

    world.send(&terminal, Command::Undo);
    world.hear(&terminal_events, |event| {
        matches!(event, Event::UndoPerformed { .. })
    });
    assert_eq!(top(&terminal), None, "taken back, so nothing left");
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
fn a_queued_send_names_the_draft_so_it_can_be_taken_back() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    world.rt.block_on(async {
        let queued = client
            .queue_send(7, a_draft(account, "Ready."), None)
            .await
            .expect("queued");
        assert!(queued.draft.is_assigned(), "the answer names no draft");
        let back = client
            .cancel_send(queued.draft)
            .await
            .expect("answered")
            .expect("the send was still waiting");
        assert_eq!(back.id, queued.draft);
        assert_eq!(back.state, postio_model::DraftState::Editing);
    });
}

#[test]
fn a_queued_send_is_dated_by_the_clock_seam() {
    // A reply sent during a storyboard was listed in the Outbox under the
    // real day, months after the frozen one, and moved between two runs.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Tui);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    let frozen = chrono::Local::now() - chrono::Duration::days(30);
    postio_model::clock::freeze(frozen);
    world.rt.block_on(async {
        client
            .queue_send(7, a_draft(account, "Ready."), None)
            .await
            .expect("queued");
    });
    postio_model::clock::thaw();
    let drafts = drafts_in(&world, account);
    assert_eq!(
        drafts[0].updated_at.timestamp_millis(),
        frozen.timestamp_millis(),
        "stored to the millisecond"
    );
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
fn a_draft_nobody_touched_is_not_recovered_after_a_crash() {
    // #491 reopens what was mid-edit when the last session died, and an
    // untouched buffer is not work worth restoring: the composer autosaves an
    // empty row for the buffer it holds, so recovering it perpetuates itself,
    // and the client opens on a stale composer at every launch.
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Focus);
    let account = world.rt.block_on(client.accounts()).expect("accounts")[0].id;
    assert_eq!(world.rt.block_on(client.recover_draft(account)), Ok(None));

    // A draft exactly as it opened: no recipient, no subject, no body.
    world.rt.block_on(async {
        client
            .save_draft(1, postio_model::Draft::new(account))
            .await
            .expect("saved");
    });

    // The session never ended, so the next start is a crash -- and finds
    // nothing worth handing back.
    assert_eq!(
        world.rt.block_on(client.recover_draft(account)),
        Ok(None),
        "an untouched compose buffer was recovered"
    );
}

#[test]
fn a_file_is_attached_as_the_type_the_frontend_sniffed_and_its_bytes_read_back() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Focus);
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
            scope: postio_search::facets::Scope::AllMail,
        }))
        .expect("an answer")
        .expect("the store was read");
    assert_eq!(found.ids, vec![matching]);
    assert_eq!(found.hits, 1);
    assert!(!found.capped);

    // The scope a facet picks: the message is in the inbox, so none of it
    // is list mail.
    let listed = world
        .rt
        .block_on(client.search(postio_client::protocol::Search {
            account: postio_model::AccountScope::Account(account),
            query: "interlock".into(),
            newest_first: false,
            scope: postio_search::facets::Scope::Lists,
        }))
        .expect("an answer")
        .expect("the store was read");
    assert_eq!(listed.hits, 0, "{listed:?}");
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
fn a_frontend_finds_a_domains_servers_through_the_host() {
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
fn a_frontend_adds_an_account_through_the_host() {
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

/// Whether the host has an engine syncing the account at `address`.
fn syncing(world: &World, client: &Client, address: &str) -> bool {
    let account = world
        .rt
        .block_on(client.accounts())
        .expect("accounts")
        .into_iter()
        .find(|account| account.address.address == address)
        .expect("the account was saved");
    world
        .host()
        .inner
        .engines
        .running
        .lock()
        .expect("never poisoned")
        .contains_key(&account.id)
}

#[test]
fn a_desktop_add_saves_the_account_and_leaves_its_sync_to_the_app() {
    // T165: the desktop apps start a new account's sync themselves -- the
    // first run once the person has chosen how far back, the add-account
    // dialog through the running window -- so an engine the host started
    // here would be a second one, or one under the wrong window.
    let world = onboarding_world(postio_account::backend::MockBackend::new());
    let (client, _) = world.frontend(ClientKind::Focus);
    world
        .rt
        .block_on(client.add_account_then(
            submission("correct horse"),
            postio_client::protocol::AfterSave::Wait,
        ))
        .expect("added");
    assert!(
        !syncing(&world, &client, "grace@example.test"),
        "the host started a sync the desktop app starts itself"
    );

    // The terminal's add, beside it, syncs at once.
    let mut terminal = submission("correct horse");
    terminal.address = "lena@example.test".into();
    world
        .rt
        .block_on(client.add_account(terminal))
        .expect("added");
    assert!(syncing(&world, &client, "lena@example.test"));
}

/// Holds every discovery step open until the probe is stopped, keeping the
/// token it was handed so a test can ask afterwards whether the stop
/// reached it.
#[derive(Default)]
struct Hanging(std::sync::Mutex<Option<postio_account::discovery::CancelToken>>);

impl Hanging {
    async fn hold(
        &self,
        cancel: &postio_account::discovery::CancelToken,
    ) -> postio_account::discovery::TransportError {
        *self.0.lock().expect("never poisoned") = Some(cancel.clone());
        cancel.cancelled().await;
        postio_account::discovery::TransportError::new("stopped")
    }
}

/// [`Hanging`], shared with the test that reads its token.
struct Held(std::sync::Arc<Hanging>);

#[async_trait::async_trait]
impl postio_account::discovery::DiscoveryTransport for Held {
    async fn autoconfig(
        &self,
        _endpoint: postio_account::discovery::AutoconfigEndpoint<'_>,
        cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoveryAutoconfig,
        postio_account::discovery::TransportError,
    > {
        Err(self.0.hold(cancel).await)
    }

    async fn srv(
        &self,
        _domain: &str,
        cancel: &postio_account::discovery::CancelToken,
    ) -> Result<
        postio_account::discovery::DiscoverySrvReport,
        postio_account::discovery::TransportError,
    > {
        Err(self.0.hold(cancel).await)
    }

    async fn mx(
        &self,
        _domain: &str,
        cancel: &postio_account::discovery::CancelToken,
    ) -> Result<Vec<String>, postio_account::discovery::TransportError> {
        Err(self.0.hold(cancel).await)
    }
}

#[test]
fn a_stopped_discovery_has_stopped_its_connections_when_the_stop_returns() {
    // #57, through the host (T165): a probe the person walked away from
    // holds no socket open. The form checks its token the moment it pulls
    // the stop, so the stop reaches the transport before it returns.
    let hanging = std::sync::Arc::new(Hanging::default());
    let transport = Held(hanging.clone());
    let world =
        World::configured(move |wiring| wiring.with_discovery(std::sync::Arc::new(transport)));
    let (client, _) = world.frontend(ClientKind::Focus);
    let stop = postio_client::protocol::Stop::new();
    let asked = world.rt.spawn({
        let client = client.clone();
        let stop = stop.clone();
        async move { client.discover_until("ada@example.com".into(), stop).await }
    });
    let token = world.rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(token) = hanging.0.lock().expect("never poisoned").clone() {
                    return token;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("the probe reached the transport")
    });
    assert!(!token.is_cancelled(), "the probe began already stopped");

    stop.stop();
    assert!(
        token.is_cancelled(),
        "the stop returned with the probe's connections still open"
    );
    let answer = world
        .rt
        .block_on(asked)
        .expect("the ask finished")
        .expect("a stopped probe still answers");
    assert!(
        matches!(answer, postio_ui::onboarding::Status::Manual { .. }),
        "{answer:?}"
    );
}

#[test]
fn a_submission_never_shows_its_password() {
    let shown = format!("{:?}", submission("correct horse"));
    assert!(!shown.contains("correct horse"), "{shown}");
}

#[test]
fn a_frontend_disables_removes_and_restores_an_account_through_the_host() {
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
    // T066: `@` and `+` offer what the store knows, read through the host.
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
    let client = host.connect(ClientKind::Focus);
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
    let client = Host::over(wiring).connect(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
    assert!(!world.rt.block_on(client.orientation_seen()).expect("asked"));
    world
        .rt
        .block_on(client.retire_orientation())
        .expect("retired");
    let (later, _) = world.frontend(ClientKind::Focus);
    assert!(
        world.rt.block_on(later.orientation_seen()).expect("asked"),
        "every later run, whichever frontend asks"
    );
}

#[test]
fn an_account_a_frontend_proved_is_saved_by_the_host() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
    let wiring = world.host().wiring();
    let route = world.rt.block_on(crate::startup::route(
        &wiring.database,
        wiring.secrets.as_ref(),
    ));
    match route {
        crate::startup::StartupRoute::Onboard(Some(account)) => {
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
    let wiring = world.host().wiring();
    let route = world.rt.block_on(crate::startup::route(
        &wiring.database,
        wiring.secrets.as_ref(),
    ));
    assert!(
        matches!(route, crate::startup::StartupRoute::Ready(_)),
        "{route:?}"
    );
}

#[test]
fn an_empty_password_is_no_password() {
    use postio_account::secret::{AccountKey, Password, SecretStore};
    let world = World::new();
    let secrets = MemorySecretStore::new();
    world
        .rt
        .block_on(secrets.store(
            &AccountKey::new("test@example.com".to_owned()),
            &Password::new(""),
        ))
        .expect("the credential stores");
    match world
        .rt
        .block_on(crate::startup::route(world.database(), &secrets))
    {
        crate::startup::StartupRoute::Onboard(Some(account)) => {
            assert_eq!(account.address.address, "test@example.com");
        }
        other => panic!("an empty password is not one to open with: {other:?}"),
    }
}

#[test]
fn a_locked_keyring_sends_a_window_back_to_onboarding_too() {
    // Not the same fault as a missing password, and the same dead end: a
    // credential that cannot be read is one the account does not have.
    use postio_account::secret::{AccountKey, SecretStore};
    let world = World::new();
    let locked = MemorySecretStore::locked();
    assert!(
        world
            .rt
            .block_on(locked.retrieve(&AccountKey::new("test@example.com".to_owned())))
            .is_err(),
        "the double has to refuse, or this test cannot fail"
    );
    assert!(matches!(
        world
            .rt
            .block_on(crate::startup::route(world.database(), &locked)),
        crate::startup::StartupRoute::Onboard(Some(_))
    ));
}

#[test]
fn an_account_marked_for_removal_is_reaped_before_startup_decides_anything() {
    // #464: "Remove" in the settings panel only marks the row, so something
    // has to delete it once, at the next launch, before an engine could
    // otherwise start against it.
    use postio_storage::repository::AccountRepository;
    let world = World::new();
    world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        AccountRepository::new(&connection)
            .mark_pending_deletion(world.account)
            .await
            .expect("marked");
    });

    assert!(
        matches!(
            world.rt.block_on(crate::startup::route(
                world.database(),
                &MemorySecretStore::new()
            )),
            crate::startup::StartupRoute::Onboard(None)
        ),
        "a pending-deletion account is not there to open or to prefill from"
    );
    let gone = world.rt.block_on(async {
        let connection = world.database().connect().await.expect("a connection");
        AccountRepository::new(&connection)
            .get(world.account)
            .await
            .expect("a read")
            .is_none()
    });
    assert!(
        gone,
        "the route must actually reap it, not merely skip past it"
    );
}

#[test]
fn a_fresh_installation_has_nothing_to_prefill_with() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    let database = rt.block_on(test_support::memory());
    assert!(matches!(
        rt.block_on(crate::startup::route(&database, &MemorySecretStore::new())),
        crate::startup::StartupRoute::Onboard(None)
    ));
}

/// Ask `read` until it answers `Some`, failing after a while: for work the
/// host does on its own time.
pub(crate) fn eventually<T>(world: &World, mut read: impl FnMut() -> Option<T>) -> T {
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
fn the_host_makes_bodies_already_on_disk_searchable_once_it_catches_up() {
    let world = World::new();
    let (client, _) = world.frontend(ClientKind::Focus);
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
                scope: postio_search::facets::Scope::AllMail,
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
pub(crate) fn server_with_one_message() -> postio_account::backend::MockBackend {
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
pub(crate) fn syncing_world(mock: postio_account::backend::MockBackend) -> World {
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
pub(crate) fn row_titled(world: &World, client: &Client, subject: &str) -> Option<MessageId> {
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
    let (client, _) = world.frontend(ClientKind::Focus);
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
fn start_syncing_asked_twice_gives_the_account_one_engine() {
    // A frontend asks once its first frame is up, and an account added
    // later asks again: the account still gets one engine.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let (client, _) = world.frontend(ClientKind::Focus);
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

    // Twice at once, then again once it is running.
    world.host().start_syncing();
    world.host().start_syncing();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));
    world.host().start_syncing();
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

// ── Focus mode (spec 007, T034) ─────────────────────────────────────────────

/// A filing pass that files nothing and remembers what it was handed: the
/// subjects of each call's messages.
#[derive(Debug, Default)]
struct FilingProbe {
    calls: std::sync::Mutex<Vec<Vec<Option<String>>>>,
}

impl FilingProbe {
    fn calls(&self) -> Vec<Vec<Option<String>>> {
        self.calls.lock().expect("not poisoned").clone()
    }
}

#[async_trait::async_trait]
impl postio_sync::FilingPass for FilingProbe {
    async fn file(
        &self,
        _transaction: &postio_storage::Connection,
        filed: &[postio_sync::FiledMessage<'_>],
    ) -> Result<postio_sync::FilingEffects, postio_sync::SyncError> {
        self.calls.lock().expect("not poisoned").push(
            filed
                .iter()
                .map(|filed| filed.message.subject.clone())
                .collect(),
        );
        Ok(postio_sync::FilingEffects::default())
    }
}

/// Mail arriving at the mock's inbox after the first sync, and the next
/// pass over it, asked for the way `R` asks.
fn deliver_and_refresh(
    world: &World,
    mock: &postio_account::backend::MockBackend,
    client: &Client,
) -> MessageId {
    use postio_account::backend::{AppendMessage, MailBackend as _};
    let raw = "Message-ID: <weir@example.com>\r\nFrom: Quinn <quinn@example.com>\r\nTo: Test \
               User <test@example.com>\r\nSubject: Weir level\r\nDate: Wed, 23 Sep 2026 \
               09:00:00 +0000\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe weir is \
               high.\r\n";
    world
        .rt
        .block_on(mock.append("INBOX", &AppendMessage::new(raw.as_bytes().to_vec())))
        .expect("delivered");
    world.send(client, Command::Refresh);
    eventually(world, || row_titled(world, client, "Weir level"))
}

/// Whether any engine the host runs hands its arrivals to a filing pass.
fn engines_file(world: &World) -> Vec<bool> {
    world
        .host()
        .inner
        .engines
        .running
        .lock()
        .expect("never poisoned")
        .values()
        .map(postio_runtime::Engine::files_arrivals)
        .collect()
}

#[test]
fn focus_mode_files_what_an_incremental_pass_brings_and_nothing_a_first_sync_does() {
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let probe = std::sync::Arc::new(FilingProbe::default());
    // Before sync starts, as Focus does at startup.
    let focus = world
        .host()
        .enable_focus(crate::FocusSetup::default().filing(probe.clone()));
    let (client, _) = world.frontend(ClientKind::Focus);
    world.host().start_syncing();

    eventually(&world, || row_titled(&world, &client, "Tide gate"));
    assert!(
        probe.calls().is_empty(),
        "the first sync handed its backlog to the filing pass: {:?}",
        probe.calls()
    );
    assert_eq!(
        engines_file(&world),
        vec![true],
        "the account's engine files"
    );
    assert!(focus.running(), "the body stage and the due timer run");
    assert!(world.host().focus_enabled());

    deliver_and_refresh(&world, &mock, &client);
    assert_eq!(
        probe.calls(),
        vec![vec![Some("Weir level".to_owned())]],
        "the arrival, once, and nothing the first sync filed"
    );
}

#[test]
fn a_host_that_never_enables_focus_mode_files_nothing() {
    // The classic app: the same store, the same sync and the same arrival,
    // and no call to `enable_focus`. (The terminal is Focus too, and makes
    // the call: postio-tui's `focus_engine` suite.)
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let probe = std::sync::Arc::new(FilingProbe::default());
    let _setup = crate::FocusSetup::default().filing(probe.clone());
    let (client, _) = world.frontend(ClientKind::Focus);
    assert!(!world.host().focus_enabled());
    world.host().start_syncing();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));

    deliver_and_refresh(&world, &mock, &client);
    assert_eq!(
        engines_file(&world),
        vec![false],
        "an engine files arrivals under a host Focus never switched on"
    );
    assert!(probe.calls().is_empty(), "{:?}", probe.calls());
}

// ── Focus's own filing pass (spec 007, T122) ────────────────────────────────

/// Mail arriving at the mock's inbox after the first sync, then the pass
/// that brings it in, asked for the way `R` asks.
fn deliver_all(
    world: &World,
    mock: &postio_account::backend::MockBackend,
    client: &Client,
    raws: &[String],
) {
    use postio_account::backend::{AppendMessage, MailBackend as _};
    for raw in raws {
        world
            .rt
            .block_on(mock.append("INBOX", &AppendMessage::new(raw.as_bytes().to_vec())))
            .expect("delivered");
    }
    world.send(client, Command::Refresh);
}

/// A notification from a build server the account never wrote to.
fn notification() -> String {
    "Message-ID: <build@example.com>\r\nFrom: Forge <notifications@forge.example>\r\nTo: Test \
     User <test@example.com>\r\nSubject: Build 2231 passed\r\nAuto-Submitted: \
     auto-generated\r\nDate: Wed, 23 Sep 2026 09:00:00 +0000\r\n\r\nGreen.\r\n"
        .to_owned()
}

/// A letter from a person, which nothing files away.
fn letter() -> String {
    "Message-ID: <weir@example.com>\r\nFrom: Quinn <quinn@example.com>\r\nTo: Test User \
     <test@example.com>\r\nSubject: Weir level\r\nDate: Wed, 23 Sep 2026 09:01:00 \
     +0000\r\n\r\nThe weir is high.\r\n"
        .to_owned()
}

/// The folder the message with `subject` is in, by role, and why Focus
/// filed it there if it did.
pub(crate) fn filed_where(
    world: &World,
    subject: &str,
) -> Option<(
    postio_model::MailboxRole,
    Option<postio_storage::repository::FilterReason>,
)> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let id: i64 = postio_storage::sql::first(
            &connection,
            "SELECT id FROM messages WHERE subject = ?1",
            [subject],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")?;
        let message = MessageRepository::new(&connection)
            .get(MessageId::new(id))
            .await
            .expect("a read")?;
        let role = postio_storage::repository::MailboxRepository::new(&connection)
            .get(message.mailbox_id)
            .await
            .expect("a read")?
            .role;
        let reason = postio_storage::repository::FilterDecisionRepository::new(&connection)
            .get(message.id)
            .await
            .expect("a read")
            .map(|decision| decision.reason);
        Some((role, reason))
    })
}

#[test]
fn focus_mode_files_a_notification_away_as_it_arrives() {
    // US9 scenario 1, end to end: Focus mode runs Focus's own filing pass,
    // so a notification from a sender the user never wrote to is archived
    // with its reason as it is filed, and never reaches the inbox; a
    // person's letter, in the same pass, stays.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    world.host().enable_focus(crate::FocusSetup::default());
    let (client, _) = world.frontend(ClientKind::Focus);
    world.host().start_syncing();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));

    deliver_all(&world, &mock, &client, &[notification(), letter()]);
    eventually(&world, || row_titled(&world, &client, "Weir level"));

    assert_eq!(
        filed_where(&world, "Build 2231 passed"),
        Some((
            postio_model::MailboxRole::Archive,
            Some(postio_storage::repository::FilterReason::Notification)
        )),
        "filed away with its reason, in the pass that brought it"
    );
    assert!(row_titled(&world, &client, "Build 2231 passed").is_none());
}

#[test]
fn focus_mode_files_nothing_away_when_filtering_is_off() {
    // FR-119 through the host: `[focus] filtering = false` reaches the pass
    // Focus mode runs, and the notification stays in the inbox.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let config =
        postio_config::Config::from_toml_str("[focus]\nfiltering = false\n").expect("a config");
    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config(config.focus));
    let (client, _) = world.frontend(ClientKind::Focus);
    world.host().start_syncing();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));

    deliver_all(&world, &mock, &client, &[notification(), letter()]);
    eventually(&world, || row_titled(&world, &client, "Weir level"));

    assert_eq!(
        filed_where(&world, "Build 2231 passed"),
        Some((postio_model::MailboxRole::Inbox, None))
    );
    assert!(row_titled(&world, &client, "Build 2231 passed").is_some());
}

// ── Focus's body stage (spec 007, T103) ─────────────────────────────────────

/// A message in the world's folder `mailbox`, received `ago` before now,
/// with `text` as its body when there is one.
pub(crate) fn received(
    world: &World,
    mailbox: MailboxId,
    ago: chrono::TimeDelta,
    text: Option<&str>,
) -> MessageId {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut message = Message::new(world.account, mailbox, Utc::now() - ago);
        message.subject = Some("A note".to_owned());
        let id = messages.create(&mut message).await.expect("a message");
        if let Some(text) = text {
            messages
                .set_body(
                    id,
                    &postio_storage::repository::StoredBody {
                        text: Some(text.to_owned()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::BodyState::Full,
                )
                .await
                .expect("its body");
        }
        id
    })
}

/// The world's folder with `role`.
pub(crate) fn folder(world: &World, role: postio_model::MailboxRole) -> MailboxId {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::MailboxRepository::new(&connection)
            .by_role(world.account, role)
            .await
            .expect("a read")
            .expect("the folder")
            .id
    })
}

/// The classifier version `message`'s body stage is recorded at.
pub(crate) fn body_classified_at(world: &World, message: MessageId) -> Option<i64> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::sql::first(
            &connection,
            "SELECT version FROM focus_classified WHERE message_id = ?1 AND stage = 'body'",
            [message.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")
    })
}

#[test]
fn the_body_stage_catches_up_on_recent_inbox_mail_it_has_not_classified() {
    // FR-141: at Focus's start, recent inbox mail whose body is here and
    // that has no record at this classifier's version is classified, in the
    // background. A record an older classifier left is not this one's: a
    // version bump runs it again. Mail older than 30 days, out of the inbox,
    // or with no body here is not read.
    let world = World::new();
    let inbox = folder(&world, postio_model::MailboxRole::Inbox);
    let archive = folder(&world, postio_model::MailboxRole::Archive);
    let hour = chrono::TimeDelta::hours(1);
    let fresh = received(&world, inbox, hour, Some("Minutes attached."));
    let stale = received(&world, inbox, hour * 2, Some("The agenda."));
    let old = received(&world, inbox, chrono::TimeDelta::days(40), Some("Old."));
    let bodiless = received(&world, inbox, hour, None);
    let archived = received(&world, archive, hour, Some("Filed."));
    let version = postio_classify::VERSION;
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::FocusClassifiedRepository::new(&connection)
            .record(
                &[stale],
                postio_storage::repository::FocusStage::Body,
                version - 1,
            )
            .await
            .expect("an older classifier's record");
    });

    let focus = world.host().enable_focus(crate::FocusSetup::default());
    eventually(&world, || focus.caught_up().then_some(()));

    assert_eq!(body_classified_at(&world, fresh), Some(i64::from(version)));
    assert_eq!(
        body_classified_at(&world, stale),
        Some(i64::from(version)),
        "a newer classifier runs it again"
    );
    for (message, why) in [
        (old, "older than 30 days"),
        (bodiless, "no body here"),
        (archived, "not in the inbox"),
    ] {
        assert_eq!(body_classified_at(&world, message), None, "{why}");
    }
}

#[test]
fn a_body_that_lands_while_focus_runs_is_classified() {
    // The body stage hears `BodyLoaded` for every body that lands, and
    // classifies it once the burst is over.
    let world = World::new();
    let inbox = folder(&world, postio_model::MailboxRole::Inbox);
    let focus = world.host().enable_focus(crate::FocusSetup::default());
    eventually(&world, || focus.caught_up().then_some(()));

    // After the catch-up: only hearing of it can classify it now.
    let landed = received(&world, inbox, chrono::TimeDelta::minutes(5), Some("Hello."));
    world.host().inner.hub.emit(Event::BodyLoaded {
        account: world.account,
        message: landed,
    });

    let version = eventually(&world, || body_classified_at(&world, landed));
    assert_eq!(version, i64::from(postio_classify::VERSION));
}

// ── Questions and to-dos, marked (spec 007, T117) ───────────────────────────

/// A message to the world's inbox from Tove, received an hour ago and dated
/// `date`, shaped by `shape`, with `text` as its body.
pub(crate) fn letter_from_tove(
    world: &World,
    date: chrono::DateTime<Utc>,
    text: &str,
    shape: impl FnOnce(&mut Message),
) -> MessageId {
    let inbox = folder(world, postio_model::MailboxRole::Inbox);
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut message = Message::new(
            world.account,
            inbox,
            Utc::now() - chrono::TimeDelta::hours(1),
        );
        message.subject = Some("Q3 approvals".to_owned());
        message.date = Some(date);
        message.from = vec![postio_model::EmailAddress::new(
            Some("Tove"),
            "tove@example.org",
        )];
        message.to = vec![postio_model::EmailAddress::new(
            Some("Test User"),
            "test@example.com",
        )];
        message.promoted = Some(postio_model::promoted::PromotedHeaders::default());
        shape(&mut message);
        let id = messages.create(&mut message).await.expect("a message");
        messages
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    text: Some(text.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("its body");
        id
    })
}

/// The marker on `message`, once the body stage has had it.
pub(crate) fn marker_on(
    world: &World,
    message: MessageId,
) -> Option<postio_storage::repository::Marker> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::MarkerRepository::new(&connection)
            .get(message)
            .await
            .expect("a read")
    })
}

/// Focus mode on, and the catch-up done.
pub(crate) fn focus_caught_up(world: &World) -> crate::FocusHandle {
    let focus = world.host().enable_focus(crate::FocusSetup::default());
    eventually(world, || focus.caught_up().then_some(()));
    focus
}

/// Every connection Postio has opened, by subsystem.
pub(crate) fn egress(world: &World) -> Vec<String> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::sql::all(&connection, "SELECT subsystem FROM egress_log", (), |row| {
            postio_storage::sql::RowExt::col(row, 0)
        })
        .await
        .expect("a read")
    })
}

pub(crate) const APPROVE: &str =
    "Can you approve these by Friday so finance can close the quarter?";

pub(crate) fn saturday_noon() -> chrono::DateTime<Utc> {
    use chrono::TimeZone as _;
    chrono::Local
        .with_ymd_and_hms(2026, 9, 26, 12, 0, 0)
        .single()
        .expect("a real local time")
        .with_timezone(&Utc)
}

#[test]
fn a_question_sent_to_the_user_is_marked_with_its_sentence_and_nothing_connects() {
    // US12 scenarios 1 and 7: no model configured, and the built-in
    // detector marks the question, quoting it exactly as written -- and
    // Postio opens no connection to anything to do it.
    let world = World::new();
    let question = letter_from_tove(
        &world,
        saturday_noon(),
        &format!("Hi,\n\n{APPROVE}\n\nThanks,\nTove"),
        |_| {},
    );

    focus_caught_up(&world);

    let marker = marker_on(&world, question).expect("a marker");
    assert_eq!(marker.kind, postio_model::listing::MarkerKind::Question);
    assert_eq!(marker.excerpt.as_deref(), Some(APPROVE));
    assert_eq!(
        marker.source,
        postio_storage::repository::MarkerSource::Detector
    );
    assert!(
        marker.span.is_some(),
        "the sentence, as offsets into the text"
    );
    assert_eq!(marker.due_at, None, "a question carries no due date");
    assert!(egress(&world).is_empty(), "{:?}", egress(&world));
}

#[test]
fn a_to_do_with_a_deadline_is_marked_due_that_day() {
    // US12 scenario 2: sent on Saturday 26 September, "by Wednesday" is
    // Wednesday 30 September, and the quote is the ask, not its reason.
    let world = World::new();
    let todo = letter_from_tove(
        &world,
        saturday_noon(),
        "Please leave comments by Wednesday; I'd like to freeze it Thursday.",
        |_| {},
    );

    focus_caught_up(&world);

    let marker = marker_on(&world, todo).expect("a marker");
    assert_eq!(marker.kind, postio_model::listing::MarkerKind::Todo);
    assert_eq!(
        marker.excerpt.as_deref(),
        Some("Please leave comments by Wednesday")
    );
    let due = marker
        .due_at
        .expect("a due date")
        .with_timezone(&chrono::Local)
        .date_naive();
    assert_eq!(
        due,
        chrono::NaiveDate::from_ymd_opt(2026, 9, 30).expect("a date")
    );
}

#[test]
fn the_question_quoted_signed_listed_or_copied_is_not_marked() {
    // US12 scenario 3: in quoted history, in a signature, in a newsletter,
    // or in mail the user is only copied on, no marker appears.
    let world = World::new();
    let quoted = letter_from_tove(
        &world,
        saturday_noon(),
        &format!("Done, sent them over.\n\nOn Fri, Tove wrote:\n> {APPROVE}\n"),
        |_| {},
    );
    let signed = letter_from_tove(
        &world,
        saturday_noon(),
        &format!("See you Monday.\n\n-- \nTove Lund\n{APPROVE}\n"),
        |_| {},
    );
    let listed = letter_from_tove(&world, saturday_noon(), APPROVE, |message| {
        message.list_id = Some("finance.lists.example.org".to_owned());
    });
    let copied = letter_from_tove(&world, saturday_noon(), APPROVE, |message| {
        message.cc = std::mem::take(&mut message.to);
        message.to = vec![postio_model::EmailAddress::new(
            Some("Oren"),
            "oren@example.org",
        )];
    });

    focus_caught_up(&world);

    for (message, why) in [
        (quoted, "quoted history"),
        (signed, "a signature"),
        (listed, "a newsletter"),
        (copied, "copied only"),
    ] {
        assert_eq!(
            body_classified_at(&world, message),
            Some(i64::from(postio_classify::VERSION)),
            "{why}: the body stage had it"
        );
        assert_eq!(marker_on(&world, message), None, "{why}");
    }
}

#[test]
fn a_question_brings_its_message_out_of_a_digest_hold() {
    // FR-122: mail with a question or a to-do is never held. Filing holds
    // by the headers before any body is here; once the body shows an ask,
    // the hold is let go and the message rejoins the inbox.
    let world = World::new();
    let question = letter_from_tove(&world, saturday_noon(), APPROVE, |_| {});
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::DigestRepository::new(&connection)
            .hold(question, "Newsletters", Utc::now())
            .await
            .expect("held at filing");
    });

    focus_caught_up(&world);

    assert!(marker_on(&world, question).is_some());
    let held: Option<String> = world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::sql::first(
            &connection,
            "SELECT rule FROM digest_holds WHERE message_id = ?1",
            [question.get()],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a read")
    });
    assert_eq!(held, None, "the hold was let go");
}

// ── Invitations, marked (spec 007, T110) ────────────────────────────────────

/// An instant as iCalendar writes one in UTC, to the minute.
pub(crate) fn ics_utc(at: chrono::DateTime<Utc>) -> String {
    at.format("%Y%m%dT%H%M00Z").to_string()
}

/// A calendar part: `method` for event `uid` at `sequence`, stamped
/// `stamp`, with `event` lines for its times.
pub(crate) fn calendar(
    method: &str,
    uid: &str,
    sequence: u32,
    stamp: chrono::DateTime<Utc>,
    event: &[String],
) -> String {
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//Example Corp//Test//EN".to_owned(),
        format!("METHOD:{method}"),
        "BEGIN:VEVENT".to_owned(),
        format!("UID:{uid}"),
        format!("SEQUENCE:{sequence}"),
        format!("DTSTAMP:{}", ics_utc(stamp)),
        "SUMMARY:Roadmap review".to_owned(),
        "ORGANIZER;CN=Ines:mailto:ines@example.org".to_owned(),
        "ATTENDEE;CN=Test User;PARTSTAT=NEEDS-ACTION:mailto:test@example.com".to_owned(),
    ];
    lines.extend(event.iter().cloned());
    if method == "CANCEL" {
        lines.push("STATUS:CANCELLED".to_owned());
    }
    lines.extend(["END:VEVENT".to_owned(), "END:VCALENDAR".to_owned()]);
    lines.join("\r\n") + "\r\n"
}

/// `DTSTART` and `DTEND` for an event from `start`, `minutes` long.
pub(crate) fn when(start: chrono::DateTime<Utc>, minutes: i64) -> Vec<String> {
    vec![
        format!("DTSTART:{}", ics_utc(start)),
        format!(
            "DTEND:{}",
            ics_utc(start + chrono::TimeDelta::minutes(minutes))
        ),
    ]
}

/// A message to the user, received `ago` before now, carrying `ics` as a
/// calendar part that is here, as the body backfill leaves one.
pub(crate) fn invitation_mail(world: &World, ago: chrono::TimeDelta, ics: &str) -> MessageId {
    let inbox = folder(world, postio_model::MailboxRole::Inbox);
    let blob = world
        .host()
        .wiring()
        .blobs
        .put(ics.as_bytes())
        .expect("the part stored");
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let messages = MessageRepository::new(&connection);
        let mut message = Message::new(world.account, inbox, Utc::now() - ago);
        message.subject = Some("Invitation: Roadmap review".to_owned());
        message.from = vec![postio_model::EmailAddress::new(
            Some("Ines"),
            "ines@example.org",
        )];
        message.to = vec![postio_model::EmailAddress::new(
            Some("Test User"),
            "test@example.com",
        )];
        let mut part =
            postio_model::Attachment::new(MessageId::UNASSIGNED, "text/calendar", ics.len() as u64);
        part.part_id = Some("2".to_owned());
        message.attachments = vec![part];
        let id = messages.create(&mut message).await.expect("a message");
        messages
            .set_attachment_blob(id, "2", &blob)
            .await
            .expect("the part linked");
        messages
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    text: Some("You are invited.".to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("its body");
        id
    })
}

/// Tomorrow at ten, UTC, to the minute: an event that is ahead.
pub(crate) fn ahead() -> chrono::DateTime<Utc> {
    use chrono::Timelike as _;
    (Utc::now() + chrono::TimeDelta::days(1))
        .with_hour(10)
        .and_then(|at| at.with_minute(0))
        .and_then(|at| at.with_second(0))
        .and_then(|at| at.with_nanosecond(0))
        .expect("ten o'clock")
}

#[test]
fn an_invitation_is_marked_with_its_event_and_can_be_answered() {
    // US8 scenario 1: a calendar request's row shows Invite with the
    // event's times, computed when the part arrived, not when the row is
    // drawn -- and it is open to an answer.
    let world = World::new();
    let start = ahead();
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "review@example.org",
            0,
            Utc::now(),
            &when(start, 45),
        ),
    );

    focus_caught_up(&world);

    let marker = marker_on(&world, request).expect("a marker");
    assert_eq!(marker.kind, postio_model::listing::MarkerKind::Invite);
    assert_eq!(
        marker.source,
        postio_storage::repository::MarkerSource::Calendar
    );
    assert_eq!(marker.starts_at, Some(start));
    assert_eq!(marker.ends_at, Some(start + chrono::TimeDelta::minutes(45)));
    assert_eq!(
        marker.invite_state,
        Some(postio_storage::repository::InviteState::Open)
    );
    let invite = marker.invite.expect("its identity");
    assert_eq!(
        (invite.uid.as_str(), invite.sequence),
        ("review@example.org", 0)
    );
}

#[test]
fn a_floating_time_is_placed_on_the_user_s_clock() {
    use chrono::TimeZone as _;
    let world = World::new();
    let day = ahead().date_naive();
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "standup@example.org",
            0,
            Utc::now(),
            &[
                format!("DTSTART:{}T100000", day.format("%Y%m%d")),
                format!("DTEND:{}T101500", day.format("%Y%m%d")),
            ],
        ),
    );

    focus_caught_up(&world);

    let ten = chrono::Local
        .from_local_datetime(&day.and_hms_opt(10, 0, 0).expect("ten"))
        .earliest()
        .expect("ten o'clock here")
        .with_timezone(&Utc);
    assert_eq!(
        marker_on(&world, request).and_then(|marker| marker.starts_at),
        Some(ten)
    );
}

#[test]
fn a_later_cancellation_cancels_the_invitation_and_its_answers() {
    // US8 scenario 4: the event is called off, so the invitation's row says
    // so and offers no answer -- whichever of the two the stage read first.
    let world = World::new();
    let start = ahead();
    let stamp = Utc::now() - chrono::TimeDelta::hours(3);
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(3),
        &calendar("REQUEST", "review@example.org", 0, stamp, &when(start, 45)),
    );
    let focus = focus_caught_up(&world);
    assert_eq!(
        marker_on(&world, request).and_then(|marker| marker.invite_state),
        Some(postio_storage::repository::InviteState::Open)
    );

    let cancel = invitation_mail(
        &world,
        chrono::TimeDelta::minutes(5),
        &calendar(
            "CANCEL",
            "review@example.org",
            1,
            stamp + chrono::TimeDelta::hours(2),
            &when(start, 45),
        ),
    );
    world.host().inner.hub.emit(Event::BodyLoaded {
        account: world.account,
        message: cancel,
    });
    let _ = focus;

    let cancelled = Some(postio_storage::repository::InviteState::Cancelled);
    eventually(&world, || {
        (marker_on(&world, request).and_then(|marker| marker.invite_state) == cancelled)
            .then_some(())
    });
    assert_eq!(
        marker_on(&world, cancel).and_then(|marker| marker.invite_state),
        cancelled
    );
}

#[test]
fn a_cancellation_read_before_its_invitation_still_cancels_it() {
    // The catch-up reads newest first, so the cancellation comes before
    // the request it cancels: the request is marked as the newer word says.
    let world = World::new();
    let start = ahead();
    let stamp = Utc::now() - chrono::TimeDelta::hours(3);
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(3),
        &calendar("REQUEST", "review@example.org", 0, stamp, &when(start, 45)),
    );
    invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "CANCEL",
            "review@example.org",
            1,
            stamp + chrono::TimeDelta::hours(1),
            &when(start, 45),
        ),
    );

    focus_caught_up(&world);

    assert_eq!(
        marker_on(&world, request).and_then(|marker| marker.invite_state),
        Some(postio_storage::repository::InviteState::Cancelled)
    );
}

#[test]
fn an_update_moves_the_invitation_to_its_new_time() {
    // FR-103: an updated invitation replaces the marker's time.
    let world = World::new();
    let start = ahead();
    let moved = start + chrono::TimeDelta::hours(2);
    let stamp = Utc::now() - chrono::TimeDelta::hours(3);
    let request = invitation_mail(
        &world,
        chrono::TimeDelta::hours(3),
        &calendar("REQUEST", "review@example.org", 0, stamp, &when(start, 45)),
    );
    invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "review@example.org",
            1,
            stamp + chrono::TimeDelta::hours(1),
            &when(moved, 30),
        ),
    );

    focus_caught_up(&world);

    let marker = marker_on(&world, request).expect("a marker");
    assert_eq!(marker.starts_at, Some(moved));
    assert_eq!(marker.invite.map(|invite| invite.sequence), Some(1));
}

#[test]
fn an_invitation_to_an_event_that_is_over_offers_no_answer() {
    // US8 scenario 5: an event that has ended is past, and a series is
    // past only once its last occurrence is; one with no end never is.
    let world = World::new();
    let ended = Utc::now() - chrono::TimeDelta::days(3);
    let over = invitation_mail(
        &world,
        chrono::TimeDelta::hours(1),
        &calendar(
            "REQUEST",
            "retro@example.org",
            0,
            Utc::now(),
            &when(ended, 30),
        ),
    );
    let mut finished_series = when(ended - chrono::TimeDelta::days(14), 30);
    finished_series.push("RRULE:FREQ=WEEKLY;COUNT=2".to_owned());
    let series_over = invitation_mail(
        &world,
        chrono::TimeDelta::hours(2),
        &calendar(
            "REQUEST",
            "weekly@example.org",
            0,
            Utc::now(),
            &finished_series,
        ),
    );
    let mut endless = when(ended - chrono::TimeDelta::days(14), 30);
    endless.push("RRULE:FREQ=WEEKLY".to_owned());
    let ongoing = invitation_mail(
        &world,
        chrono::TimeDelta::hours(3),
        &calendar("REQUEST", "forever@example.org", 0, Utc::now(), &endless),
    );

    focus_caught_up(&world);

    let state = |message| marker_on(&world, message).and_then(|marker| marker.invite_state);
    use postio_storage::repository::InviteState;
    assert_eq!(state(over), Some(InviteState::Past));
    assert_eq!(state(series_over), Some(InviteState::Past));
    assert_eq!(state(ongoing), Some(InviteState::Open));
}

// ── Digest deliveries (spec 007, T135) ──────────────────────────────────────

/// `[focus]` with one digest rule, "Newsletters", on `cadence`.
pub(crate) fn digesting(cadence: &str) -> postio_config::FocusConfig {
    postio_config::Config::from_toml_str(&format!(
        "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@ledger.example\"]\n{cadence}\n"
    ))
    .expect("a config")
    .focus
}

/// A message in the world's inbox held for "Newsletters" at `at`.
fn held_at(world: &World, at: chrono::DateTime<Utc>) -> MessageId {
    let inbox = folder(world, postio_model::MailboxRole::Inbox);
    let message = received(world, inbox, Utc::now() - at, None);
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        postio_storage::repository::DigestRepository::new(&connection)
            .hold(message, "Newsletters", at)
            .await
            .expect("held");
    });
    message
}

/// Every delivery: its rule, when it came due, and what it holds.
pub(crate) fn deliveries(world: &World) -> Vec<(String, chrono::DateTime<Utc>, Vec<MessageId>)> {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let rows: Vec<(i64, String, i64)> = postio_storage::sql::all(
            &connection,
            "SELECT id, rule, due_at FROM digest_deliveries ORDER BY id",
            (),
            |row| {
                use postio_storage::sql::RowExt as _;
                Ok((row.col(0)?, row.col(1)?, row.col(2)?))
            },
        )
        .await
        .expect("a read");
        let mut found = Vec::new();
        for (id, rule, due_at) in rows {
            let held: Vec<MessageId> = postio_storage::sql::all(
                &connection,
                "SELECT message_id FROM digest_holds WHERE delivery_id = ?1 ORDER BY message_id",
                [id],
                |row| Ok(MessageId::new(postio_storage::sql::RowExt::col(row, 0)?)),
            )
            .await
            .expect("a read");
            found.push((rule, postio_storage::repository::from_millis(due_at), held));
        }
        found
    })
}

/// Two hours east of UTC, as a zone with no daylight saving to cloud it.
fn east() -> chrono::FixedOffset {
    chrono::FixedOffset::east_opt(2 * 3600).expect("an offset")
}

fn on(day: u32, hour: u32) -> chrono::DateTime<chrono::FixedOffset> {
    use chrono::TimeZone as _;
    east()
        .with_ymd_and_hms(2026, 9, day, hour, 0, 0)
        .single()
        .expect("a time")
}

fn deliver_due_at(
    world: &World,
    config: &postio_config::FocusConfig,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> usize {
    world
        .rt
        .block_on(crate::focus::deliver_due(&world.database, config, &now))
        .expect("the timer's pass")
}

#[test]
fn held_mail_is_delivered_as_one_digest_when_its_rule_comes_due() {
    // US10 scenario 1: held on Wednesday, a weekly rule due Sunday at nine
    // delivers nothing on Saturday night, and one digest at nine on Sunday
    // holding it.
    let world = World::new();
    let config = digesting("cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"");
    let wednesday = held_at(&world, on(23, 10).with_timezone(&Utc));

    assert_eq!(deliver_due_at(&world, &config, on(26, 20)), 0);
    assert!(deliveries(&world).is_empty(), "not before it is due");

    assert_eq!(deliver_due_at(&world, &config, on(27, 9)), 1);
    assert_eq!(
        deliveries(&world),
        vec![(
            "Newsletters".to_owned(),
            on(27, 9).with_timezone(&Utc),
            vec![wednesday]
        )]
    );
}

#[test]
fn a_digest_missed_while_focus_was_closed_is_delivered_once() {
    // US10 scenario 3: Focus was closed through three Sundays. It delivers
    // one digest, as of the latest of them, holding everything; asked again,
    // it delivers nothing more.
    let world = World::new();
    let config = digesting("cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"");
    let first = held_at(&world, on(2, 10).with_timezone(&Utc));
    let second = held_at(&world, on(10, 18).with_timezone(&Utc));

    assert_eq!(deliver_due_at(&world, &config, on(27, 12)), 1);
    assert_eq!(deliver_due_at(&world, &config, on(27, 12)), 0);

    let mut held = vec![first, second];
    held.sort();
    assert_eq!(
        deliveries(&world),
        vec![(
            "Newsletters".to_owned(),
            on(27, 9).with_timezone(&Utc),
            held
        )],
        "one digest, as of the last due time, nothing lost or doubled"
    );
}

#[test]
fn a_due_time_with_nothing_held_makes_no_digest() {
    // US10 scenario 6.
    let world = World::new();
    let config = digesting("cadence = \"daily\"\nat = \"07:00\"");

    assert_eq!(deliver_due_at(&world, &config, on(27, 12)), 0);
    assert!(deliveries(&world).is_empty());
}

#[test]
fn focus_mode_delivers_what_came_due_when_it_starts() {
    // The due timer runs from `[focus]` as Focus mode starts, so a digest
    // that came due while Focus was closed is there when it opens.
    let world = World::new();
    let config = digesting("cadence = \"daily\"\nat = \"00:00\"");
    let held = held_at(&world, Utc::now() - chrono::TimeDelta::days(2));

    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config(config));

    let delivered = eventually(&world, || {
        let found = deliveries(&world);
        (!found.is_empty()).then_some(found)
    });
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].2, vec![held]);
}

// ── Arrivals Focus filed away are not announced (spec 007, T129) ────────────

#[test]
fn focus_mode_announces_no_arrival_it_filtered_or_held() {
    // FR-153: Focus never notifies for mail it filtered or held. The new
    // mail the host passes on names only what reached Focus's inbox, so a
    // frontend that notifies for it -- and splices it into its list --
    // stays silent about the rest.
    let mock = server_with_one_message();
    let world = syncing_world(mock.clone());
    let config = postio_config::Config::from_toml_str(
        "[[focus.digests]]\nname = \"Newsletters\"\nmatch = [\"from:news@ledger.example\"]\n\
         cadence = \"weekly\"\nday = \"sunday\"\nat = \"09:00\"\n",
    )
    .expect("a config");
    world
        .host()
        .enable_focus(crate::FocusSetup::default().with_config(config.focus));
    let (client, events) = world.frontend(ClientKind::Focus);
    world.host().start_syncing();
    eventually(&world, || row_titled(&world, &client, "Tide gate"));
    world.drain(&events);

    let newsletter = "Message-ID: <issue@ledger.example>\r\nFrom: Ledger <news@ledger.example>\r\n\
                      To: Test User <test@example.com>\r\nSubject: The weekly numbers\r\n\
                      Date: Wed, 23 Sep 2026 08:00:00 +0000\r\n\r\nNumbers.\r\n"
        .to_owned();
    deliver_all(
        &world,
        &mock,
        &client,
        &[notification(), newsletter, letter()],
    );
    let letter = eventually(&world, || row_titled(&world, &client, "Weir level"));
    assert_eq!(
        filed_where(&world, "Build 2231 passed").map(|(role, _)| role),
        Some(postio_model::MailboxRole::Archive),
        "the fixture filters the notification"
    );

    let announced: Vec<MessageId> = world
        .drain(&events)
        .into_iter()
        .filter_map(|event| match event {
            Event::NewMail { messages, .. } => Some(messages),
            _ => None,
        })
        .flatten()
        .collect();
    assert_eq!(
        announced,
        vec![letter],
        "only the letter reached Focus's inbox"
    );
}

// ── Catching up on what another app filed (spec 007, T127) ──────────────────

/// A message filed in the world's inbox from `from`, an hour ago, threaded
/// as a sync threads it: what another app's sync leaves behind.
pub(crate) fn filed_elsewhere(world: &World, from: &str, subject: &str) -> MessageId {
    let inbox = folder(world, postio_model::MailboxRole::Inbox);
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let mut message = Message::new(
            world.account,
            inbox,
            Utc::now() - chrono::TimeDelta::hours(1),
        );
        message.subject = Some(subject.to_owned());
        message.from = vec![postio_model::EmailAddress::new(None::<&str>, from)];
        message.to = vec![postio_model::EmailAddress::new(
            Some("Test User"),
            "test@example.com",
        )];
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a message");
        postio_storage::repository::ThreadingRepository::new(&connection, world.account)
            .thread(&message)
            .await
            .expect("threaded");
        id
    })
}

/// Focus last ran with the store as it is now: its mark set, as a session
/// that had just filed everything here would leave it.
pub(crate) fn focus_ran_before(world: &World) {
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let newest = MessageRepository::new(&connection)
            .newest_id()
            .await
            .expect("a read")
            .expect("a message");
        postio_storage::repository::SettingsRepository::new(&connection)
            .set(crate::focus::FILED_THROUGH, &newest.get().to_string())
            .await
            .expect("the mark");
    });
}

#[test]
fn mail_another_app_filed_while_focus_was_closed_is_sorted_when_it_opens() {
    // US9 scenario 8: a notification that arrived while the classic app was
    // open moves from the inbox to Filtered, with its reason, when Focus
    // next opens -- as if Focus had filed it. A letter stays.
    let world = World::new();
    focus_ran_before(&world);
    let notification = filed_elsewhere(&world, "notifications@forge.example", "Build passed");
    let letter = filed_elsewhere(&world, "tove@example.org", "Lunch?");

    world.host().enable_focus(crate::FocusSetup::default());

    let (role, reason) = eventually(&world, || {
        filed_where(&world, "Build passed")
            .filter(|(role, _)| *role != postio_model::MailboxRole::Inbox)
    });
    assert_eq!(role, postio_model::MailboxRole::Archive);
    assert_eq!(
        reason,
        Some(postio_storage::repository::FilterReason::Notification)
    );
    assert_eq!(
        filed_where(&world, "Lunch?"),
        Some((postio_model::MailboxRole::Inbox, None))
    );
    let _ = (notification, letter);
}

#[test]
fn focus_s_first_open_sorts_nothing_already_in_the_inbox() {
    // FR-118: filtering applies to mail filed after it is turned on, and
    // Focus's first open is what turns it on. What is already in the inbox
    // is the deliberate sweep's to move (T128), never the catch-up's.
    let world = World::new();
    filed_elsewhere(&world, "notifications@forge.example", "Build passed");

    world.host().enable_focus(crate::FocusSetup::default());

    let marked = eventually(&world, || {
        world.rt.block_on(async {
            let connection = world.database.connect().await.expect("a connection");
            postio_storage::repository::SettingsRepository::new(&connection)
                .get(crate::focus::FILED_THROUGH)
                .await
                .expect("a read")
        })
    });
    assert!(!marked.is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        filed_where(&world, "Build passed"),
        Some((postio_model::MailboxRole::Inbox, None))
    );
}

#[test]
fn the_catch_up_leaves_the_row_under_the_cursor_where_it_is() {
    // Spec Edge Cases: the catch-up never moves the row under the cursor.
    let world = World::new();
    focus_ran_before(&world);
    let watched = filed_elsewhere(&world, "notifications@forge.example", "Build passed");
    filed_elsewhere(&world, "alerts@builds.example", "Deploy finished");
    let state = SharedState::default();
    let (quiet, _) = event_channel();
    state.update(&quiet, |app| {
        let mut events = app.open_mailbox(world.inbox());
        events.extend(app.select(Vec::new(), Some(watched)));
        events
    });
    let client = world.host().connect(ClientKind::Focus).with_state(state);
    world.send(&client, Command::Refresh);
    eventually(&world, || {
        world
            .host()
            .inner
            .clients
            .lock()
            .expect("never poisoned")
            .values()
            .any(|entry| entry.state.snapshot().focus() == Some(watched))
            .then_some(())
    });

    world.host().enable_focus(crate::FocusSetup::default());

    eventually(&world, || {
        filed_where(&world, "Deploy finished")
            .filter(|(role, _)| *role == postio_model::MailboxRole::Archive)
    });
    assert_eq!(
        filed_where(&world, "Build passed"),
        Some((postio_model::MailboxRole::Inbox, None)),
        "the row under the cursor stayed"
    );
}

/// Spec 007 US3 scenario 6, in the terminal's completion (T076): the host
/// ranks by the one rule both apps share, so an address written to 42
/// times comes before one seen on a hundred messages and never written to;
/// and the directory the desktop composer holds carries the count.
#[test]
fn an_address_written_to_is_offered_before_one_only_seen() {
    let world = World::new();
    world.rt.block_on(async {
        let connection = world.database.connect().await.expect("a connection");
        let contacts = postio_storage::repository::ContactRepository::new(&connection);
        for (address, seen, days_ago) in [
            ("quill.often@example.com", 100, 0),
            ("quill.wrote@example.net", 50, 30),
        ] {
            let at = Utc::now() - chrono::Duration::days(days_ago);
            for _ in 0..seen {
                contacts
                    .record(
                        Some(world.account),
                        &postio_model::EmailAddress::new(None::<String>, address),
                        at,
                    )
                    .await
                    .expect("a sighting");
            }
        }
        for statement in [
            "INSERT INTO addresses (address, address_normalized)
             VALUES ('quill.wrote@example.net', 'quill.wrote@example.net')",
            "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
             SELECT id, 42, NULL FROM addresses
              WHERE address_normalized = 'quill.wrote@example.net'",
        ] {
            postio_storage::sql::execute(&connection, statement, ())
                .await
                .expect("written to 42 times");
        }
    });
    let (terminal, _) = world.frontend(ClientKind::Tui);
    let offered: Vec<String> = world
        .rt
        .block_on(terminal.recipients(world.account, "quill".into()))
        .expect("suggestions")
        .into_iter()
        .map(|candidate| match candidate {
            postio_model::contact_group::RecipientCandidate::Contact(address) => address.address,
            postio_model::contact_group::RecipientCandidate::Group { name, .. } => name,
        })
        .collect();
    assert_eq!(
        offered,
        ["quill.wrote@example.net", "quill.often@example.com"],
        "the address written to comes first"
    );

    let directory = world
        .rt
        .block_on(terminal.recipient_directory(world.account))
        .expect("the directory");
    let wrote = directory
        .contacts
        .iter()
        .find(|row| row.contact.address.address == "quill.wrote@example.net")
        .expect("in the directory");
    assert_eq!(wrote.sent_count, 42, "the directory carries the letters");
}
