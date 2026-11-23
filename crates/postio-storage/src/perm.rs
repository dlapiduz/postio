//! Permissions for the store's private data: `$XDG_DATA_HOME` is often `0755`, so nothing may be
//! created at the umask.
//! [`ensure_private_dir`] creates `0700` directly, since chmod-after-create leaves a
//! world-traversable window, and repairs an existing directory.

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

/// For a file a library already created at its own default mode.
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

/// Creates a new file `0600`, never briefly at the umask as chmod-after-create would.
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
