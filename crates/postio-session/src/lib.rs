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
pub mod engine;
pub mod logging;
pub mod paths;
pub mod refresh;

use std::sync::Arc;

use postio_core::bridge::EventSink;
use postio_runtime::store::{MailStore, SqliteStore};
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
        ..postio_runtime::BackfillPolicy::default()
    }
}

/// The keyring entry the store's master key lives under.
///
/// Not an address, and it cannot become one: there is no `@` in it, so it
/// can never collide with an account's own entry however many accounts an
/// installation grows. Written out rather than derived from the store path
/// because the key has to be findable by a person in `seahorse` when they
/// want to know what Postio keeps — the label reads
/// "Postio (local store encryption key)".
pub const STORE_KEY_ENTRY: &str = "local store encryption key";

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
/// [`SecretError::Locked`]: postio_imap::secret::SecretError::Locked
pub async fn store_key(
    secrets: &dyn postio_imap::secret::SecretStore,
) -> Result<postio_storage::key::StoreKey, postio_imap::secret::SecretError> {
    use postio_imap::secret::{AccountKey, SecretError};
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
    secrets: &dyn postio_imap::secret::SecretStore,
    entry: &postio_imap::secret::AccountKey,
) -> Result<postio_storage::key::StoreKey, postio_imap::secret::SecretError> {
    let key = postio_storage::key::StoreKey::generate();
    // The one place the key becomes text. `to_hex` hands back a buffer that
    // overwrites itself, and `Password` keeps that discipline from here on.
    secrets
        .store(
            entry,
            &postio_imap::secret::Password::new(key.to_hex().as_str()),
        )
        .await?;
    // No key material, no length, nothing derived from it. That the store is
    // now encrypted is worth a line; what it is encrypted with is not.
    tracing::info!("this store has a new encryption key");
    Ok(key)
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
    /// Where account passwords live.
    ///
    /// A part rather than something the modules that need it construct, for
    /// the reason `engine.rs` gives about every other part: which keyring
    /// this installation uses is a choice about *this installation*, and a
    /// module that reaches for `KeyringSecretStore::default()` itself cannot
    /// be driven by a test without a Secret Service session. Both credential
    /// paths — the one onboarding writes and the one startup reads — hang
    /// off this.
    pub secrets: Arc<dyn postio_imap::secret::SecretStore>,
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
            database,
            blobs,
            runtime,
            events,
            commands,
            engine: refresh::EngineSlot::default(),
            secrets: Arc::new(postio_imap::secret::KeyringSecretStore::default()),
            mailbox_roles: postio_model::RoleOverrides::default(),
            backfill: postio_runtime::BackfillPolicy::default(),
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

    /// The same wiring, reading and writing passwords somewhere else.
    ///
    /// The seam a test needs: `MemorySecretStore` stands in for a keyring
    /// that has no D-Bus session behind it, and `MemorySecretStore::locked`
    /// for one nobody has unlocked.
    pub fn with_secrets(mut self, secrets: Arc<dyn postio_imap::secret::SecretStore>) -> Self {
        self.secrets = secrets;
        self
    }
}

/// Open the local store, or explain why there is none.
///
/// A missing or unreadable database is not a reason to refuse to start: the
/// window opens, says it has never synced, and stays usable for everything
/// that does not need mail. A mail client that will not open is worse than one
/// with nothing in it.
pub fn open_store() -> Option<(Database, BlobStore)> {
    let path = paths::store_path();
    let database = match Database::open(&path) {
        Ok(database) => database,
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "cannot open the store");
            return None;
        }
    };
    // Beside the database, not inside it: bodies and attachments are
    // content-addressed files, and SQLite holds the key and the metadata.
    let blobs = match BlobStore::open(path.with_file_name("blobs")) {
        Ok(blobs) => blobs,
        Err(error) => {
            tracing::error!(%error, "cannot open the blob store");
            return None;
        }
    };
    if let Err(error) = ensure_search_index(&database) {
        // Recoverable: everything except search still works, and refusing to
        // open a mail client because its index would not build would be a
        // worse answer than opening one you cannot search.
        tracing::error!(%error, "the search index is unavailable");
    }

    Some((database, blobs))
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
/// Message *bodies* are a separate matter: they live in the blob store, so no
/// trigger can reach them and nothing here can either — this function has no
/// blob store. `postio_sync::backfill::fetch_body` indexes each body as it
/// lands, and [`index_local_bodies`] catches up whatever landed before that
/// call existed (#327).
pub fn ensure_search_index(database: &Database) -> Result<(), Box<dyn std::error::Error>> {
    let connection = database.connection()?;
    postio_index::index::ensure_schema(&connection)?;
    tracing::debug!("the search index is ready");
    Ok(())
}

/// How many bodies one pass of [`index_local_bodies`] reads before letting go
/// of its connection.
///
/// The pass holds a pooled connection and reads a blob per message, and the
/// rest of the application shares that pool. Batching bounds how long any one
/// checkout lasts without making the pass itself stop early: it keeps taking
/// batches until there is nothing left.
const INDEX_BODY_BATCH: u32 = 200;

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
/// The first run over an existing archive reads a blob per message, which is
/// minutes of I/O on a large one. Callers must put this on a worker — the
/// application spawns it on the runtime after the window is up — because a
/// mail client that will not draw until its search index is warm has traded
/// the wrong thing for search.
///
/// Errors on one message are logged and skipped rather than abandoning the
/// pass: one unreadable blob should cost that message its body search, not
/// every message after it.
pub fn index_local_bodies(
    database: &Database,
    blobs: &BlobStore,
) -> Result<usize, Box<dyn std::error::Error>> {
    let mut indexed = 0usize;
    loop {
        let connection = database.connection()?;
        let candidates =
            postio_index::index::messages_missing_body_text(&connection, INDEX_BODY_BATCH)?;
        if candidates.is_empty() {
            break;
        }
        let messages = postio_storage::repository::MessageRepository::new(&connection);
        for id in &candidates {
            let message = postio_model::MessageId::new(*id);
            let body = match messages.body_blobs(message) {
                Ok(Some(stored)) => postio_model::MessageBody {
                    text: stored.text.and_then(|id| read_text(blobs, &id)),
                    html: stored.html.and_then(|id| read_text(blobs, &id)),
                },
                // The row says its body is local and it names no blobs. That
                // is a message that genuinely had none -- a header-only
                // notification, say -- and writing the empty string is how it
                // stops being asked about on every start.
                Ok(None) => postio_model::MessageBody::default(),
                Err(error) => {
                    tracing::debug!(message = id, %error, "cannot read a body to index");
                    continue;
                }
            };
            match postio_index::index::index_body_of(&connection, *id, &body) {
                Ok(()) => indexed += 1,
                Err(error) => tracing::debug!(message = id, %error, "cannot index a body"),
            }
        }
        let taken = candidates.len();
        drop(connection);
        if taken < INDEX_BODY_BATCH as usize {
            break;
        }
    }
    if indexed > 0 {
        // A count and nothing else: what a log may carry about mail.
        tracing::info!(indexed, "indexed bodies that were already local");
    }
    Ok(indexed)
}

/// One body blob as text, or nothing if it cannot be read or is not UTF-8.
fn read_text(blobs: &BlobStore, id: &postio_model::BlobId) -> Option<String> {
    String::from_utf8(blobs.get(id).ok()?).ok()
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
