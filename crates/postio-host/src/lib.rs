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
}

/// A request answered on arrival, or one left to be answered concurrently.
enum InOrder {
    /// Answered now, in the order it arrived.
    Answered(Resp),
    /// A read, to answer on a task of its own.
    Later(Req),
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
            InOrder::Later(request) => request,
        };
        let store = &self.wiring.store;
        match request {
            Req::Send(..) | Req::SendTracked(..) | Req::NoteRemoved(..) => {
                unreachable!("answered in order above")
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
            Req::DraftCounts(account) => store
                .draft_counts(account)
                .await
                .map_or_else(Resp::Failed, Resp::DraftCounts),
        }
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

pub mod serve;

#[cfg(test)]
mod tests;
