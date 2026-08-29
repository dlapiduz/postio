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
    /// function. ADR 0014's rule is that [`SecretError::Locked`] must survive
    /// to the surface that asks the user to unlock, rather than being
    /// flattened into "something went wrong" and sent to onboarding — which
    /// would ask somebody with perfectly good mail to set up an account they
    /// already have. Every other keyring failure is a store that will not
    /// open, which is the honest reading: no key, no store.
    fn from_secret_error(error: postio_imap::secret::SecretError) -> Self {
        let message = error.to_string();
        match error {
            postio_imap::secret::SecretError::Locked { .. } => {
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
    secrets: Option<Arc<dyn postio_imap::secret::SecretStore>>,
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
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_imap::secret::SecretStore>) -> Self {
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
        Self {
            bridge: Some((runtime, commands)),
            ..Self::in_memory()
        }
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
                    let blobs =
                        postio_storage::BlobStore::open(scratch.path()).map_err(|error| {
                            SessionError::StoreUnavailable {
                                message: error.to_string(),
                            }
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
            let wiring = Wiring::new(database, blobs, runtime, sink, commands);
            return Ok(Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
                keys: load_key_bindings(options.config_text.as_deref()),
                list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
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
        let secrets: Arc<dyn postio_imap::secret::SecretStore> = match options.secrets {
            Some(secrets) => secrets,
            None => postio_imap::secret::platform_keyring(),
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

        let wiring = Wiring::new(database, blobs, runtime, sink, commands).with_secrets(secrets);
        Ok(Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            keys,
            engines: Mutex::new(Vec::new()),
            list: Arc::new(Mutex::new(postio_ui::list::ListWindow::new())),
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
        let listed: postio_runtime::store::ListScope = scope.into();
        let Some((store, runtime)) = self.reader() else {
            return 0;
        };
        let total = runtime.block_on(store.list_count(listed)).unwrap_or(0);
        *self.scope.lock().expect("scope lock") = Some(listed);
        self.list.lock().expect("list lock").reset(total)
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

    /// Read one page into the window, behind the caller.
    fn fetch(&self, generation: u64, page: u32) {
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
    /// Not fragments to assemble: the content security policy, the embedded
    /// font faces, the reader tokens, the sanitized body, its `.postio-body`
    /// container and the scroll markers all come from `postio_ui`, which is
    /// what the GTK reader composes through too. **The frontend's entire job
    /// is to build a hardened web view, hand it this string, and refuse
    /// navigations** — so the two readers cannot disagree about the policy,
    /// because there is only one that produces it (ADR 0019 Q6).
    pub fn reader_document(&self, message: i64, remote: crate::RemoteImagesFfi) -> String {
        use postio_ui::reader::document::{absent_html, body_html, document_for, wrap_document};

        let remote = postio_body::RemoteImages::from(remote);
        // The blob store is not consulted: a body is a compressed column on
        // the message's row since ADR 0020. Inline parts still come from it,
        // which is why `store_and_blobs` is the accessor either way.
        let Some((database, _blobs)) = self.store_and_blobs() else {
            return wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
            );
        };
        let Ok(connection) = database.connection() else {
            return wrap_document(
                &absent_html(postio_ui::reader::document::Absent::Missing),
                postio_body::RemoteImages::Blocked,
            );
        };
        let offline = self.offline.load(std::sync::atomic::Ordering::SeqCst);
        match postio_session::reading::load_body_or_reason(&connection, message.into(), offline) {
            postio_session::reading::Body::Ready(body) => {
                let (content, _held_back) = body_html(&body, remote);
                document_for(&content, remote)
            }
            // A state plate is Postio's own words, so it is served with remote
            // images blocked whatever the caller asked for: there is nothing
            // in it a sender wrote, and nothing for them to reach through.
            postio_session::reading::Body::Absent(state) => {
                wrap_document(&absent_html(state), postio_body::RemoteImages::Blocked)
            }
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
            backend: Arc::new(postio_imap::backend::MockBackend::new()),
            smtp: Arc::new(postio_smtp::transport::RustlsConnector::new().expect("a connector")),
            tokens: Arc::new(postio_imap::auth::StoredPasswordSource::new(Arc::new(
                postio_imap::secret::MemorySecretStore::default(),
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
        tokio::select! {
            engine = self.events.next() => engine.map(UiEvent::from),
            local = self.local.1.recv() => local.ok(),
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
}
