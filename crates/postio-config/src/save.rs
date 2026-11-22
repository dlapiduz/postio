//! A contract with `watch`: the watcher sees a create-or-modify of a scratch file followed by a
//! rename,
//! so an in-place write would be invisible to the running app. Both frontends save through here.

use std::path::Path;

/// The watcher is built to see exactly this shape of save, so the panel's edits reach the app the
/// way
/// `$EDITOR`'s do.
pub fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomically_replaces_the_file_and_cleans_up_the_scratch_file() {
        let dir = std::env::temp_dir().join(format!(
            "postio-settings-write-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        write_atomically(&path, "[ui]\ndensity = \"compact\"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[ui]\ndensity = \"compact\"\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name() != "config.toml")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        write_atomically(&path, "[ui]\ndensity = \"comfortable\"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[ui]\ndensity = \"comfortable\"\n"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomically_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "postio-settings-write-parent-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = dir.join("nested").join("config.toml");

        write_atomically(&path, "[ui]\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[ui]\n");

        std::fs::remove_dir_all(&dir).ok();
    }
}
