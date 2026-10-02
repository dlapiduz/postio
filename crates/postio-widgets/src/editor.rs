//! `config.toml` in the person's own editor: what `CommandId::EditConfig`
//! (`mod+e`) does in both desktop apps (ADR 0043; specs/007-postio-focus
//! T235). Moved from the classic app's `config` module, where it was the
//! only launcher.
//!
//! `[compose] editor` wins when it is set (#1297); otherwise `$VISUAL` wins
//! over `$EDITOR`, the precedence every POSIX tool gives them. Neither is run inside a terminal: many desktop users already point
//! `$EDITOR` at a GUI editor for exactly this reason, and guessing at a
//! terminal emulator to wrap a text-mode one in is not a guess this module
//! has any way to make well. A terminal-only `$EDITOR` -- vim, nano, `emacs
//! -nw` -- starts with nothing to attach to; that is a real, documented
//! limitation of opening an editor from a GUI application, not a bug to
//! paper over with a heuristic that would be wrong as often as it was right.
//!
//! # Flatpak
//!
//! This does not work sandboxed, and cannot without more than this module:
//! the sandbox has neither the host's editor binary nor a path to launch
//! one. The one relevant portal, `org.freedesktop.portal.OpenURI`, opens the
//! desktop's default handler for a file -- never an arbitrary command, so
//! never literally `$EDITOR` -- and there is no terminal portal in the
//! freedesktop spec at all. Reaching the host's own binary would need the
//! spawn portal and the manifest to grant it, a sandbox-permission decision
//! for whoever ships the Flatpak build to make deliberately. Until then the
//! settings window itself is the sandboxed fallback: it edits the same file.

use std::ffi::OsString;
use std::path::Path;

/// The editor `$VISUAL` or `$EDITOR` names, in that order, from `lookup`.
///
/// `None` when neither is set or both are empty.
pub fn chosen(lookup: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(lookup)
        .find(|editor| !editor.is_empty())
}

/// Launch the person's editor on `path`, and return at once: the editor is
/// its own process, and the app follows the file as it is saved.
///
/// `[compose] editor` first, when it is set (#1297), then `$VISUAL` and
/// `$EDITOR`. Nothing is launched, and a warning is logged, when none of
/// them names an editor or the one named will not start.
pub fn open(path: &Path) {
    // `path` is `config.toml` itself -- it is the file being opened -- so
    // the setting is read from the same file, live, rather than from a copy
    // taken at startup that a `[compose]` edit would have made stale.
    //
    // What the name *means* is `postio_ui::handoff`'s, so this and the macOS
    // frontend reach the same conclusion about the same value. What only
    // this platform can answer is what it found, and it has two of the three
    // answers available to it: freedesktop cannot tell a windowed program
    // from a terminal one by looking (see this module's note above), so it
    // never reports `TerminalProgram` -- the caveat there stays a caveat. It
    // *can* tell that a name is not on `PATH` at all, which is a typo, and
    // saying so is the whole reason that third answer exists.
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
        // Nothing chosen: the desktop's own convention.
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

/// Start `editor` on `path`, and say so if it will not start.
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

/// What this desktop has by the name in `[compose] editor`.
///
/// Two of `Found`'s three answers, and the missing one is deliberate: this
/// platform cannot tell a windowed program from a terminal one by looking, so
/// it never claims `TerminalProgram` -- the module's note above is the
/// standing caveat about that. What it can tell is that a name resolves to
/// nothing at all, which is a typo and is worth saying.
///
/// An empty setting answers `Nothing` and never reaches a lookup:
/// `handoff::target` reads a blank as "not chosen" before it looks at this.
fn found_on_path(configured: &str) -> postio_ui::handoff::Found {
    let name = configured.trim();
    if name.is_empty() {
        return postio_ui::handoff::Found::Nothing;
    }
    // A path is taken at its word, the way a shell does.
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
