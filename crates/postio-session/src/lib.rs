//! The composition root, without a toolkit.
//!
//! Everything the application *is*, minus everything it looks like: the local
//! store, the tokio runtime, the sync engines, the logging subscriber, and the
//! verb vocabulary in [`actions`] that turns a [`Command`][postio_core::Command]
//! into rows in SQLite and events on the bus.
//!
//! # Why it is its own crate
//!
//! `postio-app` used to be both this and the GTK binary, which meant
//! `actions.rs` — the whole verb vocabulary, with not one line of toolkit in
//! it — linked GTK. `ARCHITECTURE.md` listed that under known gaps with the
//! consequence spelled out: **no headless frontend is possible.**
//!
//! [ADR 0010](../docs/decisions/0010-mcp-surface.md) makes the split a hard
//! prerequisite for an MCP surface, and the reason is worth restating because
//! the alternative looks cheaper than it is. A second binary that opened the
//! SQLite store directly would give the database two writers with different
//! rules: local-first ordering, the undo stack, event emission and the
//! operation queue all live in the path *this* crate takes, and a writer that
//! skips them is not a second frontend but a second application sharing a
//! file. By the time that causes a problem, nobody remembers there were two.
//!
//! So the rule is enforced rather than intended.
//! `scripts/checks/check-crate-boundaries.py` guards this crate against `gtk4` and
//! `libadwaita` exactly as it guards `postio-core`, because a verb added in a
//! hurry that reaches for a widget would take the headless frontend away
//! silently — everything would still compile, and every test would still pass.
//!
//! ```text
//!   postio-session   store, runtime, engines, verbs. No toolkit.
//!         └── postio-app   the GTK binary. Adds a window and nothing else.
//! ```
//!
//! # What is *not* here
//!
//! The presenters that join the two halves — the composer's storage wiring,
//! the reading pane's body loads, onboarding, notifications — stay in
//! `postio-app`, because each of them names a widget. The line is not "does it
//! touch the store" but "does it touch a toolkit": `postio-app` is what is
//! left once that line is drawn, and it is smaller than it looks.

pub mod actions;
pub mod egress;
pub mod engine;
pub mod logging;
pub mod paths;
pub mod reachability;
pub mod reading;
pub mod refresh;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use postio_core::bridge::EventSink;
use postio_runtime::store::{MailStore, SqliteStore};
use postio_storage::blob::{EvictionReport, GarbageCollection, GarbageReport};
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, Database};

/// `[mailboxes]` from the file at `path`, or nothing.
///
/// Unreadable, unparseable or absent all mean the same here — no overrides,
/// which is the behaviour every account had before the section existed.
/// Problems are not swallowed: `validate` reports them, with a line number,
/// and the settings panel shows them where they can be fixed. This is the
/// same shape as `notifications::config_at`, and for the same reason.
pub fn mailbox_roles_at(path: &std::path::Path) -> postio_model::RoleOverrides {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .map(|config| config.role_overrides())
        .unwrap_or_default()
}

/// `[storage] max_bytes` from the file at `path`, or nothing.
///
/// Unreadable, unparseable, absent and unset all answer `None`, which is the
/// documented default and means unbounded — see `postio_config::storage` for
/// why a number here is a promise about somebody else's disk that Postio is
/// not in a position to make. Read once at startup, like `[mailboxes]` and
/// `[sync]` beside it.
pub fn storage_ceiling_at(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .and_then(|config| config.storage.max_bytes)
}

/// Read `[sync]` from `path` and turn it into the policy the engine backfills
/// under.
///
/// The join `body_fetch` never had. `BackfillPolicy::background` has
/// documented itself as "`[sync] body_fetch` in `config.toml`" since it was
/// written, and `engine::start` spawned `BackfillPolicy::default()` — so
/// turning bodies off in the file did nothing at all. ADR 0017's
/// `attachment_fetch` is a setting worth more than that: it is the difference
/// between a 1.4 GB store and a 12.4 GB one, so it is wired here and the
/// older knob comes with it.
///
/// Read once at startup rather than kept live, for the reason
/// `Wiring::with_mailbox_roles` gives: the engine is spawned with its parts,
/// so a change applies at the next start. A file that will not parse leaves
/// the defaults standing — the settings panel is where a broken file is
/// reported, and syncing differently because of one would be a worse answer.
pub fn backfill_policy_at(path: &std::path::Path) -> postio_runtime::BackfillPolicy {
    let sync = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .map(|config| config.sync)
        .unwrap_or_default();
    backfill_policy(&sync)
}

/// Read `[sync]` from `path` and turn it into the policy the engine watches
/// under.
///
/// The join `idle`/`poll_interval_secs` never had (#932): `WatchPolicy`
/// has documented `idle` as "`[sync] idle` in `config.toml`" since it was
/// written, and `engine::start` spawned `EngineParts { watch:
/// Default::default(), .. }` -- so turning push off, or changing how often
/// the rest of the mailbox is reconciled, did nothing at all.
///
/// Read once at startup rather than kept live, for the same reason
/// [`backfill_policy_at`] gives: the engine is spawned with its parts, so a
/// change applies at the next start. A file that will not parse leaves the
/// defaults standing.
pub fn watch_policy_at(path: &std::path::Path) -> postio_sync::WatchPolicy {
    let sync = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| postio_config::Config::from_toml_str(&text).ok())
        .map(|config| config.sync)
        .unwrap_or_default();
    watch_policy(&sync)
}

/// [`watch_policy_at`], for a `[sync]` section already in hand.
///
/// `Manual` sets [`postio_sync::WatchPolicy::manual`] (#1013): the watcher
/// answers `Watch::Wait` forever, so nothing in it ever idles or polls on a
/// timer. The `Refresh` command still reaches the server -- it calls
/// `Engine::sync` directly and never goes through the watcher -- so manual
/// mode only silences what would otherwise happen automatically.
pub fn watch_policy(sync: &postio_config::SyncConfig) -> postio_sync::WatchPolicy {
    postio_sync::WatchPolicy {
        idle: matches!(sync.check_for_mail, postio_config::CheckForMail::Idle),
        poll_interval: std::time::Duration::from_secs(sync.poll_interval_secs),
        manual: matches!(sync.check_for_mail, postio_config::CheckForMail::Manual),
        ..postio_sync::WatchPolicy::default()
    }
}

/// [`backfill_policy_at`], for a `[sync]` section already in hand.
pub fn backfill_policy(sync: &postio_config::SyncConfig) -> postio_runtime::BackfillPolicy {
    postio_runtime::BackfillPolicy {
        // `lazy` and `eager` are both "yes, backfill"; they differ in *when*,
        // and the engine has had one background lane since #318 covered a
        // whole mailbox rather than its first batch. What would turn the lane
        // off is a third value, and nobody has asked for one.
        background: matches!(
            sync.body_fetch,
            postio_config::BodyFetch::Lazy | postio_config::BodyFetch::Eager
        ),
        attachments: match sync.attachment_fetch {
            postio_config::AttachmentFetch::OnOpen => postio_runtime::AttachmentPolicy::OnOpen,
            postio_config::AttachmentFetch::Eager => postio_runtime::AttachmentPolicy::Eager,
            postio_config::AttachmentFetch::Never => postio_runtime::AttachmentPolicy::Never,
        },
        // `0` is how a config file says "no inline rule at all", which the
        // policy spells `None` — the difference between a cap of zero and no
        // cap matters nowhere else, and a cap of zero would mean the same
        // thing anyway.
        max_inline_bytes: (sync.max_inline_bytes > 0).then_some(sync.max_inline_bytes),
        ..postio_runtime::BackfillPolicy::default()
    }
}

pub use postio_storage::key::STORE_KEY_ENTRY;

/// The master key this installation's store is encrypted under, minting one
/// on first run.
///
/// ADR 0014 Q3, the service half. The material is
/// [`postio_storage::key::StoreKey`]; this is where it comes from.
///
/// # What is and is not a first run
///
/// Only a *missing* entry mints a key, and that distinction is the whole of
/// the safety here. A locked keyring, a keyring that did not answer in time,
/// a backend that errored — none of them mean "there is no key", and minting
/// one for any of them would encrypt the next thing written under something
/// the existing store knows nothing about. The mailbox would be gone rather
/// than merely unavailable, which is a far worse answer than refusing to
/// start.
///
/// An *empty* entry is the one exception, and it is not really one: nothing
/// can have been encrypted under an empty key, so the store behind it is
/// either absent or already unopenable. Treating it as a first run is what
/// gives a half-finished first run a way out, and it is the same tolerance
/// [`postio_app::startup_route`] extends to an empty password.
///
/// # No plaintext fallback
///
/// There is no "open it unencrypted anyway", here or anywhere. `secret.rs`
/// has refused that for passwords since it was written and ADR 0014 Q3
/// extends it: a locked keyring means the mail does not open, and
/// [`SecretError::Locked`] survives to the caller precisely so it can be
/// routed to the surface that asks the user to unlock it rather than to
/// onboarding, which would ask them to set up an account they already have.
///
/// [`postio_app::startup_route`]: https://github.com/dlapiduz/postio
/// [`SecretError::Locked`]: postio_account::secret::SecretError::Locked
pub async fn store_key(
    secrets: &dyn postio_account::secret::SecretStore,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    use postio_account::secret::{AccountKey, SecretError};
    use postio_storage::key::StoreKey;

    let entry = AccountKey::new(STORE_KEY_ENTRY);
    match secrets.retrieve(&entry).await {
        Ok(stored) if !stored.is_empty() => {
            // Refused rather than replaced. A corrupt entry is a store that
            // cannot be opened; a replaced one is a store that can never be
            // opened again.
            StoreKey::from_hex(stored.expose()).map_err(|error| SecretError::Backend {
                account: STORE_KEY_ENTRY.to_owned(),
                reason: error.to_string(),
            })
        }
        Ok(_) => {
            tracing::warn!("the store key entry is empty; treating this as a first run");
            mint(secrets, &entry).await
        }
        Err(SecretError::NotFound { .. }) => mint(secrets, &entry).await,
        // Locked, Timeout, Backend. None of them mean "there is no key".
        Err(error) => Err(error),
    }
}

/// Generates a key and writes it down before anything is encrypted under it.
///
/// The order matters: a key handed back and never stored would encrypt a
/// store nobody can open again, so the write is what makes the key real.
async fn mint(
    secrets: &dyn postio_account::secret::SecretStore,
    entry: &postio_account::secret::AccountKey,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    let key = postio_storage::key::StoreKey::generate();
    // The one place the key becomes text. `to_hex` hands back a buffer that
    // overwrites itself, and `Password` keeps that discipline from here on.
    secrets
        .store(
            entry,
            &postio_account::secret::Password::new(key.to_hex().as_str()),
        )
        .await?;
    // No key material, no length, nothing derived from it. That the store is
    // now encrypted is worth a line; what it is encrypted with is not.
    tracing::info!("this store has a new encryption key");
    Ok(key)
}

/// [`store_key`], for a caller that has no runtime yet.
///
/// Startup is the one place this is needed, and the shape of the sequence is
/// why: the store has to be open before the command bus can be built, and the
/// bus has to exist before the runtime that pumps it. So the one keyring read
/// that gates all of it gets a runtime of its own — a single current-thread
/// one, alive for exactly this call, on the thread that is about to open the
/// store anyway.
///
/// Bounded, not indefinite: `KeyringSecretStore` already turns a Secret
/// Service that never answers into an error rather than a hang, which is what
/// keeps a broken keyring costing the window a moment instead of the session.
pub fn store_key_blocking(
    secrets: &dyn postio_account::secret::SecretStore,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return Err(postio_account::secret::SecretError::Backend {
                account: STORE_KEY_ENTRY.to_owned(),
                reason: format!("no runtime to read the keyring with: {error}"),
            });
        }
    };
    runtime.block_on(store_key(secrets))
}

/// What the frontend needs, once there is a store to give it.
#[derive(Clone)]
pub struct Wiring {
    /// The local store every pane reads through.
    pub database: Database,
    /// Bodies and attachments, content-addressed beside the database.
    pub blobs: BlobStore,
    /// The store as the frontend sees it: rows in, no SQL.
    pub store: Arc<dyn MailStore>,
    /// The runtime every read is polled on.
    pub runtime: tokio::runtime::Handle,
    /// Where the engine and the handlers report to.
    pub events: EventSink,
    /// The command bus.
    pub commands: postio_core::bridge::CommandSender,
    /// Where the engine goes once it is running, so `Refresh` can find it.
    pub engine: refresh::EngineSlot,
    /// Where every outbound connection is recorded (#151).
    ///
    /// Built with the wiring so there is exactly one writer thread per
    /// process, however many accounts sync and however many discovery
    /// probes run. Connectors are handed sinks derived from this.
    pub egress: Arc<egress::EgressRecorder>,
    /// Where account passwords live.
    ///
    /// A part rather than something the modules that need it construct, for
    /// the reason `engine.rs` gives about every other part: which keyring
    /// this installation uses is a choice about *this installation*, and a
    /// module that reaches for `KeyringSecretStore::default()` itself cannot
    /// be driven by a test without a Secret Service session. Both credential
    /// paths — the one onboarding writes and the one startup reads — hang
    /// off this.
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    /// Folders the user assigned a role to by hand, from `[mailboxes]`.
    ///
    /// A part, like `secrets`, and for the same reason: which folder is this
    /// installation's archive is a choice about *this installation*, and
    /// anything that read the file itself could not be driven by a test.
    /// Empty is the ordinary case and resolves exactly as before.
    pub mailbox_roles: postio_model::RoleOverrides,
    /// How the engine backfills, from `[sync]`.
    ///
    /// A part, like `mailbox_roles`, and for the same reason: how hard this
    /// installation pulls at its server is a choice about *this installation*,
    /// and `engine::start` reading the file itself could not be driven by a
    /// test.
    pub backfill: postio_runtime::BackfillPolicy,
    /// How the engine watches for new mail, from `[sync]`.
    ///
    /// A part, like `backfill`, and for the same reason: whether this
    /// installation holds an `IDLE` connection open and how often it polls
    /// is a choice about *this installation*, and `engine::start` reading
    /// the file itself could not be driven by a test.
    pub watch: postio_sync::WatchPolicy,
    /// The ceiling on the blob store, from `[storage] max_bytes`.
    ///
    /// A part, like `backfill`, and for the same reason: how much of this
    /// machine's disk Postio may use is a choice about *this installation*,
    /// and a sweep that read the file itself could not be driven by a test.
    /// `None` is the default and means unbounded — see
    /// [`enforce_storage_ceiling`], which is the one place that decision is
    /// made.
    pub storage_ceiling: Option<u64>,
}

impl Wiring {
    /// Everything the panes need, over an already-open store.
    ///
    /// `runtime`, `events` and `commands` come from the `Bridge` that
    /// `postio_app::run` builds at startup; a test supplies its own, which is
    /// the whole point of this being constructible from outside. Not a link:
    /// `postio-app` depends on this crate and not the other way round, which
    /// is the split, and rustdoc cannot resolve upward.
    pub fn new(
        database: Database,
        blobs: BlobStore,
        runtime: tokio::runtime::Handle,
        events: EventSink,
        commands: postio_core::bridge::CommandSender,
    ) -> Self {
        Wiring {
            store: Arc::new(SqliteStore::new(&database)),
            egress: egress::EgressRecorder::start(database.clone()),
            database,
            blobs,
            runtime,
            events,
            commands,
            engine: refresh::EngineSlot::default(),
            secrets: postio_account::secret::platform_keyring(),
            mailbox_roles: postio_model::RoleOverrides::default(),
            backfill: postio_runtime::BackfillPolicy::default(),
            watch: postio_sync::WatchPolicy::default(),
            storage_ceiling: None,
        }
    }

    /// The same wiring, with `[mailboxes]` applied.
    ///
    /// Read once, at startup, and carried: a mapping edited while Postio is
    /// running takes effect at the next start rather than immediately. That
    /// is a real limit and worth stating — the engine is spawned with its
    /// parts and folder discovery runs inside it, so applying a change live
    /// means reaching into a running task, which is more than the mapping is
    /// worth on its own.
    pub fn with_mailbox_roles(mut self, roles: postio_model::RoleOverrides) -> Self {
        self.mailbox_roles = roles;
        self
    }

    /// The same wiring, backfilling under `[sync]`'s answer rather than the
    /// built-in default.
    pub fn with_backfill(mut self, backfill: postio_runtime::BackfillPolicy) -> Self {
        self.backfill = backfill;
        self
    }

    /// The same wiring, watching under `[sync]`'s answer rather than the
    /// built-in default.
    pub fn with_watch(mut self, watch: postio_sync::WatchPolicy) -> Self {
        self.watch = watch;
        self
    }

    /// The same wiring, holding the blob store to `[storage] max_bytes`.
    ///
    /// `None` — the default, and what every existing `config.toml` gets — is
    /// unbounded, exactly as before this was wired up.
    pub fn with_storage_ceiling(mut self, max_bytes: Option<u64>) -> Self {
        self.storage_ceiling = max_bytes;
        self
    }

    /// The same wiring, reading and writing passwords somewhere else.
    ///
    /// The seam a test needs: `MemorySecretStore` stands in for a keyring
    /// that has no D-Bus session behind it, and `MemorySecretStore::locked`
    /// for one nobody has unlocked.
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_account::secret::SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }
}

/// Open the local store, or say why there is none.
///
/// `Err` carries a sentence for the user rather than only for the log: a
/// store that will not open stops the application, and the surface that says
/// so needs words (#404).
///
/// `store_key` is the master key this installation's store is encrypted
/// under — see [`store_key`] for where it comes from and why a locked
/// keyring means there is no store to open rather than an unencrypted one.
///
/// That was not always so. This used to answer `None` and let the window open
/// anyway, on the argument that a mail client with nothing in it beats one
/// that will not start — which was right while the store was optional. ADR
/// 0014 ended that: the store is encrypted, its key is in the keyring, and
/// there is no degraded mode to fall back to. So the honest answer is a
/// sentence, and `postio_app::run` puts it on a screen with a retry.
pub fn open_store(
    store_key: &postio_storage::key::StoreKey,
) -> Result<(Database, BlobStore), String> {
    open_store_at(paths::store_path(), store_key)
}

/// [`open_store`], over a store at a path the caller chooses.
///
/// The default is [`paths::store_path`] and every shipping caller wants it;
/// this exists for the ones that cannot use a process-wide answer. A test
/// needs a store per test rather than per machine, and the macOS boundary
/// (ADR 0019) has to be able to open one before `paths` knows what
/// `~/Library/Application Support` means for this project — a decision that
/// is still open, and which this deliberately does not pre-empt.
///
/// Same contract as [`open_store`] in every other respect, including that
/// `Err` carries a sentence meant for a person.
pub fn open_store_at(
    path: impl Into<std::path::PathBuf>,
    store_key: &postio_storage::key::StoreKey,
) -> Result<(Database, BlobStore), String> {
    // The database subkey. BLAKE3-derived from the master key, so the
    // database, the blob contents and the blob ids are cryptographically
    // separated without three keyring entries (ADR 0014 Q3). #301 takes the
    // other two.
    let database_key = store_key.derive(postio_storage::key::Purpose::Database);
    let path = path.into();

    // Before anything opens the store, because a plaintext one will not open
    // at all: `Database::open` offers SQLCipher a key for a file that has none
    // and gets "file is not a database". ADR 0014 Q4's migration is what turns
    // that into a store this build can read, and it answers
    // `AlreadyEncrypted` and does no work on every open after the first.
    match postio_storage::encrypt::encrypt_store(&path, store_key) {
        Ok(postio_storage::encrypt::Outcome::Encrypted(report)) => {
            tracing::info!(
                blobs = report.blobs,
                bytes = report.bytes,
                "the local store has been encrypted"
            );
        }
        Ok(postio_storage::encrypt::Outcome::Resumed) => {
            tracing::info!("an interrupted store encryption was finished");
        }
        Ok(
            postio_storage::encrypt::Outcome::AlreadyEncrypted
            | postio_storage::encrypt::Outcome::NoStore,
        ) => {}
        // The queue is the one thing in the store that is not a copy of
        // something on a server, so the migration refuses to run over it
        // rather than deciding for somebody. The sentence has to say what to
        // do next, because "drain first" is an instruction to a person.
        Err(error @ postio_storage::Error::QueueNotDrained { .. }) => {
            tracing::error!(path = %path.display(), %error, "the store cannot be encrypted yet");
            return Err(format!(
                "Postio could not encrypt its local store. {error} Open the previous \
                 version, let it finish syncing, and start this one again."
            ));
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "the store could not be encrypted");
            return Err(format!(
                "Postio could not encrypt its local store: {error}. Nothing was \
                 changed; the store is exactly as it was."
            ));
        }
    }

    let database = match Database::open(&path, &database_key) {
        Ok(database) => database,
        // A wrong key is its own sentence. `Error::WrongStoreKey` says the
        // store belongs to another installation and is *intact*, where
        // SQLite's own wording for the same condition is "file is not a
        // database" — which would tell somebody their mail is corrupt when
        // the only thing wrong is which key we offered.
        Err(error @ postio_storage::Error::WrongStoreKey) => {
            tracing::error!(path = %path.display(), "the store will not decrypt with this key");
            return Err(format!("Postio could not unlock its local store. {error}"));
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "cannot open the store");
            // The sentence goes back to the caller as well as to the log,
            // because the caller is what puts it on screen (#404). A window
            // that will not open and does not say why is the one thing worse
            // than a window that will not open.
            return Err(format!("Postio could not open its local store: {error}"));
        }
    };
    // Beside the database, not inside it: bodies and attachments are
    // content-addressed files, and SQLite holds the key and the metadata.
    let blobs = match BlobStore::open(
        path.with_file_name("blobs"),
        &postio_storage::key::BlobKeys::derive(store_key),
    ) {
        Ok(blobs) => blobs,
        Err(error) => {
            tracing::error!(%error, "cannot open the blob store");
            return Err(format!(
                "Postio could not open the store that holds message bodies \
                 and attachments: {error}"
            ));
        }
    };
    if let Err(error) = ensure_search_index(&database) {
        // Recoverable: everything except search still works, and refusing to
        // open a mail client because its index would not build would be a
        // worse answer than opening one you cannot search.
        tracing::error!(%error, "the search index is unavailable");
    }

    Ok((database, blobs))
}

/// The `settings` key that records whether a session is running.
const SESSION_STATE_KEY: &str = "session_state";

/// Record that a session started, and answer whether the last one crashed.
///
/// `true` means the previous session never reached [`end_session`] — the
/// process died with the marker still saying `open` — and crash-shaped
/// recovery (reopening a mid-edit draft, #491) is warranted. `false` is
/// every other history: a clean exit, or a store this binary has never
/// opened. The distinction exists because `DraftState::Editing` alone is
/// not evidence of a crash — `Esc` on a draft with content parks it in
/// exactly that state on purpose, and a mail client that opens into a
/// stale compose buffer instead of the inbox reads as broken.
///
/// Call once per process, before anything consults the answer: the call
/// itself flips the marker to `open`, so a second call in the same process
/// would report its own session as a crash.
pub fn begin_session(database: &Database) -> bool {
    let Ok(connection) = database.connection() else {
        return false;
    };
    let settings = postio_storage::repository::SettingsRepository::new(&connection);
    let unclean = matches!(settings.get(SESSION_STATE_KEY), Ok(Some(state)) if state == "open");
    if let Err(error) = settings.set(SESSION_STATE_KEY, "open") {
        tracing::warn!(%error, "could not record the session start");
    }
    unclean
}

/// Record a clean shutdown — the other half of [`begin_session`].
///
/// Called on the orderly exit path. A process that dies without reaching
/// this is precisely what the marker exists to notice.
pub fn end_session(database: &Database) {
    let Ok(connection) = database.connection() else {
        return;
    };
    let settings = postio_storage::repository::SettingsRepository::new(&connection);
    if let Err(error) = settings.set(SESSION_STATE_KEY, "closed") {
        tracing::warn!(%error, "could not record the clean shutdown");
    }
}

/// Create the full-text index if it is not there, on every start.
///
/// The same contract `postio_storage::migrate` has, and for the same reason:
/// the schema is part of opening the store, not part of searching it. Once it
/// exists the metadata columns — subject, sender, recipients — are maintained
/// **by trigger**, exactly like the mailbox counts in migration 0003, so this
/// one call indexes every message already in the store and every one that
/// arrives after.
///
/// It was missing entirely. `postio_index::index::ensure_schema` documents
/// that it must run at startup and nothing ran it, so `search_documents` and
/// `messages_fts` did not exist on any real store and search had nothing to
/// search — `postio-x4e`, and the ninth instance of `postio-bl2`.
///
/// Message *bodies* are a separate matter. They are compressed columns on the
/// `messages` row (ADR 0020), so no trigger can reach them — a trigger would
/// see the ciphertext of a zstd frame, not words to index.
/// `postio_sync::backfill::fetch_body` indexes each body as it lands, and
/// [`index_local_bodies`] catches up whatever landed before that call existed
/// (#327).
pub fn ensure_search_index(database: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let connection = database.connection()?;
    postio_index::index::ensure_schema(&connection)?;
    tracing::debug!("the search index is ready");
    Ok(())
}

/// How long a blob must have gone untouched before a sweep may call it garbage.
///
/// One hour, `GarbageCollection::default`'s own value, named here so the
/// production caller is visibly *not* passing `Duration::ZERO`.
///
/// This is the one number in this module that is load-bearing rather than
/// tuning. A blob is written before the row that references it is committed,
/// so during that window an entirely healthy blob is indistinguishable from an
/// orphan — and a sweep with no grace period deletes the body of a message
/// that is mid-fetch. Tests pass `ZERO` deliberately and nothing else may.
pub const BLOB_GRACE_PERIOD: Duration = Duration::from_secs(60 * 60);

/// How long a dragged-out export must have sat untouched before a sweep may
/// reclaim it.
///
/// One hour, and this is a **guard rather than tuning**. #121 established
/// that the file transfer portal hands a receiver a *path* and never copies
/// the bytes, so a file deleted between the drop and the receiver's read
/// produces a silent no-op: the receiver gets nothing, Postio reports
/// success, and no error is raised anywhere. A drop's file is seconds old
/// while that is happening, so an hour is a margin of three thousand.
///
/// It is also what makes the sweep safe with several instances running,
/// which "nothing of mine is in flight" would not be: this process cannot
/// see another one's drag and does not need to, because that drag's file is
/// young.
pub const DRAG_EXPORT_GRACE_PERIOD: Duration = Duration::from_secs(60 * 60);

/// Delete dragged-out exports nothing is using any more. Answers how many.
///
/// # Why this exists
///
/// Dragging a message or an attachment out of Postio writes a file into
/// [`paths::export_dir`], and nothing ever deleted one (#278): three writers
/// and no reader. Someone who drags mail out regularly accumulated a
/// plaintext copy of every message they had ever dragged, sitting outside the
/// blob store, in a cache directory nothing audits, indefinitely.
///
/// # Why it is an age and not a purge
///
/// The obvious fix — empty the directory on start — is the dangerous one, and
/// [`paths::export_dir`]'s own documentation warns against it. The receiver
/// of a drop reads a path *after* the drop, on its own schedule, so a sweep
/// that can touch a file a drop is still using turns a successful drag into
/// nothing at all, silently.
///
/// `older_than` is the whole safety property, so it is a parameter rather
/// than a constant reached for inside: the production caller passes
/// [`DRAG_EXPORT_GRACE_PERIOD`] and is visibly not passing `ZERO`, exactly as
/// [`BLOB_GRACE_PERIOD`] arranged for blobs.
///
/// # Cost
///
/// One `read_dir` of a directory that is empty on a machine where nobody
/// drags mail out, which is why this can sit on the startup path beside
/// [`purge_fetch_debris`]. An entry whose age cannot be read is left alone:
/// the sweep's job is bounding a cache, and it may not guess.
pub fn reclaim_drag_exports(
    directory: &Path,
    older_than: Duration,
) -> Result<usize, Box<dyn std::error::Error>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        // Nobody has ever dragged anything out on this machine.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };

    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        // Unreadable metadata, or an mtime in the future because a clock
        // moved: either way this sweep does not know how old it is, and
        // "delete what you cannot date" is how the guard gets lost.
        let Ok(age) = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .map(|modified| now.duration_since(modified).unwrap_or_default())
        else {
            continue;
        };
        if age < older_than {
            continue;
        }
        // An attachment export gets a directory of its own so two parts
        // sharing a filename do not collide, so both kinds turn up here.
        let gone = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match gone {
            Ok(()) => removed += 1,
            // Another instance swept it first, or it is in use. Neither is
            // this process's problem, and neither is worth failing a start.
            Err(error) => {
                tracing::debug!(%error, path = %path.display(), "could not reclaim a drag export");
            }
        }
    }
    Ok(removed)
}

/// Delete blobs the database no longer references. Answers what went.
///
/// # Why this exists at all
///
/// `BlobStore::collect_garbage` was written, tested and documented, and no
/// production code ever called it (#416) — the third instance in this project
/// of a mechanism that exists, passes its tests, and is wired to nothing.
///
/// The consequence was not subtle. `MessageRepository::delete` removes the row
/// and never touches blobs, *deliberately*, because the schema delegates
/// reclamation to this sweep. With no caller, **deleting mail freed nothing,
/// ever**. The worst case is not a user deleting anything: a `UIDVALIDITY`
/// reset wipes and re-syncs a whole mailbox, orphaning every blob in it at
/// once — gigabytes, from one server-side event nobody caused.
///
/// # Why it cannot lose anything
///
/// It sweeps only what nothing points at. `referenced_blobs` reads every
/// column that holds a blob key and keeps anything named there; a blob that
/// survives that filter is unreachable by definition, so there is no policy
/// here and nothing to configure. That is what separates this from
/// `BlobStore::evict_to_fit`, which removes blobs something *does* reference,
/// on purpose, and pays for it with a refetch.
///
/// # Not on the startup path
///
/// It walks the blob directory, which on a backfilled archive is a lot of
/// files. Startup has a 500 ms budget; callers put this on a worker, the same
/// way [`index_local_bodies`] is spawned rather than awaited.
pub fn reclaim_orphaned_blobs(
    database: &Database,
    blobs: &BlobStore,
    min_age: Duration,
) -> Result<GarbageReport, Box<dyn std::error::Error>> {
    let connection = database.connection()?;
    let report = blobs.collect_garbage(&connection, GarbageCollection { min_age })?;
    if report.removed > 0 {
        // Counts and bytes only: what was in those blobs is somebody's mail.
        tracing::info!(
            removed = report.removed,
            bytes = report.bytes_reclaimed,
            scanned = report.scanned,
            "reclaimed blobs nothing references"
        );
    }
    Ok(report)
}

/// Bring the blob store back under `[storage] max_bytes` by dropping what can
/// be fetched again. Answers what one pass took, or `None` when no ceiling is
/// set.
///
/// # Why this exists at all
///
/// The third of the sweeps #416 was filed about, and the last to get a caller.
/// `BlobStore::evict_to_fit` was written, documented and tested five times
/// over, and no production code named it — so `max_bytes` in somebody's
/// `config.toml` was a setting that parsed, validated, round-tripped, and did
/// nothing whatever to their disk. #416 deliberately scoped it out because it
/// is the one sweep carrying a policy; #862 is that policy arriving.
///
/// # Unset means unbounded, and must not be read as zero
///
/// `[storage]`'s module docs settle the default: a number here is a promise
/// about somebody else's disk and Postio does not know how big theirs is. So
/// `None` returns before anything is counted rather than falling through to a
/// budget of nothing — the reading that would empty a store on first start.
/// That is why this takes `Option<u64>` rather than a `u64` a caller has
/// already unwrapped: the decision belongs in one place, with a test on it.
///
/// # Why it can afford to lose something
///
/// Unlike [`reclaim_orphaned_blobs`], this removes blobs a row still points
/// at. ADR 0014 is what makes that safe: everything except drafts and the
/// operation queue can be re-synced, so the store is a cache and a cache may
/// evict. What goes is only ever raw message source and attachment payloads,
/// oldest mail first, and never the text search is made of — since ADR 0020
/// bodies are columns on the row, so there is no blob here for a pass over the
/// blob store to take. Eviction costs a round trip later; it never costs mail,
/// and the row it clears is left exactly as the payload axis reads "not here
/// yet".
///
/// # Not on the startup path
///
/// It stats every blob in the store to measure it, which on a backfilled
/// archive is a great many files — the same reason [`reclaim_orphaned_blobs`]
/// is spawned rather than awaited, and the same 500 ms budget it would
/// otherwise spend.
pub fn enforce_storage_ceiling(
    database: &Database,
    blobs: &BlobStore,
    max_bytes: Option<u64>,
) -> Result<Option<EvictionReport>, Box<dyn std::error::Error>> {
    let Some(budget) = max_bytes else {
        return Ok(None);
    };
    let connection = database.connection()?;
    let report = blobs.evict_to_fit(&connection, budget)?;
    if report.removed > 0 {
        // Counts and bytes only: what was in those blobs is somebody's mail.
        tracing::info!(
            removed = report.removed,
            bytes = report.bytes_reclaimed,
            remaining = report.bytes_remaining,
            budget,
            "evicted refetchable blobs to fit the storage ceiling"
        );
    }
    if report.bytes_remaining > budget {
        // Eviction ran out of things it was allowed to take, which means the
        // store is mostly text. Worth saying once: the ceiling is not being
        // honoured, and no further pass will change that.
        tracing::warn!(
            remaining = report.bytes_remaining,
            budget,
            "the store is over its ceiling with nothing refetchable left to drop"
        );
    }
    Ok(Some(report))
}

/// How many blocks one repair batch rebuilds.
///
/// Each one is a blob read and a parse, so the batch is a bound on how long
/// the pass holds a connection between yields rather than a tuning knob. The
/// same size and the same reason as [`INDEX_BODY_BATCH`].
const REPAIR_HEADERS_BATCH: u32 = 256;

/// Rebuild the header blocks of mail downloaded before there was anywhere to
/// put them. Answers how many.
///
/// # Why there is anything to repair
///
/// `messages.body_headers` has existed since migration 0001 and has been NULL
/// on every row in every store, because both backfill paths passed
/// `headers: None` on purpose — "a copy nobody reads is a copy that can go
/// stale", which was true until ADR 0025 gave it a reader. Mail arriving from
/// now on carries its block; this is the mail that is already here, and
/// without it `header:` answers "no such mail" across a mailbox somebody has
/// been using for a year, which is indistinguishable from the feature being
/// broken.
///
/// # It reaches no network
///
/// Only messages that still have their raw source, which is already on disk —
/// that is what makes this a repair rather than a re-download. Messages with
/// no blob need a fetch and are
/// [`MessageRepository::messages_needing_a_header_fetch`]'s, deliberately kept
/// out of this pass's batches: it is windowed newest-first, so one batch of
/// unfetchable rows would make no progress, trip the guard below, and stop the
/// pass before it reached older messages it could have fixed.
///
/// # Not on the startup path
///
/// A blob read and a header parse per message, over every message in the
/// store. That is minutes of I/O on a backfilled archive and nothing on screen
/// waits for it, so callers spawn it exactly as they spawn [`index_local_bodies`].
///
/// # A blob that will not read is not an empty block
///
/// Eviction takes raw source first (`PRODUCT.md` §6), so the oldest mail in a
/// bounded store routinely has a row pointing at a blob that is gone. Those
/// are skipped rather than written as an empty block: an empty block is a
/// claim that the message *has* no such header, which the index would then
/// answer with for ever and nothing would be left to say otherwise.
pub fn repair_header_blocks(
    database: &Database,
    blobs: &BlobStore,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut repaired = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connection()?;
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let candidates = messages.messages_missing_headers(REPAIR_HEADERS_BATCH)?;
        if candidates.is_empty() {
            break;
        }
        let batch: Vec<i64> = candidates
            .iter()
            .map(|candidate| candidate.message_id.get())
            .collect();
        // The candidate query's contract is that repairing a message removes
        // it from the answer. An identical batch means that contract is broken
        // and another lap can only spin — the failure #500 recorded, where one
        // batch of messages nothing could do anything with ran a core flat out
        // for as long as the application was open.
        if batch == last_batch {
            tracing::warn!(
                batch = batch.len(),
                "a header-repair batch made no progress; stopping the pass"
            );
            break;
        }
        last_batch = batch;

        for candidate in candidates {
            let Some(blob) = candidate.raw_blob_id else {
                continue;
            };
            let Ok(raw) = blobs.get(&blob) else {
                // Evicted, or a store somebody tidied by hand. Not an empty
                // block: see the note above about what that would claim.
                continue;
            };
            let Some(block) = postio_model::headers::block_of(&raw) else {
                continue;
            };
            messages.set_headers(candidate.message_id, Some(&block))?;
            repaired += 1;
        }
    }

    if repaired > 0 {
        // A count, and nothing about what was in them: header values carry
        // `Received` chains with addresses and internal hostnames.
        tracing::info!(repaired, "rebuilt header blocks from raw source on disk");
    }
    Ok(repaired)
}

/// Train a body-compression dictionary from the mail already on this machine,
/// if the corpus has grown enough to be worth one. Answers whether it did.
///
/// # Why it is worth a pass of its own
///
/// Bodies compress about 1.57x on their own and about 2.19x against a
/// dictionary trained on the mailbox they came from (ADR 0020) — mail from one
/// correspondence is full of the same signatures, quoted headers and
/// boilerplate. On the reference account that further quarter is most of a
/// gigabyte, and it is unreachable until something calls
/// [`postio_storage::body::train_dictionary`].
///
/// # Not on the startup path, and not on every start
///
/// It decompresses a few thousand bodies to train from, so it belongs on a
/// worker beside [`index_local_bodies`]. And it asks
/// [`postio_storage::body::should_train`] first, which holds it to ADR 0017's
/// heuristic: train once, then again only when the corpus has grown tenfold.
/// A pass that retrained on every start would leave a table of near-identical
/// dictionaries that nothing may ever delete — rows name them, and the schema
/// refuses to drop a dictionary a row names, because dropping one would take
/// that message's text with it.
///
/// # Nothing is rewritten
///
/// A zstd frame can only be read with the dictionary it was written against,
/// so every body already stored keeps naming whatever it was written against
/// and goes on reading. Only writes after this use the new one. Rewriting the
/// mailbox to recompress it would be hours of somebody's disk to save a
/// fraction of a gigabyte, against a non-zero chance of losing a message.
pub fn train_body_dictionary(database: &Database) -> Result<bool, Box<dyn std::error::Error>> {
    let connection = database.connection()?;
    if !postio_storage::body::should_train(&connection)? {
        tracing::debug!("the body corpus has not grown enough to retrain a dictionary");
        return Ok(false);
    }

    // The write is one small row, but the read that precedes it is the whole
    // sample. Take the permit from the background lane so a keystroke's flag
    // write goes first.
    let _permit = connection
        .write_gate()
        .acquire(postio_storage::WritePriority::Background);
    let Some(dictionary) = postio_storage::body::train_dictionary(&connection)? else {
        return Ok(false);
    };

    // An id, and nothing about what it was trained on.
    tracing::info!(
        dictionary = dictionary.get(),
        "trained a body compression dictionary"
    );
    Ok(true)
}

/// Delete leftover `.part` files from fetches that never finished. Answers how
/// many.
///
/// A cancelled or failed fetch removes its own temporary file when its writer
/// drops, so this is for the case where no destructor ran at all — a power
/// cut, an OOM kill, a crash mid-fetch. Nothing else will ever finish those
/// files.
///
/// Cheap and bounded: one `read_dir` of a directory that is empty in the
/// ordinary case, which is why this one *can* run on the startup path.
pub fn purge_fetch_debris(blobs: &BlobStore) -> Result<usize, Box<dyn std::error::Error>> {
    let purged = blobs.purge_temporary()?;
    if purged > 0 {
        tracing::info!(purged, "removed debris from fetches that never finished");
    }
    Ok(purged)
}

/// How many bodies one pass of [`index_local_bodies`] reads before letting go
/// of its connection.
///
/// The pass holds a pooled connection and decompresses a body per message,
/// and the rest of the application shares that pool. Batching bounds how long any one
/// checkout lasts without making the pass itself stop early: it keeps taking
/// batches until there is nothing left.
const INDEX_BODY_BATCH: u32 = 200;

/// How long [`index_local_bodies`] pauses between batches.
///
/// The pass is a catch-up, not a foreground job: nothing waits on it, and the
/// search index it fills is useful incomplete. A large archive takes a few
/// hundred batches either way; the pause is what keeps the disk answering
/// searches while it happens.
const INDEX_BODY_BREATHER: Duration = Duration::from_millis(25);

/// Index every message whose body is already on this machine and whose
/// indexed text is empty. Answers how many it indexed.
///
/// # Why this exists at all
///
/// `postio_index::index::index_body` was written, tested and benched, and no
/// production code ever called it — so `search_documents.body` was empty on
/// every message in every real store, and search matched metadata only
/// (#327). The fix proper is in `postio_sync::backfill::fetch_body`, where
/// every body lands. This is the other half: mail that was already local when
/// that call did not exist, and any body whose index write was lost between
/// the storage commit point and it.
///
/// # Why it is safe to run on every start
///
/// It is driven by
/// [`messages_missing_body_text`](postio_index::index::messages_missing_body_text),
/// which asks for messages whose body is local *and* whose indexed text is
/// empty — so on a store that is caught up it does one query, finds nothing,
/// and returns. Re-indexing a message overwrites one column of one row keyed
/// by `message_id`; there is no row to duplicate.
///
/// # Not on the startup path
///
/// The first run over an existing archive reads and decompresses a body per
/// message, which is minutes of work on a large one. Callers must put this on
/// a worker — the application spawns it on the runtime after the window is up
/// — because a mail client that will not draw until its search index is warm
/// has traded the wrong thing for search.
///
/// Errors on one message are logged and skipped rather than abandoning the
/// pass: one unreadable body should cost that message its body search, not
/// every message after it.
pub fn index_local_bodies(database: &Database) -> Result<usize, Box<dyn std::error::Error>> {
    let mut indexed = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connection()?;
        let candidates =
            postio_index::index::messages_missing_body_text(&connection, INDEX_BODY_BATCH)?;
        if candidates.is_empty() {
            break;
        }
        // The candidate query's contract is that indexing a message removes
        // it from the answer. If a whole batch comes back identical, that
        // contract is broken and going around again can only spin — which is
        // not hypothetical: before #500, a store with one batch of
        // attachment-only messages ran this loop at 100% of a core for as
        // long as the app was open. Stopping leaves the index exactly as
        // caught-up as it was ever going to get this start.
        if candidates == last_batch {
            tracing::warn!(
                batch = candidates.len(),
                "a body-index batch made no progress; stopping the pass"
            );
            break;
        }

        // Read first, write after, in phases: the reads decompress a body per
        // message and must not happen inside the write transaction below,
        // where they would hold SQLite's one write lock through work that
        // needs nothing of it.
        //
        // One repository for the whole batch, deliberately: it caches the
        // compression dictionary it loads, so a batch of two hundred bodies
        // builds the decoding table once rather than two hundred times.
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut bodies: Vec<(i64, postio_model::MessageBody)> =
            Vec::with_capacity(candidates.len());
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            let body = match messages.body(message) {
                Ok(Some(stored)) => postio_model::MessageBody {
                    text: stored.text,
                    html: stored.html,
                },
                // No such row any more -- expunged between the candidate query
                // and here. Nothing to index.
                Ok(None) => postio_model::MessageBody::default(),
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a body to index");
                    continue;
                }
            };
            bodies.push((*id, body));
        }

        // One gated transaction per batch, not an autocommit per message.
        // Each of those commits was its own WAL append taken without the
        // write gate, so a long catch-up ran a stream of ungated writes
        // against whatever the user was doing. The permit comes first, and
        // from the background lane: a keystroke's flag write goes ahead of
        // this whole batch.
        {
            let _permit = connection
                .write_gate()
                .acquire(postio_storage::WritePriority::Background);
            connection.execute_batch("BEGIN IMMEDIATE")?;
            for (id, body) in &bodies {
                match postio_index::index::index_body_of(&connection, *id, body) {
                    Ok(()) => indexed += 1,
                    Err(error) => tracing::debug!(message = id, %error, "cannot index a body"),
                }
            }
            connection.execute_batch("COMMIT")?;
        }

        let taken = candidates.len();
        last_batch = candidates;
        drop(connection);
        if taken < INDEX_BODY_BATCH as usize {
            break;
        }
        // Let go of the machine between batches. The pass runs at start on a
        // worker while the window is already live; without a pause it
        // decompresses bodies and writes the index as fast as the machine
        // allows, and the search this index exists to serve pays for that in
        // evicted cache and queued reads (#500).
        std::thread::sleep(INDEX_BODY_BREATHER);
    }
    if indexed > 0 {
        // A count and nothing else: what a log may carry about mail.
        tracing::info!(indexed, "indexed bodies that were already local");
    }
    Ok(indexed)
}

/// How many header blocks one pass of [`index_local_headers`] parses before
/// letting go of its connection.
///
/// The pass holds a pooled connection and decompresses and parses a block per
/// message, and the rest of the application shares that pool. The same bound
/// and the same reason as [`INDEX_BODY_BATCH`], and the same size: a block is
/// smaller than a body but a parse is not free, and there is no reason for
/// the two catch-ups to hold a checkout for different lengths of time.
const INDEX_HEADERS_BATCH: u32 = 200;

/// How long [`index_local_headers`] pauses between batches. See
/// [`INDEX_BODY_BREATHER`].
const INDEX_HEADERS_BREATHER: Duration = Duration::from_millis(25);

/// Index the headers of every message whose block is already on this machine
/// and whose header rows are missing. Answers how many it indexed.
///
/// # Why this exists at all
///
/// `header:` matches `message_headers`, which is derived from
/// `messages.body_headers` (ADR 0025 Q2). Two populations need it: every
/// message that arrived before #884 gave the column a writer, and — for ever
/// after — every message in the store whenever the headers schema half is
/// bumped, which is the mechanism that keeps ADR 0025 Q3's two caps
/// revisable. Without the pass, `header:` answers "no such mail" across a
/// mailbox somebody has been using for a year, and nothing distinguishes
/// that from the feature being broken.
///
/// # It reaches no network, and it is not the repair pass
///
/// ADR 0025 Q5 has three populations and this is the first of them: the block
/// is already stored, so filling the rows is a read of a column the row
/// already carries. A message whose `body_headers` is NULL belongs to
/// [`repair_header_blocks`], which rebuilds the block from the raw source on
/// disk, or — with no raw source either — to the backfill lane's
/// `Want::HeaderBlock`, which is the only one of the three that dials out.
/// Keeping them apart is not tidiness: this pass is windowed newest-first, so
/// one batch of rows it could do nothing with would make no progress, trip
/// the guard below, and stop it before it reached the mail it could have
/// indexed.
///
/// # Why it is safe to run on every start
///
/// [`messages_missing_header_rows`](postio_index::index::messages_missing_header_rows)
/// asks for blocks that are stored *and* unindexed, so a caught-up store
/// costs one query that finds nothing. Re-indexing a message replaces its
/// rows rather than adding to them, so there is nothing to duplicate.
///
/// # Not on the startup path
///
/// A decompression and a header parse per message, over every message in the
/// store — minutes on a backfilled archive, and nothing on screen waits for
/// it. Callers spawn it exactly as they spawn [`index_local_bodies`].
///
/// Errors on one message are logged and skipped rather than abandoning the
/// pass: one unreadable block should cost that message its `header:` matches,
/// not every message after it.
pub fn index_local_headers(database: &Database) -> Result<usize, Box<dyn std::error::Error>> {
    let mut indexed = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connection()?;
        let candidates =
            postio_index::index::messages_missing_header_rows(&connection, INDEX_HEADERS_BATCH)?;
        if candidates.is_empty() {
            break;
        }
        // The candidate query's contract is that indexing a message removes
        // it from the answer. An identical batch means the contract is broken
        // and another lap can only spin -- the failure #500 recorded, where
        // one batch of messages nothing could do anything with ran a core
        // flat out for as long as the application was open.
        if candidates == last_batch {
            tracing::warn!(
                batch = candidates.len(),
                "a header-index batch made no progress; stopping the pass"
            );
            break;
        }

        // Read first, write after, in phases, for the reason
        // `index_local_bodies` gives: these reads decompress and parse a
        // block per message and must not happen inside the write transaction,
        // where they would hold SQLite's one write lock through work that
        // needs nothing of it.
        //
        // One repository for the whole batch, so the compression dictionary
        // is loaded once rather than two hundred times.
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut blocks: Vec<(i64, postio_model::Headers)> = Vec::with_capacity(candidates.len());
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            match messages.headers(message) {
                Ok(Some(headers)) => blocks.push((*id, headers)),
                // Expunged between the candidate query and here. Nothing to
                // index, and `index_headers` would no-op on it anyway.
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a header block to index")
                }
            }
        }

        {
            let _permit = connection
                .write_gate()
                .acquire(postio_storage::WritePriority::Background);
            connection.execute_batch("BEGIN IMMEDIATE")?;
            for (id, headers) in &blocks {
                match postio_index::index::index_headers(&connection, *id, headers) {
                    Ok(()) => indexed += 1,
                    Err(error) => {
                        tracing::debug!(message = id, %error, "cannot index a header block")
                    }
                }
            }
            connection.execute_batch("COMMIT")?;
        }

        let taken = candidates.len();
        last_batch = candidates;
        drop(connection);
        if taken < INDEX_HEADERS_BATCH as usize {
            break;
        }
        std::thread::sleep(INDEX_HEADERS_BREATHER);
    }
    if indexed > 0 {
        // A count and nothing else. Header values carry `Received` chains
        // with addresses and internal hostnames, and a log never carries
        // message content.
        tracing::info!(indexed, "indexed header blocks that were already local");
    }
    Ok(indexed)
}

/// How many bodies or header blocks one batch of [`reindex_account`] handles.
///
/// Same size and the same reason as [`INDEX_BODY_BATCH`]/
/// [`INDEX_HEADERS_BATCH`]: this holds a pooled connection and decompresses
/// a body or parses a block per message, and the rest of the application
/// shares that pool.
const REINDEX_ACCOUNT_BATCH: u32 = 200;

/// How long [`reindex_account`] pauses between batches. Same reasoning as
/// [`INDEX_BODY_BREATHER`]: search still answers while this runs, off an
/// index that is being refilled, and the disk it reads from has to keep
/// answering that too.
const REINDEX_ACCOUNT_BREATHER: Duration = Duration::from_millis(25);

/// Drops `account`'s rows from the local search index and rebuilds them from
/// mail already on this machine — bodies, then header blocks. Answers how
/// many rows it rewrote.
///
/// #981's "Rebuild search index": the account-detail action for a message
/// that is in the mailbox and missing from search, which the maintainer's
/// own decision on that issue names as the thing that actually drifts —
/// three separate catch-up passes have already shipped to repair it
/// (#327, #884, and the body-index catch-up this pass shares its shape
/// with). This is the same repair, reachable on demand and scoped to one
/// account: clearing first is what makes this account's whole local corpus
/// the candidate set again, not only the messages that already happened to
/// be missing.
///
/// # No network, ever
///
/// Everything here reads what is already local — `messages_missing_body_text_for_account`
/// only offers a message whose `body_state` says the bytes are already on
/// this machine, and the header half is windowed to blocks already stored.
/// A message this account has not downloaded yet is untouched by this pass;
/// bringing it down is sync's job, not this one's (the maintainer's own "a
/// re-download is a verb that already exists").
///
/// # Progress
///
/// `on_progress(done, total)` is called once with `total` fixed at the
/// start — the candidate count right after clearing — and again after every
/// batch of either half, `done` accumulating across both. The composition
/// root turns this into the same [`postio_core::Event::BackfillProgress`] a
/// real backfill reports (#981's own design: one progress channel, not two),
/// so a rebuild in progress shows on the account row the way any other
/// catch-up already does.
///
/// # Errors
///
/// Errors on one message are logged and skipped rather than abandoning the
/// pass, for the same reason [`index_local_bodies`]/[`index_local_headers`]
/// do: one unreadable body or block should cost that message its own search
/// terms, not every message after it.
pub fn reindex_account(
    database: &Database,
    account: postio_model::ids::AccountId,
    mut on_progress: impl FnMut(u32, u32),
) -> Result<usize, Box<dyn std::error::Error>> {
    let account_id = account.get();
    let connection = database.connection()?;
    postio_index::index::clear_account_body_index(&connection, account_id)?;
    postio_index::index::clear_account_header_index(&connection, account_id)?;
    let total = postio_index::index::messages_missing_body_text_for_account(
        &connection,
        account_id,
        u32::MAX,
    )?
    .len()
        + postio_index::index::messages_missing_header_rows_for_account(
            &connection,
            account_id,
            u32::MAX,
        )?
        .len();
    drop(connection);

    let mut done = 0usize;
    on_progress(0, total as u32);

    // -- bodies -------------------------------------------------------------
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connection()?;
        let candidates = postio_index::index::messages_missing_body_text_for_account(
            &connection,
            account_id,
            REINDEX_ACCOUNT_BATCH,
        )?;
        if candidates.is_empty() {
            break;
        }
        // The no-progress guard #500 recorded the need for, the same as
        // index_local_bodies' own.
        if candidates == last_batch {
            tracing::warn!(
                account = account_id,
                batch = candidates.len(),
                "a reindex body batch made no progress; stopping that half"
            );
            break;
        }

        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut bodies: Vec<(i64, postio_model::MessageBody)> =
            Vec::with_capacity(candidates.len());
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            let body = match messages.body(message) {
                Ok(Some(stored)) => postio_model::MessageBody {
                    text: stored.text,
                    html: stored.html,
                },
                Ok(None) => postio_model::MessageBody::default(),
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a body to reindex");
                    continue;
                }
            };
            bodies.push((*id, body));
        }

        {
            let _permit = connection
                .write_gate()
                .acquire(postio_storage::WritePriority::Background);
            connection.execute_batch("BEGIN IMMEDIATE")?;
            for (id, body) in &bodies {
                match postio_index::index::index_body_of(&connection, *id, body) {
                    Ok(()) => done += 1,
                    Err(error) => tracing::debug!(message = id, %error, "cannot reindex a body"),
                }
            }
            connection.execute_batch("COMMIT")?;
        }
        on_progress(done as u32, total as u32);

        let taken = candidates.len();
        last_batch = candidates;
        drop(connection);
        if taken < REINDEX_ACCOUNT_BATCH as usize {
            break;
        }
        std::thread::sleep(REINDEX_ACCOUNT_BREATHER);
    }

    // -- header blocks --------------------------------------------------------
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connection()?;
        let candidates = postio_index::index::messages_missing_header_rows_for_account(
            &connection,
            account_id,
            REINDEX_ACCOUNT_BATCH,
        )?;
        if candidates.is_empty() {
            break;
        }
        if candidates == last_batch {
            tracing::warn!(
                account = account_id,
                batch = candidates.len(),
                "a reindex header batch made no progress; stopping that half"
            );
            break;
        }

        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut blocks: Vec<(i64, postio_model::Headers)> = Vec::with_capacity(candidates.len());
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            match messages.headers(message) {
                Ok(Some(headers)) => blocks.push((*id, headers)),
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a header block to reindex")
                }
            }
        }

        {
            let _permit = connection
                .write_gate()
                .acquire(postio_storage::WritePriority::Background);
            connection.execute_batch("BEGIN IMMEDIATE")?;
            for (id, headers) in &blocks {
                match postio_index::index::index_headers(&connection, *id, headers) {
                    Ok(()) => done += 1,
                    Err(error) => {
                        tracing::debug!(message = id, %error, "cannot reindex a header block")
                    }
                }
            }
            connection.execute_batch("COMMIT")?;
        }
        on_progress(done as u32, total as u32);

        let taken = candidates.len();
        last_batch = candidates;
        drop(connection);
        if taken < REINDEX_ACCOUNT_BATCH as usize {
            break;
        }
        std::thread::sleep(REINDEX_ACCOUNT_BREATHER);
    }

    // `total` is an estimate taken before either pass ran, so a message that
    // stopped being a candidate between then and now (expunged, or already
    // swept by a concurrent catch-up) can leave `done` short of it -- and a
    // caller driving a progress bar off this needs the final call to say
    // "done" rather than "94 of 100 and nothing more is coming". Skipped
    // when there was never anything to do at all: the one report already
    // sent before either loop ran already said so, and a second identical
    // "(0, 0)" is a call with nothing new to tell a caller.
    if total > 0 || done > 0 {
        on_progress(done as u32, done.max(total) as u32);
    }
    if done > 0 {
        // A count and nothing else, the same restraint the two catch-ups
        // this wraps keep.
        tracing::info!(
            account = account_id,
            reindexed = done,
            "rebuilt an account's local search index"
        );
    }
    Ok(done)
}

/// The account to open, if the store holds one.
///
/// Read straight off a connection rather than through [`MailStore`]: which
/// account to open is a question about *starting up*, not about drawing mail,
/// and this crate is the one place allowed to ask it directly. It is one
/// indexed read before the window is presented.
pub fn first_account(database: &Database) -> Option<postio_model::Account> {
    let connection = database
        .connection()
        .map_err(|error| tracing::error!(%error, "cannot read the accounts"))
        .ok()?;
    AccountRepository::new(&connection)
        .list_enabled()
        .map_err(|error| tracing::error!(%error, "cannot read the accounts"))
        .ok()?
        .into_iter()
        .next()
}
