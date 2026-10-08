//! Opening a session, draining its events, and shutting it down.

use std::sync::{Arc, Mutex};

use postio_core::bridge::{CommandSender, EventStream};
use postio_host::Host;
use postio_session::Wiring;

use crate::event::UiEvent;

/// Why a session could not be opened.
///
/// Distinguishable cases rather than one string, because the frontend routes
/// on them. ADR 0014 is the reason it matters: the store's master key lives in
/// the OS keyring, so a locked keyring means *"unlock this and retry"* and not
/// *"set up an account"* — and a caller that had to match on message text to
/// tell those apart would send a user with working mail through onboarding.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SessionError {
    /// The local store could not be opened.
    #[error("{message}")]
    StoreUnavailable {
        /// A sentence for the user, already written by the store layer.
        message: String,
    },
    /// The OS keyring holds the store's key and would not give it up.
    ///
    /// Its own case, not a `StoreUnavailable`: the remedy is the user
    /// unlocking a keyring, which is a different surface from a broken store.
    #[error("{message}")]
    KeyringLocked {
        /// A sentence for the user, naming the platform's keyring.
        message: String,
    },
    /// The store was written at a schema this build cannot carry forward
    /// (`postio_session::Remedy::StartOver`).
    ///
    /// Its own case, not a `StoreUnavailable`: trying again meets the same
    /// file every time, so a surface that offers "Try again" for it is a dead
    /// end. The way forward is starting the store over, which sets it aside
    /// rather than deleting it (`postio_session::start_over`).
    #[error("{message}")]
    StoreFromAnotherBuild {
        /// A sentence for the user, already written by the store layer.
        message: String,
    },
    /// The tokio runtime the engine needs could not be started.
    #[error("{message}")]
    RuntimeUnavailable {
        /// What the runtime said.
        message: String,
    },
}

impl SessionError {
    /// Maps a store refusal onto the case the frontend routes on: whether
    /// trying again can help, or only starting the store over.
    fn from_refusal(refusal: postio_session::Refusal) -> Self {
        let message = refusal.sentence;
        match refusal.remedy {
            postio_session::Remedy::TryAgain => SessionError::StoreUnavailable { message },
            postio_session::Remedy::StartOver { .. } => {
                SessionError::StoreFromAnotherBuild { message }
            }
        }
    }

    /// Maps a keyring failure onto the case the frontend routes on.
    ///
    /// A match rather than `to_string`, and that is the entire point of this
    /// function. ADR 0014's rule is that
    /// [`SecretError::Locked`](postio_account::secret::SecretError::Locked) must survive
    /// to the surface that asks the user to unlock, rather than being
    /// flattened into "something went wrong" and sent to onboarding — which
    /// would ask somebody with perfectly good mail to set up an account they
    /// already have. Every other keyring failure is a store that will not
    /// open, which is the honest reading: no key, no store.
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

/// How many messages of one conversation are read at once.
///
/// A conversation is bounded where a mailbox is not, so this is a ceiling
/// against the pathological thread rather than a page size — nothing pages
/// through it, and the pane stacks what comes back. 500 is what the GTK
/// frontend uses for the same read.
const THREAD_LIMIT: u32 = 500;

/// How to open a session.
///
/// Not a `uniffi::Record`: it can carry a caller-supplied runtime and command
/// bus, which are Rust types with no crossing. Swift uses the exported
/// constructor on [`Session`] instead, and this is the in-process API that
/// callers and tests use.
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
}

/// What a call against a session with no store answers.
///
/// Its own function because four calls say it, and a store that is not open
/// is a different thing from a part that could not be fetched — a frontend
/// showing "that part is still downloading" for a locked keyring would send
/// the user looking in the wrong place entirely.
fn no_store() -> crate::PartsError {
    crate::PartsError::Refused {
        message: "There is no open store to read that message from".to_owned(),
    }
}

/// What adding an account with no server says. Refused rather than written:
/// an account naming no server fails later, at sync, as a connection error
/// nobody can act on.
const NO_SERVERS: &str = "Postio needs the incoming and outgoing server names — it will not guess them from your address.";

/// The next session's number within this process.
///
/// See [`Session::serial`]. Monotonic and never reused: a session that has
/// closed may still have a hand-off file somebody is editing.
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl SessionOptions {
    /// A session over the store at the platform's usual path.
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
        }
    }

    /// A session over the store at `path`.
    pub fn at(path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            store_path: Some(path.into()),
            ..Self::at_default_path()
        }
    }

    /// Reads the store's key from `secrets` rather than the OS keyring.
    ///
    /// The default is this installation's real keyring, which is right for a
    /// shipping application and wrong for a test: a test that reached for the
    /// login keyring would prompt on a developer's machine and hang on a
    /// headless one. It is also how the locked-keyring path is exercised at
    /// all, since a working keyring cannot be asked to refuse.
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_account::secret::SecretStore>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    /// Read this installation's real `config.toml`, whatever it says.
    ///
    /// The escape hatch for the rare test that is *about* the installed file.
    /// It exists so that reading it is a sentence somebody wrote on purpose:
    /// before #1219 it was the default, and every in-memory session did it
    /// without saying so or meaning to.
    ///
    /// Anything asserting on configured behaviour wants
    /// [`with_config_for_test`](Self::with_config_for_test) instead -- a test
    /// that depends on the machine it runs on has no result, only a mood.
    #[cfg(feature = "testing")]
    pub fn with_installed_config_for_test(mut self) -> Self {
        self.config = ConfigSource::Installed;
        self
    }

    /// A session over a store that exists only in memory.
    ///
    /// Configured by an **empty document**, not by whatever `config.toml` the
    /// machine has. A test that meant the real file says so with
    /// [`with_installed_config_for_test`](Self::with_installed_config_for_test);
    /// every other test gets the built-in defaults wherever it runs, which is
    /// the only way its result means anything (#1219).
    #[cfg(feature = "testing")]
    pub fn in_memory() -> Self {
        Self {
            in_memory: true,
            config: ConfigSource::Document(String::new()),
            ..Self::at_default_path()
        }
    }

    /// An in-memory session over a database the caller already seeded.
    ///
    /// A list test needs rows in the store *before* the session opens it, and
    /// there is no way to reach in afterwards -- the wiring is private, which
    /// is the point of it.
    #[cfg(feature = "testing")]
    pub fn in_memory_with(database: postio_storage::Store) -> Self {
        Self {
            seeded: Some(database),
            ..Self::in_memory()
        }
    }

    /// Use a blob store the caller already wrote bodies into.
    ///
    /// A reader test has to put a body in the blob store *before* the session
    /// opens, and the session otherwise makes its own — so a body written
    /// afterwards would land in a different directory and the reader would
    /// correctly report that there is nothing there.
    #[cfg(feature = "testing")]
    pub fn with_blobs_for_test(
        mut self,
        blobs: postio_storage::BlobStore,
        scratch: tempfile::TempDir,
    ) -> Self {
        self.seeded_blobs = Some((blobs, scratch));
        self
    }

    /// Use this `config.toml` text rather than the one on disk.
    ///
    /// For tests. Reading the developer's own config would make a rebinding
    /// on their machine fail a test on everyone else's.
    ///
    /// Since #1219 this is only needed to supply *content*: an in-memory
    /// session already ignores the installed file, so a test that wants the
    /// built-in defaults need not pass `""` to get them.
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

    /// An in-memory session on a runtime and command bus the caller owns.
    ///
    /// The classic app built its own [`Bridge`] and handed the parts to
    /// [`Wiring`]; a frontend on this boundary must be able to do the same,
    /// or it would end up with two runtimes and the deadlock that implies.
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

    /// Run on a runtime the caller owns, keeping whatever store these
    /// options already name.
    ///
    /// The store's owner (`postio_host::Host`) is adopted onto `runtime`
    /// rather than starting one of its own, and the session is its client
    /// there as it is anywhere. Its verbs are the host's: `commands` is kept
    /// in the host's wiring, but what the session sends goes through its
    /// client, so a verb reaches real handlers whether or not the caller's
    /// bus has any. Before the host, the default bus dropped every command,
    /// and this was the only way a test could make a verb reach real
    /// handlers over real rows (#721).
    pub fn on_bridge(mut self, runtime: tokio::runtime::Handle, commands: CommandSender) -> Self {
        self.bridge = Some((runtime, commands));
        self
    }
}

/// Where a session's configuration comes from.
///
/// Explicit because the absence of a document used to mean "read the
/// developer's own `config.toml`", and `SessionOptions::in_memory()` left it
/// absent (#1219). Every test that opened a session inherited whatever was on
/// the machine running it: green on CI, which has no such file, and red on a
/// workstation according to somebody's preferences. `[ui]` is where it was
/// caught; `[keys]` is where it would have been worse, since one rebinding
/// silently changes what every keyboard test resolves.
///
/// There is no variant meaning "whatever turns up". A session says which.
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
    /// The file this source reads and the session watches, if it is one.
    fn path(&self) -> Option<std::path::PathBuf> {
        match self {
            ConfigSource::Installed => postio_config::paths::config_path().ok(),
            ConfigSource::File(path) => Some(path.clone()),
            ConfigSource::Document(_) => None,
        }
    }
}

/// This session's configuration, whole.
///
/// One function and one read where there were three of each -- `[keys]`,
/// `[sync]` and `[ui]` were three copies differing only in which field they
/// plucked, so a real session opened and parsed `config.toml` three times, and
/// a fix to one of them was a fix to one of them (#1219).
///
/// A config that will not parse is a reason to use the defaults, not a reason
/// the application cannot open: the store and the mail are not downstream of
/// `[keys]`, and refusing to start over a mistyped binding would be a mail
/// client held hostage by its own preferences file.
fn load_config(source: &ConfigSource) -> postio_config::Config {
    let parsed = match source {
        ConfigSource::Document(text) => postio_config::Config::from_toml_str(text).ok(),
        ConfigSource::Installed => postio_config::Config::load().ok(),
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

/// The source `options` asks for.
///
/// Without the `testing` feature there is nothing to ask: a shipping session
/// reads the installed file, and there is no way to hand it a document.
#[cfg(feature = "testing")]
fn config_source(options: &SessionOptions) -> ConfigSource {
    options.config.clone()
}

#[cfg(not(feature = "testing"))]
fn config_source(_options: &SessionOptions) -> ConfigSource {
    ConfigSource::Installed
}

/// Switch Focus's engine on in `host`, as `[focus]` says (specs/009-focus-macos
/// R8): the filing pass, the markers, digests and reminders, which run only
/// once a frontend asks. The same setup the GTK app and the terminal build
/// (`postio_tui::run::engage_focus`), at the same moment: after the host
/// starts and before the first sync, so the filing pass is in every engine
/// before its first pass. The Mac app's syncing starts later, on
/// `start_syncing`.
fn engage_focus(
    host: &Host,
    focus: &postio_config::FocusConfig,
    source: &ConfigSource,
) -> postio_host::FocusHandle {
    let path = source.path();
    host.enable_focus(postio_host::FocusSetup::from_config(
        focus.clone(),
        path.as_deref(),
    ))
}

/// The resolver these bindings make, for the running platform.
///
/// One place, called from both construction paths, because an in-memory
/// session that resolved keys differently from a real one would make every
/// keyboard test a test of the test harness. `Platform::host()` rather than a
/// parameter: this is the *running* application's keymap, and the both-platform
/// assertion belongs where it can be made without opening a session at all
/// (`postio-ui`'s `every_default_binding_resolves_on_both_platforms`).
///
/// Problems are logged, never fatal. An override that cannot be used costs
/// that command its key and nothing else; refusing to open the session over a
/// mistyped `[keys]` entry would be a mail client held hostage by its own
/// preferences file, which is the same call `load_key_bindings` makes above.
fn build_resolver(keys: &postio_config::keys::KeyBindings) -> postio_ui::keymap::Resolver {
    let keymap = postio_core::Keymap::resolve(keys);
    // Focus's commands only: the Mac app is Focus (specs/009-focus-macos
    // FR-001), so a key the one keymap keeps for the terminal alone is bound
    // to nothing here (specs/007-postio-focus R4).
    let (resolver, problems) =
        postio_ui::keymap::Resolver::from_commands_for(&keymap, crate::FRONTEND);
    for problem in &problems {
        tracing::warn!(%problem, "a key binding could not be used");
    }
    resolver
}

/// Start the store's owner over an open store, as ADR 0041 allows here:
/// nothing else on macOS can share the store, so the host runs in this
/// process and this frontend is its one client.
///
/// Over the caller's runtime when it supplied one ([`SessionOptions::on_bridge`]):
/// the host is adopted onto it rather than starting a second, and serves
/// its verbs there. The caller's `CommandSender` stays in the host's wiring,
/// but what this session sends goes through its client to the host's verbs,
/// exactly as when the host owns its runtime -- one path for a command,
/// whoever built the threads.
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

/// What `@` and `+` match against, read through the host.
#[derive(Clone, Default)]
struct FinderSources {
    contacts: Vec<postio_model::Contact>,
    labels: Vec<postio_model::Label>,
}

/// How long the box keeps `FinderSources` before reading them again.
///
/// GTK reads them once, when the box is built. A first sync is when the
/// correspondents arrive, and a box that read them before it would offer
/// none for the whole session; a minute is how stale they may be instead.
const FINDER_SOURCES_FOR: std::time::Duration = std::time::Duration::from_secs(60);

/// What a new account is looked up and signed in through, when a test says.
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

/// The wiring, looking new accounts up the way the desktop's first run
/// does: over the network, with every connection the lookup makes written
/// to the egress log like any other (#151). The wiring's own default
/// transport records nothing, which would make the wizard the one thing on
/// this Mac that dials without saying so.
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

/// A command on its way to the host, with the aim it was issued under.
type Aimed = (postio_core::Command, postio_core::state::SharedState);

/// This session's line to the host: where its commands go, and what keeps
/// its drain fed. Dropped by [`Session::shutdown`], which is what ends both.
struct Link {
    /// The host's client itself, for the requests that are not commands --
    /// looking a new account's servers up, which the host answers with the
    /// desktop's own onboarding.
    client: postio_client::Client,
    /// Commands, in the order they were issued, each with its own aim.
    outbox: async_channel::Sender<Aimed>,
    /// Held only to be dropped: the task feeding the session's event stream
    /// stops when this goes, and the stream ends with it. The client's own
    /// stream closes only once the client has left, and a clone of it may
    /// still be finishing a page read.
    _hearing: async_channel::Sender<()>,
}

/// Connect this frontend to `host` as its client, the way the classic app's
/// window was connected to its own.
///
/// Answers the session's wiring, the stream it drains, and its [`Link`].
/// The wiring is the host's with its `store` -- what the list counts and
/// pages through -- replaced by the client. Everything else (the database
/// for the reads that have no request of their own yet, the blobs, the
/// engines' slot, the hub the engines report to) is the host's own, in this
/// process. Its `commands` are not this session's way to the verbs; the
/// link's outbox is.
///
/// Each command crosses with the aim it was issued under rather than
/// whatever the selection is when the forwarding task gets to it: two
/// verbs pressed in quick succession, the cursor moved between them, must
/// each act on the row it was pressed on.
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

    // Everybody's news, and what this session's own commands said about
    // themselves, which only this client is told.
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

/// The frontend's handle on the engine.
///
/// Commands go down and events come up; nothing else crosses. The `Wiring`
/// lives behind a lock and an `Option` so that [`shutdown`](Self::shutdown)
/// can drop it: dropping the wiring drops the event sink, which ends the
/// stream, which ends the frontend's `while let Some(event)` loop. A drain
/// that never ends is an application that cannot quit.
#[derive(uniffi::Object)]
pub struct Session {
    wiring: Mutex<Option<Wiring>>,
    /// The accounts an aggregate view could show when `Ctrl+A` was pressed.
    ///
    /// The other half of the same predicate: in the unified list a whole-view
    /// selection is about the accounts the view could actually vouch for, and
    /// that set is fixed at the gesture rather than looked up when the verb
    /// runs (#811, ADR 0005 Q10). Empty until a frontend says otherwise,
    /// which makes `Ctrl+A` in the aggregate a rejection rather than an
    /// action over accounts nobody vouched for — the behaviour this boundary
    /// had before the scope could carry them at all.
    reachable: Mutex<Vec<postio_model::ids::AccountId>>,
    /// The message the keyboard is on, as the frontend last reported it
    /// (`set_cursor`): what a verb with nothing marked acts on, until the
    /// controller drives the cursor (specs/009-focus-macos T040).
    cursor: Mutex<Option<postio_model::ids::MessageId>>,
    /// The correspondents and labels the search box's `@` and `+` match
    /// against, with when they were read (`finder_contacts`).
    finder_sources: Mutex<Option<(std::time::Instant, FinderSources)>>,
    /// The conversation the reading pane is showing, once its read lands.
    ///
    /// Held here rather than paged through Focus's list: the list is the
    /// list, and a pane that borrowed the window would have to put the folder
    /// back afterwards. A conversation is bounded — a thread, not a mailbox — so
    /// holding it whole breaks no promise §18 makes.
    conversation: Arc<Mutex<Option<crate::ConversationFfi>>>,
    /// The store this session opened, for anything that has to *name* it —
    /// the composer's footer says where a draft lives, and a footer naming a
    /// path the draft is not in is worse than no footer.
    store_at: std::path::PathBuf,
    /// The sign-in in flight, so closing the sheet can cancel it.
    ///
    /// A token rather than a task handle: cancelling has to unwind the
    /// loopback listener and stop a token exchange from firing for a flow
    /// nobody is waiting on, and `CancelToken` is what the flow already
    /// understands.
    sign_in: Mutex<Option<postio_account::cancel::CancelToken>>,
    /// The port that flow is listening on, or zero.
    sign_in_port: Arc<std::sync::atomic::AtomicU16>,
    /// Where the remote-image grants live: beside the store, because they
    /// are state Postio writes rather than configuration a person edits.
    ///
    /// Resolved once, at open, rather than derived from `store_at` on each
    /// use: an in-memory store has no directory to be beside, and deriving
    /// one gave every test in a run the same file — so a grant made by one
    /// test was in force for the next.
    allow_list_at: std::path::PathBuf,
    /// This session's number within the process.
    ///
    /// Only [`handoff_dir`](Self::handoff_dir) needs it, and only on the
    /// path where there is no store on disk to hang a directory off — but
    /// two sessions sharing one hand-off directory is two people's drafts in
    /// one file, so it is worth a counter.
    serial: u64,
    /// Each message's drawn body in the conversation document, so a redraw
    /// -- a body arriving, a grant -- sanitises only what changed (#1595).
    /// The cache GTK's reader keeps, held here because the Mac's page is
    /// composed on this side.
    thread_renders: Mutex<postio_ui::reader::document::RenderCache>,
    /// Conversation reads still in flight: what `settle_for_test` waits on.
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    /// How many reconnects this session has asked for, so a test can see the
    /// nudge without needing a server to connect to.
    reconnects: Arc<std::sync::atomic::AtomicUsize>,
    /// Whether the engine currently has no connection at all.
    ///
    /// Pushed down by the frontend rather than observed here: reachability is
    /// a platform question, and Swift has `NWPathMonitor` while Rust would
    /// need `unsafe` bindings in a crate that forbids it. It only changes
    /// which *absence* the reader reports — "offline" against "still
    /// downloading" — so being briefly wrong costs a word, not correctness.
    offline: Arc<std::sync::atomic::AtomicBool>,
    /// The engines this session started, kept alive for as long as it is.
    ///
    /// Retained rather than leaked, for the reason the classic app recorded:
    /// dropping an engine at process exit can leave a sync pass's write torn
    /// mid-commit, and the pre-1.0 store engine's recovery is not one to bet
    /// on when waiting for the pass is cheap.
    ///
    /// Keyed by account, so that `start_syncing` can tell an account that is
    /// already running from one that was added while the application was up.
    /// Without the key the only question it could answer was "is *any* engine
    /// running", and under that question a newly added account got none.
    engines: Mutex<Vec<(postio_model::ids::AccountId, postio_runtime::Engine)>>,
    /// `[keys]` as this installation has it.
    ///
    /// Read once at open. A menu accelerator has to reflect what the user
    /// actually bound, and re-reading `config.toml` on every menu draw would
    /// be a file read per repaint.
    keys: Mutex<postio_config::keys::KeyBindings>,
    /// The `[ui]` table this session was opened with — row density, theme and
    /// what the message list draws. Read once here so the list and the
    /// settings pane cannot disagree about what the file says.
    ui: postio_config::ui::UiConfig,
    /// The live keymap: the binding table, plus whatever sequence is
    /// half-typed.
    ///
    /// **Held here, not in Swift** (ADR 0019 Q4). A sequence is state -- `g`
    /// is pending until its second chord or the leader timeout -- and state
    /// the frontend kept would be a second implementation of the trie the
    /// moment either side was edited. So the frontend sends one reduced press
    /// at a time and this remembers what it means.
    ///
    /// Built from the same `[keys]` above, resolved for the running platform,
    /// so `mod+k` is ⌘K here and Ctrl+K on Linux from one table.
    resolver: Mutex<postio_ui::keymap::Resolver>,
    /// The bindings in force, resolved once and kept: see [`Session::keymap`].
    /// Keyed by how many commands the registry holds, so an extension that
    /// registers later is not left out of it.
    keymap: Mutex<Option<(usize, postio_core::Keymap)>>,
    /// Events this boundary raises itself, merged into the drain alongside
    /// the engine's. `FocusPageReady` lives here rather than in
    /// `postio-core` because paging is how this frontend reads a list, not
    /// something the engine does.
    local: (
        async_channel::Sender<UiEvent>,
        async_channel::Receiver<UiEvent>,
    ),
    events: EventStream,
    /// Where commands go, and what keeps [`events`](Self::events) fed from
    /// the host. Taken by [`shutdown`](Self::shutdown), which is what ends
    /// the drain.
    link: Mutex<Option<Link>>,
    /// The verbs the host answers. A command outside them is not sent: the
    /// host would only answer that it is not wired up, and this boundary
    /// has always let such a gesture pass in silence.
    wired: Vec<postio_core::CommandId>,
    /// The store's owner, in this process (ADR 0041): its verbs are what a
    /// command reaches, and its runtime is what every read is polled on.
    /// Kept alive for as long as the session is, since dropping it stops that
    /// runtime -- unless the caller supplied its own, which is not ours to
    /// stop.
    _host: Host,
    /// Focus's engine -- filing, markers, digests, reminders -- switched on
    /// for this host (specs/009-focus-macos R8). Held for as long as the
    /// session is, as the GTK app and the terminal hold theirs.
    _focus: postio_host::FocusHandle,
    /// `[focus]` as it stands: what the strip's words depend on (filtering
    /// on, how many digest rules), kept current by `follow_config`.
    focus_config: Mutex<postio_config::FocusConfig>,
    /// `config.toml`, watched while the session lives: a change to `[keys]`
    /// rebinds at once and a change to `[focus]` reaches the engine, as in
    /// the GTK app (`follow_config`). `None` for a session given a document
    /// rather than a file, or when the file cannot be watched.
    config_watch: Mutex<Option<postio_config::watch::ConfigWatcher>>,
    /// Focus's list, driven by the controller (specs/009-focus-macos T027).
    focus_list: Arc<crate::focus_list::FocusDriver>,
    /// The in-memory blob directory, removed when the session is dropped.
    #[cfg(feature = "testing")]
    _scratch: Option<tempfile::TempDir>,
}

/// The surface Swift sees.
///
/// Deliberately narrower than the Rust `impl` below it. `SessionOptions` can
/// carry a caller-supplied runtime and command bus, which are Rust types with
/// no crossing, so the exported constructor takes the one thing a frontend
/// actually chooses — where the store lives — and everything else is decided
/// on this side.
// ---------------------------------------------------------------------------
// The exported surface. EVERYTHING IN THIS BLOCK CROSSES TO SWIFT.
//
// A plain Rust method added here is exported too, and the failure is
// bewildering: uniffi generates scaffolding for it, and a method the rest of
// the crate calls normally reports "not found for struct `Arc<Session>`" *at
// its own definition*. Test-only methods behind `#[cfg(feature = "testing")]`
// produce exactly that when the feature is off.
//
// Rust-side methods go in the second `impl Session` block, below.
// ---------------------------------------------------------------------------
#[uniffi::export]
impl Session {
    /// Opens a session over the store at `store_path`, or the usual path.
    #[uniffi::constructor]
    pub fn open_at(store_path: Option<String>) -> Result<Arc<Self>, SessionError> {
        Self::open(match store_path {
            Some(path) => SessionOptions::at(path),
            None => SessionOptions::at_default_path(),
        })
    }

    /// The next event, or `None` once the session has stopped.
    ///
    /// Swift drives this as
    /// `Task { @MainActor in while let e = await session.nextEvent() { … } }`
    /// — the same drain the GTK window runs on the main context, so no
    /// backend work reaches the UI thread on either platform.
    #[uniffi::method(name = "nextEvent")]
    pub async fn next_event_ffi(&self) -> Option<UiEvent> {
        self.next_event().await
    }

    /// Drops the store and ends the event drain. Idempotent.
    #[uniffi::method(name = "shutdown")]
    pub fn shutdown_ffi(&self) {
        self.shutdown();
    }

    /// Read a conversation into the reading pane.
    ///
    /// Returns at once; `ConversationReady` says when there is something to
    /// draw, and [`conversation`](Self::conversation_ffi) is what to draw.
    /// The list is untouched — a conversation is what the *pane* is showing,
    /// and the list stays the list (#1003).
    #[uniffi::method(name = "openConversation")]
    pub fn open_conversation_ffi(&self, thread: i64) {
        self.open_conversation(thread);
    }

    /// `thread` as one document, with each message's anchor. See
    /// [`Session::thread_document`].
    ///
    /// Blocks on the store -- one body load per message -- so off the main
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

    /// A new message, from the account that would send it.
    #[uniffi::method(name = "newDraft")]
    pub fn new_draft_ffi(&self) -> Option<crate::DraftFfi> {
        blocking(self.new_draft())
    }

    /// A draft prefilled from a `mailto:` link.
    ///
    /// The recipients arrive already parsed — RFC 6068 is the platform's URL
    /// machinery to unpick, and each frontend has one. What is *not* the
    /// platform's is what a mail client does with the result, which is this.
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

    /// A reply to `message` — to its sender, or to everyone on it.
    #[uniffi::method(name = "replyDraft")]
    pub fn reply_draft_ffi(&self, message: i64, all: bool) -> Option<crate::DraftFfi> {
        blocking(self.reply_draft(message, all))
    }

    /// A forward of `message`, addressed to nobody yet.
    #[uniffi::method(name = "forwardDraft")]
    pub fn forward_draft_ffi(&self, message: i64) -> Option<crate::DraftFfi> {
        blocking(self.forward_draft(message))
    }

    /// Attach `path` to the draft and answer it with the file on it.
    ///
    /// `mime_type` comes from the frontend because sniffing a file's type is
    /// a platform service — `UniformTypeIdentifiers` here, shared-mime-info
    /// there — and everything that is not platform-specific happens on the
    /// other side of this call.
    #[uniffi::method(name = "attachToDraft")]
    pub fn attach_to_draft_ffi(
        &self,
        draft: crate::DraftFfi,
        path: String,
        mime_type: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.attach_to_draft(draft, path, mime_type))
    }

    /// Put a picture into the draft's body and answer the draft with it, and
    /// the script that draws it at the caret (#1571).
    ///
    /// Bytes rather than a path, because a picture arrives from a paste as
    /// often as from a file. `mime_type` is the frontend's for
    /// [`attach_to_draft_ffi`](Self::attach_to_draft_ffi)'s reason; that it
    /// must be an image is decided on this side.
    #[uniffi::method(name = "insertInlineImage")]
    pub fn insert_inline_image_ffi(
        &self,
        draft: crate::DraftFfi,
        bytes: Vec<u8>,
        mime_type: String,
    ) -> Result<crate::InlineImageFfi, crate::ComposeError> {
        blocking(self.insert_inline_image(draft, bytes, mime_type))
    }

    /// One picture in draft `draft`'s own body, by its `Content-ID`.
    ///
    /// What the composer's `postio-cid:` handler answers with. Scoped to the
    /// draft for the reason [`resolve_cid_ffi`](Self::resolve_cid_ffi) is
    /// scoped to a message: an id means something only inside the message
    /// that declared it. `nil` is a broken picture, never a fetch.
    #[uniffi::method(name = "resolveDraftCid")]
    pub fn resolve_draft_cid_ffi(
        &self,
        draft: i64,
        content_id: String,
    ) -> Option<crate::InlinePart> {
        blocking(self.resolve_draft_cid(draft, content_id))
    }

    /// Write the draft where another editor can open it, and answer where.
    ///
    /// The file is the user's alone — a private directory, mode 0600 — for
    /// the reason `postio_session::handoff` records: a draft is mail that has
    /// not been sent, which is often the most private mail there is.
    #[uniffi::method(name = "beginHandoff")]
    pub fn begin_handoff_ffi(&self, draft: crate::DraftFfi) -> Result<String, crate::ComposeError> {
        blocking(self.begin_handoff(draft))
    }

    /// Take back what the other editor wrote, and answer the draft.
    #[uniffi::method(name = "endHandoff")]
    pub fn end_handoff_ffi(
        &self,
        draft: crate::DraftFfi,
        path: String,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.end_handoff(draft, path))
    }

    /// Take an attachment off a draft again.
    #[uniffi::method(name = "detachFromDraft")]
    pub fn detach_from_draft_ffi(
        &self,
        draft: crate::DraftFfi,
        attachment: i64,
    ) -> Result<crate::DraftFfi, crate::ComposeError> {
        blocking(self.detach_from_draft(draft, attachment))
    }

    /// The draft `id` as the store has it. See [`Session::draft`].
    ///
    /// `nil` when there is no such draft -- a composer reopened on a draft
    /// somebody deleted elsewhere gets nothing rather than a blank one.
    #[uniffi::method(name = "draft")]
    pub fn draft_ffi(&self, id: i64) -> Option<crate::DraftFfi> {
        blocking(self.draft(id))
    }

    /// The draft behind message `message`, for a `Continue editing` link in
    /// a conversation (#1212). See [`Session::draft_for_message`].
    #[uniffi::method(name = "draftForMessage")]
    pub fn draft_for_message_ffi(&self, message: i64) -> Option<crate::DraftFfi> {
        blocking(self.draft_for_message(message))
    }

    /// Narrow pasted markup to the dialect. See [`Session::narrow_paste`].
    #[uniffi::method(name = "narrowPaste")]
    pub fn narrow_paste_ffi(&self, html: String) -> crate::PastedFfi {
        self.narrow_paste(html)
    }

    /// The plain text of a rich body. See [`Session::plain_text_of`].
    #[uniffi::method(name = "plainTextOf")]
    pub fn plain_text_of_ffi(&self, html: String) -> String {
        self.plain_text_of(&html)
    }

    /// The script that applies a mark to the composer's selection.
    ///
    /// `nil` for a command that is not one of the marks. See
    /// [`Session::mark_script`].
    #[uniffi::method(name = "markScript")]
    pub fn mark_script_ffi(&self, command: String) -> Option<String> {
        self.mark_script(&command)
    }

    /// The script that links the selection to `href`, or `nil` when a
    /// message may not point there. See [`Session::link_script`].
    #[uniffi::method(name = "linkScript")]
    pub fn link_script_ffi(&self, href: String) -> Option<String> {
        self.link_script(&href)
    }

    /// The one script the composer's editing surface runs.
    ///
    /// See [`Session::editor_script`]. Crosses so that both frontends run
    /// one dialect rather than two that happen to agree today.
    #[uniffi::method(name = "editorScript")]
    pub fn editor_script_ffi(&self) -> String {
        self.editor_script().to_owned()
    }

    /// Write the draft to the store, and answer it with its id.
    /// Throw a draft away — the row and the server copy.
    #[uniffi::method(name = "discardDraft")]
    pub fn discard_draft_ffi(&self, draft: i64) -> Option<String> {
        blocking(self.discard_draft(draft))
    }

    /// Write the draft to the store, and answer it with its id.
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

    /// Queue a draft to leave at `when`. See
    /// [`Session::send_draft_later`].
    #[uniffi::method(name = "sendDraftLater")]
    pub fn send_draft_later_ffi(&self, draft: crate::DraftFfi, when: i64) -> Option<String> {
        blocking(self.send_draft_later(draft, when))
    }

    /// Everything the pane asks about one open message — the notice, the
    /// caveat, the unsubscribe offer and the recipients — in one call, one
    /// body load and one render (#1589). Blocking; call it off the main
    /// actor and publish the answer.
    #[uniffi::method(name = "messageFacts")]
    pub fn message_facts_ffi(&self, message: i64) -> crate::MessageFactsFfi {
        blocking(self.message_facts(message))
    }

    /// The verbs the reading pane offers. See [`Session::reader_actions`].
    #[uniffi::method(name = "readerActions")]
    pub fn reader_actions_ffi(&self) -> Vec<crate::ReaderActionFfi> {
        self.reader_actions()
    }

    /// The header bar of a conversation of `messages`, each verb saying
    /// what it will act on. See [`Session::conversation_actions`].
    #[uniffi::method(name = "conversationActions")]
    pub fn conversation_actions_ffi(&self, messages: u32) -> Vec<crate::ConversationActionFfi> {
        self.conversation_actions(messages)
    }

    /// Always allow this address's remote images, across restarts.
    #[uniffi::method(name = "allowSender")]
    pub fn allow_sender_ffi(&self, address: String) {
        self.allow_sender(address);
    }

    /// `always_show_images` from the keyboard or the palette: what the
    /// notice's "Always allow" presses, when the notice is up. See
    /// [`Session::always_show_images_for`].
    ///
    /// **Off the main actor**: it renders the message to learn whether
    /// anything was held back, the same render the notice comes from.
    #[uniffi::method(name = "alwaysShowImagesFor")]
    pub fn always_show_images_for_ffi(&self, message: i64) -> bool {
        blocking(self.always_show_images_for(message))
    }

    /// Always allow every address at this domain.
    #[uniffi::method(name = "allowDomain")]
    pub fn allow_domain_ffi(&self, domain: String) {
        self.allow_domain(domain);
    }

    /// Every remote-image grant, addresses first, each already sorted.
    ///
    /// The Privacy pane's whole model. A grant the user cannot see is one
    /// they cannot take back, and "images blocked until allowed per sender"
    /// only means something if *allowed* is reviewable.
    #[uniffi::method(name = "remoteImageGrants")]
    pub fn remote_image_grants_ffi(&self) -> Vec<crate::GrantFfi> {
        self.remote_image_grants()
    }

    /// Take a grant back. Images from it are blocked again at once.
    #[uniffi::method(name = "revokeRemoteImages")]
    pub fn revoke_remote_images_ffi(&self, subject: String) {
        self.revoke_remote_images(subject);
    }

    /// Leave the list this message came from — **the deliberate activation**,
    /// and the only thing in this boundary that records one.
    ///
    /// Call it from a button and from nothing else. `None` when the
    /// activation was recorded, a sentence when it was not; a message that
    /// offers nothing is refused rather than logged, so a frontend cannot
    /// unsubscribe anyone from a message the reader never offered it on.
    /// See [`Session::activate_unsubscribe`].
    ///
    /// **Off the main actor**, unlike the offer above it. This is the only
    /// call in the pair that writes, and a write waits on the store's
    /// machine-wide gate — so it queues behind whatever the sync engine is
    /// committing, which on a first sync is not a few milliseconds. The
    /// offer, `decodeCaveat` and everything else the reading pane asks per
    /// message are point reads and belong where the message opens; this one
    /// belongs in a task, with the banner left as it is until it answers.
    #[uniffi::method(name = "activateUnsubscribe")]
    pub fn activate_unsubscribe_ffi(&self, message: i64) -> Option<String> {
        blocking(self.activate_unsubscribe(message))
    }

    /// Every activation this store holds, newest first — the Privacy pane's
    /// list. See [`Session::unsubscribe_activations`].
    #[uniffi::method(name = "unsubscribeActivations")]
    pub fn unsubscribe_activations_ffi(&self) -> Vec<crate::UnsubscribeActivationFfi> {
        blocking(self.unsubscribe_activations())
    }

    /// What looking `address` up finds: the first-run card (canvas 09).
    ///
    /// Asked on a deliberate step -- the address field losing focus, or
    /// `Connect` -- never per keystroke: each lookup is DNS and HTTPS to the
    /// address's domain, and each is written to the egress log. Blocks for
    /// as long as the lookup takes, bounded by the probe's own deadlines, so
    /// not from the main actor.
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

    /// Whether `path` is a mail store Postio can open: `None` when it is, a
    /// sentence naming the directory when it is not.
    ///
    /// Asked while somebody is still looking at the field, so the sheet can
    /// say what is wrong with the directory before it is written down —
    /// rather than adding an account that turns out to have no mail in it.
    #[uniffi::method(name = "inspectLocalStore")]
    pub fn inspect_local_store_ffi(&self, path: String) -> Option<String> {
        self.inspect_local_store(path)
    }

    /// Sign in to `address` through the system browser, and add the account.
    ///
    /// Returns when the flow is over: `None` on success, a sentence on
    /// failure, and `Some("cancelled")`'s own wording when the user closed
    /// the tab. Blocks — it is waiting on a person — so a Swift caller runs
    /// it off the main actor, the way it opens a session.
    #[uniffi::method(name = "signInWithBrowser")]
    pub fn sign_in_with_browser_ffi(
        &self,
        address: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Option<String> {
        blocking(self.sign_in_with_browser(address, client_id, client_secret))
    }

    /// What the sign-in in flight is doing — the port, mostly.
    #[uniffi::method(name = "signInProgress")]
    pub fn sign_in_progress_ffi(&self) -> crate::SignInProgressFfi {
        self.sign_in_progress()
    }

    /// Give up on the sign-in in flight. Closing the sheet means this.
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

    /// Rebuild `account`'s search index from the mail already in the store.
    #[uniffi::method(name = "reindexAccount")]
    pub fn reindex_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.reindex_account(account))
    }

    /// How much disk this account's mail takes, in the canvas' words.
    #[uniffi::method(name = "accountWeight")]
    pub fn account_weight_ffi(&self, account: i64) -> Option<String> {
        blocking(self.account_weight(account))
    }

    /// Change what an account calls itself — the one field the account form
    /// edits.
    #[uniffi::method(name = "setDisplayName")]
    pub fn set_display_name_ffi(&self, account: i64, name: String) -> Option<String> {
        blocking(self.set_display_name(account, name))
    }

    /// Switch an account's syncing on or off. See
    /// [`set_account_enabled`](Self::set_account_enabled).
    #[uniffi::method(name = "setAccountEnabled")]
    pub fn set_account_enabled_ffi(&self, account: i64, enabled: bool) -> Option<String> {
        blocking(self.set_account_enabled(account, enabled))
    }

    /// Make an account the one new mail comes from. See
    /// [`set_default_account`](Self::set_default_account).
    #[uniffi::method(name = "setDefaultAccount")]
    pub fn set_default_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.set_default_account(account))
    }

    /// Take an account away — its row, and its credentials.
    #[uniffi::method(name = "removeAccount")]
    pub fn remove_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.remove_account(account))
    }

    /// What one key press means here. See [`Session::key`].
    ///
    /// The frontend reduces its own event to these three things and asks;
    /// it owns no keymap (ADR 0019 Q4). The answer says whether to swallow
    /// the key: a command and a pending sequence are handled, and only
    /// `Unhandled` may reach the toolkit.
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

    /// Run a command, aimed the way this view says it should be.
    ///
    /// `id` is the registry's own name for the verb, as
    /// [`commands`](Session::commands_ffi) reports it. Nothing comes back:
    /// a verb is local-first, and what happened arrives on `nextEvent` like
    /// everything else. See [`Session::invoke`].
    #[uniffi::method(name = "invoke")]
    pub fn invoke_ffi(&self, id: String) {
        self.invoke(&id);
    }

    /// Whether `id` can run in `context`. See [`Session::is_available`].
    ///
    /// What a menu asks before drawing an item enabled.
    #[uniffi::method(name = "isAvailable")]
    pub fn is_available_ffi(&self, id: String, context: crate::UiContext) -> bool {
        self.is_available(&id, context)
    }

    /// The palette's rows for `query`. See [`Session::palette_entries`].
    #[uniffi::method(name = "paletteEntries")]
    pub fn palette_entries_ffi(
        &self,
        query: String,
        context: crate::UiContext,
    ) -> Vec<crate::PaletteEntryFfi> {
        self.palette_entries(&query, context)
    }

    /// Every command reachable here, with the binding in force.
    ///
    /// The same list the palette reads, unfiltered — see
    /// [`Session::cheat_sheet`].
    #[uniffi::method(name = "cheatSheet")]
    pub fn cheat_sheet_ffi(&self, context: crate::UiContext) -> Vec<crate::PaletteEntryFfi> {
        self.cheat_sheet(context)
    }

    /// The `?` sheet, grouped the way the product groups it.
    /// See [`Session::cheat_sheet_sections`].
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

    /// Put `label` on the selection. See [`apply_label`](Self::apply_label).
    #[uniffi::method(name = "applyLabel")]
    pub fn apply_label_ffi(&self, label: i64) {
        self.apply_label(label);
    }

    /// Report which row the keyboard is on, or `None` for no row.
    #[uniffi::method(name = "setCursor")]
    pub fn set_cursor_ffi(&self, message: Option<i64>) {
        self.set_cursor(message);
    }

    /// Report which accounts the aggregate view can currently vouch for.
    ///
    /// Call it whenever a connection changes, from the same states the
    /// "showing local mail" disclosure is drawn from: it is what a whole-view
    /// selection in the unified list is scoped to, and it is read when the
    /// selection is *made* rather than when a verb runs (#811).
    #[uniffi::method(name = "setReachableAccounts")]
    pub fn set_reachable_accounts_ffi(&self, accounts: Vec<i64>) {
        self.set_reachable_accounts(&accounts);
    }

    /// The whole document for a message, ready to hand a `WKWebView` — plus
    /// the notice and the caveat the render already paid for (#1589).
    ///
    /// Swift's job is to build a hardened configuration, hand the HTML over,
    /// refuse navigations, and publish the two facts. It composes no reader
    /// HTML of its own.
    #[uniffi::method(name = "readerDocument")]
    pub fn reader_document_ffi(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        reduced: bool,
    ) -> crate::ReaderDocumentFfi {
        blocking(self.reader_answers(message, remote, reduced))
    }

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// What a `WKURLSchemeHandler` for `postio-cid:` answers with. `nil` is a
    /// broken image, deliberately — never a fetch.
    #[uniffi::method(name = "resolveCid")]
    pub fn resolve_cid_ffi(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        blocking(self.resolve_cid(message, content_id))
    }

    /// One part's bytes, fetched first if they are not on this machine yet.
    ///
    /// **A deliberate act, and the only call here that may reach the
    /// network.** The user pressed save, or dragged this part, or chose
    /// "Open with…" on it — they named these bytes. Never call it to fill a
    /// preview or to find out how big something really is; the whole
    /// arrangement above depends on this being the one door and it being
    /// opened on purpose.
    ///
    /// `partId` is the MIME path from `PartFfi.partId`. An error carries the
    /// sentence to show the person who asked; there is never an empty
    /// success, because a zero-byte file looks like a saved attachment.
    ///
    /// **Never from the main actor.** This is the one call in the parts
    /// surface that can reach the network, so it is also the one that can
    /// take real time: a part nobody has downloaded is queued and then
    /// *waited on*, for up to thirty seconds, before the wait gives up and
    /// says so. Every other blocking method here reads SQLite and answers in
    /// milliseconds; this one answers at the speed of somebody's IMAP server.
    /// Call it from a task and publish the result.
    #[uniffi::method(name = "partBytes")]
    pub fn part_bytes_ffi(
        &self,
        message: i64,
        part_id: String,
    ) -> Result<Vec<u8>, crate::PartsError> {
        blocking(self.part_bytes(message, part_id))
    }

    /// Tell the engine whether the machine currently has a connection.
    ///
    /// Pushed down from Swift's `NWPathMonitor`: reachability is a platform
    /// question, and the platform's own language is where it gets asked.
    #[uniffi::method(name = "setOffline")]
    pub fn set_offline_ffi(&self, offline: bool) {
        self.set_offline(offline);
    }

    /// Whether the platform has told us there is no connection.
    #[uniffi::method(name = "isOffline")]
    pub fn is_offline_ffi(&self) -> bool {
        self.is_offline()
    }

    /// Start syncing every configured account; answers how many started.
    ///
    /// Zero is not an error — a store with no account configured is the
    /// ordinary first-run state. Does not block: the connection attempt
    /// happens on the engine's own runtime.
    #[uniffi::method(name = "startSyncing")]
    pub fn start_syncing_ffi(&self) -> Result<u32, SessionError> {
        blocking(self.start_syncing())
    }

    /// How many accounts are configured and enabled.
    #[uniffi::method(name = "configuredAccounts")]
    pub fn configured_accounts_ffi(&self) -> u32 {
        blocking(self.configured_accounts())
    }

    /// Every folder of every enabled account, for the sidebar.
    #[uniffi::method(name = "mailboxes")]
    pub fn mailboxes_ffi(&self) -> Vec<crate::MailboxFfi> {
        blocking(self.mailboxes())
    }

    /// Every configured account, in the order the pane lists them.
    ///
    /// Synchronous at the boundary like `mailboxes`: the settings pane reads
    /// it from a computed property, and an async crossing for a handful of
    /// rows would push a `Task` into every caller. See [`Session::accounts`].
    ///
    /// **Not from the main actor once an OAuth account exists.** Each one
    /// costs a keyring read, to learn whether its token has expired, and a
    /// Keychain that decides to ask the user about it blocks until they
    /// answer. Read it in a task and publish the rows.
    #[uniffi::method(name = "accounts")]
    pub fn accounts_ffi(&self) -> Vec<crate::AccountFfi> {
        blocking(self.accounts())
    }

    /// Give an account a new password: the repair `AccountFfi.repair` calls
    /// `Password`. `None` when it was stored, a sentence when it was not.
    ///
    /// For the two states that leave an account unable to sign in without
    /// anything being wrong with its row — a provider that rotated its app
    /// password, and a row whose keyring entry never arrived or was removed
    /// (the pane's *Partial* state). This writes the keyring and nothing
    /// else: no server is asked, and the row is left as it is.
    ///
    /// Blocks on the keyring; run it off the main actor.
    #[uniffi::method(name = "repairCredential")]
    pub fn repair_credential_ffi(&self, account: i64, password: String) -> Option<String> {
        blocking(self.repair_credential(account, password))
    }

    /// Sign an account in again through the system browser: the repair
    /// `AccountFfi.repair` calls `Browser`. `None` when the grant was
    /// renewed, a sentence when it was not.
    ///
    /// Takes no client id, unlike `signInWithBrowser`. The account already
    /// carries the one it registered and the keyring carries its secret, so
    /// Reconnect is one press rather than a form asking somebody to find a
    /// credential again in order to fix an account that used to work.
    ///
    /// Returns when the flow is over, which is when a person comes back from
    /// a browser tab: run it off the main actor.
    #[uniffi::method(name = "reconnectAccount")]
    pub fn reconnect_account_ffi(&self, account: i64) -> Option<String> {
        blocking(self.reconnect_account(account))
    }

    /// The binding in force for a command, for drawing a native accelerator.
    #[uniffi::method(name = "bindingFor")]
    pub fn binding_for_ffi(&self, command: String) -> Option<String> {
        self.binding_for(command)
    }

    /// Every binding in force for a command, the primary first. See
    /// [`bindings_for`](Self::bindings_for).
    #[uniffi::method(name = "bindingsFor")]
    pub fn bindings_for_ffi(&self, command: String) -> Vec<String> {
        self.bindings_for(command)
    }

    /// Every command the registry knows, in cheat-sheet order.
    #[uniffi::method(name = "commands")]
    pub fn commands_ffi(&self) -> Vec<crate::CommandSpecFfi> {
        self.commands()
    }

    /// Whether this session still holds its store.
    #[uniffi::method(name = "isOpen")]
    pub fn is_open_ffi(&self) -> bool {
        self.is_open()
    }
    /// The `[ui]` table this session was opened with.
    ///
    /// The message list draws by these: row height comes from the density,
    /// and the three flags say what a row shows. Read from the session rather
    /// than from the file by whoever is drawing, so the list and the settings
    /// pane cannot end up with two different opinions of the same table.
    pub fn appearance(&self) -> crate::AppearanceFfi {
        crate::AppearanceFfi {
            density: self.ui.density.into(),
            theme: self.ui.theme.into(),
            show_hover_actions: self.ui.show_hover_actions,
            sender_avatars: self.ui.sender_avatars,
        }
    }
    /// The key hints the search bar announces — `Ret open · Tab refine ·
    /// C-s save as folder` (canvas 05).
    ///
    /// Read from this session's keymap, so a rebinding
    /// reaches the footer. It is the only place most people will ever read
    /// these keys, which is what makes teaching the wrong one worse than
    /// teaching none.
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

/// What resuming an account's browser sign-in needs, resolved off its row.
///
/// Does not cross to Swift, and must not: it carries a client secret, and
/// the frontend has nothing to do with one. `reconnectAccount` is what
/// crosses — a button press, answered with a sentence or with nothing.
/// This is the join behind it, public so a test can assert that the right
/// client reaches the provider rather than an empty one (#1584).
#[derive(Debug)]
pub struct BrowserReconnect {
    /// The account's own address, which is what the consent screen shows.
    pub address: String,
    /// The OAuth client the first sign-in registered, off the account row.
    pub client_id: String,
    /// Its secret, when the provider issued one, out of the keyring.
    pub client_secret: Option<postio_account::secret::Password>,
}

// ---------------------------------------------------------------------------
// The Rust surface. Nothing here crosses to Swift; the block above wraps what
// should. Test-only methods belong here.
// ---------------------------------------------------------------------------
impl Session {
    /// Every configured account, in the order the pane lists them.
    ///
    /// Disabled ones included: a list that hid them would make "where did my
    /// account go" the next question. An empty answer means no store, which
    /// on a machine that has never signed in is exactly the claim.
    ///
    /// # Why this reaches the keyring
    ///
    /// A row has to say when a token has expired, and the expiry lives in
    /// the keyring beside the token it is about — `config.toml` strips
    /// anything token-shaped on the way through, which is the whole reason
    /// it is there (#870). So the row cannot be assembled from the store
    /// alone, and the classic app reached for exactly the same value the same
    /// way before handing it to its panel's `set_token_expiries`.
    ///
    /// **Only for an account that has one.** The filter is
    /// `account.oauth.is_some()`, which is the only case anything ever
    /// persisted an expiry under — so a machine with none of them makes no
    /// keyring call here at all, and one with a single Gmail account makes
    /// one. It is still a round trip that can block, which is why the
    /// exported wrapper says to keep this off the main actor.
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
            // `None` all the way through for everything that is not an
            // OAuth account of Postio's own minting, which is what
            // `TokenStanding::Unknown` means and is a real answer rather
            // than a failed read.
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

    /// Put a new password in the keyring for an account that is already
    /// here. See [`repair_credential_ffi`](Self::repair_credential_ffi).
    ///
    /// `None` when the credential was stored, a sentence when it was not.
    ///
    /// The route `AccountFfi::repair` names as `Password`: the keyring alone,
    /// for an account whose row is right. `postio_session::provision::repair`'s
    /// own docs argue why that is shared with the headless helper.
    ///
    /// The password goes to the OS keyring and nowhere else — never
    /// `config.toml`, never a log (ADR 0014). It is not echoed back in the
    /// answer either: everything below names the account, never the secret.
    pub async fn repair_credential(&self, account: i64, password: String) -> Option<String> {
        // Refused here rather than stored. An empty secret is worse than no
        // secret: every later "is this account signed in?" question reads it
        // as one, so the *Partial* state that says "put a credential in"
        // would be unreachable for exactly the account that had been through
        // this field.
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

    /// What reconnecting `account` in a browser would sign in with, or why
    /// it cannot be reconnected.
    ///
    /// The route `AccountFfi::repair` names as `Browser`, and the same
    /// accounts: an OAuth client on the row is what makes a reconnect
    /// possible at all, so this refuses exactly where the row offers
    /// nothing. Two answers that could disagree would put a button on a row
    /// it does not work on.
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

    /// Sign this account in again, with the client it signed in with before.
    /// See [`reconnect_account_ffi`](Self::reconnect_account_ffi).
    ///
    /// Everything about the flow is [`sign_in_with_browser`](Self::sign_in_with_browser)'s
    /// — the consent, the loopback port, the proof that the token opens the
    /// account's real IMAP session before anything is written. The only
    /// difference is where the client comes from, and that is the whole
    /// feature: a person whose grant has lapsed presses one button.
    ///
    /// `provision_oauth` seeds the keyring before it notices the row is
    /// already there, which is what makes a re-consent land rather than be
    /// thrown away (#1584's first half).
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

    /// Opens a session, or says why it could not.
    ///
    /// # This blocks
    ///
    /// Reading the store's key from the OS keyring is a synchronous round
    /// trip that can wait on a user prompt, and it has to finish before there
    /// is a store — so this blocks the calling thread, bounded by the
    /// keyring's own timeout rather than indefinitely. The classic app did the
    /// same thing before any window exists. **A Swift caller must not invoke
    /// it on the main actor**: it belongs in a launch task, with the unlock
    /// surface shown if it comes back [`SessionError::KeyringLocked`].
    pub fn open(options: SessionOptions) -> Result<Arc<Self>, SessionError> {
        // Read before anything moves out of `options`, and once: both paths
        // below build the same configuration from it.
        let source = config_source(&options);
        let seams = seams(&options);
        let caller = options.bridge;

        #[cfg(feature = "testing")]
        if options.in_memory {
            let database = match options.seeded {
                Some(database) => database,
                None => {
                    // A fresh key per session, from the OS RNG. The database
                    // lives and dies with this process, so there is nothing
                    // to reopen it with later — and it still runs the
                    // encrypted path, which is the whole point of ADR 0014
                    // Q3's "nothing tests a plaintext configuration that no
                    // longer ships". No keyring is touched.
                    //
                    // A file in a temporary directory rather than an
                    // in-memory database: the engine refuses to key one
                    // (research.md Q6), and `test_support::memory` has been a
                    // file on /dev/shm since #204 for a separate reason.
                    // Neither survives the process, which is what "in memory"
                    // meant to a caller of this.
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
            // The default secret store is left in place. It is the real
            // keyring type, but it does not reach the keyring until something
            // asks it for a secret, and nothing in this slice does — so an
            // in-memory session still needs no Secret Service, no Keychain and
            // no prompt. The moment a slice *does* read a secret, this is
            // where a `MemorySecretStore` goes.
            let config = load_config(&source);
            let sync_config = config.sync;
            // Honour `with_secrets` here too. It was read only on the real
            // path, so an in-memory session that had been handed a test
            // keyring quietly used the **login keychain** instead — which is
            // how a test suite came to hang on a macOS permission prompt
            // nobody could see, after writing a password into a developer's
            // own keychain.
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
            let session = Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
                resolver: Mutex::new(build_resolver(&keys)),
                keymap: Mutex::new(None),
                ui: config.ui,
                keys: Mutex::new(keys),
                reachable: Mutex::new(Vec::new()),
                cursor: Mutex::new(None),
                finder_sources: Mutex::new(None),
                conversation: Arc::default(),
                sign_in: Mutex::new(None),
                sign_in_port: Arc::default(),
                store_at: std::path::PathBuf::from(":memory:"),
                // A file of its own per session: an in-memory store has no
                // directory to sit beside, and one shared path made a grant
                // from one test true for the next.
                allow_list_at: std::env::temp_dir().join(format!(
                    "postio-allowed-{}-{}.toml",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|since| since.as_nanos())
                        .unwrap_or_default()
                )),
                serial: next_serial(),
                thread_renders: Mutex::new(postio_ui::reader::document::RenderCache::default()),
                in_flight: Arc::default(),
                reconnects: Arc::default(),
                offline: Arc::default(),
                engines: Mutex::new(Vec::new()),
                local: local.clone(),
                events,
                link: Mutex::new(Some(link)),
                wired: host.wired(),
                _focus: engage_focus(&host, &config.focus, &source),
                focus_config: Mutex::new(config.focus.clone()),
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
            return Ok(session);
        }

        // The keyring first, and only then the store. ADR 0014: the store is
        // encrypted under a key that lives in the OS keyring, and there is no
        // "open it unencrypted anyway" — so a keyring that will not answer
        // means there is no store to open, not a store to open differently.
        // Asking in this order is what makes that true rather than merely
        // intended: nothing has touched the database by the time the key is
        // refused, so a locked keyring leaves no half-made store behind.
        let secrets: Arc<dyn postio_account::secret::SecretStore> = match options.secrets {
            Some(secrets) => secrets,
            None => postio_account::secret::platform_keyring(),
        };
        let key = postio_session::store_key_blocking(secrets.as_ref())
            .map_err(SessionError::from_secret_error)?;

        let path = options
            .store_path
            .unwrap_or_else(postio_session::paths::store_path);
        // Kept before `path` is moved: the composer's footer names where a
        // draft lives, and the remote-image grants are written beside the
        // store rather than into `config.toml`.
        let store_at = path.clone();
        // Blocked on here, before the host exists: opening the store is
        // async, and this constructor's documented contract is that it
        // blocks. That is what the sentence above about the keyring is
        // already telling a Swift caller -- do not invoke this on the main
        // actor -- and it covers the store open for exactly the same reason.
        let (database, blobs) =
            blocking(postio_session::open_store_at_reporting(path, &key, &|_| {}))
                .map_err(SessionError::from_refusal)?;

        let config = load_config(&source);
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
        let session = Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            resolver: Mutex::new(build_resolver(&keys)),
            keymap: Mutex::new(None),
            ui: ui_config,
            keys: Mutex::new(keys),
            engines: Mutex::new(Vec::new()),
            reachable: Mutex::new(Vec::new()),
            cursor: Mutex::new(None),
            finder_sources: Mutex::new(None),
            conversation: Arc::default(),
            sign_in: Mutex::new(None),
            sign_in_port: Arc::default(),
            allow_list_at: store_at
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(&store_at)
                .join("allowed-senders.toml"),
            store_at,
            serial: next_serial(),
            thread_renders: Mutex::new(postio_ui::reader::document::RenderCache::default()),
            in_flight: Arc::default(),
            reconnects: Arc::default(),
            offline: Arc::default(),
            local: local.clone(),
            events,
            link: Mutex::new(Some(link)),
            wired: host.wired(),
            _focus: engage_focus(&host, &focus_config, &source),
            focus_config: Mutex::new(focus_config.clone()),
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
        Ok(session)
    }

    /// Read `thread` and hold it for the pane. See
    /// [`open_conversation_ffi`](Self::open_conversation_ffi).
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
            // An unreadable thread folds to an empty conversation rather than
            // leaving the last one on screen: a pane still drawing the
            // previous conversation under a new selection is worse than an
            // empty one, because it looks like an answer.
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

    /// `thread`, drawn as one document (ADR 0032, #1595).
    ///
    /// Gathered from the store the way GTK's pane gathers it: every message
    /// in the thread in the order the stacked pane used (`fold`'s), each body
    /// loaded or said to be coming, recipients through the shared header, the
    /// per-sender image decision made against the allow list the Privacy
    /// pane edits, and the user's own messages marked. Every message opens as
    /// its sender built it (spec 006 FR-031); `reduced` are the ones the
    /// reader asked to see in reader view (`toggle_reader_view`).
    ///
    /// Every message opens (FR-013) -- except one whose body has not arrived,
    /// which stays its one line unless it is the newest: the newest opens
    /// anyway so the "still coming" plate has somewhere to appear, and one
    /// such plate is an explanation where thirty would be noise.
    ///
    /// Reads the store and nothing else; a body that is not here is not
    /// fetched from here.
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
        // The stacked pane's order, from the same fold, so moving to one
        // document does not reorder anybody's conversation.
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
            // The account's own addresses: its primary one and every identity
            // it sends as. Read once per account, not per message.
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

    /// What the pane should draw. See
    /// [`conversation_ffi`](Self::conversation_ffi).
    pub fn conversation(&self) -> Option<crate::ConversationFfi> {
        self.conversation.lock().expect("conversation lock").clone()
    }

    /// The row's own facts about one open message: the unsubscribe offer
    /// and the recipients. One connection, one row read, one `send_state`
    /// read — **no body load**: the two facts that need the body (the
    /// notice and the caveat) ride the document render now, as by-products
    /// of [`reader_answers`](Self::reader_answers) (#1589).
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

        // Through the shared header, which is also what GTK's own reader
        // renders from — one answer to "how does a recipient list read".
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

        // The send state is the offer's gate (#1525): without it the domain
        // fallback would offer to unsubscribe the user from their own
        // account, so a read that fails means "do not offer", never "no
        // send state".
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

    /// What the reader is holding back for `message`.
    ///
    /// Rendered with images blocked whatever the sender's standing is: the
    /// question this answers is "what would be loaded", and asking it of an
    /// already-allowed render would answer "nothing" and take the notice off
    /// screen — which is where a person goes to take a grant back. That rule
    /// lives in [`message_facts`](Self::message_facts) now, which this is a
    /// view over.
    pub async fn reader_notice(&self, message: i64) -> Option<crate::ReaderNoticeFfi> {
        // A view over `reader_answers`, kept for its tests: they are the
        // behavioural record of what a notice means. Blocked and as sent,
        // which were always this question's terms.
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, false)
            .await
            .notice
    }

    /// Always allow `address`. See [`allow_sender_ffi`](Self::allow_sender_ffi).
    pub fn allow_sender(&self, address: String) {
        self.amend_allow_list(|list| list.allow(&address));
    }

    /// Grant `message`'s sender a standing exception, if the reader is asking
    /// for one -- and answer whether it was.
    ///
    /// The command form of the notice's "Always allow" (#1706), and only
    /// where that button would be: something held back, from a sender not
    /// yet allowed. Anywhere else a key does nothing, GTK's rule for the
    /// same command, because a grant written where no notice asked is
    /// consent nobody was asked for. The address is the notice's own, so the
    /// key and the button cannot grant different people.
    pub async fn always_show_images_for(&self, message: i64) -> bool {
        match self.reader_notice(message).await {
            Some(notice) if !notice.allowed && !notice.sender.is_empty() => {
                self.allow_sender(notice.sender);
                true
            }
            _ => false,
        }
    }

    /// Always allow `domain`. See [`allow_domain_ffi`](Self::allow_domain_ffi).
    pub fn allow_domain(&self, domain: String) {
        self.amend_allow_list(|list| list.allow_domain(&domain));
    }

    /// The standing grants, read fresh.
    ///
    /// Not cached: the file is small, this is asked once per message drawn,
    /// and a cached copy is a copy that can disagree with the settings pane
    /// that revokes a grant.
    fn allow_list(&self) -> postio_ui::allowlist::AllowList {
        postio_ui::allowlist::AllowList::load_from(&self.allow_list_path())
    }

    /// Every grant. See
    /// [`remote_image_grants_ffi`](Self::remote_image_grants_ffi).
    pub fn remote_image_grants(&self) -> Vec<crate::GrantFfi> {
        let list = postio_ui::allowlist::AllowList::load_from(&self.allow_list_path());
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

    /// Take one back. See
    /// [`revoke_remote_images_ffi`](Self::revoke_remote_images_ffi).
    ///
    /// One entry point for both kinds: `revoke` removes whichever list holds
    /// it, so the pane does not have to say which it was — and a caller that
    /// guessed wrong would leave a grant in place while reporting it gone.
    pub fn revoke_remote_images(&self, subject: String) {
        self.amend_allow_list(|list| list.revoke(&subject));
    }

    /// Read, change, write. Best-effort: a grant that could not be written
    /// is one the user will be asked about again, which is the safe failure.
    fn amend_allow_list(&self, change: impl FnOnce(&mut postio_ui::allowlist::AllowList)) {
        let path = self.allow_list_path();
        let mut list = postio_ui::allowlist::AllowList::load_from(&path);
        change(&mut list);
        if let Err(error) = list.save_to(&path) {
            tracing::error!(%error, "the remote-image allow list could not be saved: {error}");
        }
    }

    /// Where the grants live.
    ///
    /// Beside the store rather than in `config.toml`: it is state the
    /// application writes, not configuration a person edits, and mixing the
    /// two would mean Postio rewriting a file the user owns.
    fn allow_list_path(&self) -> std::path::PathBuf {
        self.allow_list_at.clone()
    }

    /// What the reader says above a body that did not fully decode, or
    /// `None` when it decoded cleanly.
    ///
    /// The end of the road for `StoredBody::encoding_problems` on this
    /// platform, which was carried all the way to
    /// [`reader_document`](Self::reader_document) and then bound to `_`
    /// (#901, #1585): a body that silently lost a part reads exactly like a
    /// body the sender wrote that way. The GTK reader has said so since #901
    /// and this frontend said nothing.
    ///
    /// A read of its own rather than a field on the document, because the
    /// document is a string handed to a web view and this is native chrome
    /// above it — the same split every notice in the strip has.
    pub async fn decode_caveat(&self, message: i64) -> Option<String> {
        // A view over `reader_answers` — see `reader_notice`.
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, false)
            .await
            .caveat
    }

    /// What `message` offers about the list it came from, or `None`.
    ///
    /// **A read, and only a read.** It opens no connection to anything,
    /// writes no row, and hands back no way to act — a sentence, an
    /// identifier and a button label. That is what makes "only on deliberate
    /// activation" (CLAUDE.md, privacy) a property of the boundary rather
    /// than a habit of whoever writes the frontend: drawing a message can
    /// reach this and cannot reach
    /// [`activate_unsubscribe`](Self::activate_unsubscribe).
    pub async fn unsubscribe_offer(&self, message: i64) -> Option<crate::UnsubscribeOfferFfi> {
        // A view over `message_facts` — see `reader_notice`.
        self.message_facts(message).await.offer
    }

    /// Record that the user asked to leave this message's list.
    ///
    /// `None` when it was recorded, a sentence when it was not.
    ///
    /// # What this does and does not do
    ///
    /// It appends to the activation log and stops. Sending the real RFC 8058
    /// request is #972 and has never been built on either platform — the GTK
    /// banner has only ever asked, too. When it is built it belongs **here**,
    /// inside the one function a frontend can only reach from a button,
    /// rather than anywhere on the rendering path.
    ///
    /// # Why it re-derives the offer
    ///
    /// The caller passes a message id and nothing else: no list identifier,
    /// no URL, nothing read off a banner and handed back. So there is no
    /// value a frontend — or a message, through a frontend — can supply that
    /// decides what gets recorded, and a message the reader offers nothing on
    /// is refused rather than logged. Without that, a frontend bug over the
    /// Outbox would record the user leaving their own account's domain
    /// (#1525 is that bug, on the drawing side).
    ///
    /// See [`activate_unsubscribe_ffi`](Self::activate_unsubscribe_ffi).
    pub async fn activate_unsubscribe(&self, message: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no open store to record the activation in.".to_owned());
        };
        // An interactive write, the same priority the composer's saves take:
        // somebody is waiting on a button they just pressed.
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

    /// Every activation this store holds, newest first.
    ///
    /// Across every account, like the remote-image grants beside it in the
    /// same pane and for the same reason: the pane draws no account
    /// distinction anywhere, and the privacy question is "what have I left",
    /// not "what have I left from this address".
    ///
    /// See [`unsubscribe_activations_ffi`](Self::unsubscribe_activations_ffi).
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
        // The merge rule, not a `sort_by_key` written out here: each
        // account's rows come back newest-first on their own, and joining
        // several of those lists is a decision about what the pane shows,
        // which the classic app's privacy pane was already making with its own
        // copy of the same line. `postio_ui::unsubscribe::newest_first` is
        // the one answer now, tie-break included.
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

    /// Look `address` up. See [`discover_account_ffi`](Self::discover_account_ffi).
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

    /// Sign in, then add. See [`connect_account_ffi`](Self::connect_account_ffi).
    ///
    /// The desktop's order (`postio_session::onboarding`): prove the login
    /// against the real server, then the credential to the keyring, then the
    /// row -- so a wrong password writes nothing, and a keyring that refuses
    /// leaves no account without one.
    ///
    /// The proof and the writes are the host's helpers but not the host's
    /// `AddAccount` request, which also starts the account's engine. On the
    /// Mac the engines are this session's (`start_syncing`), and the window
    /// starts the new one when this answers; the host starting a second
    /// would be two connections syncing one mailbox.
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
            // Handed a mail server (a test's), the proof is signing in to it.
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

    /// Add a local account. See
    /// [`add_local_account_ffi`](Self::add_local_account_ffi).
    ///
    /// One write and no keyring: a maildir account signs in to nothing, so
    /// there is no credential to store first and nothing to roll back.
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

    /// Look at a directory. See
    /// [`inspect_local_store_ffi`](Self::inspect_local_store_ffi).
    pub fn inspect_local_store(&self, path: String) -> Option<String> {
        let root = postio_session::provision::absolute(&path);
        postio_account::maildir::LocalStore::looks_like_a_maildir(&root).err()
    }

    /// The keyring this session was opened with.
    fn secret_store(&self) -> Option<Arc<dyn postio_account::secret::SecretStore>> {
        let guard = self.wiring.lock().expect("wiring lock");
        Some(guard.as_ref()?.secrets.clone())
    }

    /// Test a connection. See [`test_connection_ffi`](Self::test_connection_ffi).
    pub async fn test_connection(&self, account: i64) -> crate::ConnectionReportFfi {
        let refusal = |message: &str| crate::ConnectionReportFfi {
            reachable: false,
            missing_credential: false,
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

        // Awaited rather than driven on a runtime built here. The session's
        // own runtime is still not borrowed — it belongs to the engines, and
        // this waits on a server — but the thing that turns this future back
        // into a value is `testConnection`'s exported wrapper, through
        // `postio_session::blocking`, which is one implementation of the
        // `Handle::try_current` dance rather than a copy of it with only
        // half the cases.
        let report = postio_session::checkup::test_connection(&found, secrets).await;
        crate::ConnectionReportFfi {
            reachable: report.reachable,
            missing_credential: report.missing_credential,
            message: report.message,
        }
    }

    /// Change what an account calls itself. See
    /// [`set_display_name_ffi`](Self::set_display_name_ffi).
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

    /// What this account's mail weighs. See
    /// [`account_weight_ffi`](Self::account_weight_ffi).
    ///
    /// `None` when there is nothing to weigh — a fresh account says nothing
    /// rather than `0 B`, which reads as a failure.
    ///
    /// Three aggregates over `messages`, asked when a settings window opens
    /// and not otherwise. Deliberately not cached: a stale size is a claim
    /// about a store that has changed since, and the whole line is one a
    /// person glances at once.
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
            // What is on this disk, which is what the row is about. Whether
            // the attachments *would* add more is the Sync pane's question.
            false,
        )
    }

    /// Re-index an account. See [`reindex_account_ffi`](Self::reindex_account_ffi).
    ///
    /// Bounded by the mail already on this machine: nothing here reaches a
    /// server. It reports as it goes, because a pass over five thousand
    /// messages takes long enough that a button with no progress is
    /// indistinguishable from a button that does nothing (#1284).
    pub async fn reindex_account(&self, account: i64) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open.".to_owned());
        };
        let local = self.local.0.clone();
        match postio_session::reindex_account(
            &database,
            postio_model::ids::AccountId::new(account),
            |done, total| {
                // `try_send` on an unbounded channel: the only way it fails
                // is a session that has already shut down, and a re-index
                // that kept running to report to nobody would be worse than
                // one that quietly finishes.
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

    /// Flip whether an account is synced at all.
    ///
    /// A single-column write, and deliberately not routed through the
    /// account update path: the caller is a toggle in a settings pane, not
    /// code holding a freshly-loaded account, and `update` would rewrite the
    /// identity list from whatever copy the pane happened to be drawing.
    ///
    /// The row keeps its place in the list either way. A disabled account is
    /// configured and not syncing, which is a state to show rather than one
    /// to hide — "where did my account go" is the worse question.
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
            // `false` is "no row matched", which the keyboard path can
            // genuinely produce: a row removed in one window while the
            // command is pressed in another. A silent no-op there would look
            // like a switch that does not work.
            Ok(true) => None,
            Ok(false) => Some("That account is no longer here.".to_owned()),
            Err(error) => Some(error.to_string()),
        }
    }

    /// Make `account` the one new messages come from when the message itself
    /// does not say.
    ///
    /// And nothing else (#960). It does not order the sidebar, prioritise
    /// sync, or decide a reply's from address, which is settled from the
    /// message being replied to.
    ///
    /// There is no way to clear it: the reversal of marking an account is
    /// marking another, which is why the command carries no undo.
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

    /// Remove an account. See [`remove_account_ffi`](Self::remove_account_ffi).
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

    /// Sign in through the browser. See
    /// [`sign_in_with_browser_ffi`](Self::sign_in_with_browser_ffi).
    ///
    /// Every decision here is `postio_session::signin`'s, which is where the
    /// GTK frontend's own sign-in is moving: the endpoints, the consent, the
    /// **proof that the token opens the account's real IMAP session before
    /// anything is written**, and the credential-then-row order. A consent
    /// path implemented twice is two answers to what Postio asked permission
    /// for, and the wrong one is invisible.
    pub async fn sign_in_with_browser(
        &self,
        address: String,
        client_id: String,
        client_secret: Option<String>,
    ) -> Option<String> {
        use postio_session::signin;

        if client_id.trim().is_empty() {
            // ADR 0006 Q1: Postio ships no client id. Said as a fact about
            // the product rather than as a validation error, because it is
            // one — and the alternative is a shared credential every user of
            // an open-source mail client would be sharing.
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

        // Awaited. This waits on a person — a browser tab they have to come
        // back from — which is why it had a runtime of its own rather than
        // the session's, since that one belongs to the engines. What that
        // reasoning missed is the caller: `blocking::now` at the FFI edge
        // already picks the right runtime, and a private one built while the
        // caller is on one panics.
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
                &database, secrets, &settings, &client, signed_in, &scopes, None,
            )
            .await
            .map_err(signin::SignInError::Failed)
        }
        .await;

        *self.sign_in.lock().expect("sign-in lock") = None;
        progress.store(0, std::sync::atomic::Ordering::SeqCst);
        match outcome {
            Ok(_) => None,
            // Closing the tab is not a failure and must not be reported as
            // one; the sheet stays where it was.
            Err(signin::SignInError::Cancelled) => Some("The sign-in was cancelled.".to_owned()),
            Err(signin::SignInError::Failed(message)) => Some(message),
        }
    }

    /// What the sign-in in flight is doing. See
    /// [`sign_in_progress_ffi`](Self::sign_in_progress_ffi).
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

    /// Give up on the sign-in in flight.
    pub fn cancel_sign_in(&self) {
        if let Some(cancel) = self.sign_in.lock().expect("sign-in lock").take() {
            cancel.cancel();
        }
    }

    /// A new message. See [`new_draft_ffi`](Self::new_draft_ffi).
    pub async fn new_draft(&self) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let account = self.writing_account(&database).await?;
        let draft = postio_model::Draft::new(account.id);
        Some(crate::compose::to_ffi(
            &draft,
            account.address.to_string(),
            self.drafts_path(),
        ))
    }

    /// A draft from a link. See [`mailto_draft_ffi`](Self::mailto_draft_ffi).
    ///
    /// The body **is** honoured, and it is worth saying why that is safe: it
    /// goes into a composer the user is looking at, unsent, with the send button
    /// under their hand. A `mailto:` that arrives with a body prefilled is
    /// RFC 6068's own design, and refusing it would break every "email us
    /// about this" link that carries a reference number.
    ///
    /// Nothing is sent, nothing is fetched, and no header a link cannot
    /// legitimately set is set from here.
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

    /// A reply. See [`reply_draft_ffi`](Self::reply_draft_ffi).
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

    /// A forward. See [`forward_draft_ffi`](Self::forward_draft_ffi).
    pub async fn forward_draft(&self, message: i64) -> Option<crate::DraftFfi> {
        self.answer(message, |source, account| {
            let body = postio_model::reply::plain_forward(source);
            postio_model::reply::forward(source, account, body)
        })
        .await
    }

    /// The shared half of replying and forwarding: read the message, find the
    /// account, hand both to `build`.
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
        // The body is not on the row: it is a compressed column read through
        // the same path the reader uses (ADR 0020), and a quote built from
        // the message as `get` returns it would quote nothing at all — which
        // is a reply that silently loses what it is answering.
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
        // An HTML-only message has no `text` at all: `postio_model::mime`
        // fills that field from a `text/plain` part and never invents one,
        // which is right for a parser and wrong for a quote. `plain_quote`
        // then falls through to its `_` arm and produces the attribution
        // line with **nothing under it** — a reply that silently loses the
        // message it is answering, on the majority of real mail.
        //
        // Rendered here rather than in `postio-model`, which cannot depend
        // on `postio-body` (see that crate's `outgoing` docs) — this is the
        // nearest layer that can, and it is the one that already assembles
        // the draft.
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
        let draft = build(&source, &account);
        Some(crate::compose::to_ffi(
            &draft,
            account.address.to_string(),
            self.drafts_path(),
        ))
    }

    /// Save. See [`save_draft_ffi`](Self::save_draft_ffi).
    /// The draft `id` as the store has it, or `None`.
    ///
    /// What a composer reopens with, and what a test asserting that marks
    /// survived a save has to read: a round trip proved against the value
    /// `save` handed back proves only that `save` returned its argument.
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
        Some(crate::compose::to_ffi(&draft, from, self.drafts_path()))
    }

    /// The draft whose message row is `message`, or `None` when it is not a
    /// draft this machine holds.
    ///
    /// A draft in a conversation is a message row; what resumes the composer
    /// is the draft behind it. `None` covers another client's draft too --
    /// there is nothing here to edit, which is #175's dead end said honestly
    /// rather than papered over with a blank composer.
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
        Some(crate::compose::to_ffi(&draft, from, self.drafts_path()))
    }

    /// Narrow pasted markup to what a message may carry, and say what that
    /// cost.
    ///
    /// The composer calls this on paste rather than letting the editing
    /// surface keep whatever a browser put on the clipboard. Two reasons,
    /// and only the first is about tidiness: the dialect is what the store
    /// can hold, so markup the surface kept would be silently narrowed at
    /// save time anyway -- and the person would watch their table turn into
    /// four lines of text with nothing on screen explaining it.
    ///
    /// Pure: no store, no network, no state. It answers the same way for the
    /// same input on either frontend, which is the point.
    pub fn narrow_paste(&self, html: String) -> crate::PastedFfi {
        let narrowed = postio_body::narrow(&html);
        let (text, html) = postio_body::render(&narrowed.document);
        crate::PastedFfi {
            html,
            text,
            dropped: narrowed.lost.summary(),
        }
    }

    /// What a rich body reads as in plain text.
    ///
    /// What the Rich/Plain switch needs at the moment it flips to Plain
    /// (#1293). The rule the composer follows is "whichever surface is
    /// active is authoritative" -- rich edits the document, plain edits the
    /// text -- and that rule says nothing about the switch itself, which is
    /// exactly when the inactive field is stale and about to become the only
    /// one that matters. Composing in Rich and switching to Plain sent an
    /// empty message.
    ///
    /// Flowed, like every other plain part Postio builds, so a paragraph
    /// rewraps in a narrow window rather than arriving with a ragged
    /// 72-column edge.
    ///
    /// Its own call rather than reading `narrow_paste`'s `text`: that
    /// returns the same string, and a call to `narrowPaste` at the switch
    /// would tell the next reader this was a paste.
    pub fn plain_text_of(&self, html: &str) -> String {
        postio_body::render(&postio_body::parse(html)).0
    }

    /// The script that applies a mark, from
    /// [`postio_ui::compose::mark_script`].
    ///
    /// Shared for the same reason the bridge is: two hosts running different
    /// scripts for `bold` would produce different markup, and the two
    /// composers would disagree about what the same button did.
    pub fn mark_script(&self, command: &str) -> Option<String> {
        postio_ui::compose::mark_script(command)
    }

    /// The script that links the selection to `href`, or `None` when a
    /// message may not point there.
    ///
    /// The gate is the canonical subset's, applied before the document is
    /// touched -- so a refused scheme is something the composer can say,
    /// rather than a link that is created, looks right, and vanishes at the
    /// next parse.
    pub fn link_script(&self, href: &str) -> Option<String> {
        postio_ui::compose::link_script(href)
    }

    /// The composer's editing bridge, for a frontend to inject.
    ///
    /// See [`postio_ui::compose::editor_script`]. It crosses rather than
    /// being written again in Swift because it is what decides the *dialect*
    /// the surface emits -- a second copy would emit `<div>`s where this one
    /// emits `<p>`s, `parse` would narrow them differently, and the two
    /// composers would disagree about what the same keystrokes wrote while
    /// both still round-tripped through a `Document`.
    ///
    /// **Assembled, not the bare constant.** The body reads `POSTIO_MARKDOWN`
    /// and does not define it; handing over
    /// [`postio_ui::compose::EDITOR_SCRIPT`] alone gives the composer a
    /// `ReferenceError` on the first keystroke, inside a WebView, where
    /// nothing on this side of the boundary would hear about it.
    pub fn editor_script(&self) -> &'static str {
        postio_ui::compose::editor_script()
    }

    /// Write the draft to the store and answer it with its id.
    ///
    /// Idempotent on that id: a composer that forgot it would insert a
    /// second row on every autosave, and the Drafts folder would fill with
    /// one half-written message.
    pub async fn save_draft(&self, edited: crate::DraftFfi) -> Option<crate::DraftFfi> {
        let (database, _) = self.store_and_blobs()?;
        let mut draft = self.rehydrate(&database, &edited).await?;
        let (connection, _permit) = database.interactive_write().await.ok()?;
        // `save_and_sync`, for the reason `write_draft` gives: a local write
        // without its queue row never reaches the server. This is the second
        // of the two save paths and had the same omission.
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, chrono::Utc::now())
            .await
            .ok()?;
        Some(crate::compose::to_ffi(
            &draft,
            edited.from.clone(),
            self.drafts_path(),
        ))
    }

    /// Throw a draft away. See
    /// [`discard_draft_ffi`](Self::discard_draft_ffi).
    ///
    /// The row *and* the server copy: `DraftRepository::discard` queues an
    /// `Operation::DiscardDraft` carrying the UID and its generation rather
    /// than naming the draft, because by the time that drains there is no row
    /// left to read them from.
    ///
    /// Discarding one that is already gone is **not an error** — a retried
    /// discard, or one racing a send that already cleared the row, is the
    /// expected case. The window has closed either way, and a complaint about
    /// it would be a dialog about nothing.
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

    /// Attach a file. See [`attach_to_draft_ffi`](Self::attach_to_draft_ffi).
    ///
    /// The bytes first, then the row: a draft must never name a blob that is
    /// not there. The draft is saved on the way out, so an attachment
    /// survives the window closing — which is the whole reason to put it in
    /// the store rather than hold a path.
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

    /// Put a picture in the body. See
    /// [`insert_inline_image_ffi`](Self::insert_inline_image_ffi).
    ///
    /// The bytes first, then the row, as for a file -- and the script only
    /// once both are written, so nothing can draw a picture whose part is not
    /// in the store.
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

    /// Resolve a picture in a draft. See
    /// [`resolve_draft_cid_ffi`](Self::resolve_draft_cid_ffi).
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

    /// Take one off again. See
    /// [`detach_from_draft_ffi`](Self::detach_from_draft_ffi).
    ///
    /// The row goes; the blob is left to the store's own reclaim pass, which
    /// is what `reclaim_orphaned_blobs` is for. Deleting it here would race a
    /// second draft that had attached the same file — the store deduplicates
    /// by content, so two drafts can name one blob.
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

    /// Hand a draft out. See [`begin_handoff_ffi`](Self::begin_handoff_ffi).
    ///
    /// The draft is saved first: an editor that is opened on a body Postio
    /// has not written down is one crash away from having been the only copy.
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

    /// Take it back. See [`end_handoff_ffi`](Self::end_handoff_ffi).
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
            // **Per session, not one directory for the machine.** A session
            // with no store on disk had them all writing to
            // `$TMPDIR/postio-drafts-out`, and a draft's filename is its id
            // -- which starts at 1 in every fresh store. So two sessions
            // handing out their first draft wrote to the same file and each
            // read back the other's body.
            //
            // In the suite that is two test binaries racing, which passes
            // alone and fails beside `postio-session`: a red that reads as
            // noise. It is not only a test problem. This fallback is what a
            // Postio with no store yet uses, and two of them on one machine
            // would trade drafts through it.
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!(
                    "postio-drafts-out-{}-{}",
                    std::process::id(),
                    self.serial
                ))
            })
    }

    /// Save `draft` and answer it as the frontend should now hold it.
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
        // `save_and_sync`, not `save`. **A local write without its queue row
        // never reaches the server** -- that is the repository method's own
        // sentence, and it is why the classic app had used this one since drafts
        // existed. Every macOS save path goes through here: autosave, attach,
        // detach, and both ends of the editor hand-off. With the plain `save`
        // a reply begun on the Mac was in Drafts on the Mac and nowhere else.
        //
        // An account with no Drafts folder enqueues nothing and is not an
        // error: that is a real state -- a server that publishes no such
        // role -- and the local row is still the right thing to have written.
        postio_storage::repository::DraftRepository::new(&connection)
            .save_and_sync(&mut draft, chrono::Utc::now())
            .await
            .map_err(|error| crate::ComposeError::Refused {
                message: format!("The draft could not be saved: {error}"),
            })?;
        Ok(crate::compose::to_ffi(
            &draft,
            edited.from.clone(),
            self.drafts_path(),
        ))
    }

    /// Send. See [`send_draft_ffi`](Self::send_draft_ffi).
    ///
    /// **Nothing here opens a connection.** The write is one local
    /// transaction and `postio-sync::send` drains the row it leaves whenever
    /// there is a network, which is what lets a compose window close on the
    /// keystroke rather than on a server.
    pub async fn send_draft(&self, edited: crate::DraftFfi) -> Option<String> {
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to send from.".to_owned());
        };
        let Some(mut draft) = self.rehydrate(&database, &edited).await else {
            return Some("This draft is no longer in the store.".to_owned());
        };
        // Refused rather than queued: an unaddressed draft would close the
        // window and drain as impossible — the words gone and no message
        // sent. The same check the classic app's composer made, for the same
        // two reasons.
        if !draft.has_recipients() {
            return Some("This message has no recipient yet.".to_owned());
        }
        if !draft.is_sendable() {
            return Some("This draft has already been queued to send.".to_owned());
        }
        // What leaves is what the switch says, and the switch is not the
        // presence of an HTML part (#1271).
        //
        // A plain draft may still be *holding* marks: turning Rich off does
        // not throw them away, in case it is turned back on. But
        // `postio_model::outgoing` builds a `multipart/alternative` from
        // `body.html.is_some()`, so queueing that draft as it stands would
        // send HTML under a footer that says `text/plain, format=flowed`.
        // The footer is a claim about what leaves; this is where it is kept
        // true, at the one point after which the marks no longer matter.
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
            Ok(_) => None,
            Err(error) => {
                tracing::error!(%error, "could not queue the draft for sending: {error}");
                Some("The draft could not be queued for sending.".to_owned())
            }
        }
    }

    /// Queue a draft to leave at `when` — *Schedule send…*.
    ///
    /// The same queue as an immediate send with a time on the row;
    /// `postio-sync` is what holds it back. So the composer closes on the
    /// keystroke exactly as it does for `⌘↵`, which is the local-first rule
    /// applied to a send that has not happened yet.
    ///
    /// **Every check `send_draft` makes, made here too.** A scheduled send is
    /// still a send, and refusing an unaddressed message at 8am tomorrow —
    /// when nobody is watching the composer — is strictly worse than
    /// refusing it now.
    ///
    /// `when` is milliseconds since the epoch, because that is what crosses
    /// a uniffi boundary without a date type on either side of it.
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
        // The footer's claim about what leaves, kept true at the one point
        // after which the marks no longer matter. See `send_draft`.
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
            Ok(_) => None,
            Err(error) => {
                tracing::error!(%error, "could not schedule the draft: {error}");
                Some("The draft could not be scheduled.".to_owned())
            }
        }
    }

    /// The stored draft this edit is about, with the frontend's fields on it.
    ///
    /// A round trip through the store rather than a draft rebuilt from the
    /// fields: the kind, the ancestor and the reserved `Message-ID` are not
    /// things a composer edits, and rebuilding would drop all three — the
    /// last of which is what stops one message being sent twice (ADR 0021).
    async fn rehydrate(
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
            fresh
        };
        Some(crate::compose::from_ffi(base, edited))
    }

    /// The account a new message is written from: the default one.
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

    /// Where drafts live, for the composer's footer.
    ///
    /// The store this session actually opened, not a guess at where one
    /// would be: a footer naming a path the draft is not in is worse than no
    /// footer, and this application can be pointed at another store.
    fn drafts_path(&self) -> String {
        self.store_at.display().to_string()
    }

    /// Turn a command id into the command it means here, and run it.
    ///
    /// **The whole of this frontend's aiming**, and it decides nothing: what
    /// a gesture acts on is `postio_core::aim`'s rule, and this hands it the
    /// facts — the scope on screen, what is marked, where the keyboard is,
    /// and the rows the window is holding. The classic app was the same three
    /// lines over GTK's own widgets (#589, #721). Two adapters, one rule; a
    /// second copy of the rule here is exactly what that issue removed.
    ///
    /// `id` is the registry's own string — `"archive"`, `"open_message"` —
    /// parsed through [`postio_core::CommandId`]'s `FromStr`, which is
    /// generated from the same table the names come from. A `uniffi` enum
    /// would be a second copy of that vocabulary, kept by hand, free to
    /// drift; the string is the file format `ARCHITECTURE.md` §3 already
    /// says it is.
    ///
    /// Returns nothing, deliberately. A verb is local-first: it writes to
    /// SQLite, enqueues, and the frontend learns what happened from the
    /// events it is already draining. A `Result` here would imply the caller
    /// should wait for an answer, which is the shape this architecture spends
    /// its effort not having.
    ///
    /// An id this build does not know is ignored rather than panicking — it
    /// arrived from another process, and a boundary that aborts on a typo is
    /// a boundary that can be crashed from Swift.
    pub fn invoke(&self, id: &str) {
        let Ok(id) = id.parse::<postio_core::CommandId>() else {
            tracing::debug!(id, "not a command this build knows; ignored");
            return;
        };
        let Some(outbox) = self.outbox() else {
            return;
        };

        let (command, aimed) = self.with_aim(|aim| {
            let command = postio_core::aim::command_for(id, aim);
            // What a verb left aimed at the selection resolves against on
            // the host: this view's selection, cursor and scope, as they are
            // now. `refine` names the rows only for a conversation; a
            // message row is resolved there from this.
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

    /// Where this session's commands go, while it is open.
    fn outbox(&self) -> Option<async_channel::Sender<Aimed>> {
        self.link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.outbox.clone())
    }

    /// Resolve one key press against the bindings in force.
    ///
    /// **The whole of the frontend's keyboard, and it decides nothing.** The
    /// caller reduces its own event to the three things every toolkit can
    /// supply -- the character the key would type, the key's name when it
    /// types none, and the modifiers held -- and this hands them to
    /// `postio_ui::keymap`, which owns the table, the chords, the sequences
    /// and the leader timeout. The classic app's `resolve_key` was the same shape
    /// over GDK. Two adapters, one keymap; that is what keeps `[keys]`
    /// meaning the same thing on both platforms (ADR 0019 Q4).
    ///
    /// `in_text_entry` is whether the focused surface takes text, and it is
    /// the caller's to answer because only the caller can see its own focus.
    /// Getting it wrong is the most visible bug this boundary can have: a
    /// search field that archives mail on `a` reads as a broken application
    /// rather than a misrouted key.
    ///
    /// Does **not** run the command. The caller needs the answer before it
    /// acts on it -- a `Command` and a `Pending` are swallowed, an `Unhandled`
    /// must propagate to whatever the toolkit would have done with the key --
    /// and a method that both ran the verb and reported it would give the
    /// caller no way to tell the third case from the first two.
    pub fn key(
        &self,
        character: Option<&str>,
        name: Option<&str>,
        modifiers: crate::ModifiersFfi,
        context: crate::UiContext,
        in_text_entry: bool,
    ) -> crate::KeyOutcomeFfi {
        // A `String` crosses the boundary because a `char` has no uniffi
        // type, and a frontend that sent two characters would otherwise
        // silently bind the first. Take the whole scalar or nothing.
        let character = match character.map(|text| {
            let mut characters = text.chars();
            (characters.next(), characters.next())
        }) {
            Some((Some(one), None)) => Some(one),
            // A grapheme cluster, or an empty string: neither is a key a
            // binding can name, and pretending otherwise would bind whichever
            // half came first.
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

        let key_context = postio_ui::keymap::KeyContext::from(postio_core::Context::from(context));
        let outcome = self.resolver.lock().expect("resolver lock").press(
            &chord,
            key_context,
            in_text_entry,
            std::time::Instant::now(),
        );

        // The silent path, and the one that is impossible to diagnose without
        // it: a key that does nothing, with nothing said about why.
        // The classic app's `resolve_key` logged the same three inputs for the same
        // reason -- "it randomly stopped working" becomes one line naming
        // which of them it was. No message content: a chord, a context and a
        // flag are not mail.
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

    /// Whether `id` can run in `context`, given the open view.
    ///
    /// The same question `reachable_in` answers for the palette, asked one
    /// command at a time — which is what a menu needs, because a menu draws
    /// every item whether or not the focused surface can run it and has to
    /// grey the ones it cannot.
    ///
    /// **The menu had no filter at all** (#1158): every registry command with
    /// a menu section was drawn enabled, so a Mac showed Send, Bold and the
    /// whole Format menu as live options against a build with no composer.
    /// Seventeen of those are `Context::Composer`-only and the palette was
    /// already hiding them — the two surfaces disagreed because only one of
    /// them was asking.
    ///
    /// Asked here rather than answered in a frontend so they cannot disagree
    /// again, and so a command added in Rust is filtered without a Swift
    /// change — the same rule the menu's *contents* already follow (#657).
    pub fn is_available(&self, id: &str, context: crate::UiContext) -> bool {
        let Ok(id) = id.parse::<postio_core::ActionId>() else {
            return false;
        };
        // A command this platform does not offer is never available on it,
        // whatever the context: see `postio_core::registry::offered_on`.
        if !postio_core::registry::offered_on(id, postio_config::paths::Platform::host()) {
            return false;
        }
        postio_core::registry::reachable_in(
            postio_core::Context::from(context),
            self.availability(),
        )
        .any(|spec| spec.id == id)
    }

    /// What this session can currently do, as the registry evaluates it.
    ///
    /// Watch `source`'s file, if it has one, and follow it: `[keys]` rebuilds
    /// the resolver and says [`UiEvent::KeymapChanged`], so the menu bar and
    /// every keycap re-read their keys; `[focus]` reaches the engine
    /// (specs/009-focus-macos R6, R8). Mirrors the GTK app's `follow_config`
    /// and the terminal's `follow_focus_config`.
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
                *session.resolver.lock().expect("resolver lock") = build_resolver(&keys);
                *session.keys.lock().expect("keys lock") = keys;
                *session.keymap.lock().expect("keymap lock") = None;
                let _ = session.local.0.try_send(UiEvent::KeymapChanged);
            }
            if update.changed.focus {
                *session.focus_config.lock().expect("focus config lock") =
                    service.config().focus.clone();
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

    /// `[focus]` as it stands.
    pub(crate) fn focus_config(&self) -> postio_config::FocusConfig {
        self.focus_config.lock().expect("focus config lock").clone()
    }

    /// Focus's list.
    pub(crate) fn focus_driver(&self) -> &Arc<crate::focus_list::FocusDriver> {
        &self.focus_list
    }

    /// The host's client, while this session is open.
    pub(crate) fn client(&self) -> Option<postio_client::Client> {
        self.link
            .lock()
            .expect("link lock")
            .as_ref()
            .map(|link| link.client.clone())
    }

    /// `store_open` is unconditionally true here, and that is a fact about
    /// this type rather than an assumption: a `Session` is constructed *over*
    /// an open store, so there is no interval in which one does not exist.
    /// The window-first startup that makes [`Requirement::StoreOpen`] worth
    /// evaluating was the classic app's (#1114), and a frontend that ever grows
    /// the same shape answers here instead of at a menu.
    ///
    /// [`Requirement::StoreOpen`]: postio_core::Requirement::StoreOpen
    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability {
            frontend: crate::FRONTEND,
            // Focus's lists span every account, so the view is always the
            // unified one: a command needing a single account is not offered.
            ..postio_core::Availability::open(postio_core::Scope::Unified)
        }
    }

    /// The palette's rows for `query`, best first.
    ///
    /// **The matcher is `postio_ui::palette`'s.** Swift must not write its
    /// own: the ranking is a product decision, and two rankings mean the same
    /// query offers different things on each platform.
    ///
    /// Filtered to what `context` can actually run and to what the open scope
    /// satisfies. Offering a command the focused surface will ignore is worse
    /// than omitting it — the user presses Return, nothing happens, and that
    /// reads as a broken application rather than an unavailable command.
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

    /// Every command with the binding actually in force, in cheat-sheet order.
    ///
    /// The same list the palette reads, unfiltered by a query — *"they are the
    /// same list read two ways"* (#658). Building them separately would mean
    /// two places deciding what "available here" means, and they would
    /// disagree.
    ///
    /// The flat form. [`Session::cheat_sheet_sections`] is what a `?` overlay
    /// should draw: the same rows, grouped the way the product groups them.
    pub fn cheat_sheet(&self, context: crate::UiContext) -> Vec<crate::PaletteEntryFfi> {
        self.palette_entries("", context)
    }

    /// The `?` sheet, grouped: Everywhere, the box's prefixes, the reader's
    /// own surface, then one section per extension namespace.
    ///
    /// The grouping is [`postio_ui::cheatsheet::sections`]'s — the same
    /// function the GTK overlay draws from, so the two frontends teach the
    /// same sheet. The flat [`Session::cheat_sheet`] predates it and is what
    /// an ungrouped list should keep using.
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

    /// The bindings in force, resolved for this platform.
    ///
    /// Resolved once and kept. Every button's tooltip asks for its key, the
    /// reader's and the toolbar's re-render on every move between messages,
    /// and resolving the keymap from the registry for each ask was about 50 ms
    /// of the main thread per move (sampled in the running app). `[keys]`
    /// does not change while a session is open; the registry can, when an
    /// extension registers, and the cache is keyed by its size for that.
    fn keymap(&self) -> postio_core::Keymap {
        let commands = postio_core::registry::every_action().count();
        let mut cached = self.keymap.lock().expect("keymap lock");
        match &*cached {
            Some((size, keymap)) if *size == commands => keymap.clone(),
            _ => {
                let keymap = postio_core::Keymap::resolve(&self.keys.lock().expect("keys lock"));
                *cached = Some((commands, keymap.clone()));
                keymap
            }
        }
    }

    /// `#` in the search box: folders matching `query`. See
    /// [`crate::finder::folders`].
    pub async fn finder_folders(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::folders(&self.mailboxes().await, &query)
    }

    /// `@` in the search box: correspondents matching `query`.
    pub async fn finder_contacts(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::contacts(&self.finder_sources().await.contacts, &query)
    }

    /// `+` in the search box: labels matching `query`.
    pub async fn finder_labels(&self, query: String) -> crate::FinderAnswerFfi {
        crate::finder::labels(&self.finder_sources().await.labels, &query)
    }

    /// The correspondents and labels of every enabled account, read through
    /// the host as GTK's box reads them, and kept for
    /// [`FINDER_SOURCES_FOR`].
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

    /// `+`'s Return: put `label` on the selection -- the marked messages, or
    /// the one under the cursor when nothing is marked -- as GTK's box does
    /// (`Command::AddLabel`, toggled, so `u` takes it back).
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

    /// This view's selection, cursor and scope as they are now, mirrored into
    /// the state a command is aimed with -- what `invoke` sends beside every
    /// verb, for a command that is built here rather than from an id.
    fn aimed(&self) -> postio_core::state::SharedState {
        self.with_aim(|aim| {
            let aimed = postio_core::state::SharedState::default();
            let (quiet, _) = postio_core::bridge::event_channel();
            postio_core::aim::mirror(&aimed, &quiet, aim);
            aimed
        })
    }

    /// Run `f` with what a verb would be aimed at: Focus's list for the rows,
    /// the place it shows for the scope, and the cursor the frontend last
    /// reported.
    ///
    /// **The whole of this frontend's aiming**, and it decides nothing: what
    /// a gesture acts on is `postio_core::aim`'s rule, and this hands it the
    /// facts. Nothing is marked here -- the selection is the controller's
    /// (specs/009-focus-macos T040), and until it drives the Mac's cursor a
    /// verb acts on the cursor's row, which is `PRODUCT.md` section 9's rule
    /// for nothing marked.
    fn with_aim<R>(&self, f: impl FnOnce(&postio_core::aim::Aim<'_>) -> R) -> R {
        let rows = self.focus_list.rows();
        let selection = postio_core::state::Selection::default();
        let aim = postio_core::aim::Aim {
            // The shared conversion, not a second one: `aim::view_scope` is
            // the one rule for what a whole-view gesture is relative to.
            scope: self.focus_list.scope().and_then(|scope| {
                postio_core::aim::view_scope(scope, &self.reachable.lock().expect("reachable lock"))
            }),
            selection: &selection,
            cursor: *self.cursor.lock().expect("cursor lock"),
            rows: &*rows,
        };
        f(&aim)
    }

    /// The cursor rested on `message` long enough for it to count as read.
    ///
    /// **Not `invoke`, and the difference matters.** `MarkReadOnDwell` is
    /// deliberately not a registry command: it routes to
    /// `CommandId::ToggleRead`'s handler so there is one "mark read" in the
    /// vocabulary, and it is the one dispatch that is *not* recorded on the
    /// undo stack — `u` takes back what you did, and reading a mailbox
    /// produces one of these per message rested on. Going through `invoke`
    /// would make every message read an undo entry and bury the archive you
    /// actually wanted back.
    ///
    /// The message is named rather than taken from the cursor, for the reason
    /// The classic app gave on the same call: the cursor may have moved on
    /// between the frontend's timer firing and this running, and the message
    /// that was read is the one the clock was started for.
    pub fn mark_read_on_dwell(&self, message: i64) {
        let Some(outbox) = self.outbox() else {
            return;
        };
        let command = postio_core::Command::MarkReadOnDwell {
            message: postio_model::ids::MessageId::new(message),
        };
        // Named, so there is nothing to aim: the default state is enough.
        let aimed = postio_core::state::SharedState::default();
        if outbox.try_send((command, aimed)).is_err() {
            tracing::debug!("the runtime has stopped and did not mark that read");
        }
    }

    /// Report where the keyboard is, so a verb with nothing marked knows
    /// which row it is about.
    pub fn set_cursor(&self, message: Option<i64>) {
        *self.cursor.lock().expect("cursor lock") = message.map(postio_model::ids::MessageId::new);
    }

    /// Say which accounts the aggregate view can currently vouch for.
    ///
    /// Reported by the frontend, from the same connection states its own
    /// "showing local mail" banner is drawn from, and read at the moment a
    /// whole-view selection is *made*. A frontend that never calls this gets
    /// the safe answer: `Ctrl+A` in the unified list selects nothing rather
    /// than acting on accounts nothing vouched for (#811).
    pub fn set_reachable_accounts(&self, accounts: &[i64]) {
        *self.reachable.lock().expect("reachable lock") = accounts
            .iter()
            .copied()
            .map(postio_model::ids::AccountId::new)
            .collect();
    }

    /// The store and the runtime, while the session is open.
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

    /// How many pages of Focus's list have been read from the store.
    /// Test-only.
    #[cfg(feature = "testing")]
    pub fn focus_page_reads_for_test(&self) -> usize {
        self.focus_list.page_reads()
    }

    /// How many rows Focus's list is holding. Test-only.
    #[cfg(feature = "testing")]
    pub fn focus_resident_rows_for_test(&self) -> usize {
        self.focus_list.resident_rows()
    }

    /// Wait until no conversation read is in flight.
    ///
    /// Test-only. A production frontend never waits for this -- it redraws
    /// when `ConversationReady` arrives, which is the whole design.
    #[cfg(feature = "testing")]
    pub fn settle_for_test(&self) {
        let ordering = std::sync::atomic::Ordering::SeqCst;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.in_flight.load(ordering) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// The whole document for a message, ready to hand a web view.
    ///
    /// Not fragments to assemble: the content security policy, the
    /// `@font-face` rules, the reader tokens, the sanitized body, its
    /// `.postio-body` container and the scroll markers all come from
    /// `postio_ui`, which is what the GTK reader composes through too. **The
    /// frontend's entire job is to build a hardened web view, hand it this
    /// string, and refuse navigations** — so the two readers cannot disagree
    /// about the policy, because there is only one that produces it (ADR
    /// 0019 Q6).
    ///
    /// # Two scheme handlers, not one
    ///
    /// The document *references* Postio's typefaces rather than carrying
    /// them (ADR 0023): `font-src` is `postio-font:`, and a frontend that
    /// does not serve it renders in system sans — silently, because a font
    /// that never arrives is not an error. So a frontend registers two
    /// handlers beside each other, both answering from compiled-in bytes,
    /// neither touching the filesystem or the network:
    ///
    /// * `postio-cid:` — inline parts, through `postio_ui::reader::parts`.
    /// * `postio-font:` — the eight vendored faces, through
    ///   `postio_ui::reader::document::font_bytes`, which answers only for
    ///   names in its `FACES` table and `None` for everything else.
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
        // The blob store is not consulted: a body is a compressed column on
        // the message's row since ADR 0020. Inline parts still come from it,
        // which is why `store_and_blobs` is the accessor either way.
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
        match postio_session::reading::load_body_or_reason(&connection, message.into(), offline)
            .await
        {
            // `encoding_problems` is bound and not used *here* deliberately,
            // and is no longer the gap it was: the caveat it carries is
            // native chrome above the document rather than markup inside it
            // — the same split the classic app's reader `DecodeNotice` had (#901) —
            // so it crosses as [`decode_caveat`](Self::decode_caveat), which
            // reads the same flag through the same load. Named rather than
            // elided so the next reader of this arm finds the other half.
            postio_session::reading::Body::Ready {
                body,
                encoding_problems,
            } => {
                // Every message opens as its sender built it (spec 006
                // FR-031), the rule GTK and the shared thread page follow;
                // reader view is the reader's own choice for this message
                // and this view (`toggle_reader_view`), never the default.
                // Whether it reads as bulk still picks the sheet below.
                let bulk = suits_reader_view(&body);
                let rendering = if reduced {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                let drawn = body_html(&body, remote, rendering);
                // The sender's own sheet — paper white, inset from the app's
                // chrome — for an original that reader view would otherwise
                // have reduced. `sheet_for` is the rule, from the same
                // function GTK calls: the app's palette is never injected
                // into a sender's markup.
                // The two facts the render already paid for (#1589). The
                // notice only from a blocked render — the question it
                // answers is "what would be loaded" — and its counts come
                // out of the sanitize pass, which runs before reader view's
                // reduce, so Reader and Original renders agree on them (a
                // test pins that).
                let held_back = drawn.held_back;
                let notice = if remote == postio_body::RemoteImages::Blocked {
                    let summary = held_back.summary();
                    if summary.is_empty() {
                        None
                    } else {
                        // The sender, for the grant the notice offers. A row
                        // read, not a body load — and only on the messages
                        // that actually held something back.
                        let sender =
                            postio_storage::repository::MessageRepository::new(&connection)
                                .get(postio_model::ids::MessageId::new(message))
                                .await
                                .ok()
                                .flatten()
                                .and_then(|row| {
                                    row.from
                                        .first()
                                        .map(|address| address.address.to_lowercase())
                                })
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
                };
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
            // A state plate is Postio's own words, so it is served with remote
            // images blocked whatever the caller asked for: there is nothing
            // in it a sender wrote, and nothing for them to reach through.
            postio_session::reading::Body::Absent(state) => plate(wrap_document(
                &absent_html(state),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            )),
        }
    }

    /// The document alone. A view over
    /// [`reader_answers`](Self::reader_answers), kept because a page of
    /// tests asserts on the HTML and has no use for the facts.
    pub async fn reader_document(
        &self,
        message: i64,
        remote: crate::RemoteImagesFfi,
        reduced: bool,
    ) -> String {
        self.reader_answers(message, remote, reduced).await.html
    }

    /// Who `message` was addressed to, already rendered.
    ///
    /// `None` for a message the store does not hold. Answered from the
    /// envelope, which is known as soon as headers have synced — so a message
    /// still waiting for its body still says who it went to.
    ///
    /// A read of its own rather than a field on `RowFfi`, because the list
    /// does not draw recipients and paying for them per row would load a
    /// mailbox's addresses to show one message's.
    pub async fn recipients(&self, message: i64) -> Option<crate::RecipientsFfi> {
        // A view over `message_facts` — see `reader_notice`.
        self.message_facts(message).await.recipients
    }

    /// The verbs the reading pane offers, in canvas order.
    ///
    /// No key travels with them: this boundary's other frontend draws `⌘R`
    /// rather than `e`, and the chord is [`Session::binding_for`]'s answer.
    /// What is shared is *which* verbs, which is a product decision — a
    /// reader offering three on one platform and four on the other is two
    /// applications.
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

    /// The header bar of a conversation of `messages` (FR-008, FR-008a).
    ///
    /// The same four verbs as [`reader_actions`](Self::reader_actions), each
    /// with what it will act on in words and whether that is the whole
    /// thread. The four are the reader's verbs; which message each acts on is
    /// `ReaderAction::scope`'s, and the frontend passes the latest message or
    /// none accordingly rather than deciding.
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

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// Synchronous and local, matching the contract the GTK scheme handler
    /// works under: a URL scheme handler runs on the main thread on both
    /// platforms, so this must never block on I/O the reader would await.
    ///
    /// `message` is a parameter rather than ambient state because a
    /// `Content-ID` means something only inside the message that declared it
    /// — resolving one globally would let a sender address another sender's
    /// parts. `None` when the bytes are not already here, which is the
    /// privacy commitment rather than a gap: fetching would be the tracking
    /// pixel arriving through the back door.
    pub async fn resolve_cid(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        let (database, blobs) = self.store_and_blobs()?;
        postio_session::reading::resolve_cid(&database, &blobs, message.into(), &content_id)
            .await
            .map(|(bytes, mime_type)| crate::InlinePart { bytes, mime_type })
    }

    /// One part's bytes, fetching them if the user's asking is what it takes.
    ///
    /// See `partBytes` above for why this is the only call here allowed near
    /// the network.
    pub async fn part_bytes(
        &self,
        message: i64,
        part_id: String,
    ) -> Result<Vec<u8>, crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        Ok(postio_session::reading::part_bytes_at(
            &database,
            &blobs,
            self.engine(),
            message.into(),
            &part_id,
        )
        .await?)
    }

    /// The engine for this store, once one has been started.
    ///
    /// `None` is an ordinary state rather than a fault — a store nobody has
    /// signed into yet — and it is what turns "fetch this part" into a
    /// sentence saying the account is not syncing instead of a wait that
    /// never ends.
    fn engine(&self) -> Option<postio_runtime::Engine> {
        self.wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .and_then(|wiring| wiring.engine.get().cloned())
    }

    /// Every folder of every enabled account, for the sidebar.
    ///
    /// Blocks on a local read, like `openScope` does and for the same reason:
    /// a sidebar is drawn before anything can be selected in it, and the read
    /// is a few milliseconds of SQLite rather than the network.
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
                // The rows that are views rather than folders -- Flagged,
                // Snoozed, and the Outbox when it holds something. Built by
                // the same shared layer the classic app's feed asked, which is the whole
                // point: this boundary has never carried them, so the macOS
                // sidebar has never drawn them (#1155 moved the *order* here
                // and left the rows behind).
                let counts = postio_ui::sidebar::ViewCounts {
                    flagged: found.iter().map(|folder| folder.counts.flagged).sum(),
                    snoozed: found.iter().map(|folder| folder.counts.snoozed).sum(),
                    // Counted by the sidebar's own query in spec 003 T066;
                    // zero keeps the row hidden, which is right until
                    // something can count it.
                    outbox: 0,
                    drafts: 0,
                    attention: 0,
                };
                found.extend(postio_ui::sidebar::view_rows(account.id, &found, counts));
                // Ordered and split here rather than in the frontend.
                // `postio_ui::sidebar` is the canvas' order -- Inbox first --
                // and the rule that a role gets one row however many folders
                // carry it. A frontend sorting for itself is a second answer
                // to "where is my inbox", and the duplicate rule took a bug
                // report to find (#501, #1155).
                let (special, ordinary) = postio_ui::sidebar::sections(&found);
                // The name comes from the same place too, not from
                // `mailbox.name`. A special-use folder is called what Postio
                // calls the role rather than what the server named it -- an
                // iCloud account's junk folder is "Junk E-mail" and the
                // sidebar is not where somebody learns that -- and a view row
                // has no server name at all, so the raw field is empty.
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

    /// The binding in force for a command, for drawing a native accelerator.
    ///
    /// The user's override if there is one, the built-in default otherwise.
    /// A menu must ask rather than read `CommandSpec::default_binding`
    /// directly: drawing the default for a command somebody rebound is
    /// confidently wrong, which is worse for a menu item than showing no key
    /// at all.
    ///
    /// Resolved for the running platform, so a Mac gets `cmd+k` for the
    /// palette rather than the `mod+k` the table stores. Swift renders a
    /// `KeyboardShortcut` from this and must never see the token: it has no
    /// way to know which key `mod` means, and that decision belongs to the
    /// core anyway.
    pub fn binding_for(&self, command: String) -> Option<String> {
        // The *resolved* keymap, not the override table. `KeyBindings` knows
        // only what `postio-config`'s own short list of defaults says, which
        // is 24 commands out of about eighty (#1227) -- so this answered
        // `None` for `delete`, `flag`, `send` and 53 others whose keys work,
        // and the menu drew no accelerator for any of them.
        //
        // `Keymap::resolve` is defaults-from-the-registry plus the user's
        // overrides, with `mod` already expanded for this platform, which is
        // exactly what a menu wants to draw.
        let Ok(action) = command.parse::<postio_core::ActionId>() else {
            return None;
        };
        self.keymap().binding(action).map(str::to_string)
    }

    /// Every binding in force for `command`, the primary first.
    ///
    /// A menu wants the chord and the cheat sheet wants the mnemonic, and
    /// both are true at once: the canvas' two keyboard layers are two
    /// bindings on one command (`e` and `⌘R`), not a mode. Which of them a
    /// surface draws is that surface's business; which of them *exist* is
    /// the registry's, and `mod` is already expanded for this platform.
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

    /// How many accounts are configured and enabled.
    ///
    /// ADR 0005 Q3: the first account is not special, so this counts every
    /// enabled one rather than looking for a primary.
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

    /// Whether an engine has been started and reached the slot.
    pub fn has_engine(&self) -> bool {
        self.wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .is_some_and(|wiring| wiring.engine.get().is_some())
    }

    /// Start syncing every configured account, and answer how many started.
    ///
    /// **The gap this closes:** the application opened a store and it stayed
    /// empty forever, because nothing here ever started a sync. The store
    /// being empty was never a rendering problem — nothing had fetched
    /// anything.
    ///
    /// Zero accounts is `Ok(0)`, not an error. A fresh store with nothing
    /// configured is the ordinary first-run state, and putting an error on
    /// screen for somebody who has simply not finished setting up is worse
    /// than saying nothing.
    ///
    /// Does not block: `engine::start_all` spawns onto the runtime the
    /// session already holds, and the connection attempt happens there. The
    /// UI never awaits the network.
    pub async fn start_syncing(&self) -> Result<u32, SessionError> {
        // Cloned out and the guard dropped, rather than held across the awaits
        // below. A `std::sync::MutexGuard` across an `.await` is a lock held
        // for as long as the future is suspended -- and this one is suspended
        // on the network -- so a second caller reaching this method would
        // block a runtime worker until a server answered. `Wiring` is handles
        // over `Arc`s; cloning it copies no mail.
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
        // Idempotent per account, not per session. It used to return early
        // whenever *any* engine existed — which is right for the reason that
        // guard was written (a window reopening, a wake from sleep, and a
        // second set of engines doubling every connection) and wrong for the
        // case nobody had yet: an account added while the application is
        // running gets no engine, syncs nothing, and looks to the user as
        // though it was never saved at all. It was saved; it was never
        // started. Filtering by account keeps both properties.
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
            wiring.engine.fill(engine.clone());
            postio_runtime::retain(engine.clone());
            self.engines
                .lock()
                .expect("engines lock")
                .push((account, engine));
        }
        Ok(already + count)
    }

    /// Adopt an engine over `MockBackend`, so adoption is testable.
    ///
    /// The real path builds a TLS connector and then connects, which no test
    /// in the default suite may do. This exercises the half that is this
    /// boundary's own — retaining the engine and filling the slot — against
    /// the seam CLAUDE.md names for it.
    #[cfg(feature = "testing")]
    pub fn adopt_mock_engine_for_test(&self) {
        let guard = self.wiring.lock().expect("wiring lock");
        let Some(wiring) = guard.as_ref() else { return };
        let parts = postio_runtime::EngineParts {
            account: 1.into(),
            database: wiring.database.clone(),
            blobs: wiring.blobs.clone(),
            backend: Arc::new(postio_account::backend::MockBackend::new()),
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
            wiring.engine.fill(engine.clone());
            // Account 1, matching the `parts.account` above: the engines list
            // is keyed by account now, so that `start_syncing` can tell an
            // account that is already running from one that has just been
            // added.
            self.engines
                .lock()
                .expect("engines lock")
                .push((postio_model::ids::AccountId::new(1), engine));
        }
    }

    /// Tell the engine whether the machine currently has a connection.
    pub fn set_offline(&self, offline: bool) {
        let ordering = std::sync::atomic::Ordering::SeqCst;
        let was = self.offline.swap(offline, ordering);

        // Only the transition back, and only when it really is one.
        //
        // `NWPathMonitor` repeats itself -- an interface changing while the
        // path stays satisfied is a fresh callback with the same answer -- and
        // reconnecting on each of those would hammer a server through exactly
        // the flapping connection backoff exists to protect it from.
        if was && !offline {
            self.reconnect();
        }
    }

    /// Whether the platform has told us there is no connection.
    pub fn is_offline(&self) -> bool {
        self.offline.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Ask every engine to try the folder in view again, now.
    ///
    /// The engine reconnects with backoff on its own and works with no
    /// reachability signal at all, which is why this is a nudge rather than a
    /// mechanism: all knowing buys is *promptness*. Waking a laptop syncs
    /// immediately instead of at whatever backoff step the engine had reached,
    /// which can be minutes.
    ///
    /// Failures are the engine's to report. It announces connection state and
    /// progress as it goes, so a nudge that could not connect has already been
    /// said once and must not be said twice.
    fn reconnect(&self) {
        // Counted, and nothing more: Focus's lists span every mailbox, so
        // there is no one folder in view to refresh, and the engines' own
        // backoff loops are what reconnect (specs/009-focus-macos).
        self.reconnects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// How many reconnects have been asked for. Test-only.
    #[cfg(feature = "testing")]
    pub fn reconnects_for_test(&self) -> usize {
        self.reconnects.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The database and blob store, while the session is open.
    fn store_and_blobs(&self) -> Option<(postio_storage::Store, postio_storage::BlobStore)> {
        let guard = self.wiring.lock().expect("wiring lock");
        let wiring = guard.as_ref()?;
        Some((wiring.database.clone(), wiring.blobs.clone()))
    }

    /// Every command the registry knows, in cheat-sheet order.
    ///
    /// The frontend asks once and builds its palette, cheat sheet, menu bar
    /// and key hints from the answer — the same derivation the GTK side makes.
    /// That is what keeps `PRODUCT.md` §8 true on both platforms: *a command
    /// that is not in the registry does not exist*, and equally, one that is
    /// in it needs no second list to be discoverable.
    pub fn commands(&self) -> Vec<crate::CommandSpecFfi> {
        crate::registry::commands()
    }

    /// Whether this session still holds its store.
    ///
    /// False after [`shutdown`](Self::shutdown). Exists so a caller can tell a
    /// session that reported success from one that is actually holding
    /// something — an assertion that would otherwise pass vacuously.
    pub fn is_open(&self) -> bool {
        self.wiring
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// Drops the store and ends the event drain.
    ///
    /// Idempotent: an application lifecycle can deliver a termination twice —
    /// a window close racing an explicit quit — and taking the process down on
    /// the second one would turn an ordinary shutdown into a crash report.
    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.wiring.lock() {
            guard.take();
        }
        if let Ok(mut link) = self.link.lock() {
            link.take();
        }
    }

    /// The next event, or `None` once the session has stopped.
    ///
    /// This is the whole reason the boundary is UniFFI rather than a
    /// hand-written C ABI: it becomes Swift `async`, so the frontend's drain
    /// is `while let event = await session.nextEvent()` on the main actor —
    /// the same shape as `glib::spawn_future_local` on the GTK side, with no
    /// callback, no continuation and no manual cancellation in between.
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

    /// The next event if one is already waiting; `None` rather than a wait.
    ///
    /// Test-only. `next_event_blocking` is the honest shape for a headless
    /// consumer and the wrong one for a test that wants to drain what has
    /// arrived *so far*: with the channel still open it blocks forever, and
    /// a test asking for one report more than was emitted hangs the binary
    /// rather than failing (which is how two of these were found — see
    /// `docs/notes/2026-09-06-a-test-keyring-that-was-quietly-the-login-keychain.md`
    /// for the same symptom from a different cause).
    #[cfg(feature = "testing")]
    pub fn try_next_event(&self) -> Option<UiEvent> {
        // Both channels, the way `next_event` selects over both: the engine's
        // and this boundary's own. A drain that read only the engine's would
        // silently miss every event this side invents --
        // `ConversationReady`, `ReindexProgress` — which is most of what a
        // test about this crate wants to see.
        match self.events.try_next() {
            Some(engine) => Some(UiEvent::from(engine)),
            None => self.local.1.try_recv().ok(),
        }
    }

    /// [`next_event`](Self::next_event), for callers that are not async.
    ///
    /// Rust-only. Swift always awaits.
    pub fn next_event_blocking(&self) -> Option<UiEvent> {
        self.events.next_blocking().map(UiEvent::from)
    }

    /// Emits an event as the engine would.
    ///
    /// Test-only, and behind the feature for that reason: a frontend that can
    /// invent events is a frontend whose repaints stop meaning anything.
    #[cfg(feature = "testing")]
    pub fn emit_for_test(&self, event: postio_core::Event) {
        if let Ok(guard) = self.wiring.lock()
            && let Some(wiring) = guard.as_ref()
        {
            wiring.events.emit(event);
        }
    }

    /// Whether this session's `Wiring` was actually built with the backfill
    /// and watch policy `[sync]` (as `expected` parses it) implies (#1014).
    ///
    /// Test-only, the same reason `emit_for_test` is: the wiring is private
    /// so that nothing outside this crate can reach in, and a test proving
    /// `open` read `[sync]` rather than merely compiling has to reach in
    /// anyway. A comparison rather than a raw accessor so this crate need
    /// not name `postio_sync`/`postio_runtime`'s policy types at its own
    /// boundary — the classic app reached `with_backfill`/`with_watch` the same
    /// way, through `postio_session`'s functions, and never names them
    /// either.
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

/// Run `future` to completion on this thread, blocking until it answers.
///
/// # Why the FFI blocks
///
/// The exported surface is synchronous, because that is what a Swift caller
/// asked for: `session.search(query)` returns a count, not a task. The store
/// underneath is async now, so something has to turn a future back into a
/// value, and it is here rather than in every method.
///
/// These were blocking reads before as well -- the storage layer was
/// synchronous and these methods called it directly. What changed is the
/// spelling. The contract on the surface is unchanged and is documented on
/// `Session::open`: a caller must not invoke these on the main actor.
///
/// # One implementation, in `postio-session`
///
/// This used to be a fourth copy of the same four lines, and it was the copy
/// that had only half of them: a runtime built here and blocked on panics
/// outright when the caller is already on one. `postio_session::blocking::now`
/// is the whole of it, and the reason it is shared is that every crate that
/// has needed this has got it wrong once.
/// What the reader offers about `message`'s list, with the account that
/// would own the activation.
///
/// One read serving both the offer and the activation, so the sentence a
/// person saw and the row that gets written cannot be derived differently.
/// Free rather than a method because it holds no session state: what it
/// needs is a connection and an id.
async fn unsubscribe_offer_for(
    connection: &postio_storage::Checkout,
    id: postio_model::ids::MessageId,
) -> Option<(postio_model::ids::AccountId, postio_ui::unsubscribe::Offer)> {
    let repository = postio_storage::repository::MessageRepository::new(connection);
    let message = repository.get(id).await.ok()??;
    // `send_state` is a column on the row rather than a field on `Message`,
    // so it is a second point read. It is also the gate (#1525): without it
    // the domain fallback offers to unsubscribe the user from their own
    // account, so a read that fails is treated as "do not offer" rather than
    // as "no send state".
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

pub(crate) fn blocking<T>(future: impl std::future::Future<Output = T>) -> T {
    postio_session::blocking::now(future)
}
