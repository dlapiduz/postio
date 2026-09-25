//! What owns Postio's store, in the process of the app that opened it.
//!
//! Turso holds an exclusive lock on the database file, and the operation
//! queue is correct only with one drainer, so one app at a time opens the
//! store -- the desktop app or the terminal -- and runs the engines in its
//! own process (ADR 0041). Whichever it is starts a [`Host`] over the store
//! ([`Host::open`]) and reads and writes only through `postio-client`'s
//! [`Client`], handed out by [`Host::connect`] with no encoding in between:
//! a request is a value on a channel, answered on the host's runtime.
//!
//! # A selection per client
//!
//! Each client has its own selection, focus and undo history, and the host
//! has none of its own. So each connected client gets its own verbs over the
//! store: its own [`SharedState`], which adopts the snapshot each of its
//! commands carries, and its own [`Actions`], whose undo stack is therefore
//! that client's alone (research R1a). A window's first-run screen and its
//! panes are two clients of one host.
//!
//! Commands from every client run **one at a time, in arrival order**, on
//! one queue. That is what the single bridge pump guaranteed, and the verbs
//! were written against it.
//!
//! # Whose events are whose
//!
//! What a command changed in the store is everybody's news. What a command
//! *says about itself* is its sender's alone: the "Archived 12 messages —
//! Undo" notice, a refusal, an error, the end of a tracked send.
//! [`is_feedback`] draws that line.

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

/// The store's owner in this process, serving the clients connected to it.
///
/// Dropping it stops its runtime, and with it the engines.
pub struct Host {
    inner: Arc<Inner>,
    // Last: the runtime outlives everything spawned on it. `None` for a host
    // over a wiring whose runtime somebody else holds ([`Host::over`]).
    _bridge: Option<Bridge>,
}

struct Inner {
    wiring: Wiring,
    /// What each address's last probe offered for JMAP, for the submission
    /// that follows it.
    offers: Mutex<HashMap<String, Option<postio_account::discovery::JmapOffer>>>,
    /// What each address's last probe offered for a browser sign-in.
    oauth_offers: Mutex<HashMap<String, postio_account::discovery::OAuthOffer>>,
    /// The browser sign-ins under way, by address.
    sign_ins: Mutex<HashMap<String, SignIn>>,
    /// Where everybody's news goes: a sink on the event hub, or over a
    /// wiring built elsewhere, whatever that wiring reports to.
    hub: EventSink,
    clients: Mutex<HashMap<ClientId, Entry>>,
    next_client: AtomicU64,
    queue: async_channel::Sender<Queued>,
    /// `[sync]`'s notification settings: which folders' arrivals notify.
    notify: Mutex<postio_config::SyncConfig>,
    /// The engine syncing each account, once started.
    engines: Engines,
}

/// The engine syncing each account: at most one per account, however many
/// times a frontend asks for sync to start.
type Engines = Arc<EngineTable>;

#[derive(Default)]
struct EngineTable {
    running: Mutex<HashMap<postio_model::AccountId, postio_runtime::Engine>>,
    /// Held while engines start, so two asks at once -- the app's own at
    /// startup and one after an account is added -- cannot both find none
    /// running.
    starting: tokio::sync::Mutex<()>,
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

/// Start syncing every enabled account that is not syncing yet.
///
/// From nothing, every account is started together, under the connection
/// budget's judgement of the whole set; once some are running, each one
/// missing joins them, as an account added while Postio runs does. Either
/// way an account never gets a second engine.
async fn start_sync(wiring: Wiring, engines: Engines) {
    let _starting = engines.starting.lock().await;
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
    let running = engines.running.lock().expect("never poisoned").len();
    if running == 0 {
        match postio_session::engine::start_all(&accounts, &wiring).await {
            Ok(started) => {
                for (account, engine) in started {
                    adopt(&wiring, &engines, account, engine);
                }
            }
            Err(refusal) => {
                tracing::error!(%refusal, "not starting the sync engines: {refusal}");
            }
        }
        return;
    }
    let total = accounts.len();
    for account in accounts {
        if engines
            .running
            .lock()
            .expect("never poisoned")
            .contains_key(&account.id)
        {
            continue;
        }
        match postio_session::engine::start_joining(&account, total, &wiring).await {
            Ok(Some(engine)) => adopt(&wiring, &engines, account.id, engine),
            Ok(None) => {}
            Err(refusal) => {
                tracing::error!(%refusal, "not starting an account's sync: {refusal}");
            }
        }
    }
}

/// Keep `engine` as `account`'s: retained so the process stops it before
/// exiting, and in the slot `Refresh` reads if it is the first.
fn adopt(
    wiring: &Wiring,
    engines: &Engines,
    account: postio_model::AccountId,
    engine: postio_runtime::Engine,
) {
    postio_runtime::retain(engine.clone());
    wiring.engine.fill(engine.clone());
    engines
        .running
        .lock()
        .expect("never poisoned")
        .insert(account, engine);
}

/// Start the sync of the account saved for `address`, and no other.
async fn start_engine_for(wiring: &Wiring, engines: &Engines, address: &str) {
    let Ok(connection) = wiring.database.connect().await else {
        return;
    };
    let account = postio_storage::repository::AccountRepository::new(&connection)
        .list()
        .await
        .ok()
        .and_then(|accounts| {
            accounts
                .into_iter()
                .find(|account| account.address.address.eq_ignore_ascii_case(address))
        });
    if let Some(account) = account
        && let Some(engine) = postio_session::engine::start(&account, wiring)
    {
        adopt(wiring, engines, account.id, engine);
    }
}

/// Rebuild `account`'s local search index (#981), announcing each reading
/// as [`Event::BackfillProgress`] on the account's own id: one progress
/// channel, not two.
async fn rebuild(database: Store, events: EventSink, account: postio_model::AccountId) {
    let rebuilt = postio_session::reindex_account(&database, account, |done, total| {
        events.emit(Event::BackfillProgress {
            account,
            done,
            total,
            footprint: None,
        });
    });
    if let Err(error) = rebuilt.await {
        tracing::warn!(%error, "could not rebuild an account's local search index");
    }
}

/// A write's answer: done, or the store's sentence for why not.
fn done(written: Result<(), postio_model::listing::StoreError>) -> Resp {
    written.map_or_else(Resp::Failed, |()| Resp::Done)
}

/// A browser sign-in under way.
struct SignIn {
    /// Gives it up.
    cancel: postio_account::cancel::CancelToken,
    /// How it ended, once it has.
    done: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}

/// The host's browser opener: it opens nothing, and reports the URL to the
/// frontend that asked, which opens it only when the person does.
struct Announcer(Mutex<Option<tokio::sync::oneshot::Sender<postio_account::oauth::Url>>>);

impl postio_account::oauth::BrowserOpener for Announcer {
    fn open(&self, url: &postio_account::oauth::Url) -> std::io::Result<()> {
        if let Some(announce) = self.0.lock().expect("never poisoned").take() {
            let _ = announce.send(url.clone());
        }
        Ok(())
    }
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

/// Worker threads for the host's runtime: the desktop app's number, and its
/// reason (#1502). The work is I/O-bound command handling -- the engines and
/// the store have threads of their own -- and every idle worker is another
/// malloc arena kept after a burst.
const WORKER_THREADS: usize = 2;

/// The blocking pool's ceiling, as the desktop app bounded it: the passes
/// after the first frame and the odd synchronous read, without climbing
/// toward tokio's default of 512 in a burst.
const BLOCKING_THREADS: usize = 8;

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
            .worker_threads(WORKER_THREADS)
            .max_blocking_threads(BLOCKING_THREADS)
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

        Ok(Host::serving(wiring, hub.sink(), Some(bridge)))
    }

    /// Read the store key, open the store, and start a host over it, wired
    /// as `config.toml` at `config_path` asks: what an app does before it
    /// shows anything, in its own process.
    ///
    /// `report` hears each wait before it starts -- the keyring, then the
    /// store's own stages -- so a frontend can say what it is waiting on.
    /// Blocks the calling thread for all of it. `Err` is a sentence for a
    /// person: a keyring that will not answer, or a store that will not open
    /// -- among them [`postio_storage::Error::InUse`], another Postio having
    /// it open.
    pub fn open(
        config_path: Option<&std::path::Path>,
        secrets: Arc<dyn postio_account::secret::SecretStore>,
        report: &dyn Fn(postio_ui::list_state::Waiting),
    ) -> Result<Host, String> {
        use postio_ui::list_state::Waiting;

        report(Waiting::Keyring);
        let key = postio_session::store_key_blocking(secrets.as_ref())
            .map_err(|error| error.to_string())?;
        let (database, blobs) = {
            // Its own runtime, dropped before the host's exists: opening the
            // store is async, and nothing else is running yet to host it.
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .map_err(|error| {
                    format!("Postio could not start the worker that opens its store: {error}")
                })?;
            runtime.block_on(postio_session::open_store_reporting(&key, &|stage| {
                report(match stage {
                    postio_session::Opening::Store => Waiting::Store,
                    postio_session::Opening::Migrating => Waiting::Migrating,
                    postio_session::Opening::Indexing => Waiting::Indexing,
                })
            }))?
        };

        let sync_config = config_path
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
            .map(|config| config.sync)
            .unwrap_or_default();
        let mailbox_roles = config_path
            .map(postio_session::mailbox_roles_at)
            .unwrap_or_default();
        let storage_ceiling = config_path.and_then(postio_session::storage_ceiling_at);

        let host = Host::start(database, blobs, |wiring| {
            wiring
                .with_mailbox_roles(mailbox_roles)
                .with_backfill(postio_session::backfill_policy(&sync_config))
                .with_watch(postio_session::watch_policy(&sync_config))
                .with_storage_ceiling(storage_ceiling)
                .with_secrets(secrets)
        })?;
        // Which folders' arrivals are worth a notification.
        host.notify_with(sync_config);
        Ok(host)
    }

    /// Adopt a wiring built elsewhere -- the desktop app's, or an
    /// integration suite's -- rather than opening the store again: its
    /// runtime, its event hub, its engines' slot.
    ///
    /// A wiring whose events go to a single reader rather than a hub still
    /// has its store served, and still hears what the host's verbs change;
    /// its clients hear only what their own commands say about themselves.
    pub fn over(wiring: Wiring) -> Host {
        let hub = wiring.events.clone();
        Host::serving(wiring, hub, None)
    }

    fn serving(wiring: Wiring, hub: EventSink, bridge: Option<Bridge>) -> Host {
        let (queue, commands) = async_channel::unbounded::<Queued>();
        let inner = Arc::new(Inner {
            wiring,
            hub,
            clients: Mutex::new(HashMap::new()),
            next_client: AtomicU64::new(1),
            queue,
            notify: Mutex::new(postio_config::SyncConfig::default()),
            engines: Engines::default(),
            offers: Mutex::new(HashMap::new()),
            oauth_offers: Mutex::new(HashMap::new()),
            sign_ins: Mutex::new(HashMap::new()),
        });
        let pump = Arc::clone(&inner);
        inner.runtime().spawn(async move {
            while let Ok(queued) = commands.recv().await {
                pump.run(queued).await;
            }
        });
        Host {
            inner,
            _bridge: bridge,
        }
    }

    /// Notify about arrivals as `[sync]` in `config` says: `notify` and
    /// `notify_roles`. The defaults until this is called.
    pub fn notify_with(&self, config: postio_config::SyncConfig) {
        *self.inner.notify.lock().expect("never poisoned") = config;
    }

    /// Whether `messages` arriving in `mailbox` is worth a notification, and
    /// what it says, while the person's `attention` is where it is:
    /// [`notify::decide_arrival`] under `[sync]` as [`Host::notify_with`]
    /// last set it. Read on the host's runtime; the answer is awaited
    /// anywhere.
    pub fn notification(
        &self,
        mailbox: postio_model::MailboxId,
        messages: Vec<postio_model::MessageId>,
        attention: postio_ui::notify::Attention,
    ) -> impl std::future::Future<Output = Option<postio_ui::notify::Notification>> + Send + 'static
    {
        let inner = Arc::clone(&self.inner);
        let deciding = self.inner.runtime().spawn(async move {
            let config = inner.notify.lock().expect("never poisoned").clone();
            notify::decide_arrival(
                &inner.wiring.database,
                inner.wiring.store.as_ref(),
                &config,
                mailbox,
                &messages,
                attention,
            )
            .await
        });
        async move { deciding.await.ok().flatten() }
    }

    /// The verbs each frontend's dispatcher answers, for a frontend that
    /// filters its gestures by them as the desktop's window does.
    pub fn wired(&self) -> Vec<postio_core::CommandId> {
        verbs(&self.inner.wiring, &SharedState::default())
            .wired()
            .collect()
    }

    /// The wiring every read and the engines hang off.
    pub fn wiring(&self) -> &Wiring {
        &self.inner.wiring
    }

    /// Start a sync engine for every enabled account, on the host's runtime.
    ///
    /// Sync runs while the app that opened the store runs, and stops with
    /// the host ([`Host::stop`]).
    pub fn start_syncing(&self) {
        let wiring = self.inner.wiring.clone();
        let engines = Arc::clone(&self.inner.engines);
        self.inner
            .runtime()
            .spawn(async move { start_sync(wiring, engines).await });
    }

    /// Start the passes that catch the store up with itself: the body
    /// indexer, the header repair and index, and the disk reclaim
    /// ([`maintenance::spawn_idle_passes`]).
    pub fn start_idle_passes(&self) {
        maintenance::spawn_idle_passes(&self.inner.wiring);
    }

    /// The same passes, `delay` from now: long enough for the first
    /// frontend's first pages to have had the runtime to themselves, since
    /// nobody is waiting on any of this (#1604).
    pub fn start_idle_passes_after(&self, delay: std::time::Duration) {
        let wiring = self.inner.wiring.clone();
        self.inner.runtime().spawn(async move {
            tokio::time::sleep(delay).await;
            maintenance::spawn_idle_passes(&wiring);
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

    /// A client of this host: every frontend's, in its own process.
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
        // Over a wiring whose events go to one reader, there is no news to
        // subscribe to: the client hears what its own commands say.
        let everybody = self.hub.subscribe(&label);
        let to_client = outbox.clone();
        let hearing = self.runtime().spawn(async move {
            let Some(everybody) = everybody else {
                return;
            };
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
        let hub = self.hub.clone();
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
                let hub = self.hub.clone();
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
            Req::RecipientDirectory(account) => Resp::RecipientDirectory(
                compose::recipient_directory(&self.wiring.database, account).await,
            ),
            Req::Correspondents(account) => {
                Resp::Correspondents(compose::correspondents(&self.wiring.database, account).await)
            }
            Req::Labels(account) => {
                Resp::Labels(compose::labels(&self.wiring.database, account).await)
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
            Req::AttachmentBytes(blob) => {
                let blobs = self.wiring.blobs.clone();
                let read = tokio::task::spawn_blocking(move || compose::blob_bytes(&blobs, &blob))
                    .await
                    .ok()
                    .flatten();
                Resp::Bytes(read)
            }
            Req::RecoverDraft(account) => Resp::Draft(
                compose::recover(&self.wiring.database, account)
                    .await
                    .map(Box::new),
            ),
            Req::Attach { path, mime_type } => {
                let blobs = self.wiring.blobs.clone();
                let stored = tokio::task::spawn_blocking(move || {
                    let mime_type = mime_type.unwrap_or_else(|| compose::guess_mime_type(&path));
                    compose::attach_file(&blobs, &path, mime_type)
                })
                .await
                .ok()
                .flatten();
                Resp::Attached(stored)
            }
            Req::Search(search) => Resp::Found(self.search(search).await),
            Req::SearchHits {
                account,
                query,
                scope,
                order,
                snippets,
            } => Resp::Hits(
                search::hits(
                    &self.wiring.database,
                    account,
                    &query,
                    scope,
                    order,
                    snippets as usize,
                )
                .await
                .map(|results| Box::new(postio_client::protocol::Hits(results))),
            ),
            Req::Facets {
                account,
                query,
                scope,
            } => Resp::Facets(search::facets(&self.wiring.database, account, &query, scope).await),
            Req::StoredBody(message) => {
                Resp::StoredBody(search::stored_body(&self.wiring.database, message).await)
            }
            Req::ExportMessages(messages) => match export::write_messages(
                &self.wiring.database,
                &self.wiring.blobs,
                self.wiring.engine.get().cloned(),
                &messages,
            )
            .await
            {
                Ok(written) => Resp::Exported(written),
                Err(reason) => Resp::Failed(postio_model::listing::StoreError::new(reason)),
            },
            Req::Account(op) => match self.account(op).await {
                Ok(()) => Resp::Done,
                Err(error) => Resp::Failed(error),
            },
            Req::BeginOAuth(submission) => match self.begin_oauth(*submission).await {
                Ok(consent) => Resp::Consent(Box::new(consent)),
                Err(sentence) => Resp::Failed(postio_model::listing::StoreError::new(sentence)),
            },
            Req::FinishOAuth(address) => match self.finish_oauth(&address).await {
                Ok(()) => Resp::Done,
                Err(sentence) => Resp::Failed(postio_model::listing::StoreError::new(sentence)),
            },
            Req::CancelOAuth(address) => {
                if let Some(sign_in) = self
                    .sign_ins
                    .lock()
                    .expect("never poisoned")
                    .get(&address.to_ascii_lowercase())
                {
                    sign_in.cancel.cancel();
                }
                Resp::Done
            }
            Req::Discover(address) => Resp::Onboarding(Box::new(self.discover(&address).await)),
            Req::AddAccount(submission) => match self.add_account(*submission).await {
                Ok(()) => Resp::Done,
                Err(sentence) => Resp::Failed(postio_model::listing::StoreError::new(sentence)),
            },
            Req::AccountSettings { weights } => settings::accounts(&self.wiring.database, weights)
                .await
                .map_or_else(Resp::Failed, Resp::AccountSettings),
            Req::EditAccount(account, field) => {
                done(settings::edit(&self.wiring.database, account, field).await)
            }
            Req::SaveSignature {
                account,
                signature,
                name,
                text,
            } => done(
                settings::save_signature(&self.wiring.database, account, signature, &name, &text)
                    .await,
            ),
            Req::DeleteSignature(signature) => {
                done(settings::delete_signature(&self.wiring.database, signature).await)
            }
            Req::RebuildIndex(account) => {
                self.rebuild_index(account).await;
                Resp::Done
            }
            Req::EgressLog(limit) => settings::egress(&self.wiring.database, limit)
                .await
                .map_or_else(Resp::Failed, Resp::Egress),
            Req::PrivacyLog => settings::privacy(&self.wiring.database)
                .await
                .map_or_else(Resp::Failed, Resp::Privacy),
            Req::SetBackfillExcluded { mailbox, excluded } => {
                settings::set_backfill_excluded(&self.wiring.database, mailbox, excluded)
                    .await
                    .map_or_else(Resp::Failed, Resp::Mailboxes)
            }
            Req::OrientationSeen => settings::orientation_seen(&self.wiring.database)
                .await
                .map_or_else(Resp::Failed, Resp::Seen),
            Req::RetireOrientation => {
                done(settings::retire_orientation(&self.wiring.database).await)
            }
            Req::SaveAccount {
                submission,
                backend,
            } => done(
                postio_session::onboarding::persist(
                    &self.wiring.database,
                    self.wiring.secrets.as_ref(),
                    &submission,
                    backend,
                )
                .await
                .map_err(postio_model::listing::StoreError::new),
            ),
            Req::SaveOAuthAccount(grant) => done(
                onboarding::save_oauth(&self.wiring.database, self.wiring.secrets.clone(), *grant)
                    .await
                    .map_err(postio_model::listing::StoreError::new),
            ),
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
            // A removed account is gone for every frontend until the removal
            // is undone or carried out, as the desktop's settings show it.
            Req::Accounts => match self.wiring.database.connect().await {
                Ok(connection) => postio_storage::repository::AccountRepository::new(&connection)
                    .list()
                    .await
                    .map_or_else(
                        |error| Resp::Failed(postio_model::listing::StoreError::from(error)),
                        |accounts| {
                            Resp::Accounts(
                                accounts
                                    .into_iter()
                                    .filter(|account| !account.pending_deletion)
                                    .collect(),
                            )
                        },
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
            Req::Readings { messages, offline } => {
                reading::readings(&self.wiring.database, &messages, offline)
                    .await
                    .map_or_else(Resp::Failed, Resp::Readings)
            }
            Req::ThreadReadings {
                thread,
                limit,
                offline,
            } => reading::thread_readings(&self.wiring.database, thread, limit as usize, offline)
                .await
                .map_or_else(Resp::Failed, Resp::Readings),
            Req::InlinePart {
                message,
                content_id,
            } => Resp::InlinePart(
                postio_session::reading::resolve_cid(
                    &self.wiring.database,
                    &self.wiring.blobs,
                    message,
                    &content_id,
                )
                .await,
            ),
            Req::SaveParts { message, parts } => {
                let failed = parts::save_parts(
                    &self.wiring.database,
                    &self.wiring.blobs,
                    self.wiring.engine.get().cloned(),
                    message,
                    &parts,
                )
                .await;
                Resp::SavedParts(u32::try_from(failed).unwrap_or(u32::MAX))
            }
            Req::FetchBody(message) => {
                self.fetch_body(message);
                Resp::Done
            }
            Req::StorageCeiling(max_bytes) => {
                maintenance::enforce_ceiling(&self.wiring, max_bytes);
                Resp::Done
            }
            Req::DraftCounts(account) => store
                .draft_counts(account)
                .await
                .map_or_else(Resp::Failed, Resp::DraftCounts),
        }
    }

    /// An account change, as the desktop's settings make it
    /// (`postio-app`'s `settings_accounts`). Everyone's sidebar hears of it.
    async fn account(
        &self,
        op: postio_client::protocol::AccountOp,
    ) -> Result<(), postio_model::listing::StoreError> {
        use postio_client::protocol::AccountOp;
        use postio_storage::repository::AccountRepository;
        let connection = self.wiring.database.connect().await?;
        let accounts = AccountRepository::new(&connection);
        match op {
            AccountOp::SetEnabled { account, enabled } => {
                accounts.set_enabled(account, enabled).await?;
            }
            AccountOp::Remove(account) => {
                accounts.mark_pending_deletion(account).await?;
            }
            AccountOp::Restore(account) => {
                accounts.restore(account).await?;
            }
            AccountOp::SetDefault(account) => accounts.set_default(account).await?,
            AccountOp::RebuildIndex(account) => {
                drop(connection);
                self.runtime().spawn(rebuild(
                    self.wiring.database.clone(),
                    self.wiring.events.clone(),
                    account,
                ));
                return Ok(());
            }
        }
        // The account's folders appear, go or come back in every sidebar:
        // what a mailbox tree changing already makes each frontend redraw.
        let account = match op {
            AccountOp::SetEnabled { account, .. }
            | AccountOp::Remove(account)
            | AccountOp::Restore(account)
            | AccountOp::SetDefault(account)
            | AccountOp::RebuildIndex(account) => account,
        };
        self.hub.emit(Event::MailboxesChanged { account });
        Ok(())
    }

    /// Ask every engine for `message`'s body ahead of its backfill: a person
    /// opened it. Each engine answers for its own account's mail. Over a
    /// wiring whose engine someone else started, that one.
    fn fetch_body(&self, message: postio_model::MessageId) {
        let mut engines: Vec<postio_runtime::Engine> = self
            .engines
            .running
            .lock()
            .expect("never poisoned")
            .values()
            .cloned()
            .collect();
        if engines.is_empty() {
            engines.extend(self.wiring.engine.get().cloned());
        }
        for engine in engines {
            self.runtime().spawn(async move {
                if let Err(error) = engine.request_body(message).await {
                    tracing::warn!(message = message.get(), %error, "cannot fetch that body");
                }
            });
        }
    }

    /// Rebuild `account`'s local search index, and return when it is over.
    async fn rebuild_index(&self, account: postio_model::AccountId) {
        rebuild(
            self.wiring.database.clone(),
            self.wiring.events.clone(),
            account,
        )
        .await;
    }

    /// What discovery finds for `address`, as the first-run screen shows it:
    /// the desktop's probe, options and reading of it
    /// (`postio_session::onboarding`). What the probe offered for JMAP is
    /// kept for the submission that follows.
    async fn discover(&self, address: &str) -> postio_ui::onboarding::Status {
        let probe = postio_account::discovery::Probe::with_options(
            self.wiring.discovery.clone(),
            postio_session::onboarding::probe_options(),
        );
        let cancel = postio_account::discovery::CancelToken::new();
        match probe.run(address, &cancel).await {
            Ok(report) => {
                let jmap = report.settings().and_then(|settings| {
                    (settings.backends.first().map(String::as_str) == Some("jmap"))
                        .then(|| settings.jmap.clone())
                        .flatten()
                });
                self.offers
                    .lock()
                    .expect("never poisoned")
                    .insert(address.to_ascii_lowercase(), jmap);
                if let Some(oauth) = report
                    .settings()
                    .and_then(|settings| settings.oauth.clone())
                {
                    self.oauth_offers
                        .lock()
                        .expect("never poisoned")
                        .insert(address.to_ascii_lowercase(), oauth);
                }
                postio_session::onboarding::status_for(&report)
            }
            Err(error) => {
                tracing::info!(%error, "autoconfig found nothing");
                postio_ui::onboarding::Status::Manual { suggestion: None }
            }
        }
    }

    /// Begin the desktop's browser sign-in for `submission`
    /// (`postio_session::onboarding::run_sign_in`) and answer where it waits
    /// for the person: the consent URL, in full.
    ///
    /// **Nothing is opened here.** The host's opener only reports the URL;
    /// the frontend shows it and opens it, or copies it, when the person
    /// asks (US7 scenario 2). The sign-in runs on, waiting at its loopback
    /// redirect, and [`finish_oauth`](Self::finish_oauth) is how a frontend
    /// hears the end of it.
    async fn begin_oauth(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<postio_ui::onboarding::BrowserSignIn, String> {
        let key = submission.address.to_ascii_lowercase();
        let offer = self
            .oauth_offers
            .lock()
            .expect("never poisoned")
            .get(&key)
            .cloned()
            .ok_or_else(|| "This address's provider has no browser sign-in.".to_owned())?;
        let client = submission
            .oauth_client
            .clone()
            .ok_or_else(|| "A browser sign-in needs the OAuth client's id.".to_owned())?;
        let cancel = postio_account::cancel::CancelToken::new();
        let (announce, announced) = tokio::sync::oneshot::channel();
        let (finished, done) = tokio::sync::watch::channel(None);
        self.sign_ins.lock().expect("never poisoned").insert(
            key.clone(),
            SignIn {
                cancel: cancel.clone(),
                done,
            },
        );
        let opener = Announcer(Mutex::new(Some(announce)));
        let wiring = self.wiring.clone();
        let engines = Arc::clone(&self.engines);
        let scopes = offer.scopes.clone();
        let refresh = offer.refresh_token_lifetime_days;
        let provider = postio_session::onboarding::provider_name(&submission.settings);
        self.runtime().spawn(async move {
            let settings = postio_session::onboarding::connection_settings(&submission);
            let signed_in = postio_session::onboarding::run_sign_in(
                &settings, &client, &offer, &opener, &cancel,
            )
            .await;
            let outcome = match signed_in {
                Ok((endpoints, tokens)) => {
                    postio_session::onboarding::persist_oauth(
                        &wiring.database,
                        wiring.secrets.clone(),
                        &submission,
                        &endpoints,
                        &scopes,
                        refresh,
                        tokens,
                    )
                    .await
                }
                Err(postio_session::onboarding::SignInError::Cancelled) => {
                    Err("The sign-in was cancelled.".to_owned())
                }
                Err(postio_session::onboarding::SignInError::Failed(reason)) => Err(reason),
            };
            if outcome.is_ok() {
                start_engine_for(&wiring, &engines, &submission.address).await;
            }
            let _ = finished.send(Some(outcome));
        });
        let url = announced
            .await
            .map_err(|_| "The sign-in ended before it reached the browser step.".to_owned())?;
        let redirect_uri = url
            .query_pairs()
            .find(|(key, _)| key == "redirect_uri")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        Ok(postio_ui::onboarding::BrowserSignIn {
            provider,
            scopes: self
                .oauth_offers
                .lock()
                .expect("never poisoned")
                .get(&key)
                .map(|offer| offer.scopes.clone())
                .unwrap_or_default(),
            redirect_uri,
            authorize_url: url.to_string(),
        })
    }

    /// Wait for the sign-in for `address` to end.
    async fn finish_oauth(&self, address: &str) -> Result<(), String> {
        let done = self
            .sign_ins
            .lock()
            .expect("never poisoned")
            .get(&address.to_ascii_lowercase())
            .map(|sign_in| sign_in.done.clone());
        let Some(mut done) = done else {
            return Err("There is no sign-in under way for that address.".to_owned());
        };
        loop {
            if let Some(outcome) = done.borrow().clone() {
                self.sign_ins
                    .lock()
                    .expect("never poisoned")
                    .remove(&address.to_ascii_lowercase());
                return outcome;
            }
            if done.changed().await.is_err() {
                return Err("The sign-in stopped without an answer.".to_owned());
            }
        }
    }

    /// Prove `submission`'s credentials, save the account, and start its
    /// sync: the desktop's order, so a refused sign-in writes nothing. The
    /// error is the first-run screen's sentence.
    async fn add_account(
        &self,
        submission: postio_ui::onboarding::Submission,
    ) -> Result<(), String> {
        let jmap = self
            .offers
            .lock()
            .expect("never poisoned")
            .get(&submission.address.to_ascii_lowercase())
            .cloned()
            .flatten();
        let backend = match &self.wiring.mail {
            // Handed a mail server (a test's), the proof is signing in to it.
            Some(mail) => postio_account::backend::MailBackend::connect(mail.backend.as_ref())
                .await
                .map(|_| postio_model::account::Backend::Imap)
                .map_err(|error| postio_session::onboarding::explain(&error))?,
            None => postio_session::onboarding::prove(&submission, jmap.as_ref()).await?,
        };
        postio_session::onboarding::persist(
            &self.wiring.database,
            self.wiring.secrets.as_ref(),
            &submission,
            backend,
        )
        .await?;
        // Its engine, and only its: the others are already running.
        start_engine_for(&self.wiring, &self.engines, &submission.address).await;
        Ok(())
    }

    /// A search, as the desktop's bar runs it: in the scope asked (every
    /// folder but drafts, junk and trash, unless the query names one, by
    /// default), up to the desktop's hit limit, with no excerpts -- a
    /// terminal list shows none.
    async fn search(
        &self,
        search: postio_client::protocol::Search,
    ) -> Option<postio_client::protocol::Found> {
        let query = postio_search::parse(&search.query, chrono::Local::now().date_naive());
        let order = if search.newest_first {
            postio_search::ResultOrder::Newest
        } else {
            postio_search::ResultOrder::Relevance
        };
        let connection = self
            .wiring
            .database
            .read()
            .await
            .map_err(|error| tracing::warn!(%error, "could not open the store to search"))
            .ok()?;
        let results = postio_session::search::execute_with_snippets(
            &connection,
            search.account,
            &query,
            search.scope,
            order,
            0,
        )
        .await?;
        Some(postio_client::protocol::Found {
            ids: results.hits.iter().map(|hit| hit.message_id).collect(),
            hits: results.total_hits,
            capped: results.total_hits_capped,
            corpus_complete: results.corpus_complete,
            elapsed: results.elapsed,
        })
    }

    /// A message's body, or which kind of "no body" it is.
    ///
    /// "Offline" is not told apart from "not fetched yet" here: the host has
    /// no reachability signal of its own yet, and saying "downloading" about
    /// a body that is not is the milder of the two mistakes.
    async fn body(&self, message: postio_model::MessageId) -> Resp {
        let connection = match self.wiring.database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Resp::Failed(postio_model::listing::StoreError::from(error)),
        };
        Resp::Body(reading::wire_body(
            postio_session::reading::load_body_or_reason(&connection, message, false).await,
        ))
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
        let written = parts::save_part(
            &self.wiring.database,
            &self.wiring.blobs,
            self.wiring.engine.get().cloned(),
            message,
            attachment,
            &to,
        )
        .await;
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
    fn call(&self, request: Req) -> Call<'static> {
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
pub mod export;
pub mod maintenance;
pub mod notify;
pub mod onboarding;
pub mod parts;
pub mod reading;
pub mod search;
pub mod settings;
pub mod startup;

#[cfg(test)]
mod tests;
