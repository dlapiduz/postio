//! The composition root without a toolkit: the local store, the tokio runtime, the sync engines,
//! logging, and the verb vocabulary in `actions`.
//! No toolkit is allowed here (`scripts/checks/check-crate-boundaries.py`), so a headless frontend
//! stays possible; a second binary writing the store directly would skip local-first ordering,
//! undo, events and the queue (ADR 0010).
//! Presenters that name a widget stay in the frontends.

pub mod actions;
pub mod agent_access;
pub mod attaching;
mod attachment_text;
pub mod auto_empty;
pub mod blocking;
pub mod checkup;
pub mod diag;
pub mod egress;
pub mod engine;
mod extraction;
pub mod focus;
pub mod handoff;
pub mod list_mail;
pub mod logging;
pub mod onboarding;
pub mod paths;
pub mod provision;
pub mod reachability;
pub mod reading;
pub mod refresh;
pub mod search;
pub mod signature;
pub mod signin;
pub mod start_over;
pub mod thread_compose;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use postio_core::bridge::EventSink;
use postio_runtime::store::{LocalStore, MailStore};
use postio_storage::blob::{EvictionReport, GarbageCollection, GarbageReport};
use postio_storage::repository::AccountRepository;
use postio_storage::{BlobStore, Store};

/// `[mailboxes]` from the file at `path`, or nothing. A file that does not load means no overrides;
/// `validate` reports the problem.
pub fn mailbox_roles_at(path: &std::path::Path) -> postio_model::RoleOverrides {
    postio_config::Config::load_from_path(path)
        .ok()
        .map(|config| config.role_overrides())
        .unwrap_or_default()
}

/// `[storage] max_bytes` from the file at `path`; unreadable, absent and unset all answer `None`,
/// which means unbounded. Read once at startup.
pub fn storage_ceiling_at(path: &std::path::Path) -> Option<u64> {
    postio_config::Config::load_from_path(path)
        .ok()
        .and_then(|config| config.storage.max_bytes)
}

/// Read `[sync]` from `path` and turn it into the policy the engine backfills under. Read once at
/// startup, so a change applies at the next start; a file that will not parse leaves the defaults.
pub fn backfill_policy_at(path: &std::path::Path) -> postio_runtime::BackfillPolicy {
    let sync = postio_config::Config::load_from_path(path)
        .ok()
        .map(|config| config.sync)
        .unwrap_or_default();
    backfill_policy(&sync)
}

/// Read `[sync]` from `path` and turn it into the policy the engine watches under (#932). Read once
/// at startup; a file that will not parse leaves the defaults.
pub fn watch_policy_at(path: &std::path::Path) -> postio_sync::WatchPolicy {
    let sync = postio_config::Config::load_from_path(path)
        .ok()
        .map(|config| config.sync)
        .unwrap_or_default();
    watch_policy(&sync)
}

/// `watch_policy_at`, for a `[sync]` section already in hand. `Manual` sets `WatchPolicy::manual`
/// (#1013): the watcher never idles or polls, though `Refresh` still reaches the server.
pub fn watch_policy(sync: &postio_config::SyncConfig) -> postio_sync::WatchPolicy {
    postio_sync::WatchPolicy {
        idle: matches!(sync.check_for_mail, postio_config::CheckForMail::Idle),
        poll_interval: std::time::Duration::from_secs(sync.poll_interval_secs),
        manual: matches!(sync.check_for_mail, postio_config::CheckForMail::Manual),
        ..postio_sync::WatchPolicy::default()
    }
}

pub fn backfill_policy(sync: &postio_config::SyncConfig) -> postio_runtime::BackfillPolicy {
    postio_runtime::BackfillPolicy {
        // `lazy` and `eager` differ in when, not whether, and there is one background lane.
        background: matches!(
            sync.body_fetch,
            postio_config::BodyFetch::Lazy | postio_config::BodyFetch::Eager
        ),
        attachments: match sync.attachment_fetch {
            postio_config::AttachmentFetch::OnOpen => postio_runtime::AttachmentPolicy::OnOpen,
            postio_config::AttachmentFetch::Eager => postio_runtime::AttachmentPolicy::Eager,
            postio_config::AttachmentFetch::Never => postio_runtime::AttachmentPolicy::Never,
        },
        // `0` in the file means no inline rule, which the policy spells `None`.
        max_inline_bytes: (sync.max_inline_bytes > 0).then_some(sync.max_inline_bytes),
        ..postio_runtime::BackfillPolicy::default()
    }
}

pub use attachment_text::{
    index_local_attachments, index_local_attachments_with, index_named_attachments,
    spawn_attachment_indexer,
};
pub use extraction::{Extraction, Extractor, HELPER_ENV, HELPER_GRACE, HELPER_NAME, Via};
pub use postio_storage::key::STORE_KEY_ENTRY;

/// The master key this installation's store is encrypted under, minting one on first run (ADR 0014
/// Q3).
///
/// Only a missing entry mints a key. A locked keyring, a timeout or a backend error never do: a key
/// minted then would encrypt new writes under something the existing store cannot open. An empty
/// entry counts as missing, since nothing can be encrypted under it.
/// There is no plaintext fallback: `SecretError::Locked` reaches the caller so it can ask the user
/// to unlock.
pub async fn store_key(
    secrets: &dyn postio_account::secret::SecretStore,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    use postio_account::secret::{AccountKey, SecretError};
    use postio_storage::key::StoreKey;

    let entry = AccountKey::new(STORE_KEY_ENTRY);
    match secrets.retrieve(&entry).await {
        Ok(stored) if !stored.is_empty() => {
            // Refused, not replaced: a replaced key makes the store unopenable for good.
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
        Err(error) => Err(error),
    }
}

/// Generates a key and stores it before anything is encrypted under it, or the store could never be
/// opened again.
async fn mint(
    secrets: &dyn postio_account::secret::SecretStore,
    entry: &postio_account::secret::AccountKey,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    let key = postio_storage::key::StoreKey::generate();
    // The one place the key becomes text; `to_hex` returns a buffer that overwrites itself.
    secrets
        .store(
            entry,
            &postio_account::secret::Password::new(key.to_hex().as_str()),
        )
        .await?;
    tracing::info!("this store has a new encryption key");
    Ok(key)
}

/// The refusal for a keyring that would not give up the store's key, in words for a person.
pub fn key_refusal(error: &postio_account::secret::SecretError) -> Refusal {
    use postio_account::secret::SecretError;
    Refusal::try_again(match error {
        SecretError::Locked { .. } => postio_ui::keyring_refusal::locked(),
        SecretError::Backend { reason, .. } => postio_ui::keyring_refusal::unreadable(reason),
        SecretError::Timeout { .. } => {
            postio_ui::keyring_refusal::unreadable("it did not answer in time")
        }
        other => postio_ui::keyring_refusal::unreadable(&other.to_string()),
    })
}

/// `store_key`, for a caller that has no runtime yet: startup needs the key before the bus and
/// runtime exist. `KeyringSecretStore` already turns a hung Secret Service into an error.
pub fn store_key_blocking(
    secrets: &dyn postio_account::secret::SecretStore,
) -> Result<postio_storage::key::StoreKey, postio_account::secret::SecretError> {
    // Through `blocking::now`, not a new runtime: a caller may already be on one (the FFI's
    // `Session`), and a runtime started inside a runtime panics.
    crate::blocking::now(store_key(secrets))
}

#[derive(Clone)]
pub struct Wiring {
    pub database: Store,
    pub blobs: BlobStore,
    pub store: Arc<dyn MailStore>,
    pub runtime: tokio::runtime::Handle,
    pub events: EventSink,
    pub commands: postio_core::bridge::CommandSender,
    pub engine: refresh::EngineSlot,
    /// Where every outbound connection is recorded (#151); one writer thread per process.
    pub egress: Arc<egress::EgressRecorder>,
    /// Where account passwords live. A part so tests need no Secret Service session.
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    /// Folders the user assigned a role to by hand, from `[mailboxes]`. Empty is the ordinary case.
    pub mailbox_roles: postio_model::RoleOverrides,
    /// How the engine backfills, from `[sync]`.
    pub backfill: postio_runtime::BackfillPolicy,
    /// How the engine watches for new mail, from `[sync]`.
    pub watch: postio_sync::WatchPolicy,
    /// The ceiling on the blob store, from `[storage] max_bytes`; `None` means unbounded.
    pub storage_ceiling: Option<u64>,
    /// How long Junk and Trash keep mail before it is purged, from `[sync]`. Off by default.
    pub auto_empty: postio_core::retention::AutoEmpty,
    pub gnupg_keyring: bool,
    /// The mail transport every account's engine uses instead of the one its settings name; `None`
    /// in the application. Lets tests drive the engine through a mock server.
    pub mail: Option<MailOverride>,
    /// Where a new account's servers are looked up: the network in the application, the provider
    /// table in tests.
    pub discovery: Arc<dyn postio_account::discovery::DiscoveryTransport>,
    /// The filing pass every engine hands incremental arrivals to. Empty until Focus mode fills it.
    pub filing: postio_runtime::FilingSlot,
    pub focus: focus::FocusSettings,
    /// What makes Download and attach's one request (spec 014 D7): the web,
    /// in the application. A part, like `discovery`, so a demo or a test
    /// answers without dialing.
    pub images: Arc<dyn attaching::Fetcher>,
}

#[derive(Clone, Debug)]
pub struct MailOverride {
    pub backend: Arc<dyn postio_account::backend::MailBackend>,
    pub smtp: Arc<dyn postio_smtp::transport::SmtpConnector>,
}

impl Wiring {
    /// Everything the panes need, over an already-open store. `runtime`, `events` and `commands`
    /// come from the app's `Bridge`; tests supply their own.
    pub fn new(
        database: Store,
        blobs: BlobStore,
        runtime: tokio::runtime::Handle,
        events: EventSink,
        commands: postio_core::bridge::CommandSender,
    ) -> Self {
        let egress = egress::EgressRecorder::start(database.clone());
        Wiring {
            store: Arc::new(LocalStore::new(&database)),
            discovery: Arc::new(
                postio_account::discovery::PimalayaTransport::new().with_egress(egress.clone()),
            ),
            egress,
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
            auto_empty: postio_core::retention::AutoEmpty::default(),
            gnupg_keyring: false,
            mail: None,
            filing: postio_runtime::FilingSlot::default(),
            focus: focus::FocusSettings::default(),
            images: Arc::new(attaching::HttpFetcher),
        }
    }

    /// The same wiring, with `[mailboxes]` applied. Read once at startup: the engine runs folder
    /// discovery inside its task, so a change applies at the next start.
    pub fn with_mailbox_roles(mut self, roles: postio_model::RoleOverrides) -> Self {
        self.mailbox_roles = roles;
        self
    }

    pub fn with_backfill(mut self, backfill: postio_runtime::BackfillPolicy) -> Self {
        self.backfill = backfill;
        self
    }

    pub fn with_watch(mut self, watch: postio_sync::WatchPolicy) -> Self {
        self.watch = watch;
        self
    }

    /// The same wiring, holding the blob store to `[storage] max_bytes`; `None` is unbounded.
    pub fn with_storage_ceiling(mut self, max_bytes: Option<u64>) -> Self {
        self.storage_ceiling = max_bytes;
        self
    }

    pub fn with_gnupg_keyring(mut self, on: bool) -> Self {
        self.gnupg_keyring = on;
        self
    }

    pub fn with_auto_empty(mut self, policy: postio_core::retention::AutoEmpty) -> Self {
        self.auto_empty = policy;
        self
    }

    pub fn with_secrets(mut self, secrets: Arc<dyn postio_account::secret::SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }

    pub fn with_mail(mut self, mail: MailOverride) -> Self {
        self.mail = Some(mail);
        self
    }

    pub fn with_discovery(
        mut self,
        discovery: Arc<dyn postio_account::discovery::DiscoveryTransport>,
    ) -> Self {
        self.discovery = discovery;
        self
    }
}

/// Open the local store, or say why there is none. `Err` carries a sentence for the user (#404);
/// there is no degraded mode since the store is encrypted (ADR 0014), so a locked keyring means no
/// store.
pub async fn open_store(
    store_key: &postio_storage::key::StoreKey,
) -> Result<(Store, BlobStore), String> {
    open_store_at(paths::store_path(), store_key).await
}

/// What `open_store_at_reporting` is doing right now: reading, migrating, or indexing, the waits
/// that can take tens of seconds on a real mailbox (#1114).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    Store,
    Migrating,
    Indexing,
}

/// Why the store did not open, and what would get past it: "Try again" is a dead end for a store
/// the same file will refuse every time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub sentence: String,
    pub remedy: Remedy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Remedy {
    TryAgain,
    /// A store this installation can never read, for the reason `because`; trying again meets the
    /// same file and key, so start over.
    StartOver {
        store: std::path::PathBuf,
        because: Unreadable,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unreadable {
    AnotherBuild,
    AnotherKey,
}

impl Refusal {
    pub fn try_again(sentence: impl Into<String>) -> Self {
        Refusal {
            sentence: sentence.into(),
            remedy: Remedy::TryAgain,
        }
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.sentence)
    }
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> String {
        refusal.sentence
    }
}

/// `open_store`, over a store at a path the caller chooses: tests need one per test, and the macOS
/// boundary (ADR 0019) opens before `paths` knows its location.
pub async fn open_store_at(
    path: impl Into<std::path::PathBuf>,
    store_key: &postio_storage::key::StoreKey,
) -> Result<(Store, BlobStore), String> {
    open_store_at_reporting(path, store_key, &|_| {})
        .await
        .map_err(String::from)
}

pub async fn open_store_at_reporting(
    path: impl Into<std::path::PathBuf>,
    store_key: &postio_storage::key::StoreKey,
    report: &dyn Fn(Opening),
) -> Result<(Store, BlobStore), Refusal> {
    // The database subkey, BLAKE3-derived from the master key (ADR 0014 Q3).
    let database_key = store_key.derive(postio_storage::key::Purpose::Database);
    let path = path.into();

    // A store an earlier build wrote is migrated in place inside `open`, saying so first; one no
    // step reaches is refused with `Remedy::StartOver`.
    report(Opening::Store);
    let database = match Store::open_reporting(&path, &database_key, || report(Opening::Migrating))
        .await
    {
        Ok(database) => database,
        // A wrong key gets its own sentence: SQLite's "file is not a database" would tell someone
        // their mail is corrupt. The error carries its own opening, so nothing goes in front of it.
        // The key does not change between attempts, so retrying cannot help (#1598).
        Err(error @ postio_storage::Error::WrongStoreKey) => {
            tracing::error!(path = %path.display(), "the store will not decrypt with this key");
            return Err(Refusal {
                sentence: error.to_string(),
                remedy: Remedy::StartOver {
                    store: path,
                    because: Unreadable::AnotherKey,
                },
            });
        }
        Err(error @ postio_storage::Error::InUse) => {
            tracing::warn!(path = %path.display(), "the store is open in another process");
            return Err(Refusal::try_again(error.to_string()));
        }
        Err(error @ postio_storage::Error::SchemaFromAnotherBuild { found, expected }) => {
            tracing::error!(path = %path.display(), found, expected, "the store's schema cannot be carried forward");
            return Err(Refusal {
                sentence: error.to_string(),
                remedy: Remedy::StartOver {
                    store: path,
                    because: Unreadable::AnotherBuild,
                },
            });
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "cannot open the store: {error}");
            // The sentence goes to the caller as well as the log, because the caller puts it on
            // screen (#404).
            return Err(Refusal::try_again(format!(
                "Postio could not open its local store: {error}"
            )));
        }
    };
    let blobs = match BlobStore::open(
        path.with_file_name("blobs"),
        &postio_storage::key::BlobKeys::derive(store_key),
    ) {
        Ok(blobs) => blobs,
        Err(error) => {
            tracing::error!(%error, "cannot open the blob store: {error}");
            return Err(Refusal::try_again(format!(
                "Postio could not open the store that holds message bodies \
                 and attachments: {error}"
            )));
        }
    };
    report(Opening::Indexing);
    if let Err(error) = ensure_search_index(&database).await {
        // Recoverable: everything except search still works.
        tracing::error!(%error, "the search index is unavailable: {error}");
    }

    Ok((database, blobs))
}

const SESSION_STATE_KEY: &str = "session_state";

/// Record that a session started, and answer whether the last one crashed: `true` when the previous
/// session never reached `end_session` (#491).
///
/// `DraftState::Editing` alone is not evidence of a crash: `Esc` parks a draft there on purpose.
/// Call once per process, before anything consults the answer; a second call would report its own
/// session as a crash.
pub async fn begin_session(database: &Store) -> bool {
    let Ok(connection) = database.connect().await else {
        return false;
    };
    let settings = postio_storage::repository::SettingsRepository::new(&connection);
    let unclean =
        matches!(settings.get(SESSION_STATE_KEY).await, Ok(Some(state)) if state == "open");
    if let Err(error) = settings.set(SESSION_STATE_KEY, "open").await {
        tracing::warn!(%error, "could not record the session start");
    }
    unclean
}

/// Record a clean shutdown, the other half of `begin_session`.
pub async fn end_session(database: &Store) {
    let Ok(connection) = database.connect().await else {
        return;
    };
    let settings = postio_storage::repository::SettingsRepository::new(&connection);
    if let Err(error) = settings.set(SESSION_STATE_KEY, "closed").await {
        tracing::warn!(%error, "could not record the clean shutdown");
    }
}

/// Create the full-text index if it is not there, on every start. Metadata columns are then
/// maintained by trigger.
///
/// Bodies are compressed columns (ADR 0020) that no trigger can reach; `fetch_body` indexes each as
/// it lands and `index_local_bodies` catches up the rest (#327).
pub async fn ensure_search_index(database: &Store) -> Result<(), Box<dyn std::error::Error>> {
    let connection = database.connect().await?;
    postio_index::index::ensure_schema(&connection).await?;
    tracing::debug!("the search index is ready");
    Ok(())
}

/// How long a blob must have gone untouched before a sweep may call it garbage. Load-bearing: a
/// blob is written before its row commits, so with no grace period a sweep deletes the body of a
/// message mid-fetch. Tests pass `ZERO`; nothing else may.
pub const BLOB_GRACE_PERIOD: Duration = Duration::from_secs(60 * 60);

/// How long a dragged-out export must have sat untouched before a sweep may reclaim it. The file
/// transfer portal hands a receiver a path and never copies the bytes, so deleting early is a
/// silent no-op (#121); it also makes the sweep safe with several instances running.
pub const DRAG_EXPORT_GRACE_PERIOD: Duration = Duration::from_secs(60 * 60);

/// Delete dragged-out exports nothing is using any more; returns how many (#278).
///
/// `older_than` is the whole safety property, so it is a parameter: a drop's receiver reads the
/// path after the drop, and emptying the directory would silently break it. An entry whose age
/// cannot be read is left alone.
pub fn reclaim_drag_exports(
    directory: &Path,
    older_than: Duration,
) -> Result<usize, Box<dyn std::error::Error>> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };

    let now = SystemTime::now();
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        // Unreadable metadata or a future mtime: an undatable entry is left alone.
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
        let gone = if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match gone {
            Ok(()) => removed += 1,
            Err(error) => {
                tracing::debug!(%error, path = %path.display(), "could not reclaim a drag export");
            }
        }
    }
    Ok(removed)
}

/// Delete blobs the database no longer references; returns what went (#416).
///
/// `MessageRepository::delete` deliberately leaves blobs to this sweep. It removes only what
/// nothing points at (`referenced_blobs` covers every blob-key column), unlike
/// `BlobStore::evict_to_fit`. It walks the blob directory, so callers put it on a worker, not the
/// startup path.
pub async fn reclaim_orphaned_blobs(
    database: &Store,
    blobs: &BlobStore,
    min_age: Duration,
) -> Result<GarbageReport, Box<dyn std::error::Error>> {
    let connection = database.connect_background().await?;
    let report = blobs
        .collect_garbage(&connection, GarbageCollection { min_age })
        .await?;
    if report.removed > 0 {
        tracing::info!(
            removed = report.removed,
            bytes = report.bytes_reclaimed,
            scanned = report.scanned,
            "reclaimed blobs nothing references"
        );
    }
    Ok(report)
}

/// Bring the blob store back under `[storage] max_bytes` by dropping what can be fetched again;
/// returns what one pass took, or `None` when no ceiling is set (#862).
///
/// `None` returns before anything is counted, so unset is never read as a budget of zero. Unlike
/// `reclaim_orphaned_blobs` this removes blobs a row still points at, safe because the store is a
/// cache (ADR 0014): only raw source and attachment payloads, oldest first, never body text (ADR
/// 0020). It stats every blob, so keep it off the startup path.
pub async fn enforce_storage_ceiling(
    database: &Store,
    blobs: &BlobStore,
    max_bytes: Option<u64>,
) -> Result<Option<EvictionReport>, Box<dyn std::error::Error>> {
    let Some(budget) = max_bytes else {
        return Ok(None);
    };
    let connection = database.connect_background().await?;
    let report = blobs.evict_to_fit(&connection, budget).await?;
    if report.removed > 0 {
        tracing::info!(
            removed = report.removed,
            bytes = report.bytes_reclaimed,
            remaining = report.bytes_remaining,
            budget,
            "evicted refetchable blobs to fit the storage ceiling"
        );
    }
    if report.bytes_remaining > budget {
        // Eviction ran out of things it may take: the ceiling is not being honoured and no further
        // pass will change that.
        tracing::warn!(
            remaining = report.bytes_remaining,
            budget,
            "the store is over its ceiling with nothing refetchable left to drop"
        );
    }
    Ok(Some(report))
}

const REPAIR_HEADERS_BATCH: u32 = 256;

/// Rebuild the header blocks of mail downloaded before there was anywhere to put them; returns how
/// many.
///
/// Only messages that still have their raw source, so it reaches no network; messages without a
/// blob need a fetch and are kept out of these batches, since the pass is windowed newest-first and
/// unfetchable rows would stall it.
/// A blob that will not read is skipped, not written as an empty block, which would claim the
/// message has no such header. Keep it off the startup path.
pub async fn repair_header_blocks(
    database: &Store,
    blobs: &BlobStore,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut repaired = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connect_background().await?;
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let candidates = messages
            .messages_missing_headers(REPAIR_HEADERS_BATCH)
            .await?;
        if candidates.is_empty() {
            break;
        }
        let batch: Vec<i64> = candidates
            .iter()
            .map(|candidate| candidate.message_id.get())
            .collect();
        // The candidate query's contract is that repairing a message removes it from the answer; an
        // identical batch means it is broken and another lap would spin (#500).
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
                continue;
            };
            let Some(block) = postio_model::headers::block_of(&raw) else {
                continue;
            };
            messages
                .set_headers(candidate.message_id, Some(&block))
                .await?;
            repaired += 1;
        }
    }

    if repaired > 0 {
        // A count only: header values carry `Received` chains with addresses and hostnames.
        tracing::info!(repaired, "rebuilt header blocks from raw source on disk");
    }
    Ok(repaired)
}

/// Delete leftover `.part` files from fetches that never finished (a power cut, OOM kill or crash);
/// returns how many. One `read_dir`, so it can run at startup.
pub fn purge_fetch_debris(blobs: &BlobStore) -> Result<usize, Box<dyn std::error::Error>> {
    let purged = blobs.purge_temporary()?;
    if purged > 0 {
        tracing::info!(purged, "removed debris from fetches that never finished");
    }
    Ok(purged)
}

/// How long a settled operation (done, or failed for good) stays in the queue before housekeeping
/// removes it: long enough for a late undo.
pub const OPERATION_RETENTION: chrono::Duration = chrono::Duration::days(30);

/// Remove every account's settled operations older than `retention`; returns how many.
/// Housekeeping, off the startup and interaction paths.
pub async fn prune_settled_operations(
    database: &Store,
    retention: chrono::Duration,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let connection = database.connect_background().await?;
    let before = now - retention;
    let accounts = postio_storage::repository::AccountRepository::new(&connection)
        .list()
        .await?;
    let _permit = connection
        .write_gate()
        .acquire(postio_storage::WritePriority::Background)
        .await;
    let queue = postio_storage::repository::OperationQueueRepository::new(&connection);
    let mut removed = 0;
    for account in accounts {
        removed += queue.prune_settled(account.id, before).await?;
    }
    Ok(removed)
}

const INDEX_BODY_DEBOUNCE: Duration = Duration::from_millis(500);

/// The body indexer: every stored body reaches the search index through here, in batches, off the
/// sync lane.
///
/// Writing a body's full-text row inline updated the tantivy index on the syncing lane, where a
/// segment merge stalled every folder behind it. This task drains `messages_missing_body_text`: a
/// catch-up pass at start, then a pass after each burst of `BodyLoaded` events. It ends when the
/// event hub does; with `events` `None` only the catch-up runs.
pub fn spawn_body_indexer(
    database: Store,
    events: Option<postio_core::bridge::EventStream>,
    runtime: &tokio::runtime::Handle,
) -> tokio::task::JoinHandle<()> {
    runtime.spawn(async move {
        body_index_pass(&database).await;
        let Some(events) = events else {
            return;
        };
        loop {
            let Some(event) = events.next().await else {
                return;
            };
            let mut pending = Vec::new();
            note_body(&mut pending, &event);
            // Let the burst finish and drain its events, keeping the named bodies so the pass
            // indexes those instead of walking the recency index (640 ms per message on a
            // 61,000-message store, #1549).
            let quiet = tokio::time::sleep(INDEX_BODY_DEBOUNCE);
            tokio::pin!(quiet);
            loop {
                tokio::select! {
                    () = &mut quiet => break,
                    next = events.next() => {
                        match next {
                            None => return,
                            Some(event) => note_body(&mut pending, &event),
                        }
                    }
                }
            }
            if pending.is_empty() {
                continue;
            }
            pending.sort_unstable();
            pending.dedup();
            if let Err(error) = index_named_bodies(&database, &pending).await {
                tracing::warn!(%error, "indexing the bodies a burst named failed: {error}");
            }
        }
    })
}

fn note_body(pending: &mut Vec<i64>, event: &postio_core::Event) {
    if let postio_core::Event::BodyLoaded { message, .. } = event {
        pending.push(message.get());
    }
}

/// One pass of the indexer. It cannot fail the task: a store whose search schema was never created
/// is a real state.
async fn body_index_pass(database: &Store) {
    if let Err(error) = index_local_bodies(database).await {
        tracing::warn!(%error, "the body indexer's pass failed: {error}");
    }
}

/// How many bodies one pass reads before letting go of its connection, bounding a checkout on a
/// shared pool.
const INDEX_BODY_BATCH: u32 = 200;

/// How long `index_local_bodies` pauses between batches, so the disk keeps answering searches
/// during a large catch-up.
const INDEX_BODY_BREATHER: Duration = Duration::from_millis(25);

/// Read the bodies of `ids` and write their index rows, in two phases; returns how many were
/// indexed. A message expunged or unreadable costs itself, not the batch.
async fn index_one_batch(
    connection: &postio_storage::Checkout,
    ids: &[i64],
) -> Result<usize, Box<dyn std::error::Error>> {
    // Read first, write after: the reads decompress and must not hold the write lock.
    // One repository for the batch so the compression dictionary loads once.
    let messages = postio_storage::repository::MessageRepository::new(connection);
    let mut bodies: Vec<(i64, postio_model::MessageBody)> = Vec::with_capacity(ids.len());
    for id in ids {
        let message = postio_model::MessageId::new(*id);
        let body = match messages.body(message).await {
            Ok(Some(stored)) => postio_model::MessageBody {
                text: stored.text,
                html: stored.html,
            },
            Ok(None) => postio_model::MessageBody::default(),
            Err(error) => {
                tracing::debug!(message = id, %error, "cannot read a body to index");
                continue;
            }
        };
        bodies.push((*id, body));
    }

    // One gated transaction per batch, from the background lane: an autocommit per message ran
    // ungated writes against whatever the user was doing.
    let mut indexed = 0usize;
    let _permit = connection
        .write_gate()
        .acquire(postio_storage::WritePriority::Background)
        .await;
    connection.execute_batch("BEGIN IMMEDIATE").await?;
    for (id, body) in &bodies {
        match postio_index::index::index_body_of(connection, *id, body).await {
            Ok(()) => indexed += 1,
            Err(error) => tracing::debug!(message = id, %error, "cannot index a body"),
        }
    }
    connection.execute_batch("COMMIT").await?;
    Ok(indexed)
}

/// Index exactly these messages, named by `BodyLoaded` events, without asking which need indexing
/// (#1549): that walk was 640 ms on a 61,000-message store, paid after every burst. The start-up
/// sweep remains the backstop for bodies stored before this existed or whose event nobody heard.
pub async fn index_named_bodies(
    database: &Store,
    ids: &[i64],
) -> Result<usize, Box<dyn std::error::Error>> {
    if ids.is_empty() {
        return Ok(0);
    }
    let started = std::time::Instant::now();
    let mut indexed = 0usize;
    for chunk in ids.chunks(INDEX_BODY_BATCH as usize) {
        let connection = database.connect_background().await?;
        indexed += index_one_batch(&connection, chunk).await?;
        drop(connection);
        if chunk.len() == INDEX_BODY_BATCH as usize {
            tokio::time::sleep(INDEX_BODY_BREATHER).await;
        }
    }
    if indexed > 0 {
        tracing::info!(
            indexed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "indexed bodies"
        );
    }
    Ok(indexed)
}

/// Index every message whose body is already on this machine and whose indexed text is empty;
/// returns how many (#327).
///
/// It covers mail already local when `fetch_body` began indexing and bodies whose index write was
/// lost. A caught-up store costs one query, and re-indexing overwrites one row, so it is safe on
/// every start. Minutes of work on a large archive: spawn it after the window is up. An error on
/// one message is logged and skipped.
pub async fn index_local_bodies(database: &Store) -> Result<usize, Box<dyn std::error::Error>> {
    let started = std::time::Instant::now();
    let mut indexed = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    // Carried only within a sweep: a later sweep starts at the top, so a body that landed older
    // than this cursor is not stranded (#1549).
    let mut cursor: Option<postio_index::index::Candidate> = None;
    loop {
        let connection = database.connect_background().await?;
        let found =
            postio_index::index::messages_missing_body_text(&connection, INDEX_BODY_BATCH, cursor)
                .await?;
        if found.is_empty() {
            break;
        }
        cursor = found.last().copied();
        let candidates: Vec<i64> = found.iter().map(|row| row.id).collect();
        // The candidate query's contract is that indexing a message removes it from the answer. An
        // identical batch means it is broken and another lap would spin (#500). Kept although the
        // cursor guarantees progress: it asserts the index's contract.
        if candidates == last_batch {
            tracing::warn!(
                batch = candidates.len(),
                "a body-index batch made no progress; stopping the pass"
            );
            break;
        }

        indexed += index_one_batch(&connection, &candidates).await?;

        let taken = candidates.len();
        last_batch = candidates;
        drop(connection);
        if taken < INDEX_BODY_BATCH as usize {
            break;
        }
        // Let go of the machine between batches, or the search this index serves pays in evicted
        // cache and queued reads (#500).
        tokio::time::sleep(INDEX_BODY_BREATHER).await;
    }
    if indexed > 0 {
        tracing::info!(
            indexed,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "indexed bodies"
        );
    }
    Ok(indexed)
}

const INDEX_HEADERS_BATCH: u32 = 200;

const INDEX_HEADERS_BREATHER: Duration = Duration::from_millis(25);

/// Index the headers of every message whose block is stored and whose header rows are missing;
/// returns how many.
///
/// `header:` matches `message_headers`, derived from `messages.body_headers` (ADR 0025 Q2); this
/// covers mail from before #884 and every message when the headers schema half is bumped. It
/// reaches no network: NULL blocks belong to `repair_header_blocks` and `Want::HeaderBlock`, kept
/// apart because unfixable rows would stall this newest-first window.
/// Spawn it as `index_local_bodies` is. An error on one message is logged and skipped.
pub async fn index_local_headers(database: &Store) -> Result<usize, Box<dyn std::error::Error>> {
    let mut indexed = 0usize;
    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connect_background().await?;
        let candidates =
            postio_index::index::messages_missing_header_rows(&connection, INDEX_HEADERS_BATCH)
                .await?;
        if candidates.is_empty() {
            break;
        }
        // The candidate query's contract is that indexing removes it from the answer; an identical
        // batch means it is broken and another lap would spin (#500).
        if candidates == last_batch {
            tracing::warn!(
                batch = candidates.len(),
                "a header-index batch made no progress; stopping the pass"
            );
            break;
        }

        // Read first, write after (see `index_local_bodies`); one repository so the dictionary
        // loads once.
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        let mut blocks: Vec<(i64, postio_model::Headers)> = Vec::with_capacity(candidates.len());
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            match messages.headers(message).await {
                Ok(Some(headers)) => blocks.push((*id, headers)),
                Ok(None) => {}
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a header block to index")
                }
            }
        }

        {
            let _permit = connection
                .write_gate()
                .acquire(postio_storage::WritePriority::Background)
                .await;
            connection.execute_batch("BEGIN IMMEDIATE").await?;
            for (id, headers) in &blocks {
                match postio_index::index::index_headers(&connection, *id, headers).await {
                    Ok(()) => indexed += 1,
                    Err(error) => {
                        tracing::debug!(message = id, %error, "cannot index a header block")
                    }
                }
            }
            connection.execute_batch("COMMIT").await?;
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
        // A count only: header values carry `Received` chains with addresses and hostnames.
        tracing::info!(indexed, "indexed header blocks that were already local");
    }
    Ok(indexed)
}

const REINDEX_ACCOUNT_BATCH: u32 = 200;

const REINDEX_ACCOUNT_BREATHER: Duration = Duration::from_millis(25);

/// Drops `account`'s rows from the local search index and rebuilds them from mail already on this
/// machine, bodies then header blocks; returns how many rows it rewrote (#981).
///
/// It reads only what is local, so it reaches no network. `on_progress(done, total)` is called once
/// with `total` fixed at the candidate count after clearing, then after every batch of either half.
/// An error on one message is logged and skipped.
pub async fn reindex_account(
    database: &Store,
    account: postio_model::ids::AccountId,
    mut on_progress: impl FnMut(u32, u32),
) -> Result<usize, Box<dyn std::error::Error>> {
    let account_id = account.get();
    let connection = database.connect_background().await?;
    postio_index::index::clear_account_body_index(&connection, account_id).await?;
    postio_index::index::clear_account_header_index(&connection, account_id).await?;
    let total = postio_index::index::messages_missing_body_text_for_account(
        &connection,
        account_id,
        u32::MAX,
    )
    .await?
    .len()
        + postio_index::index::messages_missing_header_rows_for_account(
            &connection,
            account_id,
            u32::MAX,
        )
        .await?
        .len();
    drop(connection);

    let mut done = 0usize;
    on_progress(0, total as u32);

    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connect_background().await?;
        let candidates = postio_index::index::messages_missing_body_text_for_account(
            &connection,
            account_id,
            REINDEX_ACCOUNT_BATCH,
        )
        .await?;
        if candidates.is_empty() {
            break;
        }
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
            let body = match messages.body(message).await {
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
                .acquire(postio_storage::WritePriority::Background)
                .await;
            connection.execute_batch("BEGIN IMMEDIATE").await?;
            for (id, body) in &bodies {
                match postio_index::index::index_body_of(&connection, *id, body).await {
                    Ok(()) => done += 1,
                    Err(error) => tracing::debug!(message = id, %error, "cannot reindex a body"),
                }
            }
            connection.execute_batch("COMMIT").await?;
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

    let mut last_batch: Vec<i64> = Vec::new();
    loop {
        let connection = database.connect_background().await?;
        let candidates = postio_index::index::messages_missing_header_rows_for_account(
            &connection,
            account_id,
            REINDEX_ACCOUNT_BATCH,
        )
        .await?;
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
            match messages.headers(message).await {
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
                .acquire(postio_storage::WritePriority::Background)
                .await;
            connection.execute_batch("BEGIN IMMEDIATE").await?;
            for (id, headers) in &blocks {
                match postio_index::index::index_headers(&connection, *id, headers).await {
                    Ok(()) => done += 1,
                    Err(error) => {
                        tracing::debug!(message = id, %error, "cannot reindex a header block")
                    }
                }
            }
            connection.execute_batch("COMMIT").await?;
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

    // `total` is an estimate, so the final call must say done rather than "94 of 100"; skipped when
    // there was nothing to do, since the first report already said so.
    if total > 0 || done > 0 {
        on_progress(done as u32, done.max(total) as u32);
    }
    if done > 0 {
        tracing::info!(
            account = account_id,
            reindexed = done,
            "rebuilt an account's local search index"
        );
    }
    Ok(done)
}

/// The account to open, if the store holds one; one indexed read before the window is presented.
pub async fn first_account(database: &Store) -> Option<postio_model::Account> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::error!(%error, "cannot read the accounts: {error}"))
        .ok()?;
    AccountRepository::new(&connection)
        .list_enabled()
        .await
        .map_err(|error| tracing::error!(%error, "cannot read the accounts: {error}"))
        .ok()?
        .into_iter()
        .next()
}
