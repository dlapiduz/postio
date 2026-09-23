//! Opening a session, draining its events, and shutting it down.

use std::sync::{Arc, Mutex};

use postio_core::bridge::{Bridge, CommandSender, EventStream, event_channel};
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
    /// The tokio runtime the engine needs could not be started.
    #[error("{message}")]
    RuntimeUnavailable {
        /// What the runtime said.
        message: String,
    },
}

impl SessionError {
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
/// `postio-app`-shaped callers and tests use.
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
}

/// The command bus, handed over once the store behind it is open.
///
/// `Bridge::new` is called before the database exists — `open` asks the
/// keyring first and only then the store, deliberately, so that a refused key
/// leaves no half-made store behind — and the real bus needs a `Database`.
/// Rather than reorder that, the bridge is given this: a handler that holds
/// the real one once there is one to hold.
///
/// The alternative it replaces was `handler_fn(|_, _| async {})`, which
/// received every command and dropped it.
/// The commands the boundary answers itself, rather than sending down the bus.
///
/// `handle_locally` is the implementation; this is the same list as data, so
/// that `command_coverage.rs` can sweep every registry command and say which
/// of them reach nothing at all. `postio-gtk` has the same pair — its sweep is
/// `app_suite/command_wiring.rs`, and its `KNOWN_ORPHANS` list is empty
/// because that sweep has existed long enough to have emptied it.
///
/// They are here and not in Swift because what they move — the cursor, the
/// selection, the row window — is here. A frontend that moved them would need
/// its own copy of all three, which is the second model ADR 0019 exists to
/// prevent.
pub const HANDLED_HERE: &[postio_core::CommandId] = {
    use postio_core::CommandId as C;
    &[
        C::NextMessage,
        C::PrevMessage,
        C::FirstMessage,
        C::LastMessage,
        C::ToggleSelection,
        C::ExtendSelectionDown,
        C::ExtendSelectionUp,
        C::SelectAll,
        C::Back,
    ]
};

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

/// The next session's number within this process.
///
/// See [`Session::serial`]. Monotonic and never reused: a session that has
/// closed may still have a hand-off file somebody is editing.
fn next_serial() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[derive(Clone, Default)]
struct DeferredBus(Arc<Mutex<Option<Arc<postio_core::dispatch::Dispatcher>>>>);

impl DeferredBus {
    /// Build the real bus over `database` and hand it over.
    ///
    /// `state` is the same handle the session mirrors its view into before
    /// each send — the actions resolve `MessageTarget::Selection` against it,
    /// so a second `SharedState` here would resolve every such verb against
    /// an empty one (#1300).
    fn arm(
        &self,
        database: &postio_storage::Store,
        state: postio_core::state::SharedState,
        engine: postio_session::refresh::EngineSlot,
    ) {
        let actions = postio_session::actions::Actions::new(database.clone(), state.clone());
        let builder =
            postio_session::actions::wire(postio_core::dispatch::Dispatcher::builder(), actions);
        // **Both halves, the way `postio-app` composes them.** Only `actions`
        // was wired here, so `Refresh` reached no handler at all -- while it
        // sat in the File menu, on `F5` and `R`, and in the palette. Three
        // surfaces offering a key that did nothing, on the one platform where
        // "check for new mail" is the gesture people reach for first because
        // there is no push notification to beat them to it.
        // `command_coverage.rs` is what now notices.
        let bus = postio_session::refresh::wire(builder, engine, state).build();
        *self.0.lock().expect("deferred bus lock") = Some(Arc::new(bus));
    }
}

impl postio_core::bridge::CommandHandler for DeferredBus {
    fn handle(
        &self,
        command: postio_core::Command,
        events: postio_core::bridge::EventSink,
    ) -> postio_core::bridge::HandlerFuture {
        let bus = self.0.lock().expect("deferred bus lock").clone();
        Box::pin(async move {
            match bus {
                Some(bus) => bus.handle(command, events).await,
                // Only possible between `Bridge::new` and the store opening,
                // which is microseconds and has no frontend attached yet. Said
                // out loud rather than dropped, because dropping commands
                // silently is the bug this type exists to end.
                None => tracing::error!(
                    ?command,
                    "a command arrived before the store was open and was not run: {command:?}"
                ),
            }
        })
    }
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

    /// An in-memory session on a runtime and command bus the caller owns.
    ///
    /// `postio-app` builds its own [`Bridge`] and hands the parts to
    /// [`Wiring`]; a frontend on this boundary must be able to do the same,
    /// or it would end up with two runtimes and the deadlock that implies.
    #[cfg(feature = "testing")]
    pub fn in_memory_on(runtime: tokio::runtime::Handle, commands: CommandSender) -> Self {
        Self::in_memory().on_bridge(runtime, commands)
    }

    /// Run on a runtime and command bus the caller owns, keeping whatever
    /// store these options already name.
    ///
    /// The production shape: Swift owns the bridge, and the store is
    /// whichever one the session was opened over. The two used to be
    /// expressible only separately — `in_memory_with` gave a seeded store
    /// with no bus, `in_memory_on` a bus over an empty one — so a test that
    /// wanted a verb to reach real handlers over real rows could have
    /// neither (#721).
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
    Document(String),
    /// Whatever `config.toml` this installation has, or the defaults if there
    /// is none. What a shipping application wants, and what a test gets only
    /// by asking for it by name.
    Installed,
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
    };
    parsed.unwrap_or_else(|| {
        match source {
            ConfigSource::Installed => tracing::warn!(
                "using the built-in configuration: config.toml is absent or unreadable"
            ),
            ConfigSource::Document(_) => tracing::warn!(
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
    let (resolver, problems) = postio_ui::keymap::Resolver::from_commands(&keymap);
    for problem in &problems {
        tracing::warn!(%problem, "a key binding could not be used");
    }
    resolver
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
    /// What the actions resolve `MessageTarget::Selection` against (#1300).
    ///
    /// Brought into step with the view by `aim::mirror` immediately before
    /// each send, rather than kept in step by a signal — see that function.
    state: postio_core::state::SharedState,
    /// Where mirroring's events go, which is nowhere: the view is where they
    /// came from and telling it back would be a round trip to nothing.
    quiet: postio_core::bridge::EventSink,
    /// The list, windowed. Behind its own lock rather than inside `wiring`'s
    /// so that a row lookup -- which happens on every table redraw -- does not
    /// contend with whatever else is holding the session.
    list: Arc<Mutex<postio_ui::list::ListWindow<crate::RowFfi>>>,
    /// What the window is showing, what a page of it means and what an event
    /// does to it — [`postio_ui::paging::Paging`], the policy `postio-gtk`'s
    /// feed follows too, so a page fetch and an event reaction are one rule
    /// on both frontends.
    paging: Mutex<postio_ui::paging::Paging>,
    /// What the user has marked, and where the keyboard is.
    ///
    /// Held here rather than passed in with every [`Session::invoke`] (#721).
    /// A selection is not always a list of ids: `Ctrl+A` makes it a
    /// *predicate* over the whole view, and marshalling that across the
    /// boundary as an array would mean materialising a mailbox — the one
    /// thing this list exists not to do. So the predicate stays on this side,
    /// and Swift moves it with the same small verbs `postio-ui` gives GTK.
    selection: Mutex<postio_core::state::Selection>,
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
    /// The row the keyboard is on, as the frontend last reported it.
    cursor: Mutex<Option<postio_model::ids::MessageId>>,
    /// Where that row *is*, so motion and range extension have something to
    /// count from.
    ///
    /// Held beside the id rather than derived from it: finding an id's
    /// position means scanning the window, `j` happens on every keypress, and
    /// a row whose page has not arrived has no id to be found by at all.
    cursor_row: Mutex<Option<u32>>,
    /// The current result set, ranked, when a search is what the list shows.
    ///
    /// `None` means the list is showing a folder. Ranked rather than sorted,
    /// which is why no `ListScope` describes it: search hits come back in
    /// relevance order and the store has no scope that lists them.
    ///
    /// Capped at `postio_session::search::HIT_LIMIT`, so holding it is
    /// bounded — two hundred excerpts, not a mailbox. The *rows* are still
    /// paged in behind the table exactly as a folder's are; what is resident
    /// here is the ids and their excerpts.
    hits: Mutex<Option<Vec<crate::search::Hit>>>,
    /// The conversation the reading pane is showing, once its read lands.
    ///
    /// Held here rather than paged through `list`: the list is the list, and
    /// a pane that borrowed the window would have to put the folder back
    /// afterwards. A conversation is bounded — a thread, not a mailbox — so
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
    /// What the last search turned out to be, for the field's readout.
    ///
    /// Held rather than recomputed: the timing is a fact about the run that
    /// happened, and a second search to measure the first would be absurd.
    outcome: Mutex<Option<postio_ui::search::Outcome>>,
    /// This session's number within the process.
    ///
    /// Only [`handoff_dir`](Self::handoff_dir) needs it, and only on the
    /// path where there is no store on disk to hang a directory off — but
    /// two sessions sharing one hand-off directory is two people's drafts in
    /// one file, so it is worth a counter.
    serial: u64,
    /// What was last asked for, verbatim.
    ///
    /// Kept because an empty result set has to say *which* query found
    /// nothing — "No messages" over a mailbox holding thousands is a
    /// confident false statement about somebody's own mail, and the sentence
    /// that is not a lie needs the query in it (ADR 0005 Q10).
    query: Mutex<Option<String>>,
    /// The scope to come back to when a search is cleared.
    ///
    /// Held here rather than remembered by the frontend, because a frontend
    /// that remembered it would own navigation state — and would then own it
    /// differently from the GTK side. Clearing restores the previous scope
    /// rather than reloading the world.
    resting: Mutex<Option<postio_runtime::store::ListScope>>,
    /// The order results come back in, and the query they came back for.
    ///
    /// Here rather than in a frontend because it is an *answer* about the
    /// result set: `o` re-asks the same question a different way, and a
    /// frontend that re-sorted the rows it already had would be ordering a
    /// page of a windowed list rather than the search (#499).
    ///
    /// The order outlives the query on purpose. Somebody who asked for
    /// newest-first has said how they read results, not how they read that
    /// one result set, and a toggle that reset itself every search is a
    /// setting you have to keep re-pressing.
    result_order: Mutex<postio_search::ResultOrder>,
    /// Which slice of the mailbox a search looks at: the scope rail (#1157).
    ///
    /// Unlike the order, this does *not* outlive the search: a new search
    /// starts from All mail, because search is how you find what you filed
    /// and forgot, and a narrowing that silently carried into the next query
    /// would hide exactly that. [`clear_search`](Self::clear_search) resets
    /// it.
    search_scope: Mutex<postio_search::facets::Scope>,
    /// How many accounts the open view is about: one, or all of them.
    ///
    /// Resolved when the scope changes rather than on every palette keystroke:
    /// a mailbox belongs to one account and the store is what knows which, and
    /// the palette asks this on every character typed. It decides only whether
    /// a command with `Requirement::SingleAccount` is offered — `Move` needs
    /// somewhere in *that* account to put something, and a unified view has
    /// no such somewhere (#182).
    account_scope: Mutex<postio_core::Scope>,
    /// Where a shift-extension started.
    ///
    /// The anchor is what makes shift *extend* rather than accumulate: the
    /// range is always anchor-to-cursor, so shrinking it back unmarks the rows
    /// it passed. Set on the first extension from wherever the cursor was, and
    /// dropped whenever the selection is cleared or the list is re-scoped.
    anchor: Mutex<Option<u32>>,
    /// Page reads still in flight, and how many have been issued in total.
    ///
    /// The first is what `settle_for_test` waits on. The second is how a test
    /// can assert that three misses inside one page did not become three
    /// reads -- deduplication that `ListWindow` does, and that this must not
    /// undo by asking again behind its back.
    in_flight: Arc<std::sync::atomic::AtomicUsize>,
    reads: Arc<std::sync::atomic::AtomicUsize>,
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
    /// Retained rather than leaked, for the reason `postio-app` records:
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
    keys: postio_config::keys::KeyBindings,
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
    /// Events this boundary raises itself, merged into the drain alongside
    /// the engine's. `PageReady` lives here rather than in `postio-core`
    /// because paging is how this frontend reads a list, not something the
    /// engine does — see `UiEvent::PageReady`.
    local: (
        async_channel::Sender<UiEvent>,
        async_channel::Receiver<UiEvent>,
    ),
    events: EventStream,
    /// Kept alive for as long as the session is: dropping the `Bridge` stops
    /// the runtime the engine is polling on. `None` when the caller supplied
    /// their own, because then it is not ours to stop.
    _bridge: Option<Bridge>,
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

    /// Show `scope`, and answer the generation the window is now on.
    #[uniffi::method(name = "openScope")]
    pub fn open_scope_ffi(&self, scope: crate::ScopeFfi) -> u64 {
        self.open_scope(scope)
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
    pub fn thread_document_ffi(
        &self,
        thread: i64,
        originals: Vec<i64>,
    ) -> crate::ThreadDocumentFfi {
        blocking(self.thread_document(thread, originals))
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

    /// Add an account that signs in with a password. `None` when it was
    /// added, a sentence when it was not.
    #[uniffi::method(name = "addImapAccount")]
    #[allow(clippy::too_many_arguments)]
    pub fn add_imap_account_ffi(
        &self,
        address: String,
        password: String,
        imap_host: String,
        imap_port: u16,
        smtp_host: String,
        smtp_port: u16,
    ) -> Option<String> {
        blocking(self.add_imap_account(
            address, password, imap_host, imap_port, smtp_host, smtp_port,
        ))
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

    /// How many rows the current scope has — a table's `numberOfRows`.
    #[uniffi::method(name = "rowCount")]
    pub fn row_count_ffi(&self) -> u32 {
        self.row_count()
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

    /// Where the cursor is, as a row. See [`Session::cursor_row`].
    #[uniffi::method(name = "cursorRow")]
    pub fn cursor_row_ffi(&self) -> Option<u32> {
        self.cursor_row()
    }

    /// The message the cursor is on, if its page has arrived.
    #[uniffi::method(name = "cursorMessage")]
    pub fn cursor_message_ffi(&self) -> Option<i64> {
        self.cursor_message()
    }

    /// Run `query`, and show its hits. See [`Session::search`].
    ///
    /// Answers the generation the window is now on, exactly as
    /// [`openScope`](Session::open_scope_ffi) does — the frontend reloads its
    /// table against it and pages arrive behind, the same as for a folder.
    #[uniffi::method(name = "search")]
    pub fn search_ffi(&self, query: String) -> u64 {
        blocking(self.search(&query))
    }

    /// The query the rows on screen came from. See
    /// [`Session::search_query`].
    #[uniffi::method(name = "searchQuery")]
    pub fn search_query_ffi(&self) -> Option<String> {
        self.search_query()
    }

    /// Which order the results are in, for the sort control. See
    /// [`Session::result_order_label`].
    #[uniffi::method(name = "resultOrderLabel")]
    pub fn result_order_label_ffi(&self) -> String {
        self.result_order_label()
    }

    /// The scope counts and refine chips for the results on screen. See
    /// [`Session::search_facets`].
    #[uniffi::method(name = "searchFacets")]
    pub fn search_facets_ffi(&self) -> crate::SearchFacetsFfi {
        blocking(self.search_facets())
    }

    /// Which scope the search is looking in. See [`Session::search_scope`].
    #[uniffi::method(name = "searchScope")]
    pub fn search_scope_ffi(&self) -> crate::SearchScopeFfi {
        self.search_scope()
    }

    /// Look in `scope` and ask the query again. See
    /// [`Session::set_search_scope`].
    #[uniffi::method(name = "setSearchScope")]
    pub fn set_search_scope_ffi(&self, scope: crate::SearchScopeFfi) -> u64 {
        blocking(self.set_search_scope(scope))
    }

    /// Read the results the other way round. See
    /// [`Session::toggle_result_order`].
    #[uniffi::method(name = "toggleResultOrder")]
    pub fn toggle_result_order_ffi(&self) -> u64 {
        blocking(self.toggle_result_order())
    }

    /// Leave search and restore the scope that was open.
    #[uniffi::method(name = "clearSearch")]
    pub fn clear_search_ffi(&self) -> u64 {
        self.clear_search()
    }

    /// What the last search turned out to be. See [`Session::search_outcome`].
    #[uniffi::method(name = "searchOutcome")]
    pub fn search_outcome_ffi(&self) -> Option<crate::OutcomeFfi> {
        self.search_outcome()
    }

    /// What to draw over an empty list. See [`Session::empty_plate`].
    #[uniffi::method(name = "emptyPlate")]
    pub fn empty_plate_ffi(&self) -> Option<crate::EmptyPlateFfi> {
        self.empty_plate()
    }

    /// Whether the list is showing search results rather than a folder.
    #[uniffi::method(name = "isSearching")]
    pub fn is_searching_ffi(&self) -> bool {
        self.is_searching()
    }

    /// The excerpt for `message`, with the match located.
    ///
    /// Text and ranges, never marked-up text: each frontend marks it its own
    /// way from one answer about what matched.
    #[uniffi::method(name = "snippetFor")]
    pub fn snippet_for_ffi(&self, message: i64) -> Option<crate::SnippetFfi> {
        self.snippet_for(message)
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

    /// Whether `message` is marked, for a row deciding how to draw itself.
    ///
    /// The *selection*, not the cursor. A table drawing its own selection
    /// would be drawing the cursor and calling it a selection, which is the
    /// conflation `PRODUCT.md` §9 forbids.
    #[uniffi::method(name = "isSelected")]
    pub fn is_selected_ffi(&self, message: i64) -> bool {
        self.is_selected(message)
    }

    /// What to show above the list — "12 selected" — or nothing.
    #[uniffi::method(name = "selectionSummary")]
    pub fn selection_summary_ffi(&self) -> Option<String> {
        self.selection_summary()
    }

    /// The cursor rested on `message` long enough to count as read.
    /// See [`Session::mark_read_on_dwell`].
    #[uniffi::method(name = "markReadOnDwell")]
    pub fn mark_read_on_dwell_ffi(&self, message: i64) {
        self.mark_read_on_dwell(message);
    }

    /// Put the cursor on `row` — what a click on the list means.
    ///
    /// Sets the position *and* the message, which
    /// [`setCursor`](Session::set_cursor_ffi) does not: after a click, `j`
    /// has to move from where the user clicked, and a boundary told only the
    /// id would have to scan the window to find out where that was.
    ///
    /// Raises `CursorMoved`, the same as a keystroke would. A frontend that
    /// only heard about keyboard moves would have two paths to keep in step.
    #[uniffi::method(name = "setCursorRow")]
    pub fn set_cursor_row_ffi(&self, row: Option<u32>) {
        if self.cursor_row() == row {
            return;
        }
        self.put_cursor_on(row);
    }

    /// Report which row the keyboard is on, or `None` for no row.
    #[uniffi::method(name = "setCursor")]
    pub fn set_cursor_ffi(&self, message: Option<i64>) {
        self.set_cursor(message);
    }

    /// Mark a row, or take it back out.
    #[uniffi::method(name = "toggleSelection")]
    pub fn toggle_selection_ffi(&self, message: i64) {
        self.toggle_selection(message);
    }

    /// Select everything this scope holds, without reading a page of it.
    #[uniffi::method(name = "selectAll")]
    pub fn select_all_ffi(&self) {
        self.select_all();
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

    /// Unmark everything.
    #[uniffi::method(name = "clearSelection")]
    pub fn clear_selection_ffi(&self) {
        self.clear_selection();
    }

    /// The row at `position`, or `None` while its page is on its way.
    ///
    /// Synchronous and does no I/O, because `tableView(_:viewFor:row:)` is
    /// synchronous and runs on the main thread for every visible row on every
    /// redraw. A `None` means draw a placeholder; `UiEvent.pageReady` says
    /// when to ask again.
    #[uniffi::method(name = "rowAt")]
    pub fn row_at_ffi(&self, position: u32) -> Option<crate::RowFfi> {
        self.row_at(position)
    }

    /// One message as a row, by id. See [`Session::row_for`].
    #[uniffi::method(name = "rowFor")]
    pub fn row_for_ffi(&self, message: i64) -> Option<crate::RowFfi> {
        self.row_for(message)
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
        original: bool,
    ) -> crate::ReaderDocumentFfi {
        blocking(self.reader_answers(message, remote, original))
    }

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// What a `WKURLSchemeHandler` for `postio-cid:` answers with. `nil` is a
    /// broken image, deliberately — never a fetch.
    #[uniffi::method(name = "resolveCid")]
    pub fn resolve_cid_ffi(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        blocking(self.resolve_cid(message, content_id))
    }

    /// What `message` is made of: its MIME tree, flattened in walk order.
    ///
    /// **Reads the store and nothing else.** The rows came from
    /// `BODYSTRUCTURE`, which the server returns without transferring a byte
    /// of any part, so a panel drawn from this is complete and correct for a
    /// message whose attachments are all still on the server —
    /// `PartFfi.downloaded` is what says which of them are here. Drawing the
    /// panel therefore cannot touch the network, which is the shape "nothing
    /// downloads until the user asks" takes at this boundary.
    ///
    /// Blocks on a local read, like `mailboxes` does: a panel is drawn in
    /// response to a keypress and the read is a few milliseconds of SQLite.
    #[uniffi::method(name = "messageParts")]
    pub fn message_parts_ffi(&self, message: i64) -> crate::MessagePartsFfi {
        blocking(self.message_parts(message))
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

    /// Write one part to exactly `path`.
    ///
    /// For the save where the *user* named the file: an `NSSavePanel` has
    /// already run, offering `PartFfi.saveName`, and this is what happens
    /// next. Replaces rather than appends.
    ///
    /// Under App Sandbox the URL the panel returns is security-scoped, so the
    /// caller must bracket this with `startAccessingSecurityScopedResource`
    /// — the write happens on this side, and a scope that is not open here
    /// fails as a permission error rather than as a dialog.
    ///
    /// **Never from the main actor**: it fetches through `partBytes`, so it
    /// inherits that call's wait. Saving the attachment on a message that has
    /// only been described is exactly the ordinary case, not the rare one.
    #[uniffi::method(name = "savePart")]
    pub fn save_part_ffi(
        &self,
        message: i64,
        part_id: String,
        path: String,
    ) -> Result<(), crate::PartsError> {
        blocking(self.save_part(message, part_id, path))
    }

    /// Write one part into `directory`, under the name Postio chose, and say
    /// where it landed.
    ///
    /// What "Open with…" and a drag-out are built on. The caller supplies a
    /// directory and **never a filename**: the name is always the sanitised
    /// `PartFfi.saveName`, so the sender cannot choose what a file handed to
    /// another application is called. That is the whole difference from
    /// `savePart`, and it is deliberate — this is the path where the bytes
    /// leave Postio's own window.
    ///
    /// Launching is the caller's, under a `POSTIO-CONSENT:` comment. Nothing
    /// on this side opens anything.
    ///
    /// **Never from the main actor**, for `savePart`'s reason: a part that is
    /// not here yet is fetched and waited on first. A drag-out that blocked
    /// the main actor would freeze the drag it is part of.
    #[uniffi::method(name = "exportPart")]
    pub fn export_part_ffi(
        &self,
        message: i64,
        part_id: String,
        directory: String,
    ) -> Result<String, crate::PartsError> {
        blocking(self.export_part(message, part_id, directory))
    }

    /// Write every part that holds bytes into `directory`.
    ///
    /// Each under its own name, including the suffix that stops two parts
    /// both calling themselves `invoice.pdf` from becoming one file. A part
    /// that cannot be had is counted, not thrown: a message where one
    /// attachment is on an unreachable server should still give the user the
    /// other four, with one sentence saying what was missed.
    ///
    /// **Never from the main actor, and the worst of the four.** The parts
    /// are fetched one after another, each with its own thirty-second wait,
    /// so a twelve-part message against a server that has stopped answering
    /// keeps this thread for six minutes. There is no cancellation yet: a
    /// frontend that wants one has to stop *waiting* rather than stop the
    /// work, and should say on screen that the save is still running.
    #[uniffi::method(name = "saveAllParts")]
    pub fn save_all_parts_ffi(
        &self,
        message: i64,
        directory: String,
    ) -> Result<crate::SavedPartsFfi, crate::PartsError> {
        blocking(self.save_all_parts(message, directory))
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
    /// (the pane's *Partial* state). Neither is repaired by adding the
    /// account again: `addImapAccount` leaves an address the store already
    /// knows exactly as it found it.
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
            show_key_hints: self.ui.show_key_hints,
            sender_avatars: self.ui.sender_avatars,
        }
    }
    /// The key hints the focused row announces, in canvas order.
    ///
    /// From this session's keymap, so a rebinding reaches the hint: a row
    /// that taught the wrong key would be worse than one that taught none.
    /// `postio_ui::row::hints` decides which verbs get one, and both
    /// frontends show the same two.
    pub fn row_hints(&self) -> Vec<crate::RowHintFfi> {
        postio_ui::row::hints(&self.keymap())
            .into_iter()
            .map(|(key, label)| crate::RowHintFfi {
                key,
                label: label.to_string(),
            })
            .collect()
    }

    /// The key hints the search bar announces — `Ret open · Tab refine ·
    /// C-s save as folder` (canvas 05).
    ///
    /// The same shape as [`row_hints`](Self::row_hints) one pane over, and
    /// for the same reason: read from this session's keymap, so a rebinding
    /// reaches the footer. It is the only place most people will ever read
    /// these keys, which is what makes teaching the wrong one worse than
    /// teaching none.
    pub fn search_hints(&self) -> Vec<crate::RowHintFfi> {
        postio_ui::search::hints(&self.keymap())
            .into_iter()
            .map(|(key, label)| crate::RowHintFfi {
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
    /// alone, and `postio-app` reaches for exactly the same value the same
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
    /// The route `AccountFfi::repair` names as `Password`, and the only way
    /// a rotated app password reaches the keyring: `add_imap_account` writes
    /// nothing over an address the store already knows, deliberately, and
    /// `postio_session::provision::repair`'s own docs argue why that has to
    /// stay true of the headless helper it is shared with.
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
    /// keyring's own timeout rather than indefinitely. `postio-app` does the
    /// same thing before any window exists. **A Swift caller must not invoke
    /// it on the main actor**: it belongs in a launch task, with the unlock
    /// surface shown if it comes back [`SessionError::KeyringLocked`].
    pub fn open(options: SessionOptions) -> Result<Arc<Self>, SessionError> {
        // A hub rather than a channel: the frontend drains one subscription
        // and the body indexer another, the way `postio-app`'s window and
        // indexer share its hub. This boundary had no body indexer before
        // and relied on the fetch to write the search row -- which it no
        // longer does anywhere.
        let hub = postio_core::bridge::EventHub::new();
        let sink = hub.sink();
        let events = hub.subscribe("frontend");

        // Read before anything moves out of `options`, and once: both paths
        // below build the same configuration from it.
        let source = config_source(&options);

        // The bus a session builds when it was given none — which is every
        // shipped Postio, because `openAt` supplies none.
        //
        // It used to be `handler_fn(|_, _| async {})`: every command received
        // and dropped. Only the verbs `handle_locally` intercepts — the
        // cursor and the selection — did anything at all, so moving through
        // the list worked and *archive, flag, delete, mark read and undo did
        // nothing*, silently. It survived because every test that invokes a
        // real verb hands the session a bus of its own, so the one the
        // application actually runs on was never exercised.
        //
        // The real bus needs `Actions`, which needs the database, which is
        // opened further down — so the handler is deferred rather than the
        // store hoisted: `open` is delicate about its order (the keyring
        // before the store, deliberately) and this changes none of it.
        let deferred = DeferredBus::default();
        // What the actions resolve `MessageTarget::Selection` against, and
        // what `invoke` mirrors the view into immediately before sending.
        // `aim::mirror`'s own doc argues the pull: a push would have to fire
        // on every `j`, and a pull cannot be a gesture out of date.
        let state = postio_core::state::SharedState::default();
        // The events mirroring produces have nowhere to go — the view is
        // where they came from — so this reader is dropped on purpose.
        let (quiet, quiet_reader) = event_channel();
        drop(quiet_reader);
        let (runtime, commands, owned_bridge) = match options.bridge {
            Some((runtime, commands)) => (runtime, commands, None),
            None => {
                let (bridge, _replies) = Bridge::new(deferred.clone()).map_err(|error| {
                    SessionError::RuntimeUnavailable {
                        message: error.to_string(),
                    }
                })?;
                (bridge.handle(), bridge.commands(), Some(bridge))
            }
        };

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
            // One slot, shared: `refresh::wire` reads it and `Wiring` fills
            // it, and two of them is a handler watching a slot nothing ever
            // puts an engine into.
            let engine_slot = postio_session::refresh::EngineSlot::default();
            deferred.arm(&database, state.clone(), engine_slot.clone());
            let mut wiring = Wiring::new(database, blobs, runtime, sink, commands)
                .with_engine_slot(engine_slot)
                .with_backfill(postio_session::backfill_policy(&sync_config))
                .with_watch(postio_session::watch_policy(&sync_config));
            // Honour `with_secrets` here too. It was read only on the real
            // path, so an in-memory session that had been handed a test
            // keyring quietly used the **login keychain** instead — which is
            // how a test suite came to hang on a macOS permission prompt
            // nobody could see, after writing a password into a developer's
            // own keychain.
            if let Some(secrets) = options.secrets.clone() {
                wiring = wiring.with_secrets(secrets);
            }
            let keys = config.keys;
            postio_session::spawn_body_indexer(
                wiring.database.clone(),
                wiring.events.subscribe("indexer"),
                &wiring.runtime,
            );
            return Ok(Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
                state,
                quiet,
                resolver: Mutex::new(build_resolver(&keys)),
                ui: config.ui,
                keys,
                list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
                selection: Mutex::new(postio_core::state::Selection::default()),
                reachable: Mutex::new(Vec::new()),
                cursor: Mutex::new(None),
                cursor_row: Mutex::new(None),
                account_scope: Mutex::new(postio_core::Scope::default()),
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
                hits: Mutex::new(None),
                outcome: Mutex::new(None),
                serial: next_serial(),
                query: Mutex::new(None),
                resting: Mutex::new(None),
                result_order: Mutex::new(postio_search::ResultOrder::Relevance),
                search_scope: Mutex::new(postio_search::facets::Scope::AllMail),
                anchor: Mutex::new(None),
                paging: Mutex::new(postio_ui::paging::Paging::default()),
                in_flight: Arc::default(),
                reconnects: Arc::default(),
                offline: Arc::default(),
                engines: Mutex::new(Vec::new()),
                reads: Arc::default(),
                local: async_channel::unbounded(),
                events,
                _bridge: owned_bridge,
                _scratch: Some(scratch),
            }));
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
        // Blocked on `runtime`, which exists by now: opening the store is
        // async, and this constructor's documented contract is that it
        // blocks. That is what the sentence above about the keyring is
        // already telling a Swift caller -- do not invoke this on the main
        // actor -- and it covers the store open for exactly the same reason.
        let (database, blobs) = blocking(postio_session::open_store_at(path, &key))
            .map_err(|message| SessionError::StoreUnavailable { message })?;

        let config = load_config(&source);
        let keys = config.keys;
        let sync_config = config.sync;
        let ui_config = config.ui;

        let engine_slot = postio_session::refresh::EngineSlot::default();
        deferred.arm(&database, state.clone(), engine_slot.clone());
        let wiring = Wiring::new(database, blobs, runtime, sink, commands)
            .with_engine_slot(engine_slot)
            .with_secrets(secrets)
            .with_backfill(postio_session::backfill_policy(&sync_config))
            .with_watch(postio_session::watch_policy(&sync_config));
        postio_session::spawn_body_indexer(
            wiring.database.clone(),
            wiring.events.subscribe("indexer"),
            &wiring.runtime,
        );
        Ok(Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            state,
            quiet,
            resolver: Mutex::new(build_resolver(&keys)),
            ui: ui_config,
            keys,
            engines: Mutex::new(Vec::new()),
            list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
            selection: Mutex::new(postio_core::state::Selection::default()),
            reachable: Mutex::new(Vec::new()),
            cursor: Mutex::new(None),
            cursor_row: Mutex::new(None),
            account_scope: Mutex::new(postio_core::Scope::default()),
            conversation: Arc::default(),
            sign_in: Mutex::new(None),
            sign_in_port: Arc::default(),
            allow_list_at: store_at
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(&store_at)
                .join("allowed-senders.toml"),
            store_at,
            hits: Mutex::new(None),
            outcome: Mutex::new(None),
            serial: next_serial(),
            query: Mutex::new(None),
            resting: Mutex::new(None),
            result_order: Mutex::new(postio_search::ResultOrder::Relevance),
            search_scope: Mutex::new(postio_search::facets::Scope::AllMail),
            anchor: Mutex::new(None),
            paging: Mutex::new(postio_ui::paging::Paging::default()),
            in_flight: Arc::default(),
            reads: Arc::default(),
            reconnects: Arc::default(),
            offline: Arc::default(),
            local: async_channel::unbounded(),
            events,
            _bridge: owned_bridge,
            #[cfg(feature = "testing")]
            _scratch: None,
        }))
    }

    /// Show `scope`, and answer the generation the window is now on.
    ///
    /// Blocks on a `COUNT` against the local store — a few milliseconds of
    /// SQLite, never the network. It has to be synchronous because
    /// `numberOfRows` is: a table asks how tall it is before it draws
    /// anything, and there is no version of that question which can await.
    pub fn open_scope(&self, scope: crate::ScopeFfi) -> u64 {
        self.open_list_scope(scope.into())
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
            let rows = match store.list_page(request).await {
                Ok(page) => crate::list::page_of(page).rows,
                Err(error) => {
                    tracing::debug!(%error, thread, "the conversation could not be read");
                    Vec::new()
                }
            };
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
    /// pane edits, and the user's own messages marked. `originals` are the
    /// messages the reader asked to see as sent (`⌃O`).
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
        originals: Vec<i64>,
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

        let originals: std::collections::HashSet<String> =
            originals.iter().map(|id| id.to_string()).collect();
        let allow = self.allow_list();
        let html = postio_ui::reader::thread::compose(
            &messages,
            |address| allow.is_allowed(address),
            &originals,
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
        // behavioural record of what a notice means. Blocked and original,
        // which were always this question's terms.
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, true)
            .await
            .notice
    }

    /// Always allow `address`. See [`allow_sender_ffi`](Self::allow_sender_ffi).
    pub fn allow_sender(&self, address: String) {
        self.amend_allow_list(|list| list.allow(&address));
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
    ///
    /// See [`decode_caveat_ffi`](Self::decode_caveat_ffi).
    pub async fn decode_caveat(&self, message: i64) -> Option<String> {
        // A view over `reader_answers` — see `reader_notice`.
        self.reader_answers(message, crate::RemoteImagesFfi::Blocked, true)
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
    ///
    /// See [`unsubscribe_offer_ffi`](Self::unsubscribe_offer_ffi).
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
        // which `postio-app`'s privacy pane was already making with its own
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

    /// Add an account. See [`add_imap_account_ffi`](Self::add_imap_account_ffi).
    ///
    /// The password goes to the OS keyring under the address and nowhere
    /// else — never `config.toml`, never a log (ADR 0014). The row is written
    /// only after the keyring has taken it, so a locked keyring leaves no
    /// half-made account behind; `postio_session::provision` is where that
    /// order is argued.
    #[allow(clippy::too_many_arguments)]
    pub async fn add_imap_account(
        &self,
        address: String,
        password: String,
        imap_host: String,
        imap_port: u16,
        smtp_host: String,
        smtp_port: u16,
    ) -> Option<String> {
        if !address.contains('@') {
            return Some(format!("{address} does not look like an email address."));
        }
        // Refused rather than written: an account naming no server fails
        // later, at sync, as a connection error nobody can act on.
        if imap_host.trim().is_empty() || smtp_host.trim().is_empty() {
            return Some(
                "Postio needs the incoming and outgoing server names — it will not                  guess them from your address."
                    .to_owned(),
            );
        }
        let Some((database, _)) = self.store_and_blobs() else {
            return Some("There is no store open to add an account to.".to_owned());
        };

        let server = |host: String, port: u16| postio_account::discovery::ServerSettings {
            host,
            port,
            encryption: postio_account::discovery::Encryption::Tls,
        };
        let settings = postio_account::discovery::AccountSettings {
            email: address.clone(),
            imap: server(imap_host.trim().to_owned(), imap_port),
            smtp: server(smtp_host.trim().to_owned(), smtp_port),
            // The login is the address unless somebody says otherwise, and
            // the two differ more often than they look like they should —
            // every iCloud custom domain, for one. The sheet does not ask
            // yet; when it does, this is the field.
            login: address.clone(),
            // Typed by the person adding the account, which no probe can
            // claim: `Guess` is the honest source for settings nothing
            // discovered, and it is what stops this presenting itself as a
            // verified configuration.
            source: postio_account::discovery::SettingsSource::Guess,
            requires_app_password: false,
            note: None,
            password_help_url: None,
            display_name: None,
            oauth: None,
            jmap: None,
            backends: vec!["imap".to_owned()],
        };
        let account = postio_session::provision::account_from(&settings);

        let Some(secrets) = self.secret_store() else {
            return Some("There is no keyring to store the password in.".to_owned());
        };
        let password = postio_account::secret::Password::new(password);
        // Awaited rather than driven on a runtime of its own. This built one
        // per call, which is the fourth copy `blocking` was written to end:
        // borrowing the session's engine runtime deadlocked (`Handle::block_on`
        // needs that runtime driven, and under the full suite it is busy
        // elsewhere), and building a private one panics outright the moment
        // the caller is already on a runtime — which every test in this
        // suite now is. `blocking::now` at the FFI edge is what knows the
        // difference.
        let outcome =
            postio_session::provision::provision(&database, secrets.as_ref(), account, password)
                .await;
        match outcome {
            Ok(_) => None,
            Err(error) => Some(error.to_string()),
        }
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
        // sentence, and it is why `postio-app` has used this one since drafts
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
        // sent. The same check `postio-gtk`'s composer makes, for the same
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

    /// [`open_scope`](Self::open_scope), for a scope already in the store's
    /// own terms.
    ///
    /// Exists because leaving a search restores the scope it *remembered*,
    /// which never had a `ScopeFfi` spelling — it came off this side. A
    /// conversion back would be a second mapping to keep in step with the
    /// first, for no caller that needs one.
    fn open_list_scope(&self, listed: postio_runtime::store::ListScope) -> u64 {
        let Some((store, _runtime)) = self.reader() else {
            return 0;
        };
        let total = blocking(store.list_count(listed)).unwrap_or(0);
        self.paging.lock().expect("paging lock").open(listed);
        // "These twelve" means something else the moment the list does, and an
        // action carrying a selection across would land on mail the user
        // cannot see. The cursor goes with it: it named a row in a list that
        // no longer exists.
        self.drop_selection_and_cursor();
        // Opening a folder leaves a search, and there is nothing to come back
        // to: the user chose this scope rather than dismissing the query.
        *self.hits.lock().expect("hits lock") = None;
        *self.resting.lock().expect("resting lock") = None;
        // And the next search starts from All mail, as it does after
        // `Escape`: a different road out of the same search.
        *self.search_scope.lock().expect("search scope lock") =
            postio_search::facets::Scope::AllMail;
        *self.account_scope.lock().expect("account scope lock") =
            blocking(self.resolve_account_scope(listed));
        self.list.lock().expect("list lock").reset(total)
    }

    /// Turn a command id into the command it means here, and run it.
    ///
    /// **The whole of this frontend's aiming**, and it decides nothing: what
    /// a gesture acts on is `postio_core::aim`'s rule, and this hands it the
    /// facts — the scope on screen, what is marked, where the keyboard is,
    /// and the rows the window is holding. `postio-app` is the same three
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
        let Some(commands) = self
            .wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .map(|wiring| wiring.commands.clone())
        else {
            return;
        };

        // The commands that move this frontend's own state rather than the
        // engine's, handled here and not sent down. `postio-gtk`'s
        // `run_action` does exactly the same with the same ids -- the list
        // walks its own rows, and `Command::NextMessage` reaching the engine
        // would be a message to nobody.
        //
        // They live on *this* side of the boundary rather than in Swift
        // because the cursor, the selection and the row window are all here.
        // A frontend that moved them would need its own copy of all three,
        // which is the second model ADR 0019 exists to prevent -- and the
        // selection in particular is a predicate that must never be
        // enumerated to be moved.
        if self.handle_locally(id) {
            return;
        }

        // Before the list lock: resolving may take it, and a verb aimed at a
        // cursor whose page has landed since must find the message rather
        // than silently act on nothing.
        let cursor = self.resolve_cursor();

        let list = self.list.lock().expect("list lock");
        let selection = self.selection.lock().expect("selection lock");
        let aim = postio_core::aim::Aim {
            // The shared conversion, not a second one: `ScopeFfi` becomes a
            // `ListScope` on the way in, and `aim::view_scope` is the one
            // rule for what a whole-view gesture is relative to (#670).
            scope: self.scope_in_view().and_then(|scope| {
                postio_core::aim::view_scope(scope, &self.reachable.lock().expect("reachable lock"))
            }),
            selection: &selection,
            cursor,
            rows: &*list,
        };
        // Before the send and after the aim is whole: the actions resolve
        // `MessageTarget::Selection` against app state, and app state is not
        // the frontend's list — so without this every verb that `refine`
        // leaves aimed at the selection resolved against an empty state and
        // acted on nothing (#1300). `postio-app` has pulled the same way
        // since it had a bus; this is the same function, not a second copy.
        postio_core::aim::mirror(&self.state, &self.quiet, &aim);
        let command = postio_core::aim::command_for(id, &aim);
        drop(selection);
        drop(list);

        if commands.send(command).is_err() {
            // Only during teardown: the bridge has stopped and there is
            // nothing left to run the verb on.
            tracing::debug!("the runtime has stopped and did not run that");
        }
    }

    /// Resolve one key press against the bindings in force.
    ///
    /// **The whole of the frontend's keyboard, and it decides nothing.** The
    /// caller reduces its own event to the three things every toolkit can
    /// supply -- the character the key would type, the key's name when it
    /// types none, and the modifiers held -- and this hands them to
    /// `postio_ui::keymap`, which owns the table, the chords, the sequences
    /// and the leader timeout. `postio-gtk`'s `resolve_key` is the same shape
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
        // `postio-gtk`'s `resolve_key` logs the same three inputs for the same
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

    /// Run `id` here if it is this frontend's own state, and say whether it
    /// was.
    ///
    /// The split is the one `PRODUCT.md` §9 draws and `postio-gtk` already
    /// implements: **the cursor is not the selection**, and neither is
    /// anything the engine knows about. Moving down a list and marking a row
    /// are frontend state; archiving what is marked is not.
    fn handle_locally(&self, id: postio_core::CommandId) -> bool {
        use postio_core::CommandId as C;
        match id {
            C::NextMessage => self.move_cursor(1),
            C::PrevMessage => self.move_cursor(-1),
            C::FirstMessage => self.put_cursor_on(Some(0)),
            C::LastMessage => {
                let last = self.row_count().checked_sub(1);
                self.put_cursor_on(last);
            }
            C::ToggleSelection => {
                if let Some(message) = self.resolve_cursor() {
                    // The anchor follows a deliberate mark: a shift-extension
                    // afterwards runs from the row the user chose, not from
                    // wherever a previous range happened to start.
                    *self.anchor.lock().expect("anchor lock") =
                        *self.cursor_row.lock().expect("cursor row lock");
                    self.toggle_selection(message.get());
                }
            }
            C::ExtendSelectionDown => self.extend(1),
            C::ExtendSelectionUp => self.extend(-1),
            C::SelectAll => self.select_all(),
            // Escape means "get me out of here", and with mail marked the
            // thing to get out of is the selection. Only then: an Escape that
            // always cleared a selection would give the frontend no way to
            // close anything else, so an empty selection falls through to the
            // engine's own `Back`.
            C::Back if !self.selection_is_empty() => self.clear_selection(),
            _ => return false,
        }
        // The list below is what `command_coverage.rs` sweeps against, and
        // the two drift in the direction nobody notices: an arm added here
        // and not listed there looks, to the sweep, like a command nothing
        // answers -- and would be reported as an orphan that is not one. So
        // the arms say so out loud.
        debug_assert!(
            HANDLED_HERE.contains(&id),
            "{id} is answered by `handle_locally` and is not in `HANDLED_HERE`;              add it, or the coverage sweep will call it an orphan"
        );
        true
    }

    /// How many accounts `scope` is about.
    ///
    /// A mailbox is one account's, and the store is what knows whose — so
    /// this reads it, once, when the scope changes. Anything it cannot
    /// resolve is `Unified`, which is the conservative answer: it withholds
    /// the commands that need a single account rather than offering one that
    /// would have nowhere to act.
    async fn resolve_account_scope(
        &self,
        scope: postio_runtime::store::ListScope,
    ) -> postio_core::Scope {
        use postio_runtime::store::ListScope;
        match scope {
            // The Outbox names its account as plainly as these two do: every
            // row in it is that account's draft, on its way through that
            // account's server.
            ListScope::Account(account)
            | ListScope::Flagged(account)
            | ListScope::Outbox(account) => postio_core::Scope::Account(account),
            ListScope::Mailbox(mailbox) => {
                let Some((database, _)) = self.store_and_blobs() else {
                    return postio_core::Scope::Unified;
                };
                let Ok(connection) = database.connect().await else {
                    return postio_core::Scope::Unified;
                };
                postio_storage::repository::MailboxRepository::new(&connection)
                    .get(mailbox)
                    .await
                    .ok()
                    .flatten()
                    .map(|mailbox| postio_core::Scope::Account(mailbox.account_id))
                    .unwrap_or(postio_core::Scope::Unified)
            }
            ListScope::Unified | ListScope::Snoozed(_) | ListScope::Thread(_) => {
                postio_core::Scope::Unified
            }
        }
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
    /// `store_open` is unconditionally true here, and that is a fact about
    /// this type rather than an assumption: a `Session` is constructed *over*
    /// an open store, so there is no interval in which one does not exist.
    /// The window-first startup that makes [`Requirement::StoreOpen`] worth
    /// evaluating is `postio-app`'s (#1114), and a frontend that ever grows
    /// the same shape answers here instead of at a menu.
    ///
    /// [`Requirement::StoreOpen`]: postio_core::Requirement::StoreOpen
    fn availability(&self) -> postio_core::Availability {
        postio_core::Availability::open(*self.account_scope.lock().expect("account scope lock"))
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
    fn keymap(&self) -> postio_core::Keymap {
        postio_core::Keymap::resolve(&self.keys)
    }

    /// Whether nothing is marked.
    fn selection_is_empty(&self) -> bool {
        match &*self.selection.lock().expect("selection lock") {
            postio_core::state::Selection::These(marked) => marked.is_empty(),
            postio_core::state::Selection::Everything { .. } => false,
        }
    }

    /// Move the cursor by `delta` rows, clamped to the list.
    ///
    /// Clamped rather than wrapping: `j` at the bottom of a mailbox staying
    /// where it is what every list on the platform does, and jumping to the
    /// top would move the reader to a message the user did not ask for.
    fn move_cursor(&self, delta: i64) {
        let total = self.row_count();
        if total == 0 {
            return;
        }
        let at = match *self.cursor_row.lock().expect("cursor row lock") {
            // No cursor yet: the first `j` lands on the first row rather than
            // the second, and the first `k` on the last.
            None => {
                if delta > 0 {
                    0
                } else {
                    total - 1
                }
            }
            Some(row) => (row as i64 + delta).clamp(0, total as i64 - 1) as u32,
        };
        self.put_cursor_on(Some(at));
    }

    /// Put the cursor on `row`, and remember which message that is.
    ///
    /// Both, because they answer different questions: `aim` needs the id and
    /// motion needs the position. A row whose page has not arrived has a
    /// position and no id, which is a real state — the cursor is somewhere,
    /// and what is there is still being read.
    fn put_cursor_on(&self, row: Option<u32>) {
        *self.cursor_row.lock().expect("cursor row lock") = row;
        let message = row.and_then(|row| self.row_at(row)).map(|row| row.id);
        *self.cursor.lock().expect("cursor lock") = message.map(postio_model::ids::MessageId::new);
        self.emit_local(UiEvent::CursorMoved { row, message });
    }

    /// Extend the selection by one row in `delta`'s direction.
    ///
    /// Anchor-to-cursor, always, which is what makes this *extend* rather
    /// than accumulate: shrinking the range back unmarks the rows it passed,
    /// the way every list on the platform behaves.
    fn extend(&self, delta: i64) {
        {
            let mut anchor = self.anchor.lock().expect("anchor lock");
            if anchor.is_none() {
                *anchor = *self.cursor_row.lock().expect("cursor row lock");
            }
        }
        self.move_cursor(delta);

        let (Some(anchor), Some(cursor)) = (
            *self.anchor.lock().expect("anchor lock"),
            *self.cursor_row.lock().expect("cursor row lock"),
        ) else {
            return;
        };
        // `postio_ui::selection::range` rather than a loop here: it skips the
        // rows whose pages have not arrived rather than waiting for them,
        // which is the rule a selection that stutters would break.
        let rows: Vec<Option<postio_model::ids::MessageId>> = (0..self.row_count())
            .map(|row| self.list.lock().expect("list lock").peek(row))
            .collect();
        let marked = postio_ui::selection::range(&rows, anchor as usize, cursor as usize);
        *self.selection.lock().expect("selection lock") =
            postio_core::state::Selection::These(marked);
    }

    /// Where the cursor is, as a row.
    pub fn cursor_row(&self) -> Option<u32> {
        *self.cursor_row.lock().expect("cursor row lock")
    }

    /// The message the cursor is on, if its page has arrived.
    pub fn cursor_message(&self) -> Option<i64> {
        self.resolve_cursor().map(|message| message.get())
    }

    /// The cursor's message, filling the id in if its page has landed since.
    ///
    /// **The cursor is a row; the id is a cache of what is on it.** They are
    /// set together, but a cursor can land on a row whose page is still in
    /// flight — pressing `j` the instant a folder opens does exactly that —
    /// and the id is `None` then. Nothing re-resolved it when the page
    /// arrived, so the cursor stayed nameless and every verb aimed at it was
    /// a silent no-op: `a` archived nothing, space marked nothing, and the
    /// list looked like it had stopped responding to a keyboard it was in
    /// fact reading perfectly.
    ///
    /// Resolved on read rather than pushed from the page delivery, because
    /// delivery happens on the runtime's thread with only the window in hand,
    /// and reaching back for the cursor from there would put a second lock
    /// order into the one path that must not stall a redraw.
    fn resolve_cursor(&self) -> Option<postio_model::ids::MessageId> {
        if let Some(message) = *self.cursor.lock().expect("cursor lock") {
            return Some(message);
        }
        let row = (*self.cursor_row.lock().expect("cursor row lock"))?;
        // `peek`, not `row_at`: this must not start a fetch. It is called
        // from `invoke` on every keystroke, and a verb that triggered a page
        // read would be doing I/O to find out what it is about.
        let found = self.list.lock().expect("list lock").peek(row)?;
        *self.cursor.lock().expect("cursor lock") = Some(found);
        Some(found)
    }

    /// Whether `message` is marked, for a row deciding how to draw itself.
    ///
    /// Answers correctly for a whole-view selection without enumerating it,
    /// which is the point of the predicate: a row in `Everything` is marked
    /// unless it is one of the few taken out.
    pub fn is_selected(&self, message: i64) -> bool {
        let message = postio_model::ids::MessageId::new(message);
        match &*self.selection.lock().expect("selection lock") {
            postio_core::state::Selection::These(marked) => marked.contains(&message),
            postio_core::state::Selection::Everything { except } => !except.contains(&message),
        }
    }

    /// What to show above the list — "12 selected" — or nothing.
    ///
    /// From the model, which knows the answer for a whole-view selection
    /// without listing it. A frontend counting ids would be unable to draw
    /// this at all for the selection that most needs it.
    pub fn selection_summary(&self) -> Option<String> {
        postio_ui::selection::summary(
            &self.selection.lock().expect("selection lock"),
            Some(self.row_count()),
            &[],
        )
    }

    /// The cursor rested on `message` long enough for it to count as read.
    ///
    /// **Not `invoke`, and the difference matters.** `MarkReadOnDwell` is
    /// deliberately not a registry command: it routes to
    /// `CommandId::MarkUnread`'s handler so there is one "mark read" in the
    /// vocabulary, and it is the one dispatch that is *not* recorded on the
    /// undo stack — `u` takes back what you did, and reading a mailbox
    /// produces one of these per message rested on. Going through `invoke`
    /// would make every message read an undo entry and bury the archive you
    /// actually wanted back.
    ///
    /// The message is named rather than taken from the cursor, for the reason
    /// `postio-app` gives on the same call: the cursor may have moved on
    /// between the frontend's timer firing and this running, and the message
    /// that was read is the one the clock was started for.
    pub fn mark_read_on_dwell(&self, message: i64) {
        let Some(commands) = self
            .wiring
            .lock()
            .expect("wiring lock")
            .as_ref()
            .map(|wiring| wiring.commands.clone())
        else {
            return;
        };
        let command = postio_core::Command::MarkReadOnDwell {
            message: postio_model::ids::MessageId::new(message),
        };
        if commands.send(command).is_err() {
            tracing::debug!("the runtime has stopped and did not mark that read");
        }
    }

    /// Report where the keyboard is, so a verb with nothing marked knows
    /// which row it is about.
    pub fn set_cursor(&self, message: Option<i64>) {
        *self.cursor.lock().expect("cursor lock") = message.map(postio_model::ids::MessageId::new);
    }

    /// Mark `message`, or take it out of the selection again.
    pub fn toggle_selection(&self, message: i64) {
        let message = postio_model::ids::MessageId::new(message);
        let mut selection = self.selection.lock().expect("selection lock");
        *selection = match std::mem::take(&mut *selection) {
            postio_core::state::Selection::These(mut marked) => {
                if let Some(at) = marked.iter().position(|held| *held == message) {
                    marked.remove(at);
                } else {
                    marked.push(message);
                }
                postio_core::state::Selection::These(marked)
            }
            // Taking a row out of "everything" is what `except` is for —
            // turning the predicate into a list here would materialise the
            // mailbox this boundary exists not to materialise.
            postio_core::state::Selection::Everything { mut except } => {
                if let Some(at) = except.iter().position(|held| *held == message) {
                    except.remove(at);
                } else {
                    except.push(message);
                }
                postio_core::state::Selection::Everything { except }
            }
        };
    }

    /// Select everything the current scope holds — `Ctrl+A`.
    ///
    /// A predicate, not a list: the selection stays "everything in this view"
    /// however many rows that is, and no page is read to answer it.
    pub fn select_all(&self) {
        *self.selection.lock().expect("selection lock") =
            postio_core::state::Selection::Everything { except: Vec::new() };
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

    /// Unmark everything.
    pub fn clear_selection(&self) {
        *self.selection.lock().expect("selection lock") = postio_core::state::Selection::default();
    }

    /// What is marked right now, for a test or a frontend drawing a count.
    ///
    /// `None` while the selection is the whole view: there is no list to
    /// hand back, which is the point of it being a predicate.
    pub fn selected_messages(&self) -> Option<Vec<i64>> {
        match &*self.selection.lock().expect("selection lock") {
            postio_core::state::Selection::These(marked) => {
                Some(marked.iter().map(|id| id.get()).collect())
            }
            postio_core::state::Selection::Everything { .. } => None,
        }
    }

    /// How many rows the current scope has.
    pub fn row_count(&self) -> u32 {
        self.list.lock().expect("list lock").total()
    }

    /// The row at `position`, or `None` while its page is on its way.
    ///
    /// **Synchronous, and does no I/O.** This is what
    /// `tableView(_:viewFor:row:)` calls, on the main thread, for every
    /// visible row on every redraw — so a miss draws a placeholder and asks
    /// behind the caller's back rather than waiting. `ListWindow` decides
    /// which pages to ask for, including the read-ahead at a page boundary
    /// and the deduplication against what is already in flight; nothing here
    /// second-guesses it.
    pub fn row_at(&self, position: u32) -> Option<crate::RowFfi> {
        let wanted = {
            let mut list = self.list.lock().expect("list lock");
            match list.row_at(position)? {
                postio_ui::list::Lookup::Resident(row) => return Some(row.clone()),
                postio_ui::list::Lookup::Missing { request } => request,
            }
        };
        let generation = self.list.lock().expect("list lock").generation();
        for page in wanted {
            self.fetch(generation, page);
        }
        None
    }

    /// One message as a row, by id rather than by list position.
    ///
    /// `row_at` answers by *index* into whatever list is open, which is the
    /// right question for a table and the wrong one for the single-message
    /// pane: a message the store has not threaded belongs to no conversation
    /// and is drawn from an id, with no list under it to index into.
    ///
    /// `None` for a message that is not there. A row full of blanks reads as
    /// a message with no sender, which is a statement about somebody's mail;
    /// nothing is the truthful answer.
    ///
    /// Blocking, and one read — the pane asks once when a message opens,
    /// not per redraw, which is what separates this from `row_at`.
    pub fn row_for(&self, message: i64) -> Option<crate::RowFfi> {
        let (store, _runtime) = self.reader()?;
        let id = postio_model::ids::MessageId::new(message);
        let rows = blocking(async { store.message_rows(vec![id]).await.ok() })?;
        rows.into_iter().next().map(Into::into)
    }

    /// Raise an event this boundary made up itself.
    ///
    /// The frontend's drain does not distinguish these from the engine's, and
    /// should not: "the cursor moved" and "mail arrived" are both things that
    /// happened, and a second channel would be a second thing to forget to
    /// read. `try_send` because the channel is unbounded and the only way it
    /// fails is a session that has already shut down.
    fn emit_local(&self, event: UiEvent) {
        let _ = self.local.0.try_send(event);
    }

    /// Read one page into the window, behind the caller.
    ///
    /// What the page *is* — an offset read of the scope, or a slice of the
    /// search ranking — is [`postio_ui::paging::Paging::fetch_for`]'s answer,
    /// the same one `postio-gtk`'s feed gets; only the crossing to the store
    /// and back is this boundary's.
    fn fetch(&self, generation: u64, page: u32) {
        let fetch = self.paging.lock().expect("paging lock").fetch_for(page);
        match fetch {
            None => {}
            Some(postio_ui::paging::Fetch::Scope(request)) => self.fetch_scope(generation, request),
            Some(postio_ui::paging::Fetch::Hits { ids, .. }) => {
                self.fetch_hits(generation, page, ids);
            }
        }
    }

    /// One page of the scope in view, read by offset.
    fn fetch_scope(&self, generation: u64, request: postio_ui::paging::PageRequest) {
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let local = self.local.0.clone();
        let list = self.list.clone();
        let in_flight = self.in_flight.clone();
        let ordering = std::sync::atomic::Ordering::SeqCst;

        in_flight.fetch_add(1, ordering);
        self.reads.fetch_add(1, ordering);
        runtime.spawn(async move {
            let wanted = postio_runtime::store::PageRequest {
                scope: request.scope,
                offset: request.offset,
                limit: request.limit,
            };
            if let Ok(fetched) = store.list_page(wanted).await {
                let page = crate::list::page_of(fetched);
                let delivered = {
                    let mut list = list.lock().expect("list lock");
                    // The count and the rows come from one read, so every
                    // page corrects the total the scope was opened with —
                    // for the generation it was asked in, and no other.
                    if list.generation() == generation {
                        let _ = list.set_total(page.total);
                    }
                    list.deliver(generation, request.page, page.rows)
                };
                // A page for a scope the user has already left is dropped
                // rather than drawn, and saying nothing about it is the point:
                // an event here would tell the frontend to reload rows that
                // belong to a folder it is no longer showing.
                if !delivered.stale {
                    let _ = local.try_send(UiEvent::PageReady { page: request.page });
                }
            }
            in_flight.fetch_sub(1, ordering);
        });
    }

    /// One page of the current result set, read by id.
    ///
    /// `message_rows` exists for exactly this: search hits come back in
    /// relevance order, and asking the store for "rows 50..100 of this scope"
    /// would re-sort them by date. So the window pages over the *ranking*,
    /// and each page names the ids it wants.
    fn fetch_hits(&self, generation: u64, page: u32, wanted: Vec<postio_model::ids::MessageId>) {
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let local = self.local.0.clone();
        let list = self.list.clone();
        let in_flight = self.in_flight.clone();
        let ordering = std::sync::atomic::Ordering::SeqCst;

        in_flight.fetch_add(1, ordering);
        self.reads.fetch_add(1, ordering);
        runtime.spawn(async move {
            if let Ok(fetched) = store.message_rows(wanted.clone()).await {
                // Back into the ranking's order. `message_rows` answers in
                // whatever order the store finds them, and a page that
                // re-sorted the ranking would put the best match wherever its
                // date happened to fall -- which is the one thing a *ranked*
                // list must not do.
                let mut by_id: std::collections::HashMap<i64, crate::RowFfi> = fetched
                    .into_iter()
                    .map(|row| (row.id.get(), crate::RowFfi::from(row)))
                    .collect();
                let rows: Vec<crate::RowFfi> = wanted
                    .iter()
                    .filter_map(|id| by_id.remove(&id.get()))
                    .collect();
                let delivered = list
                    .lock()
                    .expect("list lock")
                    .deliver(generation, page, rows);
                if !delivered.stale {
                    let _ = local.try_send(UiEvent::PageReady { page });
                }
            }
            in_flight.fetch_sub(1, ordering);
        });
    }

    /// Run `query`, and show its hits as the list.
    ///
    /// **One query language.** `postio-search` parses it, here, for both
    /// frontends -- Swift does not re-implement operator parsing, or `from:`
    /// would mean one thing on Linux and another on a Mac. The run is
    /// `postio_session::search::execute`, the same function the GTK finder
    /// calls, so the hit limit and the excerpt rule are one decision rather
    /// than two.
    ///
    /// Blocking, like [`open_scope`](Self::open_scope) and for the same
    /// reason: a table asks how tall it is before it draws anything. Local
    /// search is budgeted under 100 ms (`PRODUCT.md` §1) and this is SQLite's
    /// FTS5 index, never the network.
    ///
    /// The scope being left is remembered, so clearing comes back to it
    /// rather than reloading the world.
    pub async fn search(&self, query: &str) -> u64 {
        let Some((_, _runtime)) = self.reader() else {
            return 0;
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return 0;
        };

        // Remembered on the way *in* only: a second query typed while search
        // results are on screen must not make the first search the thing to
        // come back to.
        {
            let mut resting = self.resting.lock().expect("resting lock");
            if resting.is_none() {
                *resting = self.scope_in_view();
            }
        }

        let order = *self.result_order.lock().expect("result order lock");
        let scope = *self.search_scope.lock().expect("search scope lock");
        let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
        let account = *self.account_scope.lock().expect("account scope lock");
        // Timed here because here is where the work happens. The field says
        // "14 hits · 11 ms" (canvas 2b), which is the 100ms budget made
        // visible — a claim the application should be willing to make on
        // screen rather than only in a note.
        let started = std::time::Instant::now();
        let found = blocking(async {
            let connection = database.connect().await.ok()?;
            postio_session::search::execute(&connection, account, &parsed, scope, order).await
        });

        let elapsed = started.elapsed();
        let outcome = postio_ui::search::Outcome {
            hits: found.as_ref().map(|r| r.total_hits).unwrap_or(0),
            capped: found.as_ref().is_some_and(|r| r.total_hits_capped),
            elapsed,
            // The corpus is complete when nothing is still backfilling. The
            // session does not track that yet, so the honest default is the
            // one that adds no caveat rather than one that cries wolf on
            // every search; #352's wording is a state that *ends*, and
            // claiming it while it is not true would make it furniture.
            corpus_complete: true,
            unreachable: Vec::new(),
        };

        let hits: Vec<crate::search::Hit> = found
            .map(|results| {
                results
                    .hits
                    .into_iter()
                    .map(|hit| crate::search::Hit {
                        message: hit.message_id.get(),
                        snippet: crate::search::snippet_of(&hit.snippet),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let ranking: Vec<postio_model::ids::MessageId> = hits
            .iter()
            .map(|hit| postio_model::ids::MessageId::new(hit.message))
            .collect();
        *self.hits.lock().expect("hits lock") = Some(hits);
        *self.outcome.lock().expect("outcome lock") = Some(outcome);
        *self.query.lock().expect("query lock") = Some(query.to_owned());
        // The ranking is the list now; the scope is set aside, not left, and
        // `scope_in_view` says why nothing sees it until the search closes.
        let total = self
            .paging
            .lock()
            .expect("paging lock")
            .show_results(ranking);
        self.drop_selection_and_cursor();
        self.list.lock().expect("list lock").reset(total)
    }

    /// The query the rows on screen came from, or `None` over a mailbox.
    ///
    /// What *Save search as folder* keeps. Not the text in the field: that
    /// is whatever has been typed since the last run, and saving it would
    /// write down a query nobody has seen the results of.
    pub fn search_query(&self) -> Option<String> {
        self.query.lock().expect("query lock").clone()
    }

    /// Which order the results are in, as the sort control says it —
    /// "Relevance" or "Newest".
    ///
    /// `ResultOrder::label`'s word, so this control and GTK's own say the
    /// same thing. Answered whatever is on screen: over a mailbox it is the
    /// order the next search would run in, which is what the control would
    /// be offering to change.
    pub fn result_order_label(&self) -> String {
        self.result_order
            .lock()
            .expect("result order lock")
            .label()
            .to_owned()
    }

    /// What the results on screen are made of: the scope rail's counts and
    /// the refine chips, from one pass over the index (#1157).
    ///
    /// Empty over a mailbox: both are about a *result set*, and offering
    /// `is:unread` or "Inbox only, 3" where there is none would be offering
    /// to search without saying so.
    ///
    /// The chips are measured against the current results rather than listed
    /// from a table, which is the whole point — a chip that keeps none of
    /// them is a dead end, and one that keeps all of them appears to do
    /// nothing when clicked. Neither is offered. The scope counts ask what
    /// *switching* would find, zeros included.
    pub async fn search_facets(&self) -> crate::SearchFacetsFfi {
        let Some(query) = self.query.lock().expect("query lock").clone() else {
            return crate::SearchFacetsFfi::default();
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return crate::SearchFacetsFfi::default();
        };
        let Ok(connection) = database.connect().await else {
            return crate::SearchFacetsFfi::default();
        };
        let parsed = postio_search::parse(&query, chrono::Utc::now().date_naive());
        let account = *self.account_scope.lock().expect("account scope lock");
        let order = *self.result_order.lock().expect("result order lock");
        let scope = *self.search_scope.lock().expect("search scope lock");
        let total = self
            .outcome
            .lock()
            .expect("outcome lock")
            .as_ref()
            .map(|outcome| outcome.hits)
            .unwrap_or(0);
        let Some(facets) =
            postio_session::search::facets(&connection, account, &parsed, scope, order).await
        else {
            return crate::SearchFacetsFfi::default();
        };
        crate::SearchFacetsFfi {
            scopes: postio_search::facets::Scope::ALL
                .iter()
                .map(|scope| {
                    let hits = facets.hits(*scope);
                    crate::ScopeCountFfi {
                        scope: (*scope).into(),
                        label: scope.label().to_owned(),
                        hits,
                        spoken: postio_ui::search::scope_spoken(*scope, hits),
                    }
                })
                .collect(),
            refinements: facets
                .suggested(total)
                .into_iter()
                .map(|refinement| crate::RefinementFfi {
                    token: refinement.token.clone(),
                    hits: refinement.hits,
                })
                .collect(),
        }
    }

    /// Which scope the search is looking in.
    pub fn search_scope(&self) -> crate::SearchScopeFfi {
        (*self.search_scope.lock().expect("search scope lock")).into()
    }

    /// Look in `scope` instead, and ask the same query again there.
    ///
    /// The scope is not written into the query -- switching it must not mean
    /// editing what was typed -- which is GTK's reasoning for its own rail.
    /// Over a mailbox this only records the choice; the rail is not drawn
    /// there, and the next search starts from All mail regardless.
    pub async fn set_search_scope(&self, scope: crate::SearchScopeFfi) -> u64 {
        *self.search_scope.lock().expect("search scope lock") = scope.into();
        let Some(query) = self.query.lock().expect("query lock").clone() else {
            return self.list.lock().expect("list lock").generation();
        };
        self.search(&query).await
    }

    /// Read the results the other way round — `o`.
    ///
    /// Toggles the order and asks the *same query* again, rather than
    /// re-sorting the rows already on screen: the list is a window over a
    /// paged store, so sorting what is resident would order one page and
    /// leave the rest where they were.
    ///
    /// Nothing happens over a mailbox. There is no other order to offer
    /// there — the list is already in the one order a mailbox has — and a
    /// key that quietly re-sorted somebody's inbox would be a different
    /// command than the one they pressed. GTK's sort control is inert in the
    /// same place, for the same reason.
    pub async fn toggle_result_order(&self) -> u64 {
        let Some(query) = self.query.lock().expect("query lock").clone() else {
            return self.list.lock().expect("list lock").generation();
        };
        {
            let mut order = self.result_order.lock().expect("result order lock");
            *order = order.toggled();
        }
        self.search(&query).await
    }

    /// Leave search, and show what was on screen before it.
    ///
    /// Restores the previous scope rather than reloading the world, which is
    /// the difference between `Escape` costing a `COUNT` against a mailbox the
    /// user never left and costing nothing.
    pub fn clear_search(&self) -> u64 {
        if !self.is_searching() {
            return self.list.lock().expect("list lock").generation();
        }
        *self.hits.lock().expect("hits lock") = None;
        *self.outcome.lock().expect("outcome lock") = None;
        *self.query.lock().expect("query lock") = None;
        // The next search starts from All mail: see `search_scope`.
        *self.search_scope.lock().expect("search scope lock") =
            postio_search::facets::Scope::AllMail;
        let resting = self.resting.lock().expect("resting lock").take();
        match resting {
            // Opening the scope again is leaving the results.
            Some(scope) => self.open_list_scope(scope),
            None => {
                self.paging.lock().expect("paging lock").close_results();
                self.drop_selection_and_cursor();
                self.list.lock().expect("list lock").reset(0)
            }
        }
    }

    /// The excerpt for `message`, when a search is what is on screen.
    ///
    /// `None` outside a search, and for a row that is not a hit. The text and
    /// the match ranges cross separately so each frontend marks them its own
    /// way -- GTK into Pango, Swift into an `AttributedString` -- from one
    /// answer about what matched.
    pub fn snippet_for(&self, message: i64) -> Option<crate::SnippetFfi> {
        self.hits
            .lock()
            .expect("hits lock")
            .as_ref()?
            .iter()
            .find(|hit| hit.message == message)
            .map(|hit| hit.snippet.clone())
    }

    /// What the last search turned out to be, or `None` outside a search.
    ///
    /// The wording is `postio_ui::search::readout`'s, so the two frontends
    /// say the same thing about the same result set — including the caveats,
    /// which are the part most worth not re-deriving: "still syncing" is a
    /// state that ends (#352) and an account named unreachable is ADR 0005
    /// Q10's promise that a view says what it left out.
    pub fn search_outcome(&self) -> Option<crate::OutcomeFfi> {
        let held = self.outcome.lock().expect("outcome lock");
        let outcome = held.as_ref()?;
        Some(crate::OutcomeFfi {
            readout: postio_ui::search::readout(outcome),
            spoken: postio_ui::search::spoken_readout(outcome),
            hits: outcome.hits,
        })
    }

    /// What to draw over a list with nothing in it.
    ///
    /// **`None` when there are rows**, and `None` when the empty list is an
    /// empty *folder* — that plate is the frontend's own and says something
    /// different. This answers the one case a frontend cannot work out for
    /// itself without re-deriving the search: the query matched nothing.
    ///
    /// The distinction is the whole point. `postio_ui::list_state` keeps
    /// `NoMatches` separate from `InboxZero` because *the mailbox is not
    /// empty — the query is*, and a list that says "This store has no mail in
    /// it yet." over a search is making a confident false statement about
    /// somebody's own mail. ADR 0005 Q10 names this exact scenario: someone
    /// searches for an invoice, finds nothing, and concludes it does not
    /// exist.
    ///
    /// The wording is the shared one, so both frontends make the same claim
    /// and disclose the same caveat.
    pub fn empty_plate(&self) -> Option<crate::EmptyPlateFfi> {
        if self.row_count() > 0 || !self.is_searching() {
            return None;
        }
        let query = self.query.lock().expect("query lock").clone()?;
        let incomplete = self
            .outcome
            .lock()
            .expect("outcome lock")
            .as_ref()
            .map(|outcome| outcome.unreachable.clone())
            .unwrap_or_default();
        Some(crate::EmptyPlateFfi {
            title: postio_ui::list_state::no_matches_title().to_owned(),
            detail: postio_ui::list_state::no_matches_detail(&query, &incomplete),
        })
    }

    /// Whether the list is showing search results rather than a folder.
    pub fn is_searching(&self) -> bool {
        self.hits.lock().expect("hits lock").is_some()
    }

    /// Forget what was marked and where the keyboard was.
    ///
    /// Shared by every re-scoping, including into and out of a search: "these
    /// twelve" means something else the moment the list does.
    fn drop_selection_and_cursor(&self) {
        *self.selection.lock().expect("selection lock") = postio_core::state::Selection::default();
        *self.cursor.lock().expect("cursor lock") = None;
        *self.cursor_row.lock().expect("cursor row lock") = None;
        *self.anchor.lock().expect("anchor lock") = None;
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

    /// Wait until no page read is in flight.
    ///
    /// Test-only. A production frontend never waits for this — it repaints
    /// when `PageReady` arrives, which is the whole design.
    #[cfg(feature = "testing")]
    pub fn settle_for_test(&self) {
        let ordering = std::sync::atomic::Ordering::SeqCst;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self.in_flight.load(ordering) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    /// How many rows the window is holding. Test-only.
    #[cfg(feature = "testing")]
    pub fn resident_rows_for_test(&self) -> usize {
        self.list.lock().expect("list lock").resident_rows()
    }

    /// How many page reads have been issued. Test-only.
    #[cfg(feature = "testing")]
    pub fn page_reads_for_test(&self) -> usize {
        self.reads.load(std::sync::atomic::Ordering::SeqCst)
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
        original: bool,
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
            // — the same split the GTK reader's `DecodeNotice` has (#901) —
            // so it crosses as [`decode_caveat`](Self::decode_caveat), which
            // reads the same flag through the same load. Named rather than
            // elided so the next reader of this arm finds the other half.
            postio_session::reading::Body::Ready {
                body,
                encoding_problems,
            } => {
                // Reader view is decided per message from the message, the
                // same rule the GTK reader uses (#1009) — unless the reader
                // asked to see the original, which is the one gesture that
                // may leave it (#1274). Asking is per message and per view:
                // nothing here is remembered, so the next message opens
                // reduced again.
                let bulk = suits_reader_view(&body);
                let rendering = if bulk && !original {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                let drawn = body_html(&body, remote, rendering);
                // The sender's own sheet — paper white, inset from the app's
                // chrome — for an original that reader view would otherwise
                // have reduced. `sheet_for` is the rule, from the same
                // function GTK calls: the app's palette is never injected
                // into a sender's markup. It was written asking rather than
                // assuming, against a frontend that could not leave reader
                // view at all; `original` above is the day it grew one, and
                // the sheet came with it.
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
        original: bool,
    ) -> String {
        self.reader_answers(message, remote, original).await.html
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

    /// What `message` is made of, as the frontend sees it.
    ///
    /// Empty rather than an error when there is no store or the message has
    /// gone: the caller is drawing a panel, and a panel with no rows is a
    /// truthful blank where a thrown error would make listing a thing every
    /// frontend has to handle failing.
    pub async fn message_parts(&self, message: i64) -> crate::MessagePartsFfi {
        let Some((database, _)) = self.store_and_blobs() else {
            return crate::MessagePartsFfi::nothing();
        };
        match postio_session::reading::message_parts(&database, message.into()).await {
            Ok(parts) => crate::MessagePartsFfi::from_parts(parts),
            Err(reason) => {
                // An id and an outcome. The sentence names no part and no
                // sender, but it is the store's wording rather than ours and
                // this is the one place it would otherwise vanish.
                tracing::debug!(message, reason, "a message's parts could not be read");
                crate::MessagePartsFfi::nothing()
            }
        }
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

    /// Write one part to exactly `path`.
    pub async fn save_part(
        &self,
        message: i64,
        part_id: String,
        path: String,
    ) -> Result<(), crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        Ok(postio_session::reading::save_part(
            &database,
            &blobs,
            self.engine(),
            message.into(),
            &part_id,
            std::path::Path::new(&path),
        )
        .await?)
    }

    /// Write one part into `directory` under the name Postio chose for it.
    pub async fn export_part(
        &self,
        message: i64,
        part_id: String,
        directory: String,
    ) -> Result<String, crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        let path = postio_session::reading::export_part(
            &database,
            &blobs,
            self.engine(),
            message.into(),
            &part_id,
            std::path::Path::new(&directory),
        )
        .await?;
        Ok(path.display().to_string())
    }

    /// Write every part that holds bytes into `directory`.
    pub async fn save_all_parts(
        &self,
        message: i64,
        directory: String,
    ) -> Result<crate::SavedPartsFfi, crate::PartsError> {
        let (database, blobs) = self.store_and_blobs().ok_or_else(no_store)?;
        let outcome = postio_session::reading::save_all_parts(
            &database,
            &blobs,
            self.engine(),
            message.into(),
            std::path::Path::new(&directory),
        )
        .await?;
        Ok(crate::SavedPartsFfi {
            saved: outcome.saved as u32,
            failed: outcome.failed as u32,
            failure: postio_ui::reader::parts::save_all_failure(outcome.failed),
        })
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
                // the same shared layer the GTK feed asks, which is the whole
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
        self.reconnects
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let Some(mailbox) = self.open_mailbox() else {
            // Nothing in view to refresh. The engines' own reconnect loops
            // still run; this is the opportunistic half.
            return;
        };
        let Some((_, runtime)) = self.reader() else {
            return;
        };
        let engines = self.engines.lock().expect("engines lock").clone();
        for (_, engine) in engines {
            let engine = engine.clone();
            runtime.spawn(async move {
                let _ = engine.sync(mailbox).await;
            });
        }
    }

    /// The folder the window currently has open, if it is a folder.
    ///
    /// A search has no mailbox to refresh: its results came from the local
    /// index, and re-running the query is the frontend's call, not a
    /// reconnection's.
    fn open_mailbox(&self) -> Option<postio_model::MailboxId> {
        self.scope_in_view()
            .and_then(postio_runtime::store::ListScope::mailbox)
    }

    /// The scope the window is showing, or `None` while a search is.
    ///
    /// No `ListScope` describes a ranking, so there is none while a search
    /// is on screen: `aim` sees `None` and refuses a whole-view gesture,
    /// which is the conservative answer — "select everything matching this
    /// query" is a predicate the engine has no way to evaluate yet — and a
    /// reconnection finds no mailbox to refresh. The scope is set aside in
    /// [`postio_ui::paging::Paging`], not forgotten, and `resting` is what
    /// brings it back when the search closes.
    fn scope_in_view(&self) -> Option<postio_runtime::store::ListScope> {
        let paging = self.paging.lock().expect("paging lock");
        if paging.showing_results() {
            None
        } else {
            paging.scope()
        }
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
            engine = self.events.next() => engine.map(|event| self.cross(event)),
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
        // silently miss every event this side invents — `PageReady`,
        // `ConversationReady`, `ReindexProgress` — which is most of what a
        // test about this crate wants to see.
        match self.events.try_next() {
            Some(engine) => Some(self.cross(engine)),
            None => self.local.1.try_recv().ok(),
        }
    }

    /// [`next_event`](Self::next_event), for callers that are not async.
    ///
    /// Rust-only. Swift always awaits.
    pub fn next_event_blocking(&self) -> Option<UiEvent> {
        self.events.next_blocking().map(|event| self.cross(event))
    }

    /// One engine event on its way to the frontend: the window reacts to it
    /// first, so that by the time the frontend redraws on it the count and
    /// the pages already say what the event said.
    fn cross(&self, event: postio_core::Event) -> UiEvent {
        self.react(&event);
        UiEvent::from(event)
    }

    /// What the window does when an event says the list moved.
    ///
    /// [`postio_ui::paging::Paging::plan`]'s table, the one `postio-gtk`'s
    /// feed follows: new mail in the open scope is inserted at the top,
    /// changed rows have the pages holding them re-read in place, and a scope
    /// whose membership or order moved is reloaded. Before the table crossed
    /// the boundary this was "count again, and reset if the count moved" —
    /// which drew a filled folder that had been opened empty (#1150) and
    /// nothing else: a flag set on macOS stayed undrawn, because a flag does
    /// not move the count.
    ///
    /// It belongs here rather than in either frontend for the reason the whole
    /// boundary does: the window is here, and a frontend that re-opened the
    /// scope to refresh it would be making a navigation decision to fix a
    /// bookkeeping one.
    ///
    /// A reload still counts synchronously — `open_scope`'s reason: the table
    /// asks how tall it is the moment it hears the event, and that question
    /// cannot await — and then re-reads the first page. The rows on screen
    /// stay until their replacements land; a reset would blank the table
    /// under the cursor. A search is never reloaded: its ranking does not
    /// change because a folder did.
    fn react(&self, event: &postio_core::Event) {
        // Rows that have left the mailbox cannot stay selected: the next
        // action would be aimed at mail that is no longer there. `postio-app`
        // says the same thing in the same words on the GTK side — and it is
        // here rather than in either frontend because the selection is here,
        // and because `Everything { except }` is a predicate a frontend
        // cannot re-derive without enumerating the mailbox it is about.
        //
        // Whole, not narrowed to the ids that went. A predicate selection has
        // no ids to subtract, and "these twelve minus the two that were
        // archived" is a thing nobody asked for: the mark was made against a
        // list that has since moved.
        if matches!(event, postio_core::Event::MessagesRemoved { .. }) && !self.selection_is_empty()
        {
            self.clear_selection();
        }
        let plan = self.paging.lock().expect("paging lock").plan(event);
        match plan {
            postio_ui::paging::Plan::Ignore => {}
            postio_ui::paging::Plan::InsertAtTop(count) => {
                self.list.lock().expect("list lock").inserted_at_top(count);
            }
            postio_ui::paging::Plan::Refetch(messages) => {
                let (generation, pages) = {
                    let list = self.list.lock().expect("list lock");
                    (list.generation(), list.pages_holding(messages))
                };
                for page in pages {
                    self.fetch(generation, page);
                }
            }
            postio_ui::paging::Plan::Reload => {
                let Some(scope) = self.scope_in_view() else {
                    return;
                };
                let Some((store, _runtime)) = self.reader() else {
                    return;
                };
                let total = blocking(store.list_count(scope)).unwrap_or(0);
                let generation = {
                    let mut list = self.list.lock().expect("list lock");
                    list.invalidate();
                    let _ = list.set_total(total);
                    list.generation()
                };
                // A list that shrank to nothing stops asking for pages, so the
                // reload asks once itself, or an emptied folder would keep
                // showing the rows it used to have.
                self.fetch(generation, 0);
            }
        }
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
    /// boundary — `postio-app` reaches `with_backfill`/`with_watch` the same
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

fn blocking<T>(future: impl std::future::Future<Output = T>) -> T {
    postio_session::blocking::now(future)
}
