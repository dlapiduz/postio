//! Opening the local store, and the connections onto it.
//!
//! # One call
//!
//! [`Store::open`] creates the file and its parent directory, opens it under
//! the key, applies [`crate::schema::HEAD`] if the file is new, and hands back
//! a handle. Everything else in this crate takes a [`Connection`] borrowed
//! from it.
//!
//! # What replaced the pool
//!
//! There was a hand-written connection pool here, with a maximum, a checkout
//! path, an idle list and a `PooledConnection` guard that returned its
//! connection on drop. It is gone: the engine keeps its own pool behind
//! [`turso::Database::connect`], which is why that call is cheap, infallible
//! in practice, and not `async`. A `Connection` is `Clone`, `Send` and `Sync`,
//! and serialises its own operations internally — so the thing the old pool
//! was protecting is protected a layer down.
//!
//! What is *not* a layer down is [`WriteGate`], and that is a different
//! question: not "may two writes run at once" but "when a background sync and
//! a person's keystroke both want the writer, who goes first". See its
//! documentation.
//!
//! # Everything is async
//!
//! The engine is async to the bottom, so this crate is too. The old shape --
//! synchronous repositories reached from tokio through `spawn_blocking` --
//! is gone, and with it the thread pool that shape needed.

use std::path::{Path, PathBuf};

use turso::Builder;

use crate::error::{Error, Result};
use crate::key::Subkey;
use crate::schema;

pub use turso::{Connection, Value};

/// The cipher the store is written under.
///
/// AES-256-GCM: authenticated, and the one mode here with hardware support on
/// every CPU Postio targets. The engine also offers AEGIS, which is faster on
/// paper and much less deployed; this is the local mail store, not a
/// benchmark.
///
/// **Both the encryption and the engine are pre-1.0 and unaudited.** That is
/// why `store_is_unreadable_without_the_key` and `another_key_is_refused` are
/// acceptance criteria in `specs/004-turso-store/spec.md` and tests in this
/// crate, rather than properties taken on trust.
pub const CIPHER: &str = "aes256gcm";

/// Which write a permit is for.
///
/// The engine will happily interleave a background backfill and a keystroke.
/// This says which one should be waiting when they collide, and it exists
/// because "the UI never awaits the network" has a quieter cousin: the UI
/// never queues behind a sync either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WritePriority {
    /// A person is waiting for this. Archiving, flagging, sending, deleting.
    Interactive,
    /// Nobody is waiting for this. Sync, backfill, indexing, eviction.
    Background,
}

/// The store: a database handle and the path it came from.
///
/// Cheap to clone — the engine's handle is an `Arc` inside — so it is passed
/// by value rather than behind another layer of sharing.
#[derive(Clone)]
pub struct Store {
    database: turso::Database,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl Store {
    /// Open the store at `path`, creating it if it is not there.
    ///
    /// The parent directory is created too, with the same restricted
    /// permissions the store itself gets — a mail store is not world-readable
    /// even for the instant between `create` and `chmod`.
    pub async fn open(path: impl AsRef<Path>, key: &Subkey) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            crate::perm::ensure_private_dir(parent)?;
        }

        let fresh = !path.exists();
        let database = Self::build(path, key).await.map_err(|error| {
            if fresh { error } else { as_key_failure(error) }
        })?;
        let store = Self {
            database,
            path: Some(path.to_path_buf()),
        };

        if fresh {
            crate::perm::tighten_file(path)?;
            store.apply_schema().await?;
        } else {
            store.prove_the_key_fits().await?;
        }

        Ok(store)
    }

    async fn build(path: &Path, key: &Subkey) -> Result<turso::Database> {
        let hexkey = key.to_hex();
        let opts = turso::EncryptionOpts {
            cipher: CIPHER.to_string(),
            hexkey: hexkey.to_string(),
        };
        let built = Builder::new_local(&path.to_string_lossy())
            .experimental_encryption(true)
            .with_encryption(opts)
            // The schema uses all three, and each is behind its own flag in
            // this release. `index_method` is what `CREATE INDEX ... USING
            // fts` is; without it the search index is a syntax error.
            .experimental_triggers(true)
            .experimental_index_method(true)
            .experimental_generated_columns(true)
            .build()
            .await;
        drop(hexkey);
        built.map_err(Into::into)
    }

    /// Read one page under the key, so a key that does not fit is discovered
    /// here rather than in the middle of the first query a person asked for.
    ///
    /// The engine reports a failed page authentication in the vocabulary of
    /// corruption — it cannot tell a wrong key from a damaged file, because
    /// they present identically. Postio can: it only ever writes one key per
    /// store, so the overwhelmingly likelier of the two is the key, and
    /// [`Error::WrongStoreKey`] says so in a sentence that is actionable
    /// (#404).
    async fn prove_the_key_fits(&self) -> Result<()> {
        let connection = self.connect()?;
        match connection
            .query("SELECT count(*) FROM sqlite_schema", ())
            .await
        {
            Ok(_) => Ok(()),
            Err(error) => Err(as_key_failure(error.into())),
        }
    }

    async fn apply_schema(&self) -> Result<()> {
        let connection = self.connect()?;
        // Off while the batch runs: the schema declares tables alphabetically,
        // so a foreign key routinely names a table that does not exist yet.
        connection.execute("PRAGMA foreign_keys = OFF", ()).await?;
        connection.execute_batch(schema::HEAD).await?;
        connection.execute("PRAGMA foreign_keys = ON", ()).await?;
        Ok(())
    }

    /// A connection onto the store.
    ///
    /// Cheap: the engine pools these itself, and the returned handle is
    /// `Clone + Send + Sync`. Make one per unit of work rather than holding
    /// one open across awaits that do not touch the database.
    pub fn connect(&self) -> Result<Connection> {
        self.database.connect().map_err(Into::into)
    }

    /// Where the store lives, or `None` for one that is not on disk.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

/// Translate a failed page authentication into the sentence a person can act
/// on.
///
/// The engine decrypts page 1 to find out whether the file is a database at
/// all, so a wrong key and a damaged file arrive here as the same error:
/// "Decryption failed for page=1". They are not equally likely. Postio writes
/// exactly one key per store and takes it from the keyring, so the way this
/// happens in practice is a keyring entry that was replaced or belongs to
/// another installation -- and [`Error::WrongStoreKey`] says that, and says
/// the mail is intact, which the engine's own wording does not (#404).
///
/// Anything else passes through untouched: an unreadable file, a full disk
/// and a directory that is not writable are all still themselves.
fn as_key_failure(error: Error) -> Error {
    let said = error.to_string().to_lowercase();
    if said.contains("decryption failed") || said.contains("not a database") {
        Error::WrongStoreKey
    } else {
        error
    }
}
