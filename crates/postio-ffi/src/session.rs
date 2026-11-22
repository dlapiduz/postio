use std::sync::{Arc, Mutex};

use postio_core::bridge::{CommandSender, EventStream};
use postio_host::Host;
use postio_session::Wiring;

use crate::event::UiEvent;

/// Distinguishable cases because the frontend routes on them: a locked keyring means "unlock this
/// and retry", not "set up an account" (ADR 0014).
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SessionError {
    #[error("{message}")]
    StoreUnavailable { message: String },
    /// Its own case: the remedy is the user unlocking a keyring, not a broken store.
    #[error("{message}")]
    KeyringLocked { message: String },
    /// Written at a schema this build cannot carry forward (`postio_session::Remedy::StartOver`).
    /// Trying again meets the
    /// same file, so the way forward is starting over, which sets the store aside rather than
    /// deleting it.
    #[error("{message}")]
    StoreFromAnotherBuild { message: String },
    /// Encrypted under a key the keyring does not hold (#1598). Starting over is the way forward,
    /// but nothing decrypts, so no account comes across.
    #[error("{message}")]
    StoreUnderAnotherKey { message: String },
    #[error("{message}")]
    RuntimeUnavailable { message: String },
}

impl SessionError {
    fn from_refusal(refusal: postio_session::Refusal) -> Self {
        let message = refusal.sentence;
        match refusal.remedy {
            postio_session::Remedy::TryAgain => SessionError::StoreUnavailable { message },
            postio_session::Remedy::StartOver {
                because: postio_session::Unreadable::AnotherBuild,
                ..
            } => SessionError::StoreFromAnotherBuild { message },
            postio_session::Remedy::StartOver {
                because: postio_session::Unreadable::AnotherKey,
                ..
            } => SessionError::StoreUnderAnotherKey { message },
        }
    }

    /// A match rather than `to_string`: `SecretError::Locked` must survive to the surface that asks
    /// the user to unlock,
    /// not be flattened and sent to onboarding (ADR 0014).
    fn from_secret_error(error: postio_account::secret::SecretError) -> Self {
        let message = error.to_string();
        match error {
            postio_account::secret::SecretError::Locked { .. } => {
                SessionError::KeyringLocked { message }
            }
            _ => SessionError::StoreUnavailable { message },
        }
    }
}

/// A ceiling against the pathological thread, not a page size: nothing pages through it. 500 is
/// what the GTK frontend uses.
const THREAD_LIMIT: u32 = 500;

/// Not a `uniffi::Record`: it can carry a caller-supplied runtime and command bus, which have no
/// crossing. Swift uses the exported constructor on `Session`.
pub struct SessionOptions {
    store_path: Option<std::path::PathBuf>,
    bridge: Option<(tokio::runtime::Handle, CommandSender)>,
    secrets: Option<Arc<dyn postio_account::secret::SecretStore>>,
    #[cfg(feature = "testing")]
    in_memory: bool,
    #[cfg(feature = "testing")]
    seeded: Option<postio_storage::Store>,
    #[cfg(feature = "testing")]
    seeded_blobs: Option<(postio_storage::BlobStore, tempfile::TempDir)>,
    #[cfg(feature = "testing")]
    config: ConfigSource,
    #[cfg(feature = "testing")]
    discovery: Option<Arc<dyn postio_account::discovery::DiscoveryTransport>>,
    #[cfg(feature = "testing")]
    mail: Option<postio_session::MailOverride>,
    #[cfg(feature = "testing")]
    agents_at: Option<std::path::PathBuf>,
}

/// Its own function because four calls say it; a store that is not open is not a part that could
/// not be fetched.
fn no_store() -> crate::PartsError {
    crate::PartsError::Refused {
        message: "There is no open store to read that message from".to_owned(),
    }
}

/// Refused rather than written: an account naming no server fails later, at sync, as a connection
/// error nobody can act on.
const NO_SERVERS: &str = "Postio needs the incoming and outgoing server names — it will not guess them from your address.";

/// Monotonic and never reused: a closed session may still have a hand-off file somebody is editing
/// (`Session::serial`).
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl SessionOptions {
    pub fn at_default_path() -> Self {
        Self {
            store_path: None,
            bridge: None,
            secrets: None,
            #[cfg(feature = "testing")]
            in_memory: false,
            #[cfg(feature = "testing")]
            seeded: None,
            #[cfg(feature = "testing")]
            seeded_blobs: None,
            #[cfg(feature = "testing")]
            config: ConfigSource::Installed,
            #[cfg(feature = "testing")]
            discovery: None,
            #[cfg(feature = "testing")]
            mail: None,
            #[cfg(feature = "testing")]
            agents_at: None,
        }
    }

    pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            ..Self::at_default_path()
        }
    }

    /// Reads the store's key from `secrets` rather than the OS keyring, which would prompt on a
    /// developer's machine and hang on a headless one.
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_account::secret::SecretStore>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Read this installation's real `config.toml`: the escape hatch for the rare test about the
    /// installed file. Other tests want `with_config_for_test`.
    #[cfg(feature = "testing")]
    pub fn with_installed_config_for_test(mut self) -> Self {
        self.config = ConfigSource::Installed;
        self
    }

    /// Configured by an empty document, not the machine's `config.toml`, so a result means the same
    /// wherever it runs (#1219).
    #[cfg(feature = "testing")]
    pub fn in_memory() -> Self {
        Self {
            in_memory: true,
            config: ConfigSource::Document(String::new()),
            ..Self::at_default_path()
        }
    }

    /// Answer agents' calls on the socket at `path` (ADR 0041), as a session over a store file
    /// always does; for the demo, which has no file.
    #[cfg(feature = "testing")]
    pub fn answering_agents_at(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.agents_at = Some(path.into());
        self
    }

    /// An in-memory session over a database the caller already seeded: the store wiring is private,
    /// so rows must exist before the session opens.
    #[cfg(feature = "testing")]
    pub fn in_memory_with(database: postio_storage::Store) -> Self {
        Self {
            seeded: Some(database),
            ..Self::in_memory()
        }
    }

    /// Use a blob store the caller already wrote bodies into: a body written after the session
    /// opens would land in a different directory.
    #[cfg(feature = "testing")]
    pub fn with_blobs_for_test(
        mut self,
        blobs: postio_storage::BlobStore,
        scratch: tempfile::TempDir,
    ) -> Self {
        self.seeded_blobs = Some((blobs, scratch));
        self
    }

    /// Use this `config.toml` text rather than the one on disk; since #1219 it only supplies
    /// content, as in-memory sessions already ignore the installed file.
    #[cfg(feature = "testing")]
    pub fn with_config_for_test(mut self, text: &str) -> Self {
        self.config = ConfigSource::Document(text.to_owned());
        self
    }

    /// Read, and watch, the `config.toml` at `path` rather than the
    /// installed one: how a test edits the file a running session reads.
    #[cfg(feature = "testing")]
    pub fn with_config_file_for_test(mut self, path: &std::path::Path) -> Self {
        self.config = ConfigSource::File(path.to_owned());
        self
    }

    /// An in-memory session on a runtime and command bus the caller owns, or it would end up with
    /// two runtimes and a deadlock.
    #[cfg(feature = "testing")]
    pub fn in_memory_on(runtime: tokio::runtime::Handle, commands: CommandSender) -> Self {
        Self::in_memory().on_bridge(runtime, commands)
    }

    /// Look new accounts' servers up through `discovery` rather than the
    /// network. The first-run wizard's tests answer from the preset table
    /// alone; nothing in the default suite may dial (CLAUDE.md).
    #[cfg(feature = "testing")]
    pub fn with_discovery_for_test(
        mut self,
        discovery: Arc<dyn postio_account::discovery::DiscoveryTransport>,
    ) -> Self {
        self.discovery = Some(discovery);
        self
    }

    /// Sign in, read and send through `mail` rather than the servers an
    /// account names -- what `Connect`'s proof is tested against.
    #[cfg(feature = "testing")]
    pub fn with_mail_for_test(mut self, mail: postio_session::MailOverride) -> Self {
        self.mail = Some(mail);
        self
    }

    /// Run on a runtime the caller owns, keeping whatever store these options name. The host is
    /// adopted onto `runtime`; what
    /// the session sends goes through its client, so a verb reaches real handlers whether or not
    /// the caller's bus has any (#721).
    pub fn on_bridge(mut self, runtime: tokio::runtime::Handle, commands: CommandSender) -> Self {
        self.bridge = Some((runtime, commands));
        self
    }
}

/// Where a session's configuration comes from. Explicit because an absent document used to mean
/// "read the developer's own
/// config", so tests passed on CI and failed on a workstation (#1219). There is no variant meaning
/// "whatever turns up".
#[derive(Debug, Clone)]
enum ConfigSource {
    /// Exactly this document. An empty one is the built-in defaults.
    /// Only a test asks for one, so a build without `testing` never makes it.
    #[cfg_attr(not(feature = "testing"), allow(dead_code))]
    Document(String),
    /// Whatever `config.toml` this installation has, or the defaults if there
    /// is none. What a shipping application wants, and what a test gets only
    /// by asking for it by name.
    Installed,
    /// The `config.toml` at this path, watched like the installed one. Only
    /// a test asks for one: it is how a test edits the file a session reads.
    #[cfg_attr(not(feature = "testing"), allow(dead_code))]
    File(std::path::PathBuf),
}

impl ConfigSource {
    fn path(&self) -> Option<std::path::PathBuf> {
        match self {
            ConfigSource::Installed => crate::settings::config_path().ok(),
            ConfigSource::File(path) => Some(path.clone()),
            ConfigSource::Document(_) => None,
        }
    }
}

/// This session's configuration, whole, from one read of `config.toml` (#1219). A config that will
/// not parse falls back to
/// the defaults: a mistyped binding must not stop the application opening.
fn load_config(source: &ConfigSource) -> postio_config::Config {
    let parsed = match source {
        ConfigSource::Document(text) => postio_config::Config::from_toml_str(text).ok(),
        ConfigSource::Installed => crate::settings::config_path()
            .ok()
            .and_then(|path| postio_config::Config::load_from_path(&path).ok()),
        ConfigSource::File(path) => postio_config::Config::load_from_path(path).ok(),
    };
    parsed.unwrap_or_else(|| {
        match source {
            ConfigSource::Installed => tracing::warn!(
                "using the built-in configuration: config.toml is absent or unreadable"
            ),
            ConfigSource::Document(_) | ConfigSource::File(_) => tracing::warn!(
                "using the built-in configuration: the given document will not parse"
            ),
        }
        Default::default()
    })
}

/// Without the `testing` feature a shipping session reads the installed file and cannot be handed a
/// document.
#[cfg(feature = "testing")]
fn config_source(options: &SessionOptions) -> ConfigSource {
    options.config.clone()
}

#[cfg(not(feature = "testing"))]
fn config_source(_options: &SessionOptions) -> ConfigSource {
    ConfigSource::Installed
}

/// Switch Focus's engine on in `host`, as `[focus]` says: the filing pass, markers, digests and
/// reminders. After the host
/// starts and before the first sync, as GTK and the terminal do (`postio_tui::run::engage_focus`).
fn engage_focus(
    host: &Host,
    focus: &postio_config::FocusConfig,
    source: &ConfigSource,
    remote_images: &std::path::Path,
) -> postio_host::FocusHandle {
    let path = source.path();
    // The Filtering pane reads a file and a person's pin is not in it: it
    // reads, and takes back, what people decided from here.
    let (focus_settings, database) = (host.wiring().focus.clone(), host.wiring().database.clone());
    let (reader, writer) = (focus_settings.clone(), (focus_settings, database));
    crate::settings::follow_contacts(crate::settings::ContactFilters {
        rules: Box::new(move || {
            reader
                .config()
                .map(|config| config.contacts().to_vec())
                .unwrap_or_default()
        }),
        take_back: Box::new(move |undo| {
            let (focus, database) = &writer;
            blocking(async {
                match undo {
                    postio_ui::filtering::Undo::Never(entry) => {
                        focus.pin(database, entry, false).await
                    }
                    postio_ui::filtering::Undo::Marker { sender, kind } => {
                        focus.stop_marker(database, sender, kind, false).await
                    }
                }
            })
            .is_ok()
        }),
    });
    host.enable_focus(
        postio_host::FocusSetup::from_config(focus.clone(), path.as_deref())
            .with_remote_images_path(remote_images.to_path_buf()),
    )
}

/// Start the store's owner over an open store (ADR 0041): nothing else on macOS can share the
/// store, so the host runs in
/// this process and this frontend is its one client. Over the caller's runtime when it supplied
/// one, adopted rather than starting a second.
fn serve(
    database: postio_storage::Store,
    blobs: postio_storage::BlobStore,
    caller: Option<(tokio::runtime::Handle, CommandSender)>,
    configure: impl FnOnce(Wiring) -> Wiring,
) -> Result<Host, SessionError> {
    match caller {
        None => Host::start(database, blobs, configure)
            .map_err(|message| SessionError::RuntimeUnavailable { message }),
        Some((runtime, commands)) => {
            // A hub, so the host can subscribe each client and the body
            // indexer to it; a direct sink would give them nothing to hear.
            let hub = postio_core::bridge::EventHub::new();
            Ok(Host::over(configure(Wiring::new(
                database,
                blobs,
                runtime,
                hub.sink(),
                commands,
            ))))
        }
    }
}

/// Answer agents on the socket at `socket` with `host` while the session lasts (ADR 0041). `[mcp]`
/// is read from the session's
/// `config.toml`, or the installation's when it has none; the access log is kept beside the store.
fn answer_agents(
    socket: std::path::PathBuf,
    host: &Host,
    source: &ConfigSource,
) -> Mutex<Option<postio_mcp::socket::Listener>> {
    let audit = socket
        .parent()
        .and_then(std::path::Path::parent)
        .map(|store_dir| postio_mcp::Audit::to_file(store_dir.join("mcp-access.log")))
        .unwrap_or_else(postio_mcp::Audit::none);
    let config = source
        .path()
        .or_else(|| crate::settings::config_path().ok());
    Mutex::new(
        postio_mcp::socket::listen(socket, host.connector(), config, audit)
            .inspect_err(|error| {
                tracing::warn!(%error, "agents will not be answered by this app: {error}");
            })
            .ok(),
    )
}

#[derive(Clone, Default)]
struct FinderSources {
    contacts: Vec<postio_model::Contact>,
    labels: Vec<postio_model::Label>,
}

/// How long the box keeps `FinderSources` before reading them again: correspondents arrive on a
/// first sync, so a box
/// that read them once would offer none all session.
const FINDER_SOURCES_FOR: std::time::Duration = std::time::Duration::from_secs(60);

type Seams = (
    Option<Arc<dyn postio_account::discovery::DiscoveryTransport>>,
    Option<postio_session::MailOverride>,
);

#[cfg(feature = "testing")]
fn seams(options: &SessionOptions) -> Seams {
    (options.discovery.clone(), options.mail.clone())
}

#[cfg(not(feature = "testing"))]
fn seams(_options: &SessionOptions) -> Seams {
    (None, None)
}

/// The wiring, looking new accounts up over the network with every connection written to the egress
/// log (#151); the
/// wiring's own default transport records nothing.
fn with_onboarding(wiring: Wiring, (discovery, mail): Seams) -> Wiring {
    let discovery = discovery.unwrap_or_else(|| {
        Arc::new(
            postio_account::discovery::PimalayaTransport::new().with_egress(wiring.egress.clone()),
        )
    });
    let wiring = wiring.with_discovery(discovery);
    match mail {
        Some(mail) => wiring.with_mail(mail),
        None => wiring,
    }
}

type Aimed = (postio_core::Command, postio_core::state::SharedState);

/// This session's line to the host. Dropped by `Session::shutdown`, which ends both.
struct Link {
    client: postio_client::Client,
    outbox: async_channel::Sender<Aimed>,
    /// Held only to be dropped: the task feeding the session's event stream stops when this goes.
    _hearing: async_channel::Sender<()>,
}

/// Connect this frontend to `host` as its client. Each command crosses with the aim it was issued
/// under, so two quick
/// verbs each act on the row they were pressed on.
fn connect(host: &Host) -> (Wiring, EventStream, Link) {
    let client = host.connect(postio_client::protocol::ClientKind::Ffi);
    let runtime = host.wiring().runtime.clone();

    let (outbox, queued) = async_channel::unbounded::<Aimed>();
    runtime.spawn({
        let client = client.clone();
        async move {
            while let Ok((command, aim)) = queued.recv().await {
                if client.clone().with_state(aim).send(command).await.is_err() {
                    return;
                }
            }
        }
    });

    let (sink, stream) = postio_core::bridge::event_channel();
    let arriving = client.events();
    let (hearing, stopped) = async_channel::bounded::<()>(1);
    runtime.spawn(async move {
        loop {
            tokio::select! {
                envelope = arriving.recv() => {
                    let Ok(envelope) = envelope else { return };
                    if !sink.emit(envelope.event) {
                        return;
                    }
                }
                _ = stopped.recv() => return,
            }
        }
    });

    let wiring = Wiring {
        store: Arc::new(client.clone()),
        ..host.wiring().clone()
    };
    (
        wiring,
        stream,
        Link {
            client,
            outbox,
            _hearing: hearing,
        },
    )
}

/// The frontend's handle on the engine: commands down, events up. The `Wiring` is behind an
/// `Option` so `shutdown`
/// can drop it, which ends the event stream and the frontend's drain loop.
#[derive(uniffi::Object)]
pub struct Session {
    wiring: Mutex<Option<Wiring>>,
    /// The accounts an aggregate view could show when `Ctrl+A` was pressed, fixed at the gesture
    /// rather than looked up
    /// when the verb runs (#811, ADR 0005 Q10). Empty makes `Ctrl+A` in the aggregate a rejection.
    reachable: Mutex<Vec<postio_model::ids::AccountId>>,
    finder_sources: Mutex<Option<(std::time::Instant, FinderSources)>>,
    pub(crate) recipient_directories: crate::contacts::Directories,
    /// The conversation the reading pane is showing, held whole rather than paged through Focus's
    /// list: a thread is bounded.
    conversation: Arc<Mutex<Option<crate::ConversationFfi>>>,
    /// The store this session opened, for the composer's footer to name where a draft lives.
    store_at: std::path::PathBuf,
    /// The agent socket, while this session answers agents (ADR 0041). Taken at shutdown, before
    /// the store goes.
    agents: Mutex<Option<postio_mcp::socket::Listener>>,
    /// The sign-in in flight, so closing the sheet can cancel it: a token unwinds the loopback
    /// listener and stops a token exchange.
    sign_in: Mutex<Option<postio_account::cancel::CancelToken>>,
    sign_in_port: Arc<std::sync::atomic::AtomicU16>,
    /// Where the remote-image grants live, beside the store: resolved once at open, since an
    /// in-memory store has no
    /// directory and deriving one gave every test the same file.
    allow_list_at: std::path::PathBuf,
    /// The grants and treatments at `allow_list_at`, read once and kept (#1589). Every write goes
    /// through here; `None`
    /// until first asked and after a failed write.
    allow_list_kept: Mutex<Option<postio_ui::allowlist::RemoteImageAllowList>>,
    /// This session's number within the process, so two sessions never share one hand-off
    /// directory.
    serial: u64,
    /// Each message's drawn body in the conversation document, so a redraw sanitises only what
    /// changed (#1595).
    thread_renders: Mutex<postio_ui::reader::document::RenderCache>,
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    /// How many reconnects this session has asked for, so a test can see the nudge.
    reconnects: Arc<std::sync::atomic::AtomicUsize>,
    /// Whether the engine currently has no connection at all. Pushed down by the frontend
    /// (`NWPathMonitor`); it only
    /// changes which absence the reader reports, so being briefly wrong costs a word.
    offline: Arc<std::sync::atomic::AtomicBool>,
    /// The engines this session started, kept alive: dropping one at exit can tear a sync pass's
    /// write mid-commit. Keyed
    /// by account so `start_syncing` can tell a running account from one added while the app was
    /// up.
    engines: Mutex<Vec<(postio_model::ids::AccountId, postio_runtime::Engine)>>,
    /// `[keys]` as this installation has it, read once at open: re-reading `config.toml` on every
    /// menu draw would be a file read per repaint.
    keys: Mutex<postio_config::keys::KeyBindings>,
    /// The `[ui]` table this session was opened with, read once so the list and settings pane
    /// cannot disagree.
    ui: postio_config::ui::UiConfig,
    /// The bindings in force, resolved once: see `Session::keymap`. Keyed by how many commands the
    /// registry holds, so a
    /// later-registered extension is not left out.
    keymap: Mutex<Option<(usize, postio_core::Keymap)>>,
    /// Events this boundary raises itself, merged into the drain alongside the engine's: paging is
    /// how this frontend reads
    /// a list, not something the engine does.
    local: (
        async_channel::Sender<UiEvent>,
        async_channel::Receiver<UiEvent>,
    ),
    events: EventStream,
    /// Where commands go, and what keeps `events` fed from the host. Taken by `shutdown`, which
    /// ends the drain.
    link: Mutex<Option<Link>>,
    /// The verbs the host answers. A command outside them is not sent.
    wired: Vec<postio_core::CommandId>,
    /// The store's owner, in this process (ADR 0041), kept alive as long as the session: dropping
    /// it stops the runtime,
    /// unless the caller supplied its own.
    _host: Host,
    /// Focus's engine, switched on for this host and held as long as the session.
    _focus: postio_host::FocusHandle,
    /// `[focus]` as it stands, kept current by `follow_config`.
    focus_config: Mutex<postio_config::FocusConfig>,
    /// `[sync] notify` and `[sync] badge` as they stand, kept current by `follow_config`.
    notices: Mutex<crate::NoticesFfi>,
    /// `config.toml`, watched while the session lives (`follow_config`); `None` for a session given
    /// a document rather
    /// than a file, or when it cannot be watched.
    config_watch: Mutex<Option<postio_config::watch::ConfigWatcher>>,
    focus_list: Arc<crate::focus_list::FocusDriver>,
    #[cfg(feature = "testing")]
    _scratch: Option<tempfile::TempDir>,
}

/// The surface Swift sees, narrower than the Rust `impl` below: the exported constructor takes only
/// where the store lives.
// Everything in this impl block crosses to Swift: a plain Rust method added here is exported too,
// and a
// `#[cfg(feature = "testing")]` method fails to build with the feature off. Rust-side methods go in
// the second `impl Session` block.
#[uniffi::export]
impl Session {
    #[uniffi::constructor]
    pub fn open_at(store_path: Option<String>) -> Result<Arc<Self>, SessionError> {
        Self::open(match store_path {
            Some(path) => SessionOptions::at(path),
            None => SessionOptions::at_default_path(),
        })
    }

    /// The next event, or `None` once the session has stopped. Swift drives it as
    /// `Task { @MainActor in while let e = await session.nextEvent() { … } }`.
    #[uniffi::method(name = "nextEvent")]
    pub async fn next_event_ffi(&self) -> Option<UiEvent> {
        self.next_event().await
    }

    /// Drops the store and ends the event drain. Idempotent.
    #[uniffi::method(name = "shutdown")]
    pub fn shutdown_ffi(&self) {
        self.shutdown();
    }

    /// Read a conversation into the reading pane. Returns at once; `ConversationReady` says when to
    /// draw. The list is untouched (#1003).
    #[uniffi::method(name = "openConversation")]
    pub fn open_conversation_ffi(&self, thread: i64) {
        self.open_conversation(thread);
    }

    /// `thread` as one document, with each message's anchor. Blocks on the store, so off the main
    /// actor.
    #[uniffi::method(name = "threadDocument")]
    pub fn thread_document_ffi(&self, thread: i64, reduced: Vec<i64>) -> crate::ThreadDocumentFfi {
        blocking(self.thread_document(thread, reduced))
    }

    /// The conversation the pane is showing, folded — `None` until one has
    /// been asked for.
    #[uniffi::method(name = "conversation")]
    pub fn conversation_ffi(&self) -> Option<crate::ConversationFfi> {
        self.conversation()
    }

    #[uniffi::method(name = "newDraft")]
    pub fn new_draft_ffi(&self) -> Option<crate::DraftFfi> {
        blocking(self.new_draft())
    }

    /// A draft prefilled from a `mailto:` link; the recipients arrive already parsed (RFC 6068 is
    /// the platform's URL machinery).
    #[uniffi::method(name = "mailtoDraft")]
    pub fn mailto_draft_ffi(
        &self,
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: Option<String>,
        body: Option<String>,
    ) -> Option<crate::DraftFfi> {
        blocking(self.mailto_draft(to, cc, bcc, subject, body))
    }

    #[uniffi::method(name = "replyDraft")]
    pub fn reply_draft_ffi(&self, message: i64, all: bool) -> Option<crate::DraftFfi> {
        blocking(self.reply_draft(message, all))
    }

    #[uniffi::method(name = "forwardDraft")]
    pub fn forward_draft_ffi(&self, message: i64) -> Option<crate::DraftFfi> {
        blocking(self.forward_draft(message))
    }

    /// Attach `path` to the draft and answer it with the file on it. `mime_type` comes from the
    /// frontend, since sniffing is a platform service.
    #[uniffi::method(name = "attachToDraft")]
    pub fn attach_to_draft_ffi(
        &self,
        draft: crate::DraftFfi,
        path: String,
        mime_type: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.attach_to_draft(draft, path, mime_type))
    }

    /// Put a picture into the draft's body and answer the draft with it and the script that draws
    /// it at the caret (#1571).
    /// Bytes, not a path, since a picture arrives from a paste; it must be an image.
    #[uniffi::method(name = "insertInlineImage")]
    pub fn insert_inline_image_ffi(
        &self,
        draft: crate::DraftFfi,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<crate::InlineImageFfi, crate::ComposeError> {
        blocking(self.insert_inline_image(draft, bytes, mime_type))
    }

    /// One picture in draft `draft`'s own body, by its `Content-ID`, for the composer's
    /// `postio-cid:` handler. `nil` is a broken picture, never a fetch.
    #[uniffi::method(name = "resolveDraftCid")]
    pub fn resolve_draft_cid_ffi(
        &self,
        draft: i64,
        content_id: String,
    ) -> Option<crate::InlinePart> {
        blocking(self.resolve_draft_cid(draft, content_id))
    }

    /// Write the draft where another editor can open it, and answer where: a private directory,
    /// mode 0600, since a draft is unsent mail.
    #[uniffi::method(name = "beginHandoff")]
    pub fn begin_handoff_ffi(&self, draft: crate::DraftFfi) -> Result<String, crate::ComposeError> {
        blocking(self.begin_handoff(draft))
    }

    #[uniffi::method(name = "endHandoff")]
    pub fn end_handoff_ffi(
        &self,
        draft: crate::DraftFfi,
        path: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.end_handoff(draft, path))
    }

    #[uniffi::method(name = "detachFromDraft")]
    pub fn detach_from_draft_ffi(
        &self,
        draft: crate::DraftFfi,
        attachment: i64,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.detach_from_draft(draft, attachment))
    }

    /// The draft `id` as the store has it; `nil` when there is no such draft.
    #[uniffi::method(name = "draft")]
    pub fn draft_ffi(&self, id: i64) -> Option<crate::DraftFfi> {
        blocking(self.draft(id))
    }

    /// The draft behind message `message`, for a `Continue editing` link (#1212).
    #[uniffi::method(name = "draftForMessage")]
    pub fn draft_for_message_ffi(&self, message: i64) -> Option<crate::DraftFfi> {
        blocking(self.draft_for_message(message))
    }

    #[uniffi::method(name = "narrowPaste")]
    pub fn narrow_paste_ffi(&self, html: String) -> crate::PastedFfi {
        self.narrow_paste(html)
    }

    #[uniffi::method(name = "plainTextOf")]
    pub fn plain_text_of_ffi(&self, html: String) -> String {
        self.plain_text_of(&html)
    }

    #[uniffi::method(name = "richHtmlOf")]
    pub fn rich_html_of_ffi(&self, text: String) -> String {
        self.rich_html_of(&text)
    }

    /// The script that applies a mark to the composer's selection; `nil` for a command that is not
    /// a mark.
    #[uniffi::method(name = "markScript")]
    pub fn mark_script_ffi(&self, command: String) -> Option<String> {
        self.mark_script(&command)
    }

    /// The script that links the selection to `href`, or `nil` when a message may not point there.
    #[uniffi::method(name = "linkScript")]
    pub fn link_script_ffi(&self, href: String) -> Option<String> {
        self.link_script(&href)
    }

    /// The one script the composer's editing surface runs, so both frontends run one dialect.
    #[uniffi::method(name = "editorScript")]
    pub fn editor_script_ffi(&self) -> String {
        self.editor_script().to_owned()
    }

    /// Throw a draft away: the row and the server copy.
    #[uniffi::method(name = "discardDraft")]
    pub fn discard_draft_ffi(&self, draft: i64) -> Option<String> {
        blocking(self.discard_draft(draft))
    }

    #[uniffi::method(name = "saveDraft")]
    pub fn save_draft_ffi(&self, draft: crate::DraftFfi) -> Option<crate::DraftFfi> {
        blocking(self.save_draft(draft))
    }

    /// Queue the draft for sending; `None` when it went, a sentence when it
    /// could not.
    #[uniffi::method(name = "sendDraft")]
    pub fn send_draft_ffi(&self, draft: crate::DraftFfi) -> Option<String> {
        blocking(self.send_draft(draft))
    }

    #[uniffi::method(name = "sendDraftLater")]
    pub fn send_draft_later_ffi(&self, draft: crate::DraftFfi, when: i64) -> Option<String> {
        blocking(self.send_draft_later(draft, when))
    }

    /// Everything the pane asks about one open message in one body load and one render (#1589).
    /// Blocking: off the main actor.
    #[uniffi::method(name = "messageFacts")]
    pub fn message_facts_ffi(&self, message: i64) -> crate::MessageFactsFfi {
        blocking(self.message_facts(message))
    }

    /// What agents have done lately, newest first, with the ids the access
    /// log holds resolved to the rows they name (#1832). Empty when the
    /// session keeps no log (an in-memory store). Blocking: it reads the log
    /// and a few rows; call it off the main actor.
    #[uniffi::method(name = "agentActivity")]
    pub fn agent_activity_ffi(&self) -> crate::AgentActivityFfi {
        blocking(self.agent_activity())
    }

    #[uniffi::method(name = "readerActions")]
    pub fn reader_actions_ffi(&self) -> Vec<crate::ReaderActionFfi> {
        self.reader_actions()
    }

    #[uniffi::method(name = "conversationActions")]
    pub fn conversation_actions_ffi(&self, messages: u32) -> Vec<crate::ConversationActionFfi> {
        self.conversation_actions(messages)
    }

    #[uniffi::method(name = "allowSender")]
    pub fn allow_sender_ffi(&self, address: String) {
        self.allow_sender(address);
    }

    /// `always_show_images` from the keyboard or the palette. Off the main actor: it renders the
    /// message to learn whether anything was held back.
    #[uniffi::method(name = "alwaysShowImagesFor")]
    pub fn always_show_images_for_ffi(&self, message: i64) -> bool {
        blocking(self.always_show_images_for(message))
    }

    #[uniffi::method(name = "allowDomain")]
    pub fn allow_domain_ffi(&self, domain: String) {
        self.allow_domain(domain);
    }

    /// Every remote-image grant, addresses first, each already sorted: the Privacy pane's whole
    /// model.
    #[uniffi::method(name = "remoteImageGrants")]
    pub fn remote_image_grants_ffi(&self) -> Vec<crate::GrantFfi> {
        self.remote_image_grants()
    }

    /// Take a grant back. Images from it are blocked again at once.
    #[uniffi::method(name = "revokeRemoteImages")]
    pub fn revoke_remote_images_ffi(&self, subject: String) {
        self.revoke_remote_images(subject);
    }

    /// Leave the list this message came from: the deliberate activation, and the only thing here
    /// that records one. Call it from a button and nothing else.
    /// `None` when recorded, a sentence when not; a message that offers nothing is refused.
    /// Off the main actor: it writes, and a write waits on the store's machine-wide gate.
    #[uniffi::method(name = "activateUnsubscribe")]
    pub fn activate_unsubscribe_ffi(&self, message: i64) -> Option<String> {
        blocking(self.activate_unsubscribe(message))
    }

    /// After leaving a list: what it left behind and the one action offered
    /// about it, or `None` when nothing is left (#1931, #1952). A read.
    #[uniffi::method(name = "listMailOffer")]
    pub fn list_mail_offer_ffi(&self, message: i64) -> Option<crate::ListMailOfferFfi> {
        blocking(self.list_mail_offer(message))
    }

    /// Archive (`delete == false`) or delete everything the list of `message`
    /// sent: one command on exactly those messages, so one undo. `None` when
    /// it was sent, a sentence when it was not. Never awaits the network.
    #[uniffi::method(name = "clearListMail")]
    pub fn clear_list_mail_ffi(&self, message: i64, delete: bool) -> Option<String> {
        blocking(self.clear_list_mail(message, delete))
    }

    #[uniffi::method(name = "unsubscribeActivations")]
    pub fn unsubscribe_activations_ffi(&self) -> Vec<crate::UnsubscribeActivationFfi> {
        blocking(self.unsubscribe_activations())
    }

    /// What looking `address` up finds. Asked on a deliberate step, never per keystroke: each
    /// lookup is DNS and HTTPS to the
    /// address's domain, written to the egress log. Blocks, so not from the main actor.
    #[uniffi::method(name = "discoverAccount")]
    pub fn discover_account_ffi(&self, address: String) -> crate::DiscoveredFfi {
        blocking(self.discover_account(address))
    }

    /// Sign in to the servers `account` names and, only if that works, add
    /// it. `None` when it was added, a sentence when it was not. Blocks on
    /// the server, so not from the main actor.
    #[uniffi::method(name = "connectAccount")]
    pub fn connect_account_ffi(&self, account: crate::NewAccountFfi) -> Option<String> {
        blocking(self.connect_account(account))
    }

    /// Add an account that is a directory on this machine. `None` when it
    /// was added, a sentence when it was not.
    #[uniffi::method(name = "addLocalAccount")]
    pub fn add_local_account_ffi(&self, address: String, path: String) -> Option<String> {
        blocking(self.add_local_account(address, path))
    }

    /// Whether `path` is a mail store Postio can open: `None` when it is, a sentence naming the
    /// directory when not. Asked
    /// while the field is still showing, so the sheet can say what is wrong before anything is
    /// written.
    #[uniffi::method(name = "inspectLocalStore")]
    pub fn inspect_local_store_ffi(&self, path: String) -> Option<String> {
        self.inspect_local_store(path)
    }

    /// Sign in to `address` through the system browser, and add the account. Returns when the flow
    /// is over: `None` on
    /// success, a sentence on failure or when the user closed the tab. Blocks, so off the main
    /// actor.
    #[uniffi::method(name = "signInWithBrowser")]
    pub fn sign_in_with_browser_ffi(
        &self,
        address: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Option<String> {
        blocking(self.sign_in_with_browser(address, client_id, client_secret))
    }

    #[uniffi::method(name = "signInProgress")]
    pub fn sign_in_progress_ffi(&self) -> crate::SignInProgressFfi {
        self.sign_in_progress()
    }

    #[uniffi::method(name = "cancelSignIn")]
    pub fn cancel_sign_in_ffi(&self) {
        self.cancel_sign_in();
    }

    /// Open a session against `account`'s server and close it again — what
    /// sync does, and then stops. Blocks; run it off the main actor.
    #[uniffi::method(name = "testConnection")]
    pub fn test_connection_ffi(&self, account: i64) -> crate::ConnectionReportFfi {
        blocking(self.test_connection(account))
    }

    #[uniffi::method(name = "reindexAccount")]
    pub fn reindex_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.reindex_account(account))
    }

    #[uniffi::method(name = "accountWeight")]
    pub fn account_weight_ffi(&self, account: i64) -> Option<String> {
        blocking(self.account_weight(account))
    }

    #[uniffi::method(name = "setDisplayName")]
    pub fn set_display_name_ffi(&self, account: i64, name: String) -> Option<String> {
        blocking(self.set_display_name(account, name))
    }

    #[uniffi::method(name = "setAccountEnabled")]
    pub fn set_account_enabled_ffi(&self, account: i64, enabled: bool) -> Option<String> {
        blocking(self.set_account_enabled(account, enabled))
    }

    #[uniffi::method(name = "setDefaultAccount")]
    pub fn set_default_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.set_default_account(account))
    }

    /// Take an account away — its row, and its credentials.
    #[uniffi::method(name = "removeAccount")]
    pub fn remove_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.remove_account(account))
    }

    /// What one key press means here: the frontend owns no keymap (ADR 0019 Q4). Only `Unhandled`
    /// may reach the toolkit.
    #[uniffi::method(name = "key")]
    pub fn key_ffi(
        &self,
        character: Option<String>,
        name: Option<String>,
        modifiers: crate::ModifiersFfi,
        context: crate::UiContext,
        in_text_entry: bool,
    ) -> crate::KeyOutcomeFfi {
        self.key(
            character.as_deref(),
            name.as_deref(),
            modifiers,
            context,
            in_text_entry,
        )
    }

    /// Run a command, aimed the way this view says. `id` is the registry's name for the verb;
    /// nothing comes back, and
    /// what happened arrives on `nextEvent`.
    #[uniffi::method(name = "invoke")]
    pub fn invoke_ffi(&self, id: String) {
        self.invoke(&id);
    }

    #[uniffi::method(name = "isAvailable")]
    pub fn is_available_ffi(&self, id: String, context: crate::UiContext) -> bool {
        self.is_available(&id, context)
    }

    #[uniffi::method(name = "paletteEntries")]
    pub fn palette_entries_ffi(
        &self,
        query: String,
        context: crate::UiContext,
    ) -> Vec<crate::PaletteEntryFfi> {
        self.palette_entries(&query, context)
    }

    /// Every command reachable here, with the binding in force: the palette's list, unfiltered.
    #[uniffi::method(name = "cheatSheet")]
    pub fn cheat_sheet_ffi(&self, context: crate::UiContext) -> Vec<crate::PaletteEntryFfi> {
        self.cheat_sheet(context)
    }

    #[uniffi::method(name = "cheatSheetSections")]
    pub fn cheat_sheet_sections_ffi(
        &self,
        context: crate::UiContext,
    ) -> Vec<crate::CheatSectionFfi> {
        self.cheat_sheet_sections(context)
    }

    /// The cursor rested on `message` long enough to count as read.
    /// See [`Session::mark_read_on_dwell`].
    #[uniffi::method(name = "markReadOnDwell")]
    pub fn mark_read_on_dwell_ffi(&self, message: i64) {
        self.mark_read_on_dwell(message);
    }

    /// `#` in the search box: the folders matching `query`, best first.
    #[uniffi::method(name = "finderFolders")]
    pub fn finder_folders_ffi(&self, query: String) -> crate::FinderAnswerFfi {
        blocking(self.finder_folders(query))
    }

    /// `@` in the search box: the correspondents matching `query`.
    #[uniffi::method(name = "finderContacts")]
    pub fn finder_contacts_ffi(&self, query: String) -> crate::FinderAnswerFfi {
        blocking(self.finder_contacts(query))
    }

    /// `+` in the search box: the labels matching `query`.
    #[uniffi::method(name = "finderLabels")]
    pub fn finder_labels_ffi(&self, query: String) -> crate::FinderAnswerFfi {
        blocking(self.finder_labels(query))
    }

    #[uniffi::method(name = "applyLabel")]
    pub fn apply_label_ffi(&self, label: i64) {
        self.apply_label(label);
    }

    /// Report which accounts the aggregate view can currently vouch for; call it whenever a
    /// connection changes. Read when
    /// the selection is made, not when a verb runs (#811).
    #[uniffi::method(name = "setReachableAccounts")]
    pub fn set_reachable_accounts_ffi(&self, accounts: Vec<i64>) {
        self.set_reachable_accounts(&accounts);
    }

    /// The whole document for a message, ready for a `WKWebView`, plus the notice and caveat the
    /// render paid for (#1589).
    /// Swift builds a hardened configuration and composes no reader HTML.
    #[uniffi::method(name = "readerDocument")]
    pub fn reader_document_ffi(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        reduced: bool,
    ) -> crate::ReaderDocumentFfi {
        blocking(self.reader_answers(message, remote, reduced))
    }

    /// One inline part of `message`, by its `Content-ID`, for the `postio-cid:` handler. `nil` is a
    /// broken image, never a fetch.
    #[uniffi::method(name = "resolveCid")]
    pub fn resolve_cid_ffi(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        blocking(self.resolve_cid(message, content_id))
    }

    /// One part's bytes, fetched first if not on this machine. A deliberate act and the only call
    /// here that may reach the
    /// network: never use it for a preview or a size. `partId` is the MIME path from
    /// `PartFfi.partId`; an error carries the
    /// sentence to show, never an empty success. Never from the main actor: a part nobody has
    /// downloaded is waited on for up to thirty seconds.
    #[uniffi::method(name = "partBytes")]
    pub fn part_bytes_ffi(
        &self,
        message: i64,
        part_id: String,
    ) -> Result<Vec<u8>, crate::PartsError> {
        blocking(self.part_bytes(message, part_id))
    }

    /// What `A` offers for `message`: the links in `document` (as `focusReaderDocument` made it),
    /// then the attachments, or
    /// the shared sentence when there is neither. Reads the stored message; nothing is fetched.
    #[uniffi::method(name = "openChoices")]
    pub fn open_choices(&self, message: i64, document: String) -> crate::OpenChoicesFfi {
        let found = self.store_and_blobs().and_then(|(database, _)| {
            blocking(postio_host::parts::read_message(&database, message.into())).ok()
        });
        let parts = found
            .map(|found| {
                crate::focus_message::chips(
                    found.content_type.as_deref(),
                    &found.body,
                    &found.attachments,
                )
            })
            .unwrap_or_default();
        let links: Vec<crate::OpenLinkFfi> = postio_body::links::links(&document)
            .into_iter()
            .map(|link| crate::OpenLinkFfi {
                words: link.words,
                target: link.target,
            })
            .collect();
        crate::OpenChoicesFfi {
            nothing: (parts.is_empty() && links.is_empty())
                .then(|| postio_ui::focus_target::NOTHING_TO_OPEN.to_owned()),
            links,
            parts,
            save: postio_ui::reader::parts::SAVE.to_owned(),
            save_all: postio_ui::reader::parts::SAVE_ALL.to_owned(),
        }
    }

    /// Write one attachment to a private file under its own name, fetching it first if needed, and
    /// answer the path.
    /// Never from the main actor (see `partBytes`). Logs carry ids, never the name.
    #[uniffi::method(name = "openAttachment")]
    pub fn open_attachment_ffi(
        &self,
        message: i64,
        attachment: i64,
    ) -> Result<String, crate::PartsError> {
        blocking(self.open_attachment(message, attachment))
    }

    /// Write one attachment to `to`, the file a person named in a save panel, fetching it first if
    /// needed; answers what to
    /// say. Replaces whatever `to` held. Never from the main actor.
    #[uniffi::method(name = "saveAttachment")]
    pub fn save_attachment_ffi(
        &self,
        message: i64,
        attachment: i64,
        to: String,
    ) -> Result<String, crate::PartsError> {
        blocking(self.save_attachment(message, attachment, to))
    }

    /// Write every part `A` offers into `folder`, each under a unique name
    /// (`postio_ui::reader::parts::save_names`); a part
    /// that cannot be had does not stop the rest, and is counted. Never from the main actor.
    #[uniffi::method(name = "saveAttachments")]
    pub fn save_attachments_ffi(
        &self,
        message: i64,
        folder: String,
    ) -> Result<String, crate::PartsError> {
        blocking(self.save_attachments(message, folder))
    }
}

impl Session {
    pub async fn open_attachment(
        &self,
        message: i64,
        attachment: i64,
    ) -> Result<String, crate::PartsError> {
        use postio_model::ids::AttachmentId;
        let (message, attachment) = (message.into(), AttachmentId::new(attachment));
        let (name, bytes) = self.attachment_bytes(message, attachment).await?;
        let to = postio_host::parts::private_copy_path(
            &std::env::temp_dir(),
            message,
            attachment,
            name.as_deref(),
        )?;
        std::fs::write(&to, bytes).map_err(|error| error.to_string())?;
        tracing::debug!(
            message = message.get(),
            attachment = attachment.get(),
            "an attachment was written to open"
        );
        Ok(to.to_string_lossy().into_owned())
    }

    pub async fn save_attachment(
        &self,
        message: i64,
        attachment: i64,
        to: String,
    ) -> Result<String, crate::PartsError> {
        use postio_model::ids::AttachmentId;
        let (message, attachment) = (message.into(), AttachmentId::new(attachment));
        // The bytes first, so a part that cannot be had leaves no empty file.
        let (_, bytes) = self.attachment_bytes(message, attachment).await?;
        let to = std::path::PathBuf::from(to);
        std::fs::write(&to, bytes).map_err(|error| error.to_string())?;
        tracing::debug!(
            message = message.get(),
            attachment = attachment.get(),
            "an attachment was saved"
        );
        let name = to
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(postio_ui::reader::parts::saved(&name))
    }

    pub async fn save_attachments(
        &self,
        message: i64,
        folder: String,
    ) -> Result<String, crate::PartsError> {
        let (database, _) = self.store_and_blobs().ok_or_else(no_store)?;
        let message: postio_model::MessageId = message.into();
        let found = postio_host::parts::read_message(&database, message).await?;
        let nodes = crate::focus_message::chip_nodes(
            found.content_type.as_deref(),
            &found.body,
            &found.attachments,
        );
        let names = postio_ui::reader::parts::save_names(&nodes);
        let folder = std::path::PathBuf::from(folder);
        let mut failed = 0;
        for (node, name) in nodes.iter().zip(names) {
            let Some(attachment) = node.attachment else {
                continue;
            };
            let saved = match self.attachment_bytes(message, attachment).await {
                Ok((_, bytes)) => std::fs::write(folder.join(name), bytes).is_ok(),
                Err(_) => false,
            };
            if !saved {
                failed += 1;
            }
        }
        tracing::debug!(
            message = message.get(),
            parts = nodes.len(),
            failed,
            "attachments were saved"
        );
        Ok(postio_ui::reader::parts::saved_all(nodes.len(), failed))
    }

    async fn attachment_bytes(
        &self,
        message: postio_model::MessageId,
        attachment: postio_model::ids::AttachmentId,
    ) -> Result<(Option<String>, Vec<u8>), crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        let engine = self.engine_for_message(message).await;
        let found = postio_host::parts::read_message(&database, message).await?;
        let name = found
            .attachments
            .iter()
            .find(|part| part.id == attachment)
            .and_then(|part| part.filename.clone());
        let events = self
            .wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .map(|wiring| wiring.events.clone());
        let bytes = postio_host::parts::part_bytes(
            &database,
            &blobs,
            engine,
            events.as_ref(),
            message,
            attachment,
        )
        .await?;
        Ok((name, bytes))
    }
}

#[uniffi::export]
impl Session {
    /// Tell the engine whether the machine has a connection, pushed down from Swift's
    /// `NWPathMonitor`.
    #[uniffi::method(name = "setOffline")]
    pub fn set_offline_ffi(&self, offline: bool) {
        self.set_offline(offline);
    }

    #[uniffi::method(name = "isOffline")]
    pub fn is_offline_ffi(&self) -> bool {
        self.is_offline()
    }

    /// Start syncing every configured account; answers how many started. Zero is not an error: no
    /// account is the first-run state. Does not block.
    #[uniffi::method(name = "startSyncing")]
    pub fn start_syncing_ffi(&self) -> Result<u32, SessionError> {
        blocking(self.start_syncing())
    }

    #[uniffi::method(name = "configuredAccounts")]
    pub fn configured_accounts_ffi(&self) -> u32 {
        blocking(self.configured_accounts())
    }

    #[uniffi::method(name = "mailboxes")]
    pub fn mailboxes_ffi(&self) -> Vec<crate::MailboxFfi> {
        blocking(self.mailboxes())
    }

    /// Every configured account, in the order the pane lists them. Not from the main actor once an
    /// OAuth account exists:
    /// each costs a keyring read, which can block on a Keychain prompt.
    #[uniffi::method(name = "accounts")]
    pub fn accounts_ffi(&self) -> Vec<crate::AccountFfi> {
        blocking(self.accounts())
    }

    /// Give an account a new password (the repair `AccountFfi.repair` calls `Password`): `None`
    /// when stored, a sentence
    /// when not. Writes the keyring and nothing else. Blocks on the keyring; off the main actor.
    #[uniffi::method(name = "repairCredential")]
    pub fn repair_credential_ffi(&self, account: i64, password: String) -> Option<String> {
        blocking(self.repair_credential(account, password))
    }

    /// Sign an account in again through the system browser (the repair `Browser`): `None` when the
    /// grant was renewed, a
    /// sentence when not. Takes no client id: the account carries the one it registered. Returns
    /// when a person comes back
    /// from the browser: off the main actor.
    #[uniffi::method(name = "reconnectAccount")]
    pub fn reconnect_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.reconnect_account(account))
    }

    #[uniffi::method(name = "bindingFor")]
    pub fn binding_for_ffi(&self, command: String) -> Option<String> {
        self.binding_for(command)
    }

    #[uniffi::method(name = "bindingsFor")]
    pub fn bindings_for_ffi(&self, command: String) -> Vec<String> {
        self.bindings_for(command)
    }

    #[uniffi::method(name = "commands")]
    pub fn commands_ffi(&self) -> Vec<crate::CommandSpecFfi> {
        self.commands()
    }

    #[uniffi::method(name = "isOpen")]
    pub fn is_open_ffi(&self) -> bool {
        self.is_open()
    }
    /// The `[ui]` table this session was opened with, so the list and the settings pane cannot
    /// disagree.
    pub fn appearance(&self) -> crate::AppearanceFfi {
        crate::AppearanceFfi {
            density: self.ui.density.into(),
            theme: self.ui.theme.into(),
            show_hover_actions: self.ui.show_hover_actions,
            sender_avatars: self.ui.sender_avatars,
        }
    }
    /// The key hints the search bar announces (`Ret open · Tab refine · C-s save as folder`), read
    /// from this session's
    /// keymap so a rebinding reaches the footer.
    pub fn search_hints(&self) -> Vec<crate::KeyHintFfi> {
        postio_ui::search::hints(&self.keymap())
            .into_iter()
            .map(|(key, label)| crate::KeyHintFfi {
                key,
                label: label.to_string(),
            })
            .collect()
    }
}

/// What resuming an account's browser sign-in needs, resolved off its row. Does not cross to Swift:
/// it carries a client
/// secret. Public so a test can assert the right client reaches the provider (#1584).
#[derive(Debug)]
pub struct BrowserReconnect {
    pub address: String,
    pub client_id: String,
    pub client_secret: Option<postio_account::secret::Password>,
}

impl Session {
    /// Every configured account, in the order the pane lists them; disabled ones included. An empty
    /// answer means no store.
    /// Reaches the keyring only for an OAuth account, to learn whether its token expired: the
    /// expiry lives beside the token, not in `config.toml` (#870).
    pub async fn accounts(&self) -> Vec<crate::AccountFfi> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Vec::new();
        };
        let Ok(connection) = database.connect().await else {
            return Vec::new();
        };
        let Ok(accounts) = postio_storage::repository::AccountRepository::new(&connection)
            .list()
            .await
        else {
            return Vec::new();
        };
        drop(connection);

        let secrets = self.secret_store();
        let now = std::time::SystemTime::now();
        let mut rows = Vec::with_capacity(accounts.len());
        for account in &accounts {
            // `None` for everything that is not an OAuth account of Postio's own minting:
            // `TokenStanding::Unknown`, a real answer.
            let expiry = match (&secrets, account.oauth.is_some()) {
                (Some(secrets), true) => {
                    postio_account::oauth::token_source::stored_expiry(
                        secrets.as_ref(),
                        &postio_account::secret::AccountKey::new(account.address.address.clone()),
                    )
                    .await
                }
                _ => None,
            };
            rows.push(crate::AccountFfi::of(
                account,
                postio_ui::account::TokenStanding::of(expiry, now),
            ));
        }
        rows
    }

    /// Put a new password in the keyring for an account already here (`AccountFfi::repair`
    /// `Password`). `None` when stored,
    /// a sentence when not. The password goes to the keyring and nowhere else, never `config.toml`
    /// or a log (ADR 0014).
    pub async fn repair_credential(&self, account: i64, password: String) -> Option<String> {
        // Refused: an empty secret reads as a stored credential, which makes the *Partial* state
        // unreachable for that account.
        if password.is_empty() {
            return Some("Type the password this account signs in with.".to_owned());
        }
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let Some(secrets) = self.secret_store() else {
            return Some("There is no keyring to store the password in.".to_owned());
        };
        postio_session::provision::repair(
            &database,
            secrets.as_ref(),
            postio_model::ids::AccountId::new(account),
            postio_account::secret::Password::new(password),
        )
        .await
        .err()
        .map(|error| error.to_string())
    }

    /// What reconnecting `account` in a browser would sign in with, or why it cannot be: refuses
    /// exactly where the row
    /// offers nothing, so the button is never on a row it does not work on.
    pub async fn browser_sign_in_for(&self, account: i64) -> Result<BrowserReconnect, String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Err("There is no store open.".to_owned());
        };
        let connection = database
            .connect()
            .await
            .map_err(|error| format!("Postio could not open its local store: {error}"))?;
        let found = postio_storage::repository::AccountRepository::new(&connection)
            .get(postio_model::ids::AccountId::new(account))
            .await
            .map_err(|error| format!("Postio could not read its local store: {error}"))?;
        drop(connection);
        let Some(found) = found else {
            return Err("That account is not in the store.".to_owned());
        };
        let Some(oauth) = found
            .oauth
            .as_ref()
            .filter(|oauth| !oauth.client_id.trim().is_empty())
        else {
            return Err(format!(
                "{} does not sign in through your browser, so there is no \
                 sign-in to resume.",
                found.address.address
            ));
        };
        let key = postio_account::secret::AccountKey::new(found.address.address.clone());
        let client_secret = match self.secret_store() {
            Some(secrets) => {
                postio_account::oauth::token_source::stored_client_secret(secrets.as_ref(), &key)
                    .await
            }
            None => None,
        };
        Ok(BrowserReconnect {
            address: found.address.address.clone(),
            client_id: oauth.client_id.clone(),
            client_secret,
        })
    }

    /// Sign this account in again with the client it signed in with before:
    /// `sign_in_with_browser`'s flow, only the
    /// client's source differs. `provision_oauth` seeds the keyring before noticing the row exists,
    /// so a re-consent lands (#1584).
    pub async fn reconnect_account(&self, account: i64) -> Option<String> {
        let resumed = match self.browser_sign_in_for(account).await {
            Ok(resumed) => resumed,
            Err(complaint) => return Some(complaint),
        };
        self.sign_in_with_browser(
            resumed.address,
            resumed.client_id,
            resumed
                .client_secret
                .as_ref()
                .map(|secret| secret.expose().to_owned()),
        )
        .await
    }

    /// Opens a session, or says why it could not. Blocks: reading the store's key from the keyring
    /// can wait on a user
    /// prompt, bounded by its own timeout. A Swift caller must not invoke it on the main actor.
    pub fn open(options: SessionOptions) -> Result<Arc<Self>, SessionError> {
        let source = config_source(&options);
        let seams = seams(&options);
        let caller = options.bridge;

        #[cfg(feature = "testing")]
        if options.in_memory {
            let agents_at = options.agents_at.clone();
            let database = match options.seeded {
                Some(database) => database,
                None => {
                    // A fresh key per session from the OS RNG, still on the encrypted path (ADR
                    // 0014 Q3); no keyring is touched.
                    // A file in a temporary directory, not an in-memory database: the engine
                    // refuses to key one.
                    let key = postio_storage::key::StoreKey::generate()
                        .derive(postio_storage::key::Purpose::Database);
                    let scratch =
                        tempfile::tempdir().map_err(|error| SessionError::StoreUnavailable {
                            message: error.to_string(),
                        })?;
                    let store = blocking(postio_storage::Store::open(
                        scratch.path().join("postio.db"),
                        &key,
                    ))
                    .map_err(|error| SessionError::StoreUnavailable {
                        message: error.to_string(),
                    })?;
                    // The directory has to outlive every connection onto it.
                    std::mem::forget(scratch);
                    store
                }
            };
            let (blobs, scratch) = match options.seeded_blobs {
                Some((blobs, scratch)) => (blobs, scratch),
                None => {
                    let scratch =
                        tempfile::tempdir().map_err(|error| SessionError::StoreUnavailable {
                            message: error.to_string(),
                        })?;
                    let blobs = postio_storage::BlobStore::open(
                        scratch.path(),
                        &postio_storage::key::BlobKeys::derive(
                            &postio_storage::key::StoreKey::generate(),
                        ),
                    )
                    .map_err(|error| SessionError::StoreUnavailable {
                        message: error.to_string(),
                    })?;
                    (blobs, scratch)
                }
            };
            // The default secret store is left in place: it does not reach the keyring until asked
            // for a secret, so an
            // in-memory session needs no Secret Service, Keychain or prompt.
            let config = load_config(&source);
            let saved = postio_ui::saved_search::pinned(&config);
            let sync_config = config.sync;
            // Honour `with_secrets` here too: without it an in-memory session handed a test keyring
            // used the login keychain and hung on a macOS prompt.
            let secrets = options.secrets.clone();
            let host = serve(database, blobs, caller, |wiring| {
                let wiring = with_onboarding(wiring, seams)
                    .with_backfill(postio_session::backfill_policy(&sync_config))
                    .with_watch(postio_session::watch_policy(&sync_config));
                match secrets {
                    Some(secrets) => wiring.with_secrets(secrets),
                    None => wiring,
                }
            })?;
            let (wiring, events, link) = connect(&host);
            let local = async_channel::unbounded();
            let (focus_client, focus_runtime) = (link.client.clone(), wiring.runtime.clone());
            let keys = config.keys;
            postio_session::spawn_body_indexer(
                wiring.database.clone(),
                wiring.events.subscribe("indexer"),
                &wiring.runtime,
            );
            postio_session::spawn_attachment_indexer(
                wiring.database.clone(),
                wiring.blobs.clone(),
                wiring.events.subscribe("attachment-indexer"),
                &wiring.runtime,
            );
            // A file of its own per session: an in-memory store has no directory, and one shared
            // path made a grant from one test true for the next.
            let allow_list_at = std::env::temp_dir().join(format!(
                "postio-allowed-{}-{}.toml",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|since| since.as_nanos())
                    .unwrap_or_default()
            ));
            let session = Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
                keymap: Mutex::new(None),
                ui: config.ui,
                keys: Mutex::new(keys),
                reachable: Mutex::new(Vec::new()),
                finder_sources: Mutex::new(None),
                recipient_directories: crate::contacts::Directories::default(),
                conversation: Arc::default(),
                sign_in: Mutex::new(None),
                sign_in_port: Arc::default(),
                store_at: std::path::PathBuf::from(":memory:"),
                agents: match agents_at {
                    Some(socket) => answer_agents(socket, &host, &source),
                    None => Mutex::new(None),
                },
                // A file of its own per session: an in-memory store has no directory, and one
                // shared path made a grant from one test true for the next.
                allow_list_at: allow_list_at.clone(),
                serial: next_serial(),
                allow_list_kept: Mutex::new(None),
                thread_renders: Mutex::new(postio_ui::reader::document::RenderCache::default()),
                in_flight: Arc::default(),
                reconnects: Arc::default(),
                offline: Arc::default(),
                engines: Mutex::new(Vec::new()),
                local: local.clone(),
                events,
                link: Mutex::new(Some(link)),
                wired: host.wired(),
                _focus: engage_focus(&host, &config.focus, &source, &allow_list_at),
                focus_config: Mutex::new(config.focus.clone()),
                notices: Mutex::new(crate::NoticesFfi::from(&sync_config)),
                _host: host,
                config_watch: Mutex::new(None),
                focus_list: crate::focus_list::FocusDriver::new(
                    focus_client.clone(),
                    focus_runtime.clone(),
                    local.0.clone(),
                ),
                _scratch: Some(scratch),
            });
            session.follow_config(&source);
            session.follow_contacts();
            session.prime_focus(saved, &source);
            return Ok(session);
        }

        // The keyring first, then the store (ADR 0014): a keyring that will not answer means no
        // store, and nothing has
        // touched the database, so a locked keyring leaves no half-made store.
        let secrets: Arc<dyn postio_account::secret::SecretStore> = match options.secrets {
            Some(secrets) => secrets,
            None => postio_account::secret::platform_keyring(),
        };
        let key = postio_session::store_key_blocking(secrets.as_ref())
            .map_err(SessionError::from_secret_error)?;

        let path = options
            .store_path
            .unwrap_or_else(postio_session::paths::store_path);
        // Kept before `path` is moved: the composer's footer names where a draft lives, and grants
        // are written beside the store.
        let store_at = path.clone();
        // Blocked on here, before the host exists: this constructor blocks by contract, so not on
        // the main actor.
        let (database, blobs) =
            blocking(postio_session::open_store_at_reporting(path, &key, &|_| {}))
                .map_err(SessionError::from_refusal)?;

        let config = load_config(&source);
        let saved = postio_ui::saved_search::pinned(&config);
        let keys = config.keys;
        let sync_config = config.sync;
        let ui_config = config.ui;
        let focus_config = config.focus;

        let host = serve(database, blobs, caller, |wiring| {
            with_onboarding(wiring, seams)
                .with_secrets(secrets)
                .with_backfill(postio_session::backfill_policy(&sync_config))
                .with_watch(postio_session::watch_policy(&sync_config))
        })?;
        let (wiring, events, link) = connect(&host);
        let local = async_channel::unbounded();
        let (focus_client, focus_runtime) = (link.client.clone(), wiring.runtime.clone());
        postio_session::spawn_body_indexer(
            wiring.database.clone(),
            wiring.events.subscribe("indexer"),
            &wiring.runtime,
        );
        postio_session::spawn_attachment_indexer(
            wiring.database.clone(),
            wiring.blobs.clone(),
            wiring.events.subscribe("attachment-indexer"),
            &wiring.runtime,
        );
        let allow_list_at = store_at
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(&store_at)
            .join("allowed-senders.toml");
        let session = Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            keymap: Mutex::new(None),
            ui: ui_config,
            keys: Mutex::new(keys),
            engines: Mutex::new(Vec::new()),
            reachable: Mutex::new(Vec::new()),
            finder_sources: Mutex::new(None),
            recipient_directories: crate::contacts::Directories::default(),
            conversation: Arc::default(),
            sign_in: Mutex::new(None),
            sign_in_port: Arc::default(),
            allow_list_at: allow_list_at.clone(),
            agents: answer_agents(postio_mcp::socket::path_for(&store_at), &host, &source),
            store_at,
            serial: next_serial(),
            allow_list_kept: Mutex::new(None),
            thread_renders: Mutex::new(postio_ui::reader::document::RenderCache::default()),
            in_flight: Arc::default(),
            reconnects: Arc::default(),
            offline: Arc::default(),
            local: local.clone(),
            events,
            link: Mutex::new(Some(link)),
            wired: host.wired(),
            _focus: engage_focus(&host, &focus_config, &source, &allow_list_at),
            focus_config: Mutex::new(focus_config.clone()),
            notices: Mutex::new(crate::NoticesFfi::from(&sync_config)),
            _host: host,
            #[cfg(feature = "testing")]
            _scratch: None,
            config_watch: Mutex::new(None),
            focus_list: crate::focus_list::FocusDriver::new(
                focus_client.clone(),
                focus_runtime.clone(),
                local.0.clone(),
            ),
        });
        session.follow_config(&source);
        session.follow_contacts();
        session.prime_focus(saved, &source);
        Ok(session)
    }

    pub fn open_conversation(&self, thread: i64) {
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let held = self.conversation.clone();
        let local = self.local.0.clone();
        let in_flight = self.in_flight.clone();
        let ordering = std::sync::atomic::Ordering::SeqCst;

        in_flight.fetch_add(1, ordering);
        runtime.spawn(async move {
            let request = postio_runtime::store::PageRequest {
                scope: postio_runtime::store::ListScope::Thread(thread.into()),
                offset: 0,
                limit: THREAD_LIMIT,
            };
            // An unreadable thread folds to an empty conversation: a pane still drawing the
            // previous one under a new selection looks like an answer.
            let asked = std::time::Instant::now();
            let rows = match store.list_page(request).await {
                Ok(page) => crate::list::page_of(page).rows,
                Err(error) => {
                    tracing::debug!(%error, thread, "the conversation could not be read");
                    Vec::new()
                }
            };
            tracing::debug!(
                thread,
                rows = rows.len(),
                elapsed_ms = asked.elapsed().as_millis() as u64,
                "conversation read"
            );
            let folded = crate::conversation::fold(thread, rows, chrono::Local::now());
            *held.lock().expect("conversation lock") = Some(folded);
            let _ = local.try_send(UiEvent::ConversationReady { thread });
            in_flight.fetch_sub(1, ordering);
        });
    }

    /// `thread` drawn as one document (ADR 0032, #1595), gathered like GTK's pane: messages in the
    /// stacked pane's order, each
    /// body loaded or said to be coming. Every message opens, except one whose body has not arrived
    /// unless it is the newest,
    /// so the "still coming" plate appears once. Reads the store only; a missing body is not
    /// fetched.
    pub async fn thread_document(
        &self,
        thread: i64,
        reduced: Vec<i64>,
    ) -> crate::ThreadDocumentFfi {
        let empty = crate::ThreadDocumentFfi::default();
        let Some((store, _runtime)) = self.reader() else {
            return empty;
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return empty;
        };
        let request = postio_runtime::store::PageRequest {
            scope: postio_runtime::store::ListScope::Thread(thread.into()),
            offset: 0,
            limit: THREAD_LIMIT,
        };
        let Ok(page) = store.list_page(request).await else {
            return empty;
        };
        let now = chrono::Local::now();
        let rows = crate::conversation::fold(thread, crate::list::page_of(page).rows, now).rows;
        if rows.is_empty() {
            return empty;
        }
        let Ok(connection) = database.connect().await else {
            return empty;
        };
        let repository = postio_storage::repository::MessageRepository::new(&connection);
        let accounts = postio_storage::repository::AccountRepository::new(&connection);
        let offline = self.offline.load(std::sync::atomic::Ordering::SeqCst);
        let mut own: std::collections::HashMap<postio_model::ids::AccountId, Vec<String>> =
            std::collections::HashMap::new();
        let newest = rows.last().map(|row| row.id);
        let many = rows.len() > 1;

        let mut messages = Vec::with_capacity(rows.len());
        let mut caveats: Vec<Option<String>> = Vec::with_capacity(rows.len());
        for row in &rows {
            let id = postio_model::ids::MessageId::new(row.id);
            let Ok(Some(stored)) = repository.get(id).await else {
                continue;
            };
            if let std::collections::hash_map::Entry::Vacant(slot) = own.entry(stored.account_id) {
                let addresses = match accounts.get(stored.account_id).await {
                    Ok(Some(account)) => std::iter::once(account.address.address.to_lowercase())
                        .chain(
                            account
                                .identities
                                .iter()
                                .map(|identity| identity.address.address.to_lowercase()),
                        )
                        .collect(),
                    _ => Vec::new(),
                };
                slot.insert(addresses);
            }
            let from = stored.from.first();
            let address = from.map(|from| from.address.clone()).unwrap_or_default();
            let mine = own
                .get(&stored.account_id)
                .is_some_and(|own| own.contains(&address.to_lowercase()));
            let (body, broken) = match postio_session::reading::load_body_or_reason(
                &connection,
                id,
                offline,
            )
            .await
            {
                postio_session::reading::Body::Ready {
                    body,
                    encoding_problems,
                    ..
                } => (Some(body), encoding_problems),
                _ => (None, false),
            };
            caveats.push(postio_ui::reader::document::decode_caveat(broken).map(str::to_owned));
            let absent = body.is_none();
            let latest = newest == Some(row.id);
            messages.push(postio_ui::reader::thread::ThreadMessage {
                scope: row.id.to_string(),
                sender: from.and_then(|from| from.name.clone()).unwrap_or_else(|| {
                    if address.is_empty() {
                        "Unknown sender".to_owned()
                    } else {
                        address.clone()
                    }
                }),
                address,
                when: postio_ui::row::timestamp(stored.received_at, now),
                recipients: postio_ui::reader::header::recipient_line(&stored.to),
                cc: postio_ui::reader::header::recipient_line(&stored.cc),
                preview: stored.preview.clone().unwrap_or_default(),
                expanded: !absent || latest,
                absent,
                latest: latest && many,
                draft: row.send_state.is_some(),
                mine,
                body: body.unwrap_or_default(),
            });
        }

        let chosen: std::collections::HashMap<String, postio_ui::reader::document::Rendering> =
            reduced
                .iter()
                .map(|id| {
                    (
                        id.to_string(),
                        postio_ui::reader::document::Rendering::Reader,
                    )
                })
                .collect();
        let allow = self.allow_list();
        let html = postio_ui::reader::thread::compose(
            &messages,
            |address| allow.is_allowed(address),
            &chosen,
            &mut self.thread_renders.lock().expect("thread renders lock"),
        );
        crate::ThreadDocumentFfi {
            html,
            messages: messages
                .iter()
                .zip(caveats)
                .map(|(message, caveat)| crate::ThreadAnchorFfi {
                    message: message.scope.parse().unwrap_or_default(),
                    anchor: postio_ui::reader::thread::message_anchor(&message.scope),
                    address: message.address.clone(),
                    caveat,
                })
                .collect(),
        }
    }

    pub fn conversation(&self) -> Option<crate::ConversationFfi> {
        self.conversation.lock().expect("conversation lock").clone()
    }

    pub async fn agent_activity(&self) -> crate::AgentActivityFfi {
        let log = postio_session::agent_access::log_path(&self.store_at)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        self.agent_activity_from(&log, chrono::Utc::now()).await
    }

    /// The activity `log` records as of `now`. A row that has since been
    /// deleted is left unnamed rather than failing the page.
    pub async fn agent_activity_from(
        &self,
        log: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> crate::AgentActivityFfi {
        let database = self.store_and_blobs().map(|(database, _)| database);
        let shown = postio_session::agent_access::activity(database.as_ref(), log, now).await;
        crate::agent_access::from_activity(shown)
    }

    /// The row's own facts about one open message: the unsubscribe offer and the recipients. No
    /// body load; the notice and
    /// caveat ride the document render (#1589).
    pub async fn message_facts(&self, message: i64) -> crate::MessageFactsFfi {
        let nothing = crate::MessageFactsFfi {
            offer: None,
            recipients: None,
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return nothing;
        };
        let Ok(connection) = database.connect().await else {
            return nothing;
        };
        let repository = postio_storage::repository::MessageRepository::new(&connection);
        let id = postio_model::ids::MessageId::new(message);
        let Ok(Some(row)) = repository.get(id).await else {
            return nothing;
        };

        let lines = postio_ui::reader::header::MessageHeader::of(
            &row.from,
            &row.to,
            &row.cc,
            row.subject.as_deref(),
            row.date.unwrap_or(row.received_at),
            chrono::Local::now(),
        );
        let recipients = Some(crate::RecipientsFfi {
            to: lines.to_line(),
            cc_label: lines.cc_toggle_label(),
            cc: lines.cc,
        });

        // The send state is the offer's gate (#1525): a failed read means "do not offer", else the
        // domain fallback would
        // offer to unsubscribe the user from their own account.
        let offer = match repository.send_state(id).await {
            Ok(send_state) => {
                postio_ui::unsubscribe::offer(send_state, row.list_id.as_deref(), &row.from).map(
                    |offer| crate::UnsubscribeOfferFfi {
                        list_identifier: offer.list_identifier,
                        summary: offer.summary,
                        action: postio_ui::unsubscribe::ACTION.to_owned(),
                    },
                )
            }
            Err(error) => {
                tracing::warn!(message, %error, "cannot read a message's send state");
                None
            }
        };

        crate::MessageFactsFfi { offer, recipients }
    }

    /// What the reader is holding back for `message`. Rendered with images blocked whatever the
    /// sender's standing: an
    /// allowed render would answer "nothing" and take away the notice where a grant is revoked.
    pub async fn reader_notice(&self, message: i64) -> Option<crate::ReaderNoticeFfi> {
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, false)
            .await
            .notice
    }

    pub fn allow_sender(&self, address: String) {
        self.amend_allow_list(|list| list.allow(&address));
    }

    /// Grant `message`'s sender a standing exception if the reader is asking for one, and answer
    /// whether it was. Only where
    /// the notice's "Always allow" would be shown: a grant written where no notice asked is consent
    /// nobody was asked for.
    pub async fn always_show_images_for(&self, message: i64) -> bool {
        match self.reader_notice(message).await {
            Some(notice) if !notice.allowed && !notice.sender.is_empty() => {
                self.allow_sender(notice.sender);
                true
            }
            _ => false,
        }
    }

    pub fn allow_domain(&self, domain: String) {
        self.amend_allow_list(|list| list.allow_domain(&domain));
    }

    /// The standing grants, from memory: the file is read once per session and every write goes
    /// through `amend_remote_images` (#1589).
    pub(crate) fn allow_list(&self) -> postio_ui::allowlist::AllowList {
        self.with_remote_images(|list| list.grants().clone())
    }

    pub(crate) fn with_remote_images<T>(
        &self,
        read: impl FnOnce(&postio_ui::allowlist::RemoteImageAllowList) -> T,
    ) -> T {
        let mut kept = self.allow_list_kept.lock().expect("allow list lock");
        read(kept.get_or_insert_with(|| {
            postio_ui::allowlist::RemoteImageAllowList::load_from(&self.allow_list_path())
        }))
    }

    /// Change the grants or treatments, write them, and keep what was written. Best-effort: a
    /// change that could not be
    /// written is not kept, so the next read goes back to the file.
    pub(crate) fn amend_remote_images(
        &self,
        change: impl FnOnce(&mut postio_ui::allowlist::RemoteImageAllowList),
    ) -> std::io::Result<()> {
        let path = self.allow_list_path();
        let mut kept = self.allow_list_kept.lock().expect("allow list lock");
        let mut list = kept
            .take()
            .unwrap_or_else(|| postio_ui::allowlist::RemoteImageAllowList::load_from(&path));
        change(&mut list);
        list.save_to(&path)?;
        *kept = Some(list);
        Ok(())
    }

    pub fn remote_image_grants(&self) -> Vec<crate::GrantFfi> {
        let list = self.allow_list();
        let addresses = list.addresses().map(|subject| crate::GrantFfi {
            subject: subject.to_owned(),
            whole_domain: false,
        });
        let domains = list.domains().map(|subject| crate::GrantFfi {
            subject: subject.to_owned(),
            whole_domain: true,
        });
        addresses.chain(domains).collect()
    }

    /// Take one back. `revoke` removes whichever list holds it, so the pane need not say which.
    pub fn revoke_remote_images(&self, subject: String) {
        self.amend_allow_list(|list| list.revoke(&subject));
    }

    /// Change the grants alone, through the shell: `AllowList` alone would drop the senders'
    /// treatments.
    fn amend_allow_list(&self, change: impl FnOnce(&mut postio_ui::allowlist::AllowList)) {
        if let Err(error) = self.amend_remote_images(|list| change(list.grants_mut())) {
            tracing::error!(%error, "the remote-image allow list could not be saved: {error}");
        }
    }

    /// Where the grants live: beside the store, since it is state the application writes, not
    /// configuration a person edits.
    pub(crate) fn allow_list_path(&self) -> std::path::PathBuf {
        self.allow_list_at.clone()
    }

    /// What the reader says above a body that did not fully decode, or `None`. A read of its own:
    /// native chrome above the web view (#901, #1585).
    pub async fn decode_caveat(&self, message: i64) -> Option<String> {
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, false)
            .await
            .caveat
    }

    /// What `message` offers about the list it came from, or `None`. A read only: no connection, no
    /// write, no way to act,
    /// so drawing a message cannot reach `activate_unsubscribe`.
    pub async fn unsubscribe_offer(&self, message: i64) -> Option<crate::UnsubscribeOfferFfi> {
        self.message_facts(message).await.offer
    }

    /// Record that the user asked to leave this message's list: `None` when recorded, a sentence
    /// when not. Appends to the
    /// activation log and stops; sending the RFC 8058 request is #972. The caller passes only a
    /// message id and the offer is
    /// re-derived, so nothing a frontend supplies decides what is recorded (#1525).
    pub async fn activate_unsubscribe(&self, message: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no open store to record the activation in.".to_owned());
        };
        // An interactive write, the priority the composer's saves take: somebody is waiting on the
        // button.
        let (connection, _permit) = match database.interactive_write().await {
            Ok(held) => held,
            Err(error) => return Some(format!("The store would not take a write: {error}")),
        };
        let Some((account, offer)) = unsubscribe_offer_for(&connection, message.into()).await
        else {
            return Some("This message offers no list to leave.".to_owned());
        };

        let mut activation = postio_model::UnsubscribeActivation::new(
            account,
            offer.list_identifier,
            chrono::Utc::now(),
        );
        postio_storage::repository::UnsubscribeRepository::new(&connection)
            .record(&mut activation)
            .await
            .err()
            .map(|error| format!("The activation could not be recorded: {error}"))
    }

    /// What the list `message` came from has left, as the offer after an
    /// unsubscribe words it (#1931). `None` when there is nothing to offer.
    pub async fn list_mail_offer(&self, message: i64) -> Option<crate::ListMailOfferFfi> {
        let (database, _) = self.store_and_blobs()?;
        let connection = database.connect().await.ok()?;
        let found = postio_session::list_mail::find(&connection, message.into())
            .await
            .ok()?;
        use postio_ui::unsubscribe::{SweepScope, sweep_notice};
        let action = self
            .focus_config
            .lock()
            .expect("focus config lock")
            .after_unsubscribe;
        let count = found.ids.len();
        let notice = sweep_notice(&SweepScope::ListId(found.name), count, found.more, action)?;
        Some(crate::ListMailOfferFfi {
            summary: notice.text,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            button: notice.button,
            delete: notice.action == postio_config::AfterUnsubscribe::Delete,
        })
    }

    pub async fn clear_list_mail(&self, message: i64, delete: bool) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no open store.".to_owned());
        };
        let Some(outbox) = self.outbox() else {
            return Some("Postio is not running its mail host.".to_owned());
        };
        let connection = match database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Some(format!("The store would not answer: {error}")),
        };
        let found = match postio_session::list_mail::find(&connection, message.into()).await {
            Ok(found) => found,
            Err(refusal) => return Some(refusal.said()),
        };
        if found.ids.is_empty() {
            return None;
        }
        let target = postio_core::MessageTarget::Messages(found.ids);
        let command = if delete {
            postio_core::Command::Delete { target }
        } else {
            postio_core::Command::Archive { target }
        };
        match outbox
            .send((command, postio_core::state::SharedState::default()))
            .await
        {
            Ok(()) => None,
            Err(_) => Some("Postio has stopped taking commands.".to_owned()),
        }
    }

    /// Every activation this store holds, newest first, across every account like the remote-image
    /// grants beside it.
    pub async fn unsubscribe_activations(&self) -> Vec<crate::UnsubscribeActivationFfi> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Vec::new();
        };
        let Ok(connection) = database.connect().await else {
            return Vec::new();
        };
        let accounts = postio_storage::repository::AccountRepository::new(&connection)
            .list()
            .await
            .unwrap_or_default();
        let log = postio_storage::repository::UnsubscribeRepository::new(&connection);
        let mut activations = Vec::new();
        for account in &accounts {
            match log.for_account(account.id).await {
                Ok(rows) => activations.extend(rows),
                Err(error) => {
                    tracing::warn!(%error, "could not read the unsubscribe-activation log")
                }
            }
        }
        // `postio_ui::unsubscribe::newest_first` is the one merge rule, tie-break included.
        postio_ui::unsubscribe::newest_first(&mut activations);
        activations
            .into_iter()
            .map(|activation| crate::UnsubscribeActivationFfi {
                when: postio_ui::unsubscribe::activated_on(activation.activated_at),
                label: postio_ui::unsubscribe::activation_label(
                    &activation.list_identifier,
                    activation.activated_at,
                ),
                list_identifier: activation.list_identifier,
            })
            .collect()
    }

    pub async fn discover_account(&self, address: String) -> crate::DiscoveredFfi {
        let client = self
            .link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.client.clone());
        let status = match client {
            Some(client) => client
                .discover(address.trim().to_owned())
                .await
                .unwrap_or_else(|error| {
                    tracing::info!(%error, "the host could not look the address up");
                    postio_ui::onboarding::Status::Manual { suggestion: None }
                }),
            None => postio_ui::onboarding::Status::Manual { suggestion: None },
        };
        crate::provisioning::discovered(&address, status)
    }

    /// The desktop's order (`postio_session::onboarding`): prove the login against the real server,
    /// then the credential to
    /// the keyring, then the row, so a wrong password writes nothing. Not the host's `AddAccount`,
    /// which would start a second engine on one mailbox.
    pub async fn connect_account(&self, account: crate::NewAccountFfi) -> Option<String> {
        if !postio_ui::onboarding::looks_like_an_address(&account.address) {
            return Some(format!(
                "{} does not look like an email address.",
                account.address.trim()
            ));
        }
        if account.imap.host.trim().is_empty() || account.smtp.host.trim().is_empty() {
            return Some(NO_SERVERS.to_owned());
        }
        let wiring = {
            let guard = self.wiring.lock().expect("wiring lock");
            guard.as_ref()?.clone()
        };
        let submission = crate::provisioning::submission(account);
        let proven = match &wiring.mail {
            Some(mail) => postio_account::backend::MailBackend::connect(mail.backend.as_ref())
                .await
                .map(|_| postio_model::account::Backend::Imap)
                .map_err(|error| postio_session::onboarding::explain(&error)),
            None => postio_session::onboarding::prove(&submission, None).await,
        };
        let backend = match proven {
            Ok(backend) => backend,
            Err(reason) => return Some(reason),
        };
        postio_session::onboarding::persist(
            &wiring.database,
            wiring.secrets.as_ref(),
            &submission,
            backend,
        )
        .await
        .err()
    }

    /// One write and no keyring: a maildir account signs in to nothing.
    pub async fn add_local_account(&self, address: String, path: String) -> Option<String> {
        if !address.contains('@') {
            return Some(format!("{address} does not look like an email address."));
        }
        if let Some(complaint) = self.inspect_local_store(path.clone()) {
            return Some(complaint);
        }
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to add an account to.".to_owned());
        };
        match postio_session::provision::provision_local(&database, &address, &path).await {
            Ok(_) => None,
            Err(error) => Some(error.to_string()),
        }
    }

    pub fn inspect_local_store(&self, path: String) -> Option<String> {
        let root = postio_session::provision::absolute(&path);
        postio_account::maildir::LocalStore::looks_like_a_maildir(&root).err()
    }

    fn secret_store(&self) -> Option<Arc<dyn postio_account::secret::SecretStore>> {
        let guard = self.wiring.lock().expect("wiring lock");
        Some(guard.as_ref()?.secrets.clone())
    }

    pub async fn test_connection(&self, account: i64) -> crate::ConnectionReportFfi {
        let refusal = |message: &str| crate::ConnectionReportFfi {
            reachable: false,
            missing_credential: false,
            rejected_credential: false,
            message: message.to_owned(),
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return refusal("There is no store open.");
        };
        let Some(secrets) = self.secret_store() else {
            return refusal("There is no keyring to read the credential from.");
        };
        let Ok(connection) = database.connect().await else {
            return refusal("Postio could not open its local store.");
        };
        let found = postio_storage::repository::AccountRepository::new(&connection)
            .get(postio_model::ids::AccountId::new(account))
            .await;
        let Ok(Some(found)) = found else {
            return refusal("That account is not in the store.");
        };
        drop(connection);

        // Awaited: `testConnection`'s exported wrapper turns the future into a value through
        // `postio_session::blocking`.
        let report = postio_session::checkup::test_connection(&found, secrets).await;
        crate::ConnectionReportFfi {
            reachable: report.reachable,
            missing_credential: report.missing_credential,
            rejected_credential: report.rejected_credential,
            message: report.message,
        }
    }

    pub async fn set_display_name(&self, account: i64, name: String) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        postio_session::checkup::set_display_name(
            &database,
            postio_model::ids::AccountId::new(account),
            &name,
        )
        .await
        .err()
    }

    /// What this account's mail weighs; `None` when there is nothing to weigh, rather than `0 B`.
    /// Read fresh each time: a stale size is a claim about a store that has changed.
    pub async fn account_weight(&self, account: i64) -> Option<String> {
        let (database, _) = self.store_and_blobs()?;
        let connection = database.connect().await.ok()?;
        let footprint = postio_storage::repository::MessageRepository::new(&connection)
            .footprint(postio_model::ids::AccountId::new(account))
            .await
            .ok()?;
        postio_ui::format::mail_weight(
            &postio_core::event::MailFootprint {
                total_bytes: footprint.total_bytes,
                attachment_bytes: footprint.attachment_bytes,
                local_bytes: footprint.local_bytes,
                complete: footprint.complete,
            },
            false,
        )
    }

    /// Bounded by the mail already on this machine: nothing here reaches a server. It reports as it
    /// goes (#1284).
    pub async fn reindex_account(&self, account: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let local = self.local.0.clone();
        match postio_session::reindex_account(
            &database,
            postio_model::ids::AccountId::new(account),
            |done, total| {
                // `try_send` fails only when the session has shut down; a re-index reporting to
                // nobody should quietly finish.
                let _ = local.try_send(UiEvent::ReindexProgress {
                    account,
                    done,
                    total,
                });
            },
        )
        .await
        {
            Ok(_) => None,
            Err(error) => Some(format!("The index could not be rebuilt: {error}")),
        }
    }

    /// A single-column write, not routed through the account update path: `update` would rewrite
    /// the identity list from whatever copy the pane was drawing.
    pub async fn set_account_enabled(&self, account: i64, enabled: bool) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let connection = match database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Some(error.to_string()),
        };
        match postio_storage::repository::AccountRepository::new(&connection)
            .set_enabled(postio_model::ids::AccountId::new(account), enabled)
            .await
        {
            // `false` is "no row matched", which the keyboard path can produce when a row was
            // removed in another window;
            // a silent no-op would look like a broken switch.
            Ok(true) => None,
            Ok(false) => Some("That account is no longer here.".to_owned()),
            Err(error) => Some(error.to_string()),
        }
    }

    /// The account new messages come from when the message does not say (#960); it does not order
    /// the sidebar, prioritise
    /// sync, or decide a reply's from address. No way to clear it: marking another is the reversal,
    /// so no undo.
    pub async fn set_default_account(&self, account: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let connection = match database.connect().await {
            Ok(connection) => connection,
            Err(error) => return Some(error.to_string()),
        };
        postio_storage::repository::AccountRepository::new(&connection)
            .set_default(postio_model::ids::AccountId::new(account))
            .await
            .err()
            .map(|error| error.to_string())
    }

    pub async fn remove_account(&self, account: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let Some(secrets) = self.secret_store() else {
            return Some("There is no keyring to take the credential out of.".to_owned());
        };
        postio_session::checkup::remove_account(
            &database,
            secrets,
            postio_model::ids::AccountId::new(account),
        )
        .await
        .err()
    }

    /// Every decision is `postio_session::signin`'s: endpoints, consent, the proof that the token
    /// opens the account's real
    /// IMAP session before anything is written, and the credential-then-row order.
    pub async fn sign_in_with_browser(
        &self,
        address: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Option<String> {
        use postio_session::signin;

        if client_id.trim().is_empty() {
            // ADR 0006 Q1: Postio ships no client id, so this is a fact about the product, not a
            // validation error.
            return Some(
                "Signing in needs an OAuth client id you registered yourself —                  Postio ships none, because a client id inside an open-source                  application is one every user of it shares."
                    .to_owned(),
            );
        }
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to add an account to.".to_owned());
        };
        let Some(secrets) = self.secret_store() else {
            return Some("There is no keyring to store the token in.".to_owned());
        };

        let domain = address
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_ascii_lowercase())
            .unwrap_or_default();
        let Some(preset) = postio_account::discovery::preset_for_domain(&domain) else {
            return Some(format!(
                "Postio does not know how to sign in to {domain}. Add it as an                  IMAP account with a password instead."
            ));
        };
        let Some(offer) = preset.oauth() else {
            return Some(format!(
                "{} does not offer a browser sign-in. Add it as an IMAP                  account with a password instead.",
                preset.display_name()
            ));
        };
        let settings = preset.settings_for(&address);
        let scopes = offer.scopes.clone();
        let client = signin::OAuthClient {
            client_id: client_id.trim().to_owned(),
            client_secret: client_secret.filter(|secret| !secret.trim().is_empty()),
        };

        let cancel = postio_account::cancel::CancelToken::new();
        *self.sign_in.lock().expect("sign-in lock") = Some(cancel.clone());
        let progress = self.sign_in_port.clone();
        progress.store(0, std::sync::atomic::Ordering::SeqCst);

        // Awaited: `blocking::now` at the FFI edge picks the right runtime, and a private one built
        // while the caller is on one panics.
        let outcome = async {
            let endpoints = signin::endpoints_for(
                offer.authorize.as_deref(),
                offer.token.as_deref(),
                offer.issuer.as_deref(),
                &cancel,
            )
            .await?;
            let signed_in = signin::sign_in(
                &settings,
                &client,
                &endpoints,
                &scopes,
                &postio_account::oauth::browser::SystemBrowserOpener,
                &cancel,
                &|port| progress.store(port, std::sync::atomic::Ordering::SeqCst),
            )
            .await?;
            signin::provision_oauth(
                &database, secrets, &settings, &client, signed_in, &scopes, None, "",
            )
            .await
            .map_err(signin::SignInError::Failed)
        }
        .await;

        *self.sign_in.lock().expect("sign-in lock") = None;
        progress.store(0, std::sync::atomic::Ordering::SeqCst);
        match outcome {
            Ok(_) => None,
            // Closing the tab is not a failure; the sheet stays where it was.
            Err(signin::SignInError::Cancelled) => Some("The sign-in was cancelled.".to_owned()),
            Err(signin::SignInError::Failed(message)) => Some(message),
        }
    }

    pub fn sign_in_progress(&self) -> crate::SignInProgressFfi {
        let waiting = self.sign_in.lock().expect("sign-in lock").is_some();
        let port = self.sign_in_port.load(std::sync::atomic::Ordering::SeqCst);
        crate::SignInProgressFfi {
            waiting,
            port,
            message: match (waiting, port) {
                (false, _) => String::new(),
                (true, 0) => "Asking the provider where to send you…".to_owned(),
                (true, port) => format!(
                    "Waiting for your browser. The answer comes back to                      127.0.0.1:{port}, and nowhere else."
                ),
            },
        }
    }

    pub fn cancel_sign_in(&self) {
        if let Some(cancel) = self.sign_in.lock().expect("sign-in lock").take() {
            cancel.cancel();
        }
    }

    pub async fn new_draft(&self) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let account = self.writing_account(&database).await?;
        let draft = postio_model::Draft::new(account.id);
        Some(
            self.named_labels(
                &database,
                crate::compose::to_ffi(&draft, account.address.to_string(), self.drafts_path()),
            )
            .await,
        )
    }

    /// The body is honoured: it goes into an unsent composer the user is looking at, and RFC 6068
    /// designs for it. Nothing
    /// is sent or fetched, and no header a link cannot legitimately set is set.
    pub async fn mailto_draft(
        &self,
        to: Vec<String>,
        cc: Vec<String>,
        bcc: Vec<String>,
        subject: Option<String>,
        body: Option<String>,
    ) -> Option<crate::DraftFfi> {
        let mut draft = self.new_draft().await?;
        let join = |addresses: Vec<String>| addresses.join(", ");
        draft.to = join(to);
        draft.cc = join(cc);
        draft.bcc = join(bcc);
        if let Some(subject) = subject {
            draft.subject = subject;
        }
        if let Some(body) = body {
            draft.body = body;
        }
        Some(draft)
    }

    pub async fn reply_draft(&self, message: i64, all: bool) -> Option<crate::DraftFfi> {
        self.answer(message, |source, account| {
            let quote = postio_model::reply::plain_quote(source);
            if all {
                postio_model::reply::reply_all(source, account, quote)
            } else {
                postio_model::reply::reply(source, account, quote)
            }
        })
        .await
    }

    pub async fn forward_draft(&self, message: i64) -> Option<crate::DraftFfi> {
        self.answer(message, |source, account| {
            let body = postio_model::reply::plain_forward(source);
            postio_model::reply::forward(source, account, body)
        })
        .await
    }

    async fn answer(
        &self,
        message: i64,
        build: impl FnOnce(&postio_model::Message, &postio_model::Account) -> postio_model::Draft,
    ) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let connection = database.connect().await.ok()?;
        let mut source = postio_storage::repository::MessageRepository::new(&connection)
            .get(postio_model::ids::MessageId::new(message))
            .await
            .ok()??;
        // The body is not on the row: it is a compressed column read through the reader's path (ADR
        // 0020), so quoting the row would quote nothing.
        if let postio_session::reading::Body::Ready { body, .. } =
            postio_session::reading::load_body_or_reason(
                &connection,
                source.id,
                self.offline.load(std::sync::atomic::Ordering::SeqCst),
            )
            .await
        {
            source.body = body;
        }
        // An HTML-only message has no `text`, so `plain_quote` would give the attribution line with
        // nothing under it; render
        // it here, since `postio-model` cannot depend on `postio-body`.
        if source.body.text.as_deref().is_none_or(str::is_empty)
            && let Some(html) = source.body.html.as_deref()
            && !html.is_empty()
        {
            let rendered = postio_body::render(&postio_body::parse(html)).0;
            if !rendered.trim().is_empty() {
                source.body.text = Some(rendered);
            }
        }
        let account = postio_storage::repository::AccountRepository::new(&connection)
            .get(source.account_id)
            .await
            .ok()??;
        let mut draft = build(&source, &account);
        // A reply starts with its conversation's labels, as GTK's composer does; a forward starts a
        // conversation of its own.
        if matches!(
            draft.kind,
            postio_model::DraftKind::Reply | postio_model::DraftKind::ReplyAll
        ) && let Some(thread) = source.thread_id
        {
            draft.labels = postio_storage::repository::LabelRepository::new(&connection)
                .for_threads(&[thread])
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(_, label)| label.id)
                .collect();
        }
        Some(
            self.named_labels(
                &database,
                crate::compose::to_ffi(&draft, account.address.to_string(), self.drafts_path()),
            )
            .await,
        )
    }

    /// The draft `id` as the store has it, or `None`: what a composer reopens with, and what a test
    /// must read to prove marks survived a save.
    pub async fn draft(&self, id: i64) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let connection = database.connect().await.ok()?;
        let draft = postio_storage::repository::DraftRepository::new(&connection)
            .get(postio_model::ids::DraftId::new(id))
            .await
            .ok()??;
        let from = self
            .writing_account(&database)
            .await
            .map(|account| account.address.to_string())
            .unwrap_or_default();
        Some(
            self.named_labels(
                &database,
                crate::compose::to_ffi(&draft, from, self.drafts_path()),
            )
            .await,
        )
    }

    /// The draft whose message row is `message`, or `None` when it is not a draft this machine
    /// holds, including another client's (#175).
    pub async fn draft_for_message(&self, message: i64) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let connection = database.connect().await.ok()?;
        let draft = postio_storage::repository::DraftRepository::new(&connection)
            .by_message(postio_model::ids::MessageId::new(message))
            .await
            .ok()??;
        let from = self
            .writing_account(&database)
            .await
            .map(|account| account.address.to_string())
            .unwrap_or_default();
        Some(
            self.named_labels(
                &database,
                crate::compose::to_ffi(&draft, from, self.drafts_path()),
            )
            .await,
        )
    }

    /// Narrow pasted markup to what a message may carry, and say what that cost. Called on paste so
    /// markup the store cannot
    /// hold is not silently narrowed at save time. Pure: no store, network or state.
    pub fn narrow_paste(&self, html: String) -> crate::PastedFfi {
        let narrowed = postio_body::narrow(&html);
        let (text, html) = postio_body::render(&narrowed.document);
        crate::PastedFfi {
            html,
            text,
            dropped: narrowed.lost.summary(),
        }
    }

    /// What a rich body reads as in plain text, for the Rich/Plain switch when it flips to Plain:
    /// the inactive field is
    /// stale then (#1293). Flowed, so a paragraph rewraps in a narrow window.
    pub fn plain_text_of(&self, html: &str) -> String {
        postio_body::render(&postio_body::parse(html)).0
    }

    /// What a plain body reads as a document, for the Rich/Plain switch when it flips to Rich: the
    /// same document `body_of`
    /// builds from a plain field, so what Rich shows is what would be sent.
    pub fn rich_html_of(&self, text: &str) -> String {
        postio_body::render(&postio_body::Document::from_text_quoting(text)).1
    }

    /// The script that applies a mark (`postio_ui::compose::mark_script`), shared so both composers
    /// do the same for `bold`.
    pub fn mark_script(&self, command: &str) -> Option<String> {
        postio_ui::compose::mark_script(command)
    }

    /// The script that links the selection to `href`, or `None` when a message may not point there.
    /// Gated before the
    /// document is touched, so a refused scheme is not a link that vanishes at the next parse.
    pub fn link_script(&self, href: &str) -> Option<String> {
        postio_ui::compose::link_script(href)
    }

    /// The composer's editing bridge (`postio_ui::compose::editor_script`): it decides the dialect
    /// the surface emits, so it
    /// crosses rather than being rewritten in Swift. Assembled, not the bare constant: the body
    /// reads `POSTIO_MARKDOWN` without defining it.
    pub fn editor_script(&self) -> &'static str {
        postio_ui::compose::editor_script()
    }

    /// Write the draft to the store and answer it with its id; idempotent on that id, or every
    /// autosave would insert a row.
    pub async fn save_draft(&self, edited: crate::DraftFfi) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let mut draft = self.rehydrate(&database, &edited).await?;
        let (connection, _permit) = database.interactive_write().await.ok()?;
        // `save_and_sync`, as `write_draft`: a local write without its queue row never reaches the
        // server.
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, chrono::Utc::now())
            .await
            .ok()?;
        Some(
            self.named_labels(
                &database,
                crate::compose::to_ffi(&draft, edited.from.clone(), self.drafts_path()),
            )
            .await,
        )
    }

    /// The row and the server copy: `discard` queues an `Operation::DiscardDraft` carrying the UID
    /// and generation, since no
    /// row is left to read them from. Discarding one already gone is not an error: a retried or
    /// racing discard is expected.
    pub async fn discard_draft(&self, draft: i64) -> Option<String> {
        let (database, _) = self.store_and_blobs()?;
        let (connection, _permit) = match database.interactive_write().await {
            Ok(held) => held,
            Err(error) => return Some(format!("The store would not take a write: {error}")),
        };
        postio_storage::repository::DraftRepository::new(&connection)
            .discard(postio_model::ids::DraftId::new(draft), chrono::Utc::now())
            .await
            .err()
            .map(|error| format!("The draft could not be discarded: {error}"))
    }

    /// The bytes first, then the row: a draft must never name a blob that is not there. The draft
    /// is saved on the way out
    /// so the attachment survives the window closing.
    pub async fn attach_to_draft(
        &self,
        edited: crate::DraftFfi,
        path: String,
        mime_type: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        let Some((database, blobs)) = self.store_and_blobs() else {
            return Err(crate::ComposeError::Refused {
                message: "There is no store to attach a file to.".to_owned(),
            });
        };
        let attachment =
            postio_session::attaching::attach_file(&blobs, std::path::Path::new(&path), &mime_type)
                .map_err(|message| crate::ComposeError::Refused { message })?;

        let mut draft = self.rehydrate(&database, &edited).await.ok_or_else(|| {
            crate::ComposeError::Refused {
                message: "This draft is no longer in the store.".to_owned(),
            }
        })?;
        draft.attachments.push(attachment);
        self.write_draft(&database, draft, &edited).await
    }

    /// The bytes first, then the row, and the script only once both are written, so nothing draws a
    /// picture whose part is not in the store.
    pub async fn insert_inline_image(
        &self,
        edited: crate::DraftFfi,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<crate::InlineImageFfi, crate::ComposeError> {
        let Some((database, blobs)) = self.store_and_blobs() else {
            return Err(crate::ComposeError::Refused {
                message: "There is no store to put a picture in.".to_owned(),
            });
        };
        let attachment = postio_session::attaching::inline_image(&blobs, &bytes, &mime_type)
            .map_err(|message| crate::ComposeError::Refused { message })?;
        let script = attachment
            .content_id
            .as_deref()
            .and_then(|id| {
                postio_ui::compose::image_script(
                    id,
                    attachment.filename.as_deref().unwrap_or("image"),
                )
            })
            .ok_or_else(|| crate::ComposeError::Refused {
                message: "The picture could not be given a name the message can refer to."
                    .to_owned(),
            })?;

        let mut draft = self.rehydrate(&database, &edited).await.ok_or_else(|| {
            crate::ComposeError::Refused {
                message: "This draft is no longer in the store.".to_owned(),
            }
        })?;
        draft.attachments.push(attachment);
        let draft = self.write_draft(&database, draft, &edited).await?;
        Ok(crate::InlineImageFfi { draft, script })
    }

    pub async fn resolve_draft_cid(
        &self,
        draft: i64,
        content_id: String,
    ) -> Option<crate::InlinePart> {
        let (database, blobs) = self.store_and_blobs()?;
        let wanted = postio_body::ContentId::parse(&content_id)?;
        let connection = database.connect().await.ok()?;
        let draft = postio_storage::repository::DraftRepository::new(&connection)
            .get(postio_model::ids::DraftId::new(draft))
            .await
            .ok()??;
        let part = draft.attachments.iter().find(|held| {
            held.content_id
                .as_deref()
                .and_then(postio_body::ContentId::parse)
                .is_some_and(|id| id == wanted)
        })?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut blobs.reader(part.blob_id.as_ref()?).ok()?, &mut bytes)
            .ok()?;
        Some(crate::InlinePart {
            bytes,
            mime_type: part.mime_type.clone(),
        })
    }

    /// The row goes; the blob is left to the store's reclaim pass, since the store deduplicates by
    /// content and two drafts can name one blob.
    pub async fn detach_from_draft(
        &self,
        edited: crate::DraftFfi,
        attachment: i64,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Err(crate::ComposeError::Refused {
                message: "There is no store open.".to_owned(),
            });
        };
        let mut draft = self.rehydrate(&database, &edited).await.ok_or_else(|| {
            crate::ComposeError::Refused {
                message: "This draft is no longer in the store.".to_owned(),
            }
        })?;
        draft.attachments.retain(|held| held.id.get() != attachment);
        self.write_draft(&database, draft, &edited).await
    }

    /// The draft is saved first: an editor opened on an unwritten body is one crash from being the
    /// only copy.
    pub async fn begin_handoff(
        &self,
        edited: crate::DraftFfi,
    ) -> Result<String, crate::ComposeError> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Err(crate::ComposeError::Refused {
                message: "There is no store open.".to_owned(),
            });
        };
        let draft = self.rehydrate(&database, &edited).await.ok_or_else(|| {
            crate::ComposeError::Refused {
                message: "This draft is no longer in the store.".to_owned(),
            }
        })?;
        let saved = self.write_draft(&database, draft, &edited).await?;

        postio_session::handoff::begin(&self.handoff_dir(), saved.id, &saved.body)
            .map(|out| out.path.display().to_string())
            .map_err(|message| crate::ComposeError::Refused { message })
    }

    pub async fn end_handoff(
        &self,
        edited: crate::DraftFfi,
        path: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        let out = postio_session::handoff::Handoff {
            path: std::path::PathBuf::from(path),
        };
        let body = postio_session::handoff::read_back(&out)
            .map_err(|message| crate::ComposeError::Refused { message })?;
        postio_session::handoff::finish(&out);

        let Some((database, _)) = self.store_and_blobs() else {
            return Err(crate::ComposeError::Refused {
                message: "There is no store open.".to_owned(),
            });
        };
        let mut draft = self.rehydrate(&database, &edited).await.ok_or_else(|| {
            crate::ComposeError::Refused {
                message: "This draft is no longer in the store.".to_owned(),
            }
        })?;
        draft.body = postio_model::message::MessageBody {
            text: Some(body),
            html: draft.body.html.clone(),
        };
        self.write_draft(&database, draft, &edited).await
    }

    /// Where a handed-off draft is written: beside the store, not in the
    /// shared temp directory, which is world-readable on every Unix.
    fn handoff_dir(&self) -> std::path::PathBuf {
        self.store_at
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|dir| dir.join("drafts-out"))
            // Per session, not one directory for the machine: a draft's filename is its id, which
            // starts at 1 in every fresh
            // store, so sessions sharing one directory read back each other's body.
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "postio-drafts-out-{}-{}",
                    std::process::id(),
                    self.serial
                ))
            })
    }

    async fn write_draft(
        &self,
        database: &postio_storage::Store,
        mut draft: postio_model::Draft,
        edited: &crate::DraftFfi,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        let (connection, _permit) =
            database
                .interactive_write()
                .await
                .map_err(|error| crate::ComposeError::Refused {
                    message: format!("The store would not take a write: {error}"),
                })?;
        // `save_and_sync`, not `save`: a local write without its queue row never reaches the
        // server. An account with no
        // Drafts folder enqueues nothing and is not an error.
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, chrono::Utc::now())
            .await
            .map_err(|error| crate::ComposeError::Refused {
                message: format!("The draft could not be saved: {error}"),
            })?;
        Ok(self
            .named_labels(
                database,
                crate::compose::to_ffi(&draft, edited.from.clone(), self.drafts_path()),
            )
            .await)
    }

    /// Nothing here opens a connection: the write is one local transaction and `postio-sync::send`
    /// drains the row when there is a network.
    pub async fn send_draft(&self, edited: crate::DraftFfi) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to send from.".to_owned());
        };
        let Some(mut draft) = self.rehydrate(&database, &edited).await else {
            return Some("This draft is no longer in the store.".to_owned());
        };
        // Refused rather than queued: an unaddressed draft would close the window and drain as
        // impossible, losing the words.
        if !draft.has_recipients() {
            return Some("This message has no recipient yet.".to_owned());
        }
        if !draft.is_sendable() {
            return Some("This draft has already been queued to send.".to_owned());
        }
        // What leaves is what the switch says, not the presence of an HTML part (#1271): `outgoing`
        // builds a
        // `multipart/alternative` from `body.html.is_some()`, so a plain draft holding marks would
        // send HTML under a `text/plain` footer.
        if !draft.rich {
            draft.body.html = None;
        }
        let Ok((connection, _permit)) = database.interactive_write().await else {
            return Some("The store would not take a write.".to_owned());
        };
        match postio_storage::repository::DraftRepository::new(&connection)
            .queue_send(&mut draft, chrono::Utc::now())
            .await
        {
            Ok(_) => {
                self.focus_list.input(postio_focus::Input::SendQueued {
                    draft: draft.id,
                    at: None,
                });
                None
            }
            Err(error) => {
                tracing::error!(%error, "could not queue the draft for sending: {error}");
                Some("The draft could not be queued for sending.".to_owned())
            }
        }
    }

    /// Queue a draft to leave at `when` (epoch milliseconds), the same queue as an immediate send
    /// with a time on the row.
    /// Every check `send_draft` makes is made here too: refusing at 8am tomorrow is worse than now.
    pub async fn send_draft_later(&self, edited: crate::DraftFfi, when: i64) -> Option<String> {
        let Some(when) = chrono::DateTime::from_timestamp_millis(when) else {
            return Some("That is not a time this message could be sent at.".to_owned());
        };
        let now = chrono::Utc::now();
        // A picker left open overnight, or a clock that moved. Sending it at
        // once would be a different command than the one that was chosen.
        if when <= now {
            return Some("That time is in the past.".to_owned());
        }
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to send from.".to_owned());
        };
        let Some(mut draft) = self.rehydrate(&database, &edited).await else {
            return Some("This draft is no longer in the store.".to_owned());
        };
        if !draft.has_recipients() {
            return Some("This message has no recipient yet.".to_owned());
        }
        if !draft.is_sendable() {
            return Some("This draft has already been queued to send.".to_owned());
        }
        if !draft.rich {
            draft.body.html = None;
        }
        let Ok((connection, _permit)) = database.interactive_write().await else {
            return Some("The store would not take a write.".to_owned());
        };
        match postio_storage::repository::DraftRepository::new(&connection)
            .queue_send_at(&mut draft, now, when)
            .await
        {
            Ok(_) => {
                self.focus_list.input(postio_focus::Input::SendQueued {
                    draft: draft.id,
                    at: Some(when),
                });
                None
            }
            Err(error) => {
                tracing::error!(%error, "could not schedule the draft: {error}");
                Some("The draft could not be scheduled.".to_owned())
            }
        }
    }

    /// A round trip through the store, not a rebuild from the fields: a composer does not edit the
    /// kind, ancestor or
    /// reserved `Message-ID`, and the last stops one message being sent twice (ADR 0021).
    pub(crate) async fn rehydrate(
        &self,
        database: &postio_storage::Store,
        edited: &crate::DraftFfi,
    ) -> Option<postio_model::Draft> {
        let base = if edited.id > 0 {
            let connection = database.connect().await.ok()?;
            postio_storage::repository::DraftRepository::new(&connection)
                .get(postio_model::ids::DraftId::new(edited.id))
                .await
                .ok()??
        } else {
            let mut fresh =
                postio_model::Draft::new(postio_model::ids::AccountId::new(edited.account));
            fresh.kind = edited.kind.into();
            fresh.in_reply_to = edited.in_reply_to.map(postio_model::ids::MessageId::new);
            fresh.forwarded_from = edited.forwarded_from.map(postio_model::ids::MessageId::new);
            fresh
        };
        Some(crate::compose::from_ffi(base, edited))
    }

    /// `draft` with its labels' names, read by id: the composer draws a
    /// chip for each. A label since deleted is left off.
    async fn named_labels(
        &self,
        database: &postio_storage::Store,
        mut draft: crate::DraftFfi,
    ) -> crate::DraftFfi {
        if draft.labels.is_empty() {
            return draft;
        }
        let Ok(connection) = database.read().await else {
            return draft;
        };
        let labels = postio_storage::repository::LabelRepository::new(&connection);
        let mut named = Vec::with_capacity(draft.labels.len());
        for label in &draft.labels {
            if let Ok(Some(found)) = labels.get(postio_model::LabelId::new(label.id)).await {
                named.push(crate::DraftLabelFfi {
                    id: label.id,
                    name: found.name,
                });
            }
        }
        draft.labels = named;
        draft
    }

    async fn writing_account(
        &self,
        database: &postio_storage::Store,
    ) -> Option<postio_model::Account> {
        let connection = database.connect().await.ok()?;
        let accounts = postio_storage::repository::AccountRepository::new(&connection);
        let enabled = accounts.list_enabled().await.ok()?;
        enabled
            .iter()
            .find(|account| account.is_default)
            .or_else(|| enabled.first())
            .cloned()
    }

    /// Where drafts live, for the composer's footer: the store this session actually opened, not a
    /// guess.
    fn drafts_path(&self) -> String {
        self.store_at.display().to_string()
    }

    /// Turn a command id into the command it means here, and run it. Aiming is `postio_core::aim`'s
    /// rule; this hands it the
    /// facts (#589, #721). `id` is the registry's string, parsed by `CommandId`'s `FromStr`, not a
    /// mirrored enum that could
    /// drift. Returns nothing: a verb is local-first and events report what happened. An unknown id
    /// is ignored, not a panic.
    pub fn invoke(&self, id: &str) {
        let Ok(id) = id.parse::<postio_core::CommandId>() else {
            tracing::debug!(id, "not a command this build knows; ignored");
            return;
        };
        // The list's own commands are Focus's controller's, as on Linux: it moves the cursor, keeps
        // the selection and aims the verbs.
        if self.focus_list.command(id) {
            return;
        }
        let Some(outbox) = self.outbox() else {
            return;
        };

        let (command, aimed) = self.with_aim(|aim| {
            let command = postio_core::aim::command_for(id, aim);
            // What a verb left aimed at the selection resolves against on the host: this view's
            // selection, cursor and scope as now.
            let aimed = postio_core::state::SharedState::default();
            let (quiet, _) = postio_core::bridge::event_channel();
            postio_core::aim::mirror(&aimed, &quiet, aim);
            (command, aimed)
        });

        if !postio_core::aim::is_wired(&self.wired, &command) {
            tracing::debug!(?id, "not a verb the host answers; ignored");
            return;
        }
        if outbox.try_send((command, aimed)).is_err() {
            // Only during teardown: the host has stopped taking commands and
            // there is nothing left to run the verb on.
            tracing::debug!("the runtime has stopped and did not run that");
        }
    }

    fn outbox(&self) -> Option<async_channel::Sender<Aimed>> {
        self.link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.outbox.clone())
    }

    /// Resolve one key press against the bindings in force (`postio_ui::keymap`, ADR 0019 Q4).
    /// `in_text_entry` is the
    /// caller's to answer: a search field that archives on `a` reads as a broken app. Does not run
    /// the command: the caller
    /// needs the answer first, since `Unhandled` must propagate to the toolkit.
    pub fn key(
        &self,
        character: Option<&str>,
        name: Option<&str>,
        modifiers: crate::ModifiersFfi,
        context: crate::UiContext,
        in_text_entry: bool,
    ) -> crate::KeyOutcomeFfi {
        // A `String` crosses because a `char` has no uniffi type; take the whole scalar or nothing.
        let character = match character.map(|text| {
            let mut characters = text.chars();
            (characters.next(), characters.next())
        }) {
            Some((Some(one), None)) => Some(one),
            // A grapheme cluster or empty string names no binding; pretending otherwise would bind
            // the first half.
            Some(_) => return crate::KeyOutcomeFfi::Unhandled,
            None => None,
        };

        let Some(chord) =
            postio_ui::keymap::Chord::from_platform_key(character, name, modifiers.into())
        else {
            // A dead key mid-composition, or a key this build has no name
            // for. It has to propagate: a monitor that swallowed a
            // composition would break every non-Latin keyboard.
            return crate::KeyOutcomeFfi::Unhandled;
        };

        // The controller's, while a surface it knows of is over the list:
        // the open message's keys are its, whatever the caller saw.
        let key_context = self.focus_list.key_context().unwrap_or_else(|| {
            postio_ui::keymap::KeyContext::from(postio_core::Context::from(context))
        });
        // In a picker, a digit or a space typed into an empty filter is the
        // picker's (`1` chooses, Space toggles): there is nothing to type
        // into yet. Typed after a letter, it is typing, as everywhere.
        let in_text_entry = if key_context == postio_ui::keymap::KeyContext::Picker {
            let bare = !(modifiers.control || modifiers.option || modifiers.command);
            let digit_or_space = character.is_some_and(|c| c.is_ascii_digit() || c == ' ');
            postio_ui::pickers::is_typing(
                in_text_entry,
                self.focus_list.in_empty_filter(),
                bare,
                digit_or_space,
            )
        } else {
            in_text_entry
        };
        // Focus's controller holds the live keymap, including a half-typed sequence (state, so not
        // in Swift: ADR 0019 Q4),
        // rebuilt when the file changes (`follow_config`).
        let outcome = self.focus_list.press(
            &chord,
            key_context,
            in_text_entry,
            std::time::Instant::now(),
        );

        // The silent path: a key that does nothing needs a log line naming chord, context and flag.
        // No message content.
        if matches!(outcome, postio_ui::keymap::Outcome::Unhandled) {
            tracing::debug!(
                chord = %chord,
                ?context,
                in_text_entry,
                "key resolved to nothing"
            );
        }
        outcome.into()
    }

    /// Whether `id` can run in `context`, given the open view: what a menu asks per item to grey
    /// the ones it cannot run
    /// (#1158), by the same rule as the palette.
    pub fn is_available(&self, id: &str, context: crate::UiContext) -> bool {
        let Ok(id) = id.parse::<postio_core::ActionId>() else {
            return false;
        };
        // A command this platform does not offer is never available on it,
        // whatever the context: see `postio_core::registry::offered_on`.
        if !postio_core::registry::offered_on(id, crate::PLATFORM) {
            return false;
        }
        postio_core::registry::reachable_in(
            postio_core::Context::from(context),
            self.availability(),
        )
        .any(|spec| spec.id == id)
    }

    /// Watch `source`'s file, if it has one, and follow it: `[keys]` reaches Focus's controller,
    /// which says
    /// `UiEvent::KeymapChanged`; `[focus]` reaches the engine. Mirrors GTK's `follow_config`.
    fn follow_config(self: &Arc<Self>, source: &ConfigSource) {
        let Some(path) = source.path() else { return };
        let mut service = postio_core::ConfigService::load(&path);
        let session = Arc::downgrade(self);
        let watcher = postio_config::watch::ConfigWatcher::new(&path, move |checked| {
            let update = service.apply(checked);
            let Some(session) = session.upgrade() else {
                return;
            };
            if update.changed.keys {
                let keys = service.config().keys.clone();
                *session.keys.lock().expect("keys lock") = keys;
                *session.keymap.lock().expect("keymap lock") = None;
                session
                    .focus_list
                    .input(postio_focus::Input::Keymap(session.keymap()));
                let _ = session.local.0.try_send(UiEvent::KeymapChanged);
            }
            if update.changed.filters {
                session.focus_list.input(postio_focus::Input::SavedSearches(
                    postio_ui::saved_search::pinned(service.config()),
                ));
            }
            if update.changed.sync {
                let notices = crate::NoticesFfi::from(&service.config().sync);
                let mut kept = session.notices.lock().expect("notices lock");
                if *kept != notices {
                    *kept = notices;
                    drop(kept);
                    let _ = session.local.0.try_send(UiEvent::NoticesChanged);
                }
            }
            if update.changed.focus {
                *session.focus_config.lock().expect("focus config lock") =
                    service.config().focus.clone();
                session
                    .focus_list
                    .input(postio_focus::Input::Config(session.focus_config()));
                session
                    ._host
                    .enable_focus(postio_host::FocusSetup::from_config(
                        service.config().focus.clone(),
                        Some(service.path()),
                    ));
            }
        });
        match watcher {
            Ok(watcher) => *self.config_watch.lock().expect("watch lock") = Some(watcher),
            Err(error) => {
                tracing::warn!(%error, "config.toml will not be watched; edits need a restart")
            }
        }
    }

    /// Tell Focus's controller what the configuration says it needs: pinned saved searches, the
    /// keys in force, `[focus]`
    /// and the saved-search file; `follow_config` keeps them current.
    fn prime_focus(&self, saved: Vec<postio_ui::saved_search::SavedSearch>, source: &ConfigSource) {
        let driver = &self.focus_list;
        driver.set_config_path(source.path());
        driver.input(postio_focus::Input::SavedSearches(saved));
        driver.input(postio_focus::Input::Keymap(self.keymap()));
        driver.input(postio_focus::Input::Config(self.focus_config()));
    }

    pub(crate) fn notices_now(&self) -> crate::NoticesFfi {
        self.notices.lock().expect("notices lock").clone()
    }

    /// `[focus]` as it stands, with what people decided added (ADR 0048).
    pub(crate) fn focus_config(&self) -> postio_config::FocusConfig {
        let raw = self.focus_config.lock().expect("focus config lock").clone();
        self._host.wiring().focus.overlaid(&raw)
    }

    /// Hand Focus's controller `[focus]` again whenever a verb changes what
    /// people decided, which no change to the file announces.
    fn follow_contacts(self: &Arc<Self>) {
        let session = Arc::downgrade(self);
        self._host
            .wiring()
            .focus
            .watch(Arc::new(move |config: &postio_config::FocusConfig| {
                if let Some(session) = session.upgrade() {
                    session
                        .focus_list
                        .input(postio_focus::Input::Config(config.clone()));
                }
            }));
    }

    pub(crate) fn focus_driver(&self) -> &Arc<crate::focus_list::FocusDriver> {
        &self.focus_list
    }

    pub(crate) fn client(&self) -> Option<postio_client::Client> {
        self.link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.client.clone())
    }

    /// `store_open` is unconditionally true: a `Session` is constructed over an open store.
    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability {
            frontend: crate::FRONTEND,
            // Focus's lists span every account, so the view is always the
            // unified one: a command needing a single account is not offered.
            ..postio_core::Availability::open(postio_core::Scope::Unified)
        }
    }

    /// The palette's rows for `query`, best first, filtered to what `context` can run and the open
    /// scope satisfies. The matcher is `postio_ui::palette`'s.
    pub fn palette_entries(
        &self,
        query: &str,
        context: crate::UiContext,
    ) -> Vec<crate::PaletteEntryFfi> {
        let keymap = self.keymap();
        postio_ui::palette::entries(
            &keymap,
            postio_core::Context::from(context),
            self.availability(),
            query,
        )
        .into_iter()
        .map(crate::PaletteEntryFfi::from)
        .collect()
    }

    /// Every command with the binding in force, in cheat-sheet order: the palette's list unfiltered
    /// (#658).
    /// `cheat_sheet_sections` is the grouped form for a `?` overlay.
    pub fn cheat_sheet(&self, context: crate::UiContext) -> Vec<crate::PaletteEntryFfi> {
        self.palette_entries("", context)
    }

    /// The `?` sheet, grouped by `postio_ui::cheatsheet::sections`, the same function GTK draws
    /// from.
    pub fn cheat_sheet_sections(&self, context: crate::UiContext) -> Vec<crate::CheatSectionFfi> {
        postio_ui::cheatsheet::sections(
            &self.keymap(),
            postio_core::Context::from(context),
            self.availability(),
        )
        .into_iter()
        .map(crate::CheatSectionFfi::from)
        .collect()
    }

    /// Resolved once and kept: resolving the keymap per tooltip cost about 50 ms of main thread per
    /// message move. The
    /// cache is keyed by the registry's size, so a late-registered extension is picked up.
    pub(crate) fn keymap(&self) -> postio_core::Keymap {
        let commands = postio_core::registry::every_action().count();
        let mut cached = self.keymap.lock().expect("keymap lock");
        match &*cached {
            Some((size, keymap)) if *size == commands => keymap.clone(),
            _ => {
                let keymap = postio_core::Keymap::resolve_on(
                    &self.keys.lock().expect("keys lock"),
                    crate::PLATFORM,
                );
                *cached = Some((commands, keymap.clone()));
                keymap
            }
        }
    }

    pub async fn finder_folders(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::folders(&self.mailboxes().await, &query)
    }

    pub async fn finder_contacts(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::contacts(&self.finder_sources().await.contacts, &query)
    }

    pub async fn finder_labels(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::labels(&self.finder_sources().await.labels, &query)
    }

    async fn finder_sources(&self) -> FinderSources {
        if let Some((read, sources)) = &*self.finder_sources.lock().expect("finder lock")
            && read.elapsed() < FINDER_SOURCES_FOR
        {
            return sources.clone();
        }
        let client = self
            .link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.client.clone());
        let Some(client) = client else {
            return FinderSources::default();
        };
        let accounts = match self.store_and_blobs() {
            Some((database, _)) => match database.connect().await {
                Ok(connection) => postio_storage::repository::AccountRepository::new(&connection)
                    .list_enabled()
                    .await
                    .unwrap_or_default(),
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        let mut sources = FinderSources::default();
        for account in accounts {
            match client.correspondents(account.id).await {
                Ok(found) => sources.contacts.extend(found),
                Err(error) => tracing::warn!(%error, "could not read the correspondents"),
            }
            match client.labels(account.id).await {
                Ok(found) => sources.labels.extend(found),
                Err(error) => tracing::warn!(%error, "could not read the labels"),
            }
        }
        *self.finder_sources.lock().expect("finder lock") =
            Some((std::time::Instant::now(), sources.clone()));
        sources
    }

    /// `+`'s Return: put `label` on the selection (the marked messages, or the one under the
    /// cursor), toggled so `u` takes it back.
    pub fn apply_label(&self, label: i64) {
        let Some(outbox) = self.outbox() else {
            return;
        };
        let command = postio_core::Command::AddLabel {
            target: postio_core::MessageTarget::Selection,
            label: Some(postio_model::ids::LabelId::new(label)),
            on: None,
        };
        if !postio_core::aim::is_wired(&self.wired, &command) {
            tracing::debug!("labelling is not a verb the host answers; ignored");
            return;
        }
        if outbox.try_send((command, self.aimed())).is_err() {
            tracing::debug!("the runtime has stopped and did not label that");
        }
    }

    fn aimed(&self) -> postio_core::state::SharedState {
        self.with_aim(|aim| {
            let aimed = postio_core::state::SharedState::default();
            let (quiet, _) = postio_core::bridge::event_channel();
            postio_core::aim::mirror(&aimed, &quiet, aim);
            aimed
        })
    }

    /// Run `f` with what a verb would be aimed at: Focus's list for the rows, the place it shows
    /// for the scope, and the
    /// cursor the frontend last reported. Nothing is marked here: the selection is the
    /// controller's, so a verb acts on the cursor's row.
    fn with_aim<R>(&self, f: impl FnOnce(&postio_core::aim::Aim<'_>) -> R) -> R {
        // Before the rows are held: reading the cursor takes the list's lock
        // too, and a thread that already holds it would wait on itself.
        let cursor = self.focus_list.cursor_message();
        let rows = self.focus_list.rows();
        let selection = postio_core::state::Selection::default();
        let aim = postio_core::aim::Aim {
            // The shared conversion, not a second one: `aim::view_scope` is
            // the one rule for what a whole-view gesture is relative to.
            scope: self.focus_list.scope().and_then(|scope| {
                postio_core::aim::view_scope(scope, &self.reachable.lock().expect("reachable lock"))
            }),
            selection: &selection,
            cursor,
            rows: &*rows,
        };
        f(&aim)
    }

    /// The cursor rested on `message` long enough to count as read. Not `invoke`: `MarkReadOnDwell`
    /// routes to `ToggleRead`'s
    /// handler but is not recorded on the undo stack, or reading a mailbox would bury the archive
    /// the user wants back.
    /// The message is named, since the cursor may have moved on.
    pub fn mark_read_on_dwell(&self, message: i64) {
        let Some(outbox) = self.outbox() else {
            return;
        };
        let command = postio_core::Command::MarkReadOnDwell {
            message: postio_model::ids::MessageId::new(message),
        };
        let aimed = postio_core::state::SharedState::default();
        if outbox.try_send((command, aimed)).is_err() {
            tracing::debug!("the runtime has stopped and did not mark that read");
        }
    }

    /// Say which accounts the aggregate view can currently vouch for, read when a whole-view
    /// selection is made. A frontend
    /// that never calls this gets `Ctrl+A` selecting nothing (#811).
    pub fn set_reachable_accounts(&self, accounts: &[i64]) {
        *self.reachable.lock().expect("reachable lock") = accounts
            .iter()
            .copied()
            .map(postio_model::ids::AccountId::new)
            .collect();
    }

    fn reader(
        &self,
    ) -> Option<(
        Arc<dyn postio_runtime::store::MailStore>,
        tokio::runtime::Handle,
    )> {
        let guard = self.wiring.lock().expect("wiring lock");
        let wiring = guard.as_ref()?;
        Some((wiring.store.clone(), wiring.runtime.clone()))
    }

    #[cfg(feature = "testing")]
    pub fn focus_aims_at_has_action_for_test(&self) -> bool {
        matches!(
            self.focus_list.scope(),
            Some(postio_model::ListScope::Focus(
                postio_model::FocusScope::HasAction
            ))
        )
    }

    #[cfg(feature = "testing")]
    pub fn focus_page_reads_for_test(&self) -> usize {
        self.focus_list.page_reads()
    }

    #[cfg(feature = "testing")]
    pub fn focus_search_reads_for_test(&self) -> usize {
        self.focus_list.search_reads()
    }

    #[cfg(feature = "testing")]
    pub fn focus_resident_rows_for_test(&self) -> usize {
        self.focus_list.resident_rows()
    }

    /// Test-only: wait until no conversation read is in flight.
    #[cfg(feature = "testing")]
    pub fn settle_for_test(&self) {
        let ordering = std::sync::atomic::Ordering::SeqCst;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.in_flight.load(ordering) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The whole document for a message, ready for a web view: policy, fonts, tokens, sanitized
    /// body and scroll markers all
    /// come from `postio_ui`, so the two readers cannot disagree (ADR 0019 Q6). The frontend
    /// registers two handlers answering
    /// from compiled-in bytes: `postio-cid:` for inline parts and `postio-font:` for the vendored
    /// faces (ADR 0023), or fonts fall back silently.
    pub async fn reader_answers(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        reduced: bool,
    ) -> crate::ReaderDocumentFfi {
        let plate = |html: String| crate::ReaderDocumentFfi {
            html,
            notice: None,
            caveat: None,
        };
        use postio_ui::reader::document::{
            Rendering, Sheet, absent_html, body_html, document_for, sheet_for, suits_reader_view,
            wrap_document,
        };

        let remote = postio_body::RemoteImages::from(remote);
        // The blob store is not consulted for the body: it is a compressed column on the row (ADR
        // 0020); inline parts still come from it.
        let Some((database, _blobs)) = self.store_and_blobs() else {
            return plate(wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            ));
        };
        let Ok(connection) = database.connect().await else {
            return plate(wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            ));
        };
        let offline = self.offline.load(std::sync::atomic::Ordering::SeqCst);
        let (body, row) =
            postio_session::reading::load_with_row(&connection, message.into(), offline).await;
        match body {
            // `encoding_problems` is unused here on purpose: the caveat is native chrome above the
            // document and crosses as `decode_caveat`.
            postio_session::reading::Body::Ready {
                body,
                encoding_problems,
                ..
            } => {
                // Every message opens as its sender built it; reader view is the reader's own
                // choice for this message and
                // view (`toggle_reader_view`), never the default.
                let bulk = suits_reader_view(&body);
                let rendering = if reduced {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                let drawn = body_html(&body, remote, rendering);
                // `sheet_for` (shared with GTK) picks the sender's own sheet for an original reader
                // view would have reduced; the
                // app's palette is never injected into a sender's markup. The notice comes only
                // from a blocked render, with counts
                // from the sanitize pass, so Reader and Original agree (#1589).
                let notice = self.held_back_notice(
                    drawn.held_back,
                    remote,
                    row.as_ref().and_then(|row| row.from.first()),
                );
                crate::ReaderDocumentFfi {
                    html: document_for(
                        &drawn.html,
                        &drawn.styles,
                        remote,
                        sheet_for(drawn.rendering, bulk),
                    ),
                    notice,
                    caveat: postio_ui::reader::document::decode_caveat(encoding_problems)
                        .map(str::to_owned),
                }
            }
            // A state plate is Postio's own words: served with remote images blocked whatever was
            // asked, since nothing in it is a sender's.
            postio_session::reading::Body::Absent(state) => plate(wrap_document(
                &absent_html(state),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            )),
        }
    }

    /// What a blocked render held back, as the notice says it, with the sender the grant would
    /// name: `None` for an allowed render or one that held nothing.
    pub(crate) fn held_back_notice(
        &self,
        held_back: postio_ui::reader::document::HeldBack,
        remote: postio_body::RemoteImages,
        from: Option<&postio_model::EmailAddress>,
    ) -> Option<crate::ReaderNoticeFfi> {
        if remote == postio_body::RemoteImages::Blocked {
            let summary = held_back.summary();
            if summary.is_empty() {
                None
            } else {
                let sender = from
                    .map(|address| address.address.to_lowercase())
                    .unwrap_or_default();
                let domain = sender
                    .rsplit_once('@')
                    .map(|(_, domain)| domain.to_owned())
                    .unwrap_or_default();
                Some(crate::ReaderNoticeFfi {
                    summary: format!("{summary} blocked"),
                    allowed: self.allow_list().is_allowed(&sender),
                    sender,
                    domain,
                    remote_images: held_back.remote_images,
                    trackers: held_back.trackers,
                })
            }
        } else {
            None
        }
    }

    pub async fn reader_document(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        reduced: bool,
    ) -> String {
        self.reader_answers(message, remote, reduced).await.html
    }

    /// Who `message` was addressed to, already rendered; `None` for a message the store does not
    /// hold. Answered from the
    /// envelope, so a message awaiting its body still says who it went to.
    pub async fn recipients(&self, message: i64) -> Option<crate::RecipientsFfi> {
        self.message_facts(message).await.recipients
    }

    /// The verbs the reading pane offers, in canvas order. No key travels with them: the chord is
    /// `Session::binding_for`'s.
    pub fn reader_actions(&self) -> Vec<crate::ReaderActionFfi> {
        postio_ui::reader::header::ReaderAction::ALL
            .iter()
            .map(|action| crate::ReaderActionFfi {
                command: action.command().as_str().to_string(),
                title: action.title().to_string(),
                primary: action.primary(),
            })
            .collect()
    }

    /// The header bar of a conversation of `messages`: the same four verbs as `reader_actions`,
    /// each with what it acts on
    /// in words. The frontend passes the latest message or none, per `ReaderAction::scope`.
    pub fn conversation_actions(&self, messages: u32) -> Vec<crate::ConversationActionFfi> {
        use postio_ui::reader::header::{ActionScope, ReaderAction};
        ReaderAction::ALL
            .iter()
            .map(|action| crate::ConversationActionFfi {
                command: action.command().as_str().to_string(),
                title: action.title().to_string(),
                primary: action.primary(),
                description: action.describe(messages as usize),
                whole_conversation: matches!(action.scope(), ActionScope::WholeConversation),
            })
            .collect()
    }

    /// One inline part of `message`, by its `Content-ID`. Synchronous and local: a URL scheme
    /// handler runs on the main
    /// thread. Scoped to a message since a `Content-ID` means something only inside the message
    /// that declared it. `None`
    /// when the bytes are not here: fetching would be the tracking pixel through the back door.
    pub async fn resolve_cid(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        let (database, blobs) = self.store_and_blobs()?;
        postio_session::reading::resolve_cid(&database, &blobs, message.into(), &content_id)
            .await
            .map(|(bytes, mime_type)| crate::InlinePart { bytes, mime_type })
    }

    pub async fn part_bytes(
        &self,
        message: i64,
        part_id: String,
    ) -> Result<Vec<u8>, crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        let message = message.into();
        let engine = self.engine_for_message(message).await;
        // The hub, so a refusal is heard rather than waited out (#1996).
        let events = self
            .wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .map(|wiring| wiring.events.clone());
        Ok(postio_session::reading::part_bytes_at(
            &database,
            &blobs,
            engine,
            events.as_ref(),
            message,
            &part_id,
        )
        .await?)
    }

    /// The engine of the account `message` belongs to, once started; `None` is an ordinary state
    /// and turns "fetch this
    /// part" into a sentence. Never another account's engine.
    async fn engine_for_message(
        &self,
        message: postio_model::MessageId,
    ) -> Option<postio_runtime::Engine> {
        let (database, _) = self.store_and_blobs()?;
        let account = postio_host::parts::account_of(&database, message).await?;
        self.wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .and_then(|wiring| wiring.engine.get(account))
    }

    /// Every folder of every enabled account, for the sidebar. Blocks on a local read of a few
    /// milliseconds.
    pub async fn mailboxes(&self) -> Vec<crate::MailboxFfi> {
        let mut folders = Vec::new();
        let Some((database, _)) = self.store_and_blobs() else {
            return folders;
        };
        let Ok(connection) = database.connect().await else {
            return folders;
        };
        let Ok(accounts) = postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
            .await
        else {
            return folders;
        };
        let Some((store, _runtime)) = self.reader() else {
            return folders;
        };
        for account in accounts {
            if let Ok(mut found) = blocking(store.mailboxes(account.id)) {
                // The rows that are views rather than folders (Flagged, Snoozed, the Outbox when it
                // holds something), built by the shared layer (#1155).
                let counts = postio_ui::sidebar::ViewCounts {
                    flagged: found.iter().map(|folder| folder.counts.flagged).sum(),
                    snoozed: found.iter().map(|folder| folder.counts.snoozed).sum(),
                    outbox: 0,
                    drafts: 0,
                    attention: 0,
                };
                found.extend(postio_ui::sidebar::view_rows(account.id, &found, counts));
                // Ordered and split by `postio_ui::sidebar`, not the frontend: Inbox first, and a
                // role gets one row however
                // many folders carry it (#501, #1155).
                let (special, ordinary) = postio_ui::sidebar::sections(&found);
                // The name is what Postio calls the role, not `mailbox.name`: an iCloud junk folder
                // is "Junk E-mail", and a view row has no server name.
                let named = |mailbox: postio_model::Mailbox, special: bool| crate::MailboxFfi {
                    name: postio_ui::sidebar::display_name(&mailbox, &found),
                    special,
                    ..crate::MailboxFfi::from(mailbox)
                };
                folders.extend(special.into_iter().map(|mailbox| named(mailbox, true)));
                folders.extend(ordinary.into_iter().map(|mailbox| named(mailbox, false)));
            }
        }
        folders
    }

    /// The binding in force for a command, for a native accelerator: the user's override or the
    /// built-in default, resolved
    /// for the running platform (`cmd+k`, not `mod+k`). A menu must ask rather than read
    /// `default_binding`, which is wrong for a rebound command.
    pub fn binding_for(&self, command: String) -> Option<String> {
        // The resolved keymap, not the override table: `KeyBindings` knows only 24 of about eighty
        // commands (#1227), so the menu drew no accelerator for the rest.
        let Ok(action) = command.parse::<postio_core::ActionId>() else {
            return None;
        };
        self.keymap().binding(action).map(str::to_string)
    }

    /// Every binding in force for `command`, the primary first: the canvas' two keyboard layers are
    /// two bindings on one command (`e` and `⌘R`).
    pub fn bindings_for(&self, command: String) -> Vec<String> {
        let Ok(action) = command.parse::<postio_core::ActionId>() else {
            return Vec::new();
        };
        self.keymap()
            .bindings(action)
            .iter()
            .map(|binding| binding.to_string())
            .collect()
    }

    /// How many accounts are configured and enabled: ADR 0005 Q3, the first account is not special.
    pub async fn configured_accounts(&self) -> u32 {
        let Some((database, _)) = self.store_and_blobs() else {
            return 0;
        };
        let Ok(connection) = database.connect().await else {
            return 0;
        };
        postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
            .await
            .map(|accounts| accounts.len() as u32)
            .unwrap_or(0)
    }

    pub fn has_engine(&self) -> bool {
        self.wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .is_some_and(|wiring| !wiring.engine.is_empty())
    }

    /// Start syncing every configured account, and answer how many started. Zero accounts is
    /// `Ok(0)`, not an error: it is
    /// the first-run state. Does not block: the connection attempt happens on the session's
    /// runtime.
    pub async fn start_syncing(&self) -> Result<u32, SessionError> {
        // Cloned out and the guard dropped, not held across the awaits: a `MutexGuard` across a
        // network await would block a runtime worker.
        let wiring = {
            let guard = self.wiring.lock().expect("wiring lock");
            match guard.as_ref() {
                Some(wiring) => wiring.clone(),
                None => {
                    return Err(SessionError::StoreUnavailable {
                        message: "the session has been shut down".to_string(),
                    });
                }
            }
        };
        let wiring = &wiring;

        let accounts = {
            let connection = wiring.database.connect().await.map_err(|error| {
                SessionError::StoreUnavailable {
                    message: error.to_string(),
                }
            })?;
            postio_storage::repository::AccountRepository::new(&connection)
                .list_enabled()
                .await
                .map_err(|error| SessionError::StoreUnavailable {
                    message: error.to_string(),
                })?
        };
        // Idempotent per account, not per session: an early return on any engine kept a second set
        // from doubling connections
        // but left an account added while the app is running with no engine.
        let running: std::collections::HashSet<postio_model::ids::AccountId> = self
            .engines
            .lock()
            .expect("engines lock")
            .iter()
            .map(|(account, _)| *account)
            .collect();
        let already = running.len() as u32;
        let accounts: Vec<_> = accounts
            .into_iter()
            .filter(|account| !running.contains(&account.id))
            .collect();
        if accounts.is_empty() {
            return Ok(already);
        }

        let started = postio_session::engine::start_all(&accounts, wiring)
            .await
            .map_err(|refusal| SessionError::StoreUnavailable {
                message: refusal.to_string(),
            })?;

        let count = started.len() as u32;
        for (account, engine) in started {
            // The slot is what `Refresh` reads, and it is pressed long after
            // the bus was built. An engine that ran but never reached it
            // would sync happily and leave the refresh command inert.
            wiring.engine.fill(account, engine.clone());
            postio_runtime::retain(engine.clone());
            self.engines
                .lock()
                .expect("engines lock")
                .push((account, engine));
        }
        Ok(already + count)
    }

    /// Adopt an engine over `MockBackend`, so retaining the engine and filling the slot are
    /// testable without a TLS connect.
    #[cfg(feature = "testing")]
    pub fn adopt_mock_engine_for_test(&self) {
        self.adopt_mock_engine_for(
            postio_model::ids::AccountId::new(1),
            Arc::new(postio_account::backend::MockBackend::new()),
        );
    }

    /// `adopt_mock_engine_for_test` for one named account, over the caller's server: each account's
    /// engine must answer from its own.
    #[cfg(feature = "testing")]
    pub fn adopt_mock_engine_for(
        &self,
        account: postio_model::ids::AccountId,
        backend: Arc<postio_account::backend::MockBackend>,
    ) {
        let guard = self.wiring.lock().expect("wiring lock");
        let Some(wiring) = guard.as_ref() else { return };
        let parts = postio_runtime::EngineParts {
            account,
            database: wiring.database.clone(),
            blobs: wiring.blobs.clone(),
            backend,
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            tokens: Arc::new(postio_account::auth::StoredPasswordSource::new(Arc::new(
                postio_account::secret::MemorySecretStore::default(),
            ))),
            events: wiring.events.clone(),
            mailbox_roles: wiring.mailbox_roles.clone(),
            clock: Arc::new(postio_runtime::SystemClock),
            retry: Default::default(),
            backfill: Default::default(),
            reconnect: Default::default(),
            watch: Default::default(),
            network: postio_runtime::NetworkSource::Ignored,
        };
        if let Ok(engine) = postio_runtime::Engine::spawn(parts) {
            wiring.engine.fill(account, engine.clone());
            self.engines
                .lock()
                .expect("engines lock")
                .push((account, engine));
        }
    }

    pub fn set_offline(&self, offline: bool) {
        let ordering = std::sync::atomic::Ordering::SeqCst;
        let was = self.offline.swap(offline, ordering);

        // Only the transition back: `NWPathMonitor` repeats itself, and reconnecting on each repeat
        // would hammer a flapping connection.
        if was && !offline {
            self.reconnect();
        }
    }

    pub fn is_offline(&self) -> bool {
        self.offline.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Ask every engine to try again now. A nudge, not a mechanism: it only buys promptness over
    /// the engine's backoff.
    /// Failures are the engine's to report, so one is not said twice.
    fn reconnect(&self) {
        // Counted, and nothing more: Focus's lists span every mailbox, so
        // there is no one folder in view to refresh, and the engines' own
        // backoff loops are what reconnect (specs/009-focus-macos).
        self.reconnects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(feature = "testing")]
    pub fn reconnects_for_test(&self) -> usize {
        self.reconnects.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn store_and_blobs(
        &self,
    ) -> Option<(postio_storage::Store, postio_storage::BlobStore)> {
        let guard = self.wiring.lock().expect("wiring lock");
        let wiring = guard.as_ref()?;
        Some((wiring.database.clone(), wiring.blobs.clone()))
    }

    /// Every command the registry knows, in cheat-sheet order: the frontend builds its palette,
    /// cheat sheet, menu bar and key hints from it (PRODUCT.md §8).
    pub fn commands(&self) -> Vec<crate::CommandSpecFfi> {
        crate::registry::commands()
    }

    /// False after `shutdown`; lets a caller tell a session that holds its store from one that does
    /// not.
    pub fn is_open(&self) -> bool {
        self.wiring
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Drops the store and ends the event drain. Idempotent: a window close racing an explicit quit
    /// can deliver termination twice.
    pub fn shutdown(&self) {
        // Agents first: nothing should be answering once the store goes.
        if let Ok(mut agents) = self.agents.lock() {
            agents.take();
        }
        if let Ok(mut guard) = self.wiring.lock() {
            guard.take();
        }
        if let Ok(mut link) = self.link.lock() {
            link.take();
        }
    }

    /// The next event, or `None` once the session has stopped. UniFFI makes it Swift `async`: the
    /// frontend's drain is
    /// `while let event = await session.nextEvent()`.
    pub async fn next_event(&self) -> Option<UiEvent> {
        // Whichever speaks first. The engine's stream ends when the session
        // shuts down, and that is what must end the frontend's loop -- so a
        // closed engine stream wins even if the local one is merely idle.
        tokio::select! {
            engine = self.events.next() => engine.map(|event| {
                self.focus_list.event(&event);
                UiEvent::from(event)
            }),
            local = self.local.1.recv() => local.ok(),
        }
    }

    /// Test-only: the next event if one is already waiting, rather than a wait; with the channel
    /// open, a blocking read hangs the binary instead of failing.
    #[cfg(feature = "testing")]
    pub fn try_next_event(&self) -> Option<UiEvent> {
        // Both channels, as `next_event`: a drain reading only the engine's would miss
        // `ConversationReady` and `ReindexProgress`.
        match self.events.try_next() {
            Some(engine) => Some(UiEvent::from(engine)),
            None => self.local.1.try_recv().ok(),
        }
    }

    pub fn next_event_blocking(&self) -> Option<UiEvent> {
        self.events.next_blocking().map(UiEvent::from)
    }

    /// Test-only, behind the feature: a frontend that can invent events has repaints that mean
    /// nothing.
    #[cfg(feature = "testing")]
    pub fn emit_for_test(&self, event: postio_core::Event) {
        if let Ok(guard) = self.wiring.lock()
            && let Some(wiring) = guard.as_ref()
        {
            wiring.events.emit(event);
        }
    }

    /// Test-only: whether this session's `Wiring` was built with the backfill and watch policy
    /// `[sync]` (as `expected` parses it) implies (#1014).
    #[cfg(feature = "testing")]
    pub fn honors_sync_config_for_test(&self, expected: &postio_config::SyncConfig) -> bool {
        let expected_backfill = postio_session::backfill_policy(expected);
        let expected_watch = postio_session::watch_policy(expected);
        self.wiring.lock().ok().is_some_and(|guard| {
            guard.as_ref().is_some_and(|wiring| {
                wiring.backfill == expected_backfill && wiring.watch == expected_watch
            })
        })
    }
}

/// What the reader offers about `message`'s list, with the account that would own the activation:
/// one read serving both,
/// so the sentence seen and the row written cannot differ. Free, since it holds no session state.
async fn unsubscribe_offer_for(
    connection: &postio_storage::Checkout,
    id: postio_model::ids::MessageId,
) -> Option<(postio_model::ids::AccountId, postio_ui::unsubscribe::Offer)> {
    let repository = postio_storage::repository::MessageRepository::new(connection);
    let message = repository.get(id).await.ok()??;
    // `send_state` is a column on the row, so a second point read. It is the gate (#1525): a failed
    // read means "do not offer".
    let send_state = match repository.send_state(id).await {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(message = id.get(), %error, "cannot read a message's send state");
            return None;
        }
    };
    let offer =
        postio_ui::unsubscribe::offer(send_state, message.list_id.as_deref(), &message.from)?;
    Some((message.account_id, offer))
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StartedOverFfi {
    /// The directory the old database, sidecars and blobs were moved to, not deleted, so a build of
    /// their schema opens them there.
    pub set_aside: String,
    pub accounts: u32,
}

/// Set aside the store `options` names and start a fresh one, carrying its accounts across
/// (`postio_session::start_over::start_over_at`).
/// The store key is the one the old store was written under, which stays in place. Blocks; refused
/// while another Postio has the store open.
pub fn start_over_with(options: SessionOptions) -> Result<StartedOverFfi, SessionError> {
    let secrets: Arc<dyn postio_account::secret::SecretStore> = match options.secrets {
        Some(secrets) => secrets,
        None => postio_account::secret::platform_keyring(),
    };
    let key = postio_session::store_key_blocking(secrets.as_ref())
        .map_err(SessionError::from_secret_error)?;
    let path = options
        .store_path
        .unwrap_or_else(postio_session::paths::store_path);
    let started = blocking(postio_session::start_over::start_over_at(&path, &key))
        .map_err(|message| SessionError::StoreUnavailable { message })?;
    Ok(StartedOverFfi {
        set_aside: started.set_aside.display().to_string(),
        accounts: u32::try_from(started.accounts).unwrap_or(u32::MAX),
    })
}

/// Run `future` to completion on this thread, on a stack this crate owns: Swift calls in from
/// threads with 512 KiB, which
/// the engine's deepest compile does not fit. The exported surface is synchronous, so callers must
/// not invoke these on the
/// main actor. `postio_session::blocking::now` is the one shared implementation: a runtime built
/// here panics when the caller is already on one.
pub(crate) fn blocking<T>(future: impl std::future::Future<Output = T>) -> T {
    crate::stack::on_engine_stack(|| postio_session::blocking::now(future))
}
