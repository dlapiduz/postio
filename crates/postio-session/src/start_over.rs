//! Starting a store over: what a store this build cannot carry forward
//! leaves a person with, done deliberately and recoverably.
//!
//! A store stamped with a schema no migration leads from
//! (`postio_storage::schema::MIGRATIONS`) is refused, and retrying meets the
//! same file. The way forward is a fresh store that syncs the mail again --
//! and this is that, in the one shape that loses nothing a person cannot
//! get back:
//!
//! * **set aside, not deleted.** The database, its sidecar files and the blob
//!   store beside it move, together, into `set-aside/<when>/` next to where
//!   they were. The keyring entry they are encrypted under stays, so moving
//!   them back under a build of their schema opens them again.
//! * **the accounts come across.** A fresh store with nothing in it would be a
//!   first run, asking for every account again; instead their settings,
//!   identities and signatures are copied over (the passwords never moved:
//!   they are in the keyring under each address), and the first sync starts
//!   from them.
//! * **`config.toml` is not touched.** It lives in the config directory, not
//!   here, and none of this reaches it.
//!
//! What stays behind is the state that exists only in the store: snoozes,
//! reminders, Focus's filing history, digests, and drafts and changes the
//! server has not heard about yet. The screen that offers this says so
//! before the person chooses it.
//!
//! Two callers, and they do the same thing: Focus's "Start a fresh store",
//! and `postio-store reset` for a store no window will open
//! (`scripts/run-isolated.sh --reset-store` runs it on the scratch store).

use std::path::{Path, PathBuf};

use postio_storage::Store;
use postio_storage::key::{Purpose, StoreKey};

/// Where a store that was started over went, and what came across.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedOver {
    /// The directory the old database, its sidecars and its blobs are in.
    pub set_aside: PathBuf,
    /// How many accounts the fresh store carried over from it.
    pub accounts: usize,
}

/// Set the store at `path` aside and start a fresh one there, carrying its
/// accounts across. `Err` is a sentence for a person.
///
/// Refused while another Postio has the store open: moving a file a running
/// process is writing would leave that process writing into the copy that
/// was set aside.
pub async fn start_over_at(path: &Path, store_key: &StoreKey) -> Result<StartedOver, String> {
    let database_key = store_key.derive(Purpose::Database);
    if let Err(error @ postio_storage::Error::InUse) =
        Store::refuse_if_in_use(path, &database_key).await
    {
        return Err(error.to_string());
    }

    let set_aside = set_aside(path).map_err(|error| {
        tracing::error!(%error, "the store could not be set aside: {error}");
        format!("Postio could not set the old store aside, and left it where it was: {error}")
    })?;
    tracing::info!(set_aside = %set_aside.display(), "the store was set aside");

    let fresh = Store::open(path, &database_key).await.map_err(|error| {
        tracing::error!(%error, "a fresh store could not be made: {error}");
        format!("Postio set the old store aside but could not start a fresh one: {error}")
    })?;
    let earlier = set_aside.join(path.file_name().unwrap_or_default());
    // Failing to carry the accounts is not failing to start over: the fresh
    // store is there, and opens as a first run asking for them.
    let accounts = match fresh.carry_accounts_from(&earlier, &database_key).await {
        Ok(accounts) => accounts,
        Err(error) => {
            tracing::warn!(%error, "the accounts did not come across to the fresh store: {error}");
            0
        }
    };
    tracing::info!(accounts, "a fresh store was started");
    Ok(StartedOver {
        set_aside,
        accounts,
    })
}

/// [`start_over_at`] for this installation's store.
pub async fn start_over(store_key: &StoreKey) -> Result<StartedOver, String> {
    start_over_at(&crate::paths::store_path(), store_key).await
}

/// Move the database at `path`, every sidecar named after it and the blob
/// store beside it into a new `set-aside/<when>/` beside them, and answer
/// that directory. Renames, so nothing is copied and nothing is half-moved
/// on one filesystem.
fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("the store path names no file"))?
        .to_string_lossy()
        .into_owned();
    let when = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let base = parent.join("set-aside");
    let mut into = base.join(&when);
    let mut again = 1;
    while into.exists() {
        again += 1;
        into = base.join(format!("{when}-{again}"));
    }
    std::fs::create_dir_all(&into)?;

    let blobs = path.with_file_name("blobs");
    let mut moving: Vec<PathBuf> = std::fs::read_dir(parent)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|entry| {
            entry.file_name().is_some_and(|file| {
                let file = file.to_string_lossy();
                file == name || file.starts_with(&format!("{name}-"))
            })
        })
        .collect();
    if blobs.exists() {
        moving.push(blobs);
    }
    for from in moving {
        let to = into.join(from.file_name().unwrap_or_default());
        std::fs::rename(&from, &to)?;
    }
    Ok(into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_database_its_sidecars_and_its_blobs_move_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postio.db");
        std::fs::write(&path, b"db").unwrap();
        std::fs::write(dir.path().join("postio.db-wal"), b"wal").unwrap();
        std::fs::create_dir(dir.path().join("blobs")).unwrap();
        std::fs::write(dir.path().join("blobs").join("ab"), b"blob").unwrap();
        // Not the store's: left where it was.
        std::fs::write(dir.path().join("notes.txt"), b"mine").unwrap();

        let into = set_aside(&path).unwrap();

        assert!(into.starts_with(dir.path().join("set-aside")));
        assert_eq!(std::fs::read(into.join("postio.db")).unwrap(), b"db");
        assert_eq!(std::fs::read(into.join("postio.db-wal")).unwrap(), b"wal");
        assert_eq!(
            std::fs::read(into.join("blobs").join("ab")).unwrap(),
            b"blob"
        );
        assert!(!path.exists() && !dir.path().join("blobs").exists());
        assert!(dir.path().join("notes.txt").exists());
    }

    #[test]
    fn a_second_start_over_in_the_same_second_does_not_overwrite_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postio.db");
        std::fs::write(&path, b"first").unwrap();
        let first = set_aside(&path).unwrap();
        std::fs::write(&path, b"second").unwrap();
        let second = set_aside(&path).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first.join("postio.db")).unwrap(), b"first");
        assert_eq!(std::fs::read(second.join("postio.db")).unwrap(), b"second");
    }
}
