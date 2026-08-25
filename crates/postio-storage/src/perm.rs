//! Directory and file permissions for the store's private data.
//!
//! The store holds mail — message metadata, the FTS5 index, raw message
//! blobs, attachments — and none of it should be readable by another local
//! user. On most distributions `$XDG_DATA_HOME` (`~/.local/share`) is `0755`,
//! so a directory created under it with the process umask (commonly `022`)
//! is world-traversable, and every file SQLite or the blob store writes
//! under it inherits the same exposure. `config.toml` is already chmodded
//! `0600` after writing; the actual mailbox was not.
//!
//! [`ensure_private_dir`] creates a missing directory `0700` directly, via
//! [`DirBuilderExt::mode`], rather than creating it with the default mode and
//! chmodding afterward — a chmod-after-create leaves a window where the
//! directory is briefly world-traversable, and a process racing to look
//! inside it does not need a large one. For a directory that already exists
//! — a store created before this existed, or one whose mode someone loosened
//! by hand — it repairs the permission instead, which is the only thing a
//! directory that already exists can be offered.

use std::path::Path;

use crate::error::{Error, Result};

#[cfg(unix)]
pub(crate) fn ensure_private_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    if path.exists() {
        return std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(
            |source| Error::Io {
                path: path.to_path_buf(),
                source,
            },
        );
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
pub(crate) fn ensure_private_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Tightens an existing file to `0600`. For a file SQLite or another library
/// already created at its own default mode, where there was no chance to
/// pass a mode at creation time.
#[cfg(unix)]
pub(crate) fn tighten_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
        Error::Io {
            path: path.to_path_buf(),
            source,
        }
    })
}

#[cfg(not(unix))]
pub(crate) fn tighten_file(_path: &Path) -> Result<()> {
    Ok(())
}

/// `OpenOptions` pre-armed to create a new file `0600`, so a blob or other
/// content this crate writes for the first time is never briefly at the
/// process umask the way a chmod-after-create would leave it.
#[cfg(unix)]
pub(crate) fn private_file_options() -> std::fs::OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = std::fs::OpenOptions::new();
    options.mode(0o600);
    options
}

#[cfg(not(unix))]
pub(crate) fn private_file_options() -> std::fs::OpenOptions {
    std::fs::OpenOptions::new()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn a_fresh_directory_is_created_private_with_no_looser_window() {
        let root = tempfile::tempdir().expect("a temp dir");
        let target = root.path().join("a").join("b");

        ensure_private_dir(&target).expect("create");

        assert!(target.is_dir());
        assert_eq!(mode_of(&target), 0o700);
        // The intermediate directory DirBuilder had to create along the way
        // is private too -- not just the leaf that was asked for.
        assert_eq!(mode_of(root.path().join("a").as_path()), 0o700);
    }

    #[test]
    fn a_looser_existing_directory_is_repaired() {
        let root = tempfile::tempdir().expect("a temp dir");
        let target = root.path().join("store");
        std::fs::create_dir(&target).expect("create");
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
            .expect("loosen it, the way a pre-fix store would be");

        ensure_private_dir(&target).expect("repair");

        assert_eq!(mode_of(&target), 0o700);
    }

    #[test]
    fn tightening_a_file_created_at_the_umask_ends_at_0600() {
        let root = tempfile::tempdir().expect("a temp dir");
        let path = root.path().join("postio.db");
        std::fs::write(&path, b"stand-in bytes").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("the mode a default umask would leave");

        tighten_file(&path).expect("tighten");

        assert_eq!(mode_of(&path), 0o600);
    }

    #[test]
    fn a_file_opened_through_private_file_options_is_never_loose() {
        let root = tempfile::tempdir().expect("a temp dir");
        let path = root.path().join("blob");

        private_file_options()
            .write(true)
            .create(true)
            .open(&path)
            .expect("open");

        assert_eq!(mode_of(&path), 0o600);
    }
}
