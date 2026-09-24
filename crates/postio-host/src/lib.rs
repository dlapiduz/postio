//! The one process that owns Postio's store.
//!
//! Turso holds an exclusive lock on the database file, and the operation
//! queue is correct only with one drainer, so exactly one process opens the
//! store and runs the engines: this one (ADR 0041). Frontends reach it through
//! `postio-client`, over a socket as `postio-daemon` or in-process where no
//! other frontend can share the store.
//!
//! # One store, a selection per frontend
//!
//! Every frontend has its own selection, focus and undo history, and the host
//! has none of its own. So each connected client gets its own verbs over the
//! shared store: its own [`SharedState`], which adopts the snapshot each of
//! its commands carries, and its own [`Actions`], whose undo stack is
//! therefore that client's alone (research R1a).
//!
//! Commands from every client still run **one at a time, in arrival order**,
//! on one queue. That is what the single bridge pump guaranteed when one
//! process had one frontend, and the verbs were written against it.
//!
//! # Whose events are whose
//!
//! What a command changed in the store is everybody's news: a message
//! archived in the terminal has to leave the desktop app's list. What a
//! command *says about itself* is its sender's alone: the "Archived 12
//! messages — Undo" notice, a refusal, an error, the end of a tracked send.
//! Showing another frontend's undo offer would invite a `u` that undoes
//! nothing there. [`is_feedback`] draws that line.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use postio_client::Client;
use postio_client::api::{Call, Disconnected, Transport};
use postio_client::protocol::{ClientId, ClientKind, Req, Resp};
use postio_core::bridge::{Bridge, EventHub, EventSink, event_channel};
use postio_core::dispatch::Dispatcher;
use postio_core::{Command, Event, EventEnvelope, InvocationId, SharedState, StateSnapshot};
use postio_session::Wiring;
use postio_session::actions::{self, Actions};
use postio_session::refresh;
use postio_storage::{BlobStore, Store};

/// The store's one owner, serving the clients connected to it.
///
/// Dropping it stops its runtime, and with it the engines.
pub struct Host {
    inner: Arc<Inner>,
    // Last: the runtime outlives everything spawned on it.
    _bridge: Bridge,
}

struct Inner {
    wiring: Wiring,
    hub: EventHub,
    clients: Mutex<HashMap<ClientId, Entry>>,
    next_client: AtomicU64,
    queue: async_channel::Sender<Queued>,
    /// How many frontends are connected, watched by the daemon's idle timer.
    connected: tokio::sync::watch::Sender<usize>,
}

/// One connected frontend, as the host holds it.
#[derive(Clone)]
struct Entry {
    kind: ClientKind,
    state: SharedState,
    verbs: Arc<Dispatcher>,
    /// Where this client's own command events go, to be sorted.
    sink: EventSink,
    /// The tasks that carry this client's events, stopped when it leaves:
    /// the hub never closes a subscription on its own.
    tasks: Vec<tokio::task::AbortHandle>,
    /// This client's draft writes, in the order it made them.
    drafts: compose::DraftWriter,
}

/// A request answered on arrival, or one left to be answered concurrently.
enum InOrder {
    /// Answered now, in the order it arrived.
    Answered(Resp),
    /// A read, to answer on a task of its own.
    Later(Req),
    /// Handed over in the order it arrived, answered when it lands: a draft
    /// write, which must not overtake the one before it.
    Pending(std::pin::Pin<Box<dyn std::future::Future<Output = Resp> + Send>>),
}

struct Queued {
    client: ClientId,
    command: Command,
    aim: StateSnapshot,
    invocation: Option<InvocationId>,
}

/// Whether `event` is a command speaking about itself, which only the
/// frontend that sent the command should hear.
pub fn is_feedback(event: &Event) -> bool {
    matches!(
        event,
        Event::ActionCompleted { .. }
            | Event::UndoPerformed { .. }
            | Event::CommandRejected { .. }
            | Event::Error { .. }
            | Event::InvocationFinished { .. }
    )
}

/// A frontend's verbs over the shared store: the session's actions and
/// refresh, resolving against `state`.
fn verbs(wiring: &Wiring, state: &SharedState) -> Dispatcher {
    let builder = actions::wire(
        Dispatcher::builder(),
        Actions::new(wiring.database.clone(), state.clone()),
    );
    refresh::wire(builder, wiring.engine.clone(), state.clone()).build()
}

impl Host {
    /// Take ownership of an open store and start serving.
    ///
    /// `configure` finishes the wiring the way the installation asks --
    /// mailbox roles, sync policy, the keyring -- exactly as the desktop app
    /// finished its own before this process existed.
    pub fn start(
        database: Store,
        blobs: BlobStore,
        configure: impl FnOnce(Wiring) -> Wiring,
    ) -> Result<Host, String> {
        let hub = EventHub::new();
        let engine = refresh::EngineSlot::default();
        let host_state = SharedState::default();
        let host_verbs = refresh::wire(
            actions::wire(
                Dispatcher::builder(),
                Actions::new(database.clone(), host_state.clone()),
            ),
            engine.clone(),
            host_state,
        )
        .build();
        let bridge = Bridge::builder()
            .build_with_events(host_verbs, hub.sink())
            .map_err(|error| format!("Postio could not start its runtime: {error}"))?;
        let wiring = configure(Wiring {
            engine,
            ..Wiring::new(
                database,
                blobs,
                bridge.handle(),
                hub.sink(),
                bridge.commands(),
            )
        });

        let (queue, commands) = async_channel::unbounded::<Queued>();
        let inner = Arc::new(Inner {
            wiring,
            hub,
            clients: Mutex::new(HashMap::new()),
            next_client: AtomicU64::new(1),
            queue,
            connected: tokio::sync::watch::Sender::new(0),
        });
        let pump = Arc::clone(&inner);
        bridge.handle().spawn(async move {
            while let Ok(queued) = commands.recv().await {
                pump.run(queued).await;
            }
        });
        Ok(Host {
            inner,
            _bridge: bridge,
        })
    }

    /// The wiring every read and the engines hang off.
    pub fn wiring(&self) -> &Wiring {
        &self.inner.wiring
    }

    /// Start a sync engine for every enabled account, on the host's runtime.
    ///
    /// The host does this rather than a frontend because the host is the
    /// one process that outlives every window: sync runs while any frontend
    /// is open and stops with the host, exactly as it ran with the desktop
    /// app before (research R1b).
    pub fn start_syncing(&self) {
        let wiring = self.inner.wiring.clone();
        self.inner.runtime().spawn(async move {
            let accounts = match wiring.database.connect().await {
                Ok(connection) => postio_storage::repository::AccountRepository::new(&connection)
                    .list_enabled()
                    .await
                    .unwrap_or_else(|error| {
                        tracing::error!(%error, "cannot read the accounts: {error}");
                        Vec::new()
                    }),
                Err(error) => {
                    tracing::error!(%error, "cannot read the accounts: {error}");
                    Vec::new()
                }
            };
            if accounts.is_empty() {
                return;
            }
            match postio_session::engine::start_all(&accounts, &wiring).await {
                Ok(engines) => {
                    for (_, engine) in engines {
                        postio_runtime::retain(engine.clone());
                        wiring.engine.fill(engine);
                    }
                }
                Err(refusal) => {
                    tracing::error!(%refusal, "not starting the sync engines: {refusal}");
                }
            }
        });
    }

    /// Stop the engines and mark a clean end, before the host is dropped.
    ///
    /// Engines first: they are the one thing still writing on threads of
    /// their own, and a write torn by the process exit is left for a pre-1.0
    /// engine to recover (`postio-app`'s `run` says the same, and did this).
    pub fn stop(&self) {
        postio_runtime::stop_retained();
        postio_session::blocking::now(postio_session::end_session(&self.inner.wiring.database));
    }

    /// A client in this process: `postio-ffi`, the integration suites, and
    /// until the socket exists, the desktop app.
    pub fn connect(&self, kind: ClientKind) -> Client {
        let (id, events) = self.inner.join(kind);
        Client::new(Arc::new(Local {
            inner: Arc::clone(&self.inner),
            client: id,
            events,
        }))
    }
}

impl Inner {
    fn runtime(&self) -> &tokio::runtime::Handle {
        &self.wiring.runtime
    }

    /// Register a client and start sorting its events.
    fn join(&self, kind: ClientKind) -> (ClientId, async_channel::Receiver<EventEnvelope>) {
        let id = ClientId(self.next_client.fetch_add(1, Ordering::Relaxed));
        let state = SharedState::default();
        let verbs = Arc::new(verbs(&self.wiring, &state));
        let (outbox, events) = async_channel::unbounded::<EventEnvelope>();

        // Everybody's news, from the engines and from every client's verbs.
        let label = format!("client:{kind:?}:{}", id.0).to_lowercase();
        let everybody = self.hub.subscribe(&label);
        let to_client = outbox.clone();
        let hearing = self.runtime().spawn(async move {
            while let Some(envelope) = everybody.next_tracked().await {
                if to_client.send(envelope).await.is_err() {
                    return;
                }
            }
        });

        // This client's own verbs report here, and are sorted: what they say
        // about themselves goes to this client alone, what they changed goes
        // to everybody (the hub, which includes this client).
        let (sink, own) = event_channel();
        let hub = self.hub.sink();
        let sorting = self.runtime().spawn(async move {
            while let Some(envelope) = own.next_tracked().await {
                if is_feedback(&envelope.event) {
                    if outbox.send(envelope).await.is_err() {
                        return;
                    }
                } else {
                    let hub = match envelope.origin {
                        Some(origin) => hub.with_origin(origin),
                        None => hub.clone(),
                    };
                    hub.emit(envelope.event);
                }
            }
        });

        self.clients.lock().expect("never poisoned").insert(
            id,
            Entry {
                kind,
                state,
                verbs,
                sink,
                tasks: vec![hearing.abort_handle(), sorting.abort_handle()],
                drafts: compose::DraftWriter::spawn(self.wiring.database.clone(), self.runtime()),
            },
        );
        self.connected.send_modify(|count| *count += 1);
        tracing::info!(client = id.0, ?kind, "a frontend connected");
        (id, events)
    }

    /// Forget a client: its events stop, and a command it queued and has
    /// not run yet is dropped with it.
    fn leave(&self, client: ClientId) {
        let entry = self.clients.lock().expect("never poisoned").remove(&client);
        if let Some(entry) = entry {
            for task in &entry.tasks {
                task.abort();
            }
            self.connected
                .send_modify(|count| *count = count.saturating_sub(1));
            tracing::info!(client = client.0, kind = ?entry.kind, "a frontend left");
        }
    }

    /// A request that must be answered in the order it was made, answered
    /// now; any other request, handed back to be answered concurrently.
    ///
    /// Commands are this kind: two keystrokes arrive in order and must run
    /// in order, and spawning each would let the second overtake the first.
    fn answer_in_order(&self, client: ClientId, request: Req) -> InOrder {
        match request {
            Req::Send(command, aim) => InOrder::Answered(self.enqueue(client, command, aim, None)),
            Req::SendTracked(command, aim) => {
                let invocation = InvocationId::next();
                InOrder::Answered(match self.enqueue(client, command, aim, Some(invocation)) {
                    Resp::Done => Resp::Tracked(invocation),
                    refused => refused,
                })
            }
            Req::NoteRemoved(mailbox, messages) => {
                self.wiring.store.note_removed(mailbox, messages);
                InOrder::Answered(Resp::Done)
            }
            Req::SaveDraft { generation, draft } => {
                let Some(entry) = self.entry(client) else {
                    return InOrder::Answered(Resp::Stopped);
                };
                let saved = entry.drafts.save(generation, *draft);
                InOrder::Pending(Box::pin(async move {
                    saved.await.map_or_else(Resp::Failed, Resp::DraftSaved)
                }))
            }
            Req::QueueSend {
                generation,
                draft,
                at,
            } => {
                let Some(entry) = self.entry(client) else {
                    return InOrder::Answered(Resp::Stopped);
                };
                let account = draft.account_id;
                let queued = entry.drafts.send(generation, *draft, at);
                // Everybody's list moved -- the row left Drafts for the
                // Outbox -- so the news goes to the hub, not only to the
                // frontend that sent it.
                let hub = self.hub.sink();
                InOrder::Pending(Box::pin(async move {
                    match queued.await {
                        Ok(moved) => {
                            if let Some(mailbox) = moved {
                                hub.emit(Event::MessageListChanged { account, mailbox });
                            }
                            Resp::Queued(moved)
                        }
                        Err(error) => Resp::Failed(error),
                    }
                }))
            }
            Req::DiscardDraft { generation, known } => {
                let Some(entry) = self.entry(client) else {
                    return InOrder::Answered(Resp::Stopped);
                };
                let discarded = entry.drafts.discard(generation, known);
                InOrder::Pending(Box::pin(async move {
                    discarded.await;
                    Resp::Done
                }))
            }
            other => InOrder::Later(other),
        }
    }

    fn entry(&self, client: ClientId) -> Option<Entry> {
        self.clients
            .lock()
            .expect("never poisoned")
            .get(&client)
            .cloned()
    }

    /// Run one client's command, aimed with that client's selection.
    async fn run(&self, queued: Queued) {
        let Some(entry) = self.entry(queued.client) else {
            return;
        };
        let (quiet, _) = event_channel();
        entry.state.update(&quiet, |app| {
            app.adopt(queued.aim);
            Vec::new()
        });
        let sink = match queued.invocation {
            Some(invocation) => entry.sink.with_origin(invocation),
            None => entry.sink.clone(),
        };
        tracing::debug!(client = queued.client.0, kind = ?entry.kind, "running a command");
        entry.verbs.dispatch(queued.command, sink).await;
    }

    /// Answer one request from local state. Nothing here waits on the
    /// network: a remote effect is a command, queued.
    async fn answer(&self, client: ClientId, request: Req) -> Resp {
        let request = match self.answer_in_order(client, request) {
            InOrder::Answered(answered) => return answered,
            InOrder::Pending(landing) => return landing.await,
            InOrder::Later(request) => request,
        };
        let store = &self.wiring.store;
        match request {
            Req::Send(..)
            | Req::SendTracked(..)
            | Req::NoteRemoved(..)
            | Req::SaveDraft { .. }
            | Req::QueueSend { .. }
            | Req::DiscardDraft { .. } => {
                unreachable!("answered in order above")
            }
            Req::Recipients { account, prefix } => {
                Resp::Recipients(compose::recipients(&self.wiring.database, account, &prefix).await)
            }
            Req::ReplySource(message) => Resp::ReplySource(
                compose::reply_source(&self.wiring.database, message)
                    .await
                    .map(Box::new),
            ),
            Req::DraftBehind(message) => Resp::Draft(
                compose::draft_behind(&self.wiring.database, message)
                    .await
                    .map(Box::new),
            ),
            Req::CancelSend(draft) => Resp::Draft(
                compose::cancel_queued_send(&self.wiring.database, draft)
                    .await
                    .map(Box::new),
            ),
            Req::SendFailure(draft) => {
                Resp::SendFailure(compose::why_the_send_failed(&self.wiring.database, draft).await)
            }
            Req::DefaultSignature { account, selected } => Resp::Signature(
                compose::default_signature(&self.wiring.database, account, selected).await,
            ),
            Req::Attach(path) => {
                let blobs = self.wiring.blobs.clone();
                let stored = tokio::task::spawn_blocking(move || {
                    let mime_type = compose::guess_mime_type(&path);
                    compose::attach_file(&blobs, &path, mime_type)
                })
                .await
                .ok()
                .flatten();
                Resp::Attached(stored)
            }
            Req::InlineImage { bytes, mime_type } => {
                let blobs = self.wiring.blobs.clone();
                let stored = tokio::task::spawn_blocking(move || {
                    compose::inline_attachment(&blobs, bytes, &mime_type)
                })
                .await
                .ok()
                .flatten();
                Resp::Attached(stored)
            }
            Req::Page(page) => store
                .list_page(page)
                .await
                .map_or_else(Resp::Failed, Resp::Page),
            Req::Count(scope) => store
                .list_count(scope)
                .await
                .map_or_else(Resp::Failed, Resp::Count),
            Req::Rows(ids) => store
                .message_rows(ids)
                .await
                .map_or_else(Resp::Failed, Resp::Rows),
            Req::RowsIn(scope, ids) => store
                .rows_in(scope, ids)
                .await
                .map_or_else(Resp::Failed, Resp::ListRows),
            Req::Mailboxes(account) => store
                .mailboxes(account)
                .await
                .map_or_else(Resp::Failed, Resp::Mailboxes),
            Req::Accounts => match self.wiring.database.connect().await {
                Ok(connection) => postio_storage::repository::AccountRepository::new(&connection)
                    .list()
                    .await
                    .map_or_else(
                        |error| Resp::Failed(postio_model::listing::StoreError::from(error)),
                        Resp::Accounts,
                    ),
                Err(error) => Resp::Failed(postio_model::listing::StoreError::from(error)),
            },
            Req::Body(message) => self.body(message).await,
            Req::Conversation(thread) => self.conversation(thread).await,
            Req::Unsubscribe(message) => self.unsubscribe(message).await,
            Req::Parts(message) => {
                match parts::read_message(&self.wiring.database, message).await {
                    Ok(found) => Resp::Parts(found.attachments),
                    Err(reason) => Resp::Failed(postio_model::listing::StoreError::new(reason)),
                }
            }
            Req::SavePart {
                message,
                attachment,
                to,
            } => self.write_part(message, attachment, to).await,
            Req::OpenPart {
                message,
                attachment,
            } => match self.private_copy_path(message, attachment).await {
                Ok(to) => self.write_part(message, attachment, to).await,
                Err(reason) => Resp::Failed(postio_model::listing::StoreError::new(reason)),
            },
            Req::DraftCounts(account) => store
                .draft_counts(account)
                .await
                .map_or_else(Resp::Failed, Resp::DraftCounts),
        }
    }

    /// A message's body, or which kind of "no body" it is.
    ///
    /// "Offline" is not told apart from "not fetched yet" here: the host has
    /// no reachability signal of its own yet, and saying "downloading" about
    /// a body that is not is the milder of the two mistakes.
    async fn body(&self, message: postio_model::MessageId) -> Resp {
        use postio_client::protocol::Body;
        use postio_session::reading::{Body as Stored, load_body_or_reason};
        use postio_ui::reader::document::Absent;
        let connection = match self.wiring.database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Resp::Failed(postio_model::listing::StoreError::from(error)),
        };
        Resp::Body(
            match load_body_or_reason(&connection, message, false).await {
                Stored::Ready {
                    body,
                    encoding_problems,
                } => Body::Ready {
                    body,
                    encoding_problems,
                },
                Stored::Absent(Absent::Partial) => Body::Partial,
                Stored::Absent(Absent::Offline) => Body::Offline,
                Stored::Absent(Absent::Missing) => Body::Missing,
                Stored::Absent(Absent::Empty) => Body::Empty,
                Stored::Absent(Absent::ForeignDraft) => Body::ForeignDraft,
            },
        )
    }

    /// A conversation's messages as list rows, oldest first: the order it
    /// happened in, which is how a reader reads down the page.
    async fn conversation(&self, thread: postio_model::ThreadId) -> Resp {
        use postio_model::listing::StoreError;
        use postio_storage::repository::{ThreadOrder, ThreadRepository};
        let ids = match self.wiring.database.connect().await {
            Ok(connection) => match ThreadRepository::new(&connection)
                .messages(thread, ThreadOrder::Oldest)
                .await
            {
                Ok(rows) => rows.into_iter().map(|row| row.id).collect::<Vec<_>>(),
                Err(error) => return Resp::Failed(StoreError::from(error)),
            },
            Err(error) => return Resp::Failed(StoreError::from(error)),
        };
        // The store's own rows, so a conversation's members look exactly like
        // the list rows every frontend already draws.
        self.wiring
            .store
            .message_rows(ids)
            .await
            .map_or_else(Resp::Failed, Resp::Rows)
    }

    /// Record that the person asked to leave `message`'s list, and name it.
    ///
    /// The list is its `List-Id`, else the sender's domain -- the desktop
    /// reader's rule (#971), moved here from `postio-app` so every frontend
    /// records it the same way. Only ever answered for a deliberate act.
    async fn unsubscribe(&self, message: postio_model::MessageId) -> Resp {
        use postio_model::listing::StoreError;
        use postio_storage::repository::{MessageRepository, UnsubscribeRepository};
        let connection = match self.wiring.database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Resp::Failed(StoreError::from(error)),
        };
        let found = match MessageRepository::new(&connection).get(message).await {
            Ok(found) => found,
            Err(error) => return Resp::Failed(StoreError::from(error)),
        };
        let Some(found) = found else {
            return Resp::Failed(StoreError::new("That message is gone."));
        };
        let list = found.list_id.clone().or_else(|| {
            found
                .from
                .first()
                .and_then(|from| from.domain())
                .map(str::to_owned)
        });
        let Some(list) = list else {
            return Resp::Failed(StoreError::new("This message names no list to leave."));
        };
        let mut activation = postio_model::UnsubscribeActivation::new(
            found.account_id,
            list.clone(),
            chrono::Utc::now(),
        );
        match UnsubscribeRepository::new(&connection)
            .record(&mut activation)
            .await
        {
            Ok(_) => Resp::Unsubscribed(list),
            Err(error) => Resp::Failed(StoreError::from(error)),
        }
    }

    /// Write one part's bytes to `to`, fetching them first if they were
    /// never downloaded -- the one reading path allowed to reach the network,
    /// and only because a person asked for these bytes.
    async fn write_part(
        &self,
        message: postio_model::MessageId,
        attachment: postio_model::ids::AttachmentId,
        to: std::path::PathBuf,
    ) -> Resp {
        let engine = self.wiring.engine.get().cloned();
        let bytes = parts::part_bytes(
            &self.wiring.database,
            &self.wiring.blobs,
            engine,
            message,
            attachment,
        )
        .await;
        let written = bytes.and_then(|bytes| {
            // Replaces rather than appends: an appended save would corrupt
            // whatever was there.
            std::fs::write(&to, bytes).map_err(|error| error.to_string())
        });
        match written {
            Ok(()) => Resp::Saved(to),
            Err(reason) => Resp::Failed(postio_model::listing::StoreError::new(reason)),
        }
    }

    /// Where a part opened by the system is written: under this user's
    /// runtime directory, readable by this user alone, named as the sender
    /// named it -- but only the file name, so a hostile name cannot write
    /// anywhere else.
    async fn private_copy_path(
        &self,
        message: postio_model::MessageId,
        attachment: postio_model::ids::AttachmentId,
    ) -> Result<std::path::PathBuf, String> {
        use std::os::unix::fs::DirBuilderExt;
        let found = parts::read_message(&self.wiring.database, message).await?;
        let name = found
            .attachments
            .iter()
            .find(|part| part.id == attachment)
            .and_then(|part| part.filename.clone())
            .and_then(|name| {
                std::path::Path::new(&name)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .filter(|name| !name.is_empty() && name != "." && name != "..")
            .unwrap_or_else(|| "part".to_owned());
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join("postio").join("parts").join(format!(
            "{}-{}",
            message.get(),
            attachment.get()
        ));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|error| error.to_string())?;
        Ok(dir.join(name))
    }

    /// Put a command on the one queue. Never waits.
    fn enqueue(
        &self,
        client: ClientId,
        command: Command,
        aim: StateSnapshot,
        invocation: Option<InvocationId>,
    ) -> Resp {
        let queued = Queued {
            client,
            command,
            aim,
            invocation,
        };
        match self.queue.try_send(queued) {
            Ok(()) => Resp::Done,
            Err(_) => Resp::Stopped,
        }
    }
}

/// The in-process transport: requests run on the host's runtime and answer
/// through a oneshot, so a caller on any executor -- GTK's main loop, a test's
/// runtime -- can await them.
struct Local {
    inner: Arc<Inner>,
    client: ClientId,
    events: async_channel::Receiver<EventEnvelope>,
}

impl Drop for Local {
    fn drop(&mut self) {
        self.inner.leave(self.client);
    }
}

impl Transport for Local {
    fn call(&self, request: Req) -> Call<'_> {
        let request = match self.inner.answer_in_order(self.client, request) {
            InOrder::Answered(answered) => return Box::pin(async move { Ok(answered) }),
            InOrder::Pending(landing) => {
                // On the host's runtime, so the answer lands whatever
                // executor the caller awaits on.
                let (answer, answered) = tokio::sync::oneshot::channel();
                self.inner.runtime().spawn(async move {
                    let _ = answer.send(landing.await);
                });
                return Box::pin(async move { answered.await.map_err(|_| Disconnected) });
            }
            InOrder::Later(request) => request,
        };
        let (answer, answered) = tokio::sync::oneshot::channel();
        let inner = Arc::clone(&self.inner);
        let client = self.client;
        self.inner.runtime().spawn(async move {
            let _ = answer.send(inner.answer(client, request).await);
        });
        Box::pin(async move { answered.await.map_err(|_| Disconnected) })
    }

    fn post(&self, request: Req) {
        let request = match self.inner.answer_in_order(self.client, request) {
            InOrder::Answered(_) => return,
            InOrder::Pending(landing) => {
                self.inner.runtime().spawn(landing);
                return;
            }
            InOrder::Later(request) => request,
        };
        let inner = Arc::clone(&self.inner);
        let client = self.client;
        self.inner.runtime().spawn(async move {
            inner.answer(client, request).await;
        });
    }

    fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        self.events.clone()
    }
}

pub mod compose;
pub mod parts;
pub mod serve;

#[cfg(test)]
mod tests;
