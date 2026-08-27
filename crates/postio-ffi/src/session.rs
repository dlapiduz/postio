//! Opening a session, draining its events, and shutting it down.

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

    /// Whether this session still holds its store.
    #[uniffi::method(name = "isOpen")]
    pub fn is_open_ffi(&self) -> bool {
        self.is_open()
    }
}

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
            let database = postio_storage::Database::open_in_memory().map_err(|error| {
                SessionError::StoreUnavailable {
                    message: error.to_string(),
                }
            })?;
            let scratch = tempfile::tempdir().map_err(|error| SessionError::StoreUnavailable {
                message: error.to_string(),
            })?;
            let blobs = postio_storage::BlobStore::open(scratch.path()).map_err(|error| {
                SessionError::StoreUnavailable {
                    message: error.to_string(),
                }
            })?;
            // The default secret store is left in place. It is the real
            // keyring type, but it does not reach the keyring until something
            // asks it for a secret, and nothing in this slice does — so an
            // in-memory session still needs no Secret Service, no Keychain and
            // no prompt. The moment a slice *does* read a secret, this is
            // where a `MemorySecretStore` goes.
            let wiring = Wiring::new(database, blobs, runtime, sink, commands);
            return Ok(Arc::new(Session {
                wiring: Mutex::new(Some(wiring)),
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
            None => Arc::new(postio_imap::secret::KeyringSecretStore::default()),
        };
        let key = postio_session::store_key_blocking(secrets.as_ref())
            .map_err(SessionError::from_secret_error)?;

        let path = options
            .store_path
            .unwrap_or_else(postio_session::paths::store_path);
        let (database, blobs) = postio_session::open_store_at(path, &key)
            .map_err(|message| SessionError::StoreUnavailable { message })?;

        let wiring = Wiring::new(database, blobs, runtime, sink, commands).with_secrets(secrets);
        Ok(Arc::new(Session {
            wiring: Mutex::new(Some(wiring)),
            events,
            _bridge: owned_bridge,
            #[cfg(feature = "testing")]
            _scratch: None,
        }))
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
        self.events.next().await.map(UiEvent::from)
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
