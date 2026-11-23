//! Opens `config.toml` in the person's editor: `[compose] editor`, else `$VISUAL`, else `$EDITOR`.
//! None is run in a terminal, so a terminal-only editor has nothing to attach to. Under Flatpak
//! there is no host editor binary and no portal that launches one; the settings window is the
//! sandboxed fallback.

use std::ffi::OsString;
use std::path::Path;

/// `None` when neither is set or both are empty.
pub fn chosen(lookup: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(lookup)
        .find(|editor| !editor.is_empty())
}

/// Returns at once: the editor is its own process. Nothing is launched, and a warning is logged,
/// when none names an editor or the one named will not start.
pub fn open(path: &Path) {
    // `path` is `config.toml` itself, read live so a `[compose]` edit is not stale. This platform
    // cannot tell a terminal program from a windowed one, so it never reports `TerminalProgram`; it
    // can tell a name not on `PATH` (a typo).
    let configured = postio_config::Config::load_from_path(path)
        .unwrap_or_default()
        .compose
        .editor;
    match postio_ui::handoff::target(&configured, found_on_path(&configured)) {
        postio_ui::handoff::Target::Application(name) => {
            spawn(OsString::from(name), path);
            return;
        }
        postio_ui::handoff::Target::Missing(name) => {
            tracing::warn!(
                editor = %name,
                "{}",
                postio_ui::handoff::missing_advice(&name)
            );
            return;
        }
        postio_ui::handoff::Target::NeedsTerminal(name) => {
            tracing::warn!(
                editor = %name,
                "{}",
                postio_ui::handoff::terminal_advice(&name)
            );
            return;
        }
        postio_ui::handoff::Target::PlatformDefault => {}
    }

    let Some(editor) = chosen(|name| std::env::var_os(name)) else {
        tracing::warn!(
            path = %path.display(),
            "neither $VISUAL nor $EDITOR is set, so there is no editor to open"
        );
        return;
    };
    spawn(editor, path);
}

fn spawn(editor: OsString, path: &Path) {
    if let Err(error) = std::process::Command::new(&editor).arg(path).spawn() {
        tracing::warn!(
            editor = %editor.to_string_lossy(),
            path = %path.display(),
            %error,
            "cannot launch the editor"
        );
    }
}

/// Never claims `TerminalProgram`: freedesktop cannot tell. An empty setting answers `Nothing`
/// before any lookup.
fn found_on_path(configured: &str) -> postio_ui::handoff::Found {
    let name = configured.trim();
    if name.is_empty() {
        return postio_ui::handoff::Found::Nothing;
    }
    if name.contains('/') {
        return match std::fs::metadata(name) {
            Ok(_) => postio_ui::handoff::Found::Application,
            Err(_) => postio_ui::handoff::Found::Nothing,
        };
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return postio_ui::handoff::Found::Nothing;
    };
    let on_path = std::env::split_paths(&paths).any(|directory| {
        std::fs::metadata(directory.join(name)).is_ok_and(|found| found.is_file())
    });
    if on_path {
        postio_ui::handoff::Found::Application
    } else {
        postio_ui::handoff::Found::Nothing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn visual_wins_over_editor_and_an_empty_one_is_unset() {
        assert_eq!(
            chosen(from(&[("VISUAL", "gnome-text-editor"), ("EDITOR", "vi")])),
            Some(OsString::from("gnome-text-editor"))
        );
        assert_eq!(
            chosen(from(&[("EDITOR", "vi")])),
            Some(OsString::from("vi"))
        );
        assert_eq!(
            chosen(from(&[("VISUAL", ""), ("EDITOR", "vi")])),
            Some(OsString::from("vi"))
        );
        assert_eq!(chosen(from(&[])), None);
    }
}
