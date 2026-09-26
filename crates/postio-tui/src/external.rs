//! Handing the draft's body to the person's own editor (FR-022).
//!
//! The Markdown goes into a file only this user can read, in a directory only
//! this user can list, the editor runs on the real terminal, and whatever it
//! saved comes back. The file is removed whatever happened: a draft is not
//! left lying in a temporary directory.

use std::io;
use std::path::{Path, PathBuf};

/// The editor to run: `$VISUAL`, else `$EDITOR`, else `vi`, as a shell
/// command, so `code -w` works as well as `vim`.
pub fn editor() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vi".to_owned())
}

/// Where the file goes: Postio's directory under `$XDG_RUNTIME_DIR`, which is
/// this user's alone and cleared at logout; the system's temporary directory
/// otherwise.
pub fn directory() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join("postio")
}

/// Run `editor` on `markdown` in a private file under `directory`, and
/// answer what it saved.
pub fn edit(markdown: &str, directory: &Path, editor: &str) -> io::Result<String> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    // `create` leaves an existing directory's mode alone.
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let path = directory.join(format!("draft-{}-{stamp}.md", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    let edited = file
        .write_all(markdown.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            // Through the shell, so an editor named with its arguments
            // (`code -w`) runs as the person would run it; the path is an
            // argument, never part of the command text.
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{editor} \"$1\""))
                .arg("sh")
                .arg(&path)
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!("{editor} exited with {status}")));
            }
            std::fs::read_to_string(&path)
        });
    let removed = std::fs::remove_file(&path);
    let edited = edited?;
    removed?;
    // Editors end a file with a newline; a body does not need one.
    Ok(edited.trim_end_matches('\n').to_owned())
}

/// Run `editor` on the file at `path` itself -- `config.toml`, say -- at
/// line `line` when there is one: `+N` is what vi, vim, nano and emacs take.
pub fn edit_in_place(path: &Path, line: Option<usize>, editor: &str) -> io::Result<()> {
    let at = line.map(|line| format!(" +{line}")).unwrap_or_default();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{editor}{at} \"$1\""))
        .arg("sh")
        .arg(path)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{editor} exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// An editor that appends a line, and writes down the file's mode.
    fn fake_editor(dir: &Path) -> String {
        let script = dir.join("fake-editor");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nstat -c %a \"$1\" > '{}'\nprintf 'Added by the editor\\n' >> \"$1\"\n",
                dir.join("mode").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script.display().to_string()
    }

    #[test]
    fn the_editor_edits_and_the_file_is_gone_afterwards() {
        // US3 scenario 6.
        let scratch = tempfile::tempdir().unwrap();
        let drafts = scratch.path().join("postio");
        let edited = edit("Half **written**\n", &drafts, &fake_editor(scratch.path())).unwrap();

        assert_eq!(edited, "Half **written**\nAdded by the editor");
        assert_eq!(
            std::fs::read_to_string(scratch.path().join("mode"))
                .unwrap()
                .trim(),
            "600",
            "only this user can read the draft"
        );
        assert_eq!(
            std::fs::metadata(&drafts).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::read_dir(&drafts).unwrap().count(),
            0,
            "nothing left behind"
        );
    }

    #[test]
    fn an_editor_that_fails_leaves_nothing_behind_either() {
        let scratch = tempfile::tempdir().unwrap();
        let drafts = scratch.path().join("postio");
        assert!(edit("words", &drafts, "false").is_err());
        assert_eq!(std::fs::read_dir(&drafts).unwrap().count(), 0);
    }

    #[test]
    fn a_file_is_edited_where_it_is_at_the_line_asked() {
        let scratch = tempfile::tempdir().unwrap();
        let config = scratch.path().join("config.toml");
        std::fs::write(&config, "[ui]\n").unwrap();
        let script = scratch.path().join("editor");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$1\" > '{}'\nprintf 'theme = \"dark\"\\n' >> \"$2\"\n",
                scratch.path().join("asked").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        edit_in_place(&config, Some(3), &script.display().to_string()).unwrap();
        assert_eq!(
            std::fs::read_to_string(scratch.path().join("asked"))
                .unwrap()
                .trim(),
            "+3"
        );
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "[ui]\ntheme = \"dark\"\n"
        );
    }

    #[test]
    fn a_body_with_no_final_newline_comes_back_without_the_one_editors_add() {
        let scratch = tempfile::tempdir().unwrap();
        let edited = edit("one line", &scratch.path().join("postio"), "true").unwrap();
        assert_eq!(edited, "one line");
    }
}
