//! Opening a session, draining its events, and shutting it down.

use postio_config::paths::Platform;
use std::sync::{Arc, Mutex};

use postio_core::bridge::{Bridge, CommandSender, EventStream, event_channel, handler_fn};
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
    seeded: Option<postio_storage::Database>,
    #[cfg(feature = "testing")]
    seeded_blobs: Option<(postio_storage::BlobStore, tempfile::TempDir)>,
    #[cfg(feature = "testing")]
    config_text: Option<String>,
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
            config_text: None,
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

    /// A session over a store that exists only in memory.
    #[cfg(feature = "testing")]
    pub fn in_memory() -> Self {
        Self {
            in_memory: true,
            ..Self::at_default_path()
        }
    }

    /// An in-memory session over a database the caller already seeded.
    ///
    /// A list test needs rows in the store *before* the session opens it, and
    /// there is no way to reach in afterwards -- the wiring is private, which
    /// is the point of it.
    #[cfg(feature = "testing")]
    pub fn in_memory_with(database: postio_storage::Database) -> Self {
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
    #[cfg(feature = "testing")]
    pub fn with_config_for_test(mut self, text: &str) -> Self {
        self.config_text = Some(text.to_owned());
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

/// This installation's `[keys]`, or the built-in defaults.
///
/// A config that will not parse is a reason to use the defaults, not a reason
/// the application cannot open: the store and the mail are not downstream of
/// `[keys]`, and refusing to start over a mistyped binding would be a mail
/// client held hostage by its own preferences file.
fn load_key_bindings(text: Option<&str>) -> postio_config::keys::KeyBindings {
    let config = match text {
        Some(text) => postio_config::Config::from_toml_str(text).ok(),
        None => postio_config::Config::load().ok(),
    };
    match config {
        Some(config) => config.keys,
        None => {
            tracing::warn!("using the built-in key bindings: config.toml is absent or unreadable");
            Default::default()
        }
    }
}

/// This installation's `[sync]`, or the built-in defaults.
///
/// The same shape as [`load_key_bindings`], and for the same reason: a
/// config that will not parse is a reason to use the defaults, not a reason
/// the session cannot open. `postio-app::open_with` reads `[sync]` this way
/// for the GTK frontend (`notifications::config_at`); this is the
/// counterpart for every `Wiring` this crate builds, which used to read
/// nothing at all and so started every engine on `BackfillPolicy::default()`
/// and `WatchPolicy::default()` regardless of what was on disk (#1014).
fn load_sync_config(text: Option<&str>) -> postio_config::SyncConfig {
    let config = match text {
        Some(text) => postio_config::Config::from_toml_str(text).ok(),
        None => postio_config::Config::load().ok(),
    };
    match config {
        Some(config) => config.sync,
        None => {
            tracing::warn!("using the built-in sync policy: config.toml is absent or unreadable");
            Default::default()
        }
    }
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
    /// The list, windowed. Behind its own lock rather than inside `wiring`'s
    /// so that a row lookup -- which happens on every table redraw -- does not
    /// contend with whatever else is holding the session.
    list: Arc<Mutex<postio_ui::list::ListWindow<crate::RowFfi>>>,
    /// What the window is currently showing, so a page fetch knows what to
    /// ask the store for.
    scope: Mutex<Option<postio_runtime::store::ListScope>>,
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
    /// The scope to come back to when a search is cleared.
    ///
    /// Held here rather than remembered by the frontend, because a frontend
    /// that remembered it would own navigation state — and would then own it
    /// differently from the GTK side. Clearing restores the previous scope
    /// rather than reloading the world.
    resting: Mutex<Option<postio_runtime::store::ListScope>>,
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
    /// Retained rather than leaked, for the reason `postio-app` records: the
    /// store is SQLCipher, and dropping an engine at process exit is exactly
    /// when libcrypto goes away underneath a thread still encrypting a page.
    engines: Mutex<Vec<postio_runtime::Engine>>,
    /// `[keys]` as this installation has it.
    ///
    /// Read once at open. A menu accelerator has to reflect what the user
    /// actually bound, and re-reading `config.toml` on every menu draw would
    /// be a file read per repaint.
    keys: postio_config::keys::KeyBindings,
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
        self.search(&query)
    }

    /// Leave search and restore the scope that was open.
    #[uniffi::method(name = "clearSearch")]
    pub fn clear_search_ffi(&self) -> u64 {
        self.clear_search()
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

    /// The whole document for a message, ready to hand a `WKWebView`.
    ///
    /// Swift's job is to build a hardened configuration, hand it this string,
    /// and refuse navigations. It composes no reader HTML of its own.
    #[uniffi::method(name = "readerDocument")]
    pub fn reader_document_ffi(&self, message: i64, remote: crate::RemoteImagesFfi) -> String {
        self.reader_document(message, remote)
    }

    /// One inline part of `message`, by its `Content-ID`.
    ///
    /// What a `WKURLSchemeHandler` for `postio-cid:` answers with. `nil` is a
    /// broken image, deliberately — never a fetch.
    #[uniffi::method(name = "resolveCid")]
    pub fn resolve_cid_ffi(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        self.resolve_cid(message, content_id)
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
        self.start_syncing()
    }

    /// How many accounts are configured and enabled.
    #[uniffi::method(name = "configuredAccounts")]
    pub fn configured_accounts_ffi(&self) -> u32 {
        self.configured_accounts()
    }

    /// Every folder of every enabled account, for the sidebar.
    #[uniffi::method(name = "mailboxes")]
    pub fn mailboxes_ffi(&self) -> Vec<crate::MailboxFfi> {
        self.mailboxes()
    }

    /// The binding in force for a command, for drawing a native accelerator.
    #[uniffi::method(name = "bindingFor")]
    pub fn binding_for_ffi(&self, command: String) -> Option<String> {
        self.binding_for(command)
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
}

// ---------------------------------------------------------------------------
// The Rust surface. Nothing here crosses to Swift; the block above wraps what
// should. Test-only methods belong here.
// ---------------------------------------------------------------------------
impl Session {
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
        let (sink, events) = event_channel();

        let (runtime, commands, owned_bridge) = match options.bridge {
            Some((runtime, commands)) => (runtime, commands, None),
            None => {
                let (bridge, _replies) =
                    Bridge::new(handler_fn(|_, _| async {})).map_err(|error| {
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
                    // lives and dies inside this process, so there is nothing
                    // to reopen it with later — and an in-memory session still
                    // runs the encrypted path, which is the whole point of ADR
                    // 0014 Q3's "nothing tests a plaintext configuration that
                    // no longer ships". No keyring is touched.
                    let key = postio_storage::key::StoreKey::generate()
                        .derive(postio_storage::key::Purpose::Database);
                    postio_storage::Database::open_in_memory(&key).map_err(|error| {
                        SessionError::StoreUnavailable {
                            message: error.to_string(),
                        }
                    })?
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
            let sync_config = load_sync_config(options.config_text.as_deref());
            let wiring = Wiring::new(database, blobs, runtime, sink, commands)
                .with_backfill(postio_session::backfill_policy(&sync_config))
                .with_watch(postio_session::watch_policy(&sync_config));
            let keys = load_key_bindings(options.config_text.as_deref());
            return Ok(Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
                resolver: Mutex::new(build_resolver(&keys)),
                keys,
                list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
                selection: Mutex::new(postio_core::state::Selection::default()),
                reachable: Mutex::new(Vec::new()),
                cursor: Mutex::new(None),
                cursor_row: Mutex::new(None),
                account_scope: Mutex::new(postio_core::Scope::default()),
                hits: Mutex::new(None),
                resting: Mutex::new(None),
                anchor: Mutex::new(None),
                scope: Mutex::new(None),
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
        let (database, blobs) = postio_session::open_store_at(path, &key)
            .map_err(|message| SessionError::StoreUnavailable { message })?;

        #[cfg(feature = "testing")]
        let keys = load_key_bindings(options.config_text.as_deref());
        #[cfg(not(feature = "testing"))]
        let keys = load_key_bindings(None);

        #[cfg(feature = "testing")]
        let sync_config = load_sync_config(options.config_text.as_deref());
        #[cfg(not(feature = "testing"))]
        let sync_config = load_sync_config(None);

        let wiring = Wiring::new(database, blobs, runtime, sink, commands)
            .with_secrets(secrets)
            .with_backfill(postio_session::backfill_policy(&sync_config))
            .with_watch(postio_session::watch_policy(&sync_config));
        Ok(Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            resolver: Mutex::new(build_resolver(&keys)),
            keys,
            engines: Mutex::new(Vec::new()),
            list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
            selection: Mutex::new(postio_core::state::Selection::default()),
            reachable: Mutex::new(Vec::new()),
            cursor: Mutex::new(None),
            cursor_row: Mutex::new(None),
            account_scope: Mutex::new(postio_core::Scope::default()),
            hits: Mutex::new(None),
            resting: Mutex::new(None),
            anchor: Mutex::new(None),
            scope: Mutex::new(None),
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

    /// [`open_scope`](Self::open_scope), for a scope already in the store's
    /// own terms.
    ///
    /// Exists because leaving a search restores the scope it *remembered*,
    /// which never had a `ScopeFfi` spelling — it came off this side. A
    /// conversion back would be a second mapping to keep in step with the
    /// first, for no caller that needs one.
    fn open_list_scope(&self, listed: postio_runtime::store::ListScope) -> u64 {
        let Some((store, runtime)) = self.reader() else {
            return 0;
        };
        let total = runtime.block_on(store.list_count(listed)).unwrap_or(0);
        *self.scope.lock().expect("scope lock") = Some(listed);
        // "These twelve" means something else the moment the list does, and an
        // action carrying a selection across would land on mail the user
        // cannot see. The cursor goes with it: it named a row in a list that
        // no longer exists.
        self.drop_selection_and_cursor();
        // Opening a folder leaves a search, and there is nothing to come back
        // to: the user chose this scope rather than dismissing the query.
        *self.hits.lock().expect("hits lock") = None;
        *self.resting.lock().expect("resting lock") = None;
        *self.account_scope.lock().expect("account scope lock") =
            self.resolve_account_scope(listed);
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
            scope: self.scope.lock().expect("scope lock").and_then(|scope| {
                postio_core::aim::view_scope(scope, &self.reachable.lock().expect("reachable lock"))
            }),
            selection: &selection,
            cursor,
            rows: &*list,
        };
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
        true
    }

    /// How many accounts `scope` is about.
    ///
    /// A mailbox is one account's, and the store is what knows whose — so
    /// this reads it, once, when the scope changes. Anything it cannot
    /// resolve is `Unified`, which is the conservative answer: it withholds
    /// the commands that need a single account rather than offering one that
    /// would have nowhere to act.
    fn resolve_account_scope(&self, scope: postio_runtime::store::ListScope) -> postio_core::Scope {
        use postio_runtime::store::ListScope;
        match scope {
            ListScope::Account(account) | ListScope::Flagged(account) => {
                postio_core::Scope::Account(account)
            }
            ListScope::Mailbox(mailbox) => {
                let Some((database, _)) = self.store_and_blobs() else {
                    return postio_core::Scope::Unified;
                };
                let Ok(connection) = database.connection() else {
                    return postio_core::Scope::Unified;
                };
                postio_storage::repository::MailboxRepository::new(&connection)
                    .get(mailbox)
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
            *self.account_scope.lock().expect("account scope lock"),
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
    pub fn cheat_sheet(&self, context: crate::UiContext) -> Vec<crate::PaletteEntryFfi> {
        self.palette_entries("", context)
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
    fn fetch(&self, generation: u64, page: u32) {
        // Search hits are ranked, not sorted, so no `ListScope` describes
        // them and the store cannot page them. They are read by id instead --
        // the same page of the same window, filled from a different call.
        if self.hits.lock().expect("hits lock").is_some() {
            self.fetch_hits(generation, page);
            return;
        }
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let Some(scope) = *self.scope.lock().expect("scope lock") else {
            return;
        };
        let local = self.local.0.clone();
        let list = self.list.clone();
        let in_flight = self.in_flight.clone();
        let ordering = std::sync::atomic::Ordering::SeqCst;

        in_flight.fetch_add(1, ordering);
        self.reads.fetch_add(1, ordering);
        runtime.spawn(async move {
            let request = postio_runtime::store::PageRequest {
                scope,
                offset: page * postio_ui::list::PAGE_SIZE,
                limit: postio_ui::list::PAGE_SIZE,
            };
            if let Ok(fetched) = store.list_page(request).await {
                let rows = crate::list::rows_of(fetched);
                let delivered = list
                    .lock()
                    .expect("list lock")
                    .deliver(generation, page, rows);
                // A page for a scope the user has already left is dropped
                // rather than drawn, and saying nothing about it is the point:
                // an event here would tell the frontend to reload rows that
                // belong to a folder it is no longer showing.
                if !delivered.stale {
                    let _ = local.try_send(UiEvent::PageReady { page });
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
    fn fetch_hits(&self, generation: u64, page: u32) {
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let wanted: Vec<postio_model::ids::MessageId> = {
            let held = self.hits.lock().expect("hits lock");
            let Some(hits) = held.as_ref() else { return };
            let first = (page * postio_ui::list::PAGE_SIZE) as usize;
            hits.iter()
                .skip(first)
                .take(postio_ui::list::PAGE_SIZE as usize)
                .map(|hit| postio_model::ids::MessageId::new(hit.message))
                .collect()
        };
        if wanted.is_empty() {
            return;
        }

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
    pub fn search(&self, query: &str) -> u64 {
        let Some((_, runtime)) = self.reader() else {
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
                *resting = *self.scope.lock().expect("scope lock");
            }
        }

        let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
        let account = *self.account_scope.lock().expect("account scope lock");
        let found = runtime.block_on(async {
            let connection = database.connection().ok()?;
            postio_session::search::execute(
                &connection,
                account,
                &parsed,
                postio_search::facets::Scope::AllMail,
                postio_search::ResultOrder::Relevance,
            )
        });

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

        let total = hits.len() as u32;
        *self.hits.lock().expect("hits lock") = Some(hits);
        // No `ListScope` describes a ranking, so there is none while a search
        // is on screen. `aim` sees `None` and refuses a whole-view gesture,
        // which is the conservative answer: "select everything matching this
        // query" is a predicate the engine has no way to evaluate yet.
        *self.scope.lock().expect("scope lock") = None;
        self.drop_selection_and_cursor();
        self.list.lock().expect("list lock").reset(total)
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
        let resting = self.resting.lock().expect("resting lock").take();
        match resting {
            Some(scope) => self.open_list_scope(scope),
            None => {
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
    pub fn reader_document(&self, message: i64, remote: crate::RemoteImagesFfi) -> String {
        use postio_ui::reader::document::{
            Rendering, Sheet, absent_html, body_html, document_for, sheet_for, suits_reader_view,
            wrap_document,
        };

        let remote = postio_body::RemoteImages::from(remote);
        // The blob store is not consulted: a body is a compressed column on
        // the message's row since ADR 0020. Inline parts still come from it,
        // which is why `store_and_blobs` is the accessor either way.
        let Some((database, _blobs)) = self.store_and_blobs() else {
            return wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            );
        };
        let Ok(connection) = database.connection() else {
            return wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            );
        };
        let offline = self.offline.load(std::sync::atomic::Ordering::SeqCst);
        match postio_session::reading::load_body_or_reason(&connection, message.into(), offline) {
            // `encoding_problems` is bound and not used here, and that is a
            // gap rather than a decision: this frontend renders a document
            // and has no native strip to put a caveat in, the way the GTK
            // reader's `DecodeNotice` is (#901). Named rather than elided so
            // whoever gives this frontend a notice surface finds it.
            postio_session::reading::Body::Ready {
                body,
                encoding_problems: _,
            } => {
                // Reader view is decided per message from the message, the
                // same rule the GTK reader uses (#1009). This frontend has no
                // notice surface to offer `View original` through yet — the
                // same gap `encoding_problems` above names — so what it draws
                // is what the rule chooses, and nothing can leave it.
                let bulk = suits_reader_view(&body);
                let rendering = if bulk {
                    Rendering::Reader
                } else {
                    Rendering::Original
                };
                let drawn = body_html(&body, remote, rendering);
                // The same rule the GTK reader applies, from the same
                // function. Nothing here can reach `Sheet::Senders` while
                // this frontend has no way to leave reader view -- which is
                // the point of asking rather than assuming: the day it grows
                // one, the sheet comes with it.
                document_for(&drawn.html, remote, sheet_for(drawn.rendering, bulk))
            }
            // A state plate is Postio's own words, so it is served with remote
            // images blocked whatever the caller asked for: there is nothing
            // in it a sender wrote, and nothing for them to reach through.
            postio_session::reading::Body::Absent(state) => wrap_document(
                &absent_html(state),
                postio_body::RemoteImages::Blocked,
                Sheet::Theme,
            ),
        }
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
    pub fn resolve_cid(&self, message: i64, content_id: String) -> Option<crate::InlinePart> {
        let (database, blobs) = self.store_and_blobs()?;
        postio_session::reading::resolve_cid(&database, &blobs, message.into(), &content_id)
            .map(|(bytes, mime_type)| crate::InlinePart { bytes, mime_type })
    }

    /// Every folder of every enabled account, for the sidebar.
    ///
    /// Blocks on a local read, like `openScope` does and for the same reason:
    /// a sidebar is drawn before anything can be selected in it, and the read
    /// is a few milliseconds of SQLite rather than the network.
    pub fn mailboxes(&self) -> Vec<crate::MailboxFfi> {
        let Some((store, runtime)) = self.reader() else {
            return Vec::new();
        };
        let Some((database, _)) = self.store_and_blobs() else {
            return Vec::new();
        };
        let Ok(connection) = database.connection() else {
            return Vec::new();
        };
        let accounts = postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
            .unwrap_or_default();
        drop(connection);

        let mut folders = Vec::new();
        for account in accounts {
            if let Ok(found) = runtime.block_on(store.mailboxes(account.id)) {
                folders.extend(found.into_iter().map(crate::MailboxFfi::from));
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
        self.keys.binding_on(&command, Platform::host())
    }

    /// How many accounts are configured and enabled.
    ///
    /// ADR 0005 Q3: the first account is not special, so this counts every
    /// enabled one rather than looking for a primary.
    pub fn configured_accounts(&self) -> u32 {
        let Some((database, _)) = self.store_and_blobs() else {
            return 0;
        };
        let Ok(connection) = database.connection() else {
            return 0;
        };
        postio_storage::repository::AccountRepository::new(&connection)
            .list_enabled()
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
    pub fn start_syncing(&self) -> Result<u32, SessionError> {
        // Idempotent. An application lifecycle calls this twice more often
        // than once — a window reopening, a wake from sleep — and a second
        // set of engines would double every connection to the server.
        if self.has_engine() {
            return Ok(self.engines.lock().expect("engines lock").len() as u32);
        }

        let guard = self.wiring.lock().expect("wiring lock");
        let Some(wiring) = guard.as_ref() else {
            return Err(SessionError::StoreUnavailable {
                message: "the session has been shut down".to_string(),
            });
        };

        let accounts =
            {
                let connection = wiring.database.connection().map_err(|error| {
                    SessionError::StoreUnavailable {
                        message: error.to_string(),
                    }
                })?;
                postio_storage::repository::AccountRepository::new(&connection)
                    .list_enabled()
                    .map_err(|error| SessionError::StoreUnavailable {
                        message: error.to_string(),
                    })?
            };
        if accounts.is_empty() {
            return Ok(0);
        }

        let started = postio_session::engine::start_all(
            &accounts,
            &wiring.database,
            wiring.blobs.clone(),
            wiring.events.clone(),
            wiring.secrets.clone(),
            wiring.mailbox_roles.clone(),
            wiring.backfill,
            wiring.watch,
            &wiring.egress,
        )
        .map_err(|refusal| SessionError::StoreUnavailable {
            message: refusal.to_string(),
        })?;

        let count = started.len() as u32;
        for (_, engine) in started {
            // The slot is what `Refresh` reads, and it is pressed long after
            // the bus was built. An engine that ran but never reached it
            // would sync happily and leave the refresh command inert.
            wiring.engine.fill(engine.clone());
            postio_runtime::retain(engine.clone());
            self.engines.lock().expect("engines lock").push(engine);
        }
        Ok(count)
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
            self.engines.lock().expect("engines lock").push(engine);
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
        for engine in engines {
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
        match *self.scope.lock().expect("scope lock") {
            Some(postio_runtime::store::ListScope::Mailbox(mailbox)) => Some(mailbox),
            _ => None,
        }
    }

    /// How many reconnects have been asked for. Test-only.
    #[cfg(feature = "testing")]
    pub fn reconnects_for_test(&self) -> usize {
        self.reconnects.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The database and blob store, while the session is open.
    fn store_and_blobs(&self) -> Option<(postio_storage::Database, postio_storage::BlobStore)> {
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
        let event = tokio::select! {
            engine = self.events.next() => engine.map(UiEvent::from),
            local = self.local.1.recv() => local.ok(),
        }?;
        self.recount_if_the_list_changed(&event);
        Some(event)
    }

    /// [`next_event`](Self::next_event), for callers that are not async.
    ///
    /// Rust-only. Swift always awaits.
    pub fn next_event_blocking(&self) -> Option<UiEvent> {
        let event = self.events.next_blocking().map(UiEvent::from)?;
        self.recount_if_the_list_changed(&event);
        Some(event)
    }

    /// Re-count the open scope when an event says its contents moved.
    ///
    /// **`open_scope` counts once**, because a table asks how tall it is
    /// before it draws and that question cannot await. Everything after that
    /// arrives as an event — and a frontend's whole answer to an event is to
    /// reload its table, which asks `row_count`, which reads the total that
    /// one count set. Nothing ever set it again.
    ///
    /// So a folder opened while it was empty and filled a moment later by the
    /// first sync stayed empty on screen: 99 messages in the store, "No
    /// messages" in the list, every layer doing exactly what it was written
    /// to do. Found by running the application against a real account (#1150);
    /// invisible to every test, because no test had a list whose contents
    /// changed after it was opened.
    ///
    /// It belongs here rather than in either frontend for the reason the whole
    /// boundary does: the count is the window's, the window is here, and a
    /// frontend that re-opened the scope to refresh it would be making a
    /// navigation decision to fix a bookkeeping one. `postio-gtk`'s feed does
    /// the same thing on the same events, one layer up.
    ///
    /// Deliberately not `PageReady` — that is this boundary telling itself a
    /// page landed, and re-counting there would reset the window inside its
    /// own fetch.
    fn recount_if_the_list_changed(&self, event: &UiEvent) {
        if !matches!(
            event,
            UiEvent::MessageListChanged { .. }
                | UiEvent::MessagesChanged { .. }
                | UiEvent::MessagesRemoved { .. }
                | UiEvent::NewMail { .. }
        ) {
            return;
        }
        // A search holds its own ranking; its hits do not change because a
        // folder did, and re-counting would reset the window to a folder's
        // size while showing search results.
        if self.is_searching() {
            return;
        }
        let Some(scope) = *self.scope.lock().expect("scope lock") else {
            return;
        };
        let Some((store, runtime)) = self.reader() else {
            return;
        };
        let total = runtime.block_on(store.list_count(scope)).unwrap_or(0);
        let mut list = self.list.lock().expect("list lock");
        if list.total() != total {
            list.reset(total);
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
