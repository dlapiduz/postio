//! What the terminal remembers between runs that is not configuration:
//! where the divider between the list and the reader was dragged to.
//!
//! The desktop keeps its pane widths in `$XDG_STATE_HOME/postio/window.ini`,
//! in pixels; a terminal's are columns, so it keeps its own file beside that
//! one, `terminal.ini`, in the same plain key-file form. Best-effort, like the
//! desktop's: a missing or nonsensical file is the default layout, never a
//! failure to start.

use std::path::{Path, PathBuf};

/// The widest a stored width may be before it is taken for a corrupt file.
const SANE_MAX: u16 = 2_000;

/// The layout a terminal reopens with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalState {
    /// The reading pane's width in columns, once someone has dragged it;
    /// `None` is the default proportion.
    pub reader_columns: Option<u16>,
}

impl TerminalState {
    /// Where the file lives.
    pub fn path() -> Option<PathBuf> {
        let state = std::env::var_os("XDG_STATE_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/state"))
            })?;
        Some(state.join("postio").join("terminal.ini"))
    }

    /// The saved layout, or the default.
    pub fn load() -> Self {
        Self::path().map_or_else(Self::default, |path| Self::load_from(&path))
    }

    /// The layout saved at `path`, or the default.
    pub fn load_from(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let reader_columns = text
            .lines()
            .filter_map(|line| line.split_once('='))
            .find(|(key, _)| key.trim() == "reader_columns")
            .and_then(|(_, value)| value.trim().parse::<u16>().ok())
            .filter(|columns| (1..=SANE_MAX).contains(columns));
        TerminalState { reader_columns }
    }

    /// Save the layout at the usual place.
    pub fn save(&self) -> std::io::Result<()> {
        match Self::path() {
            Some(path) => self.save_to(&path),
            None => Ok(()),
        }
    }

    /// Save the layout at `path`.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut text = String::from("[Terminal]\n");
        if let Some(columns) = self.reader_columns {
            text.push_str(&format!("reader_columns={columns}\n"));
        }
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dragged_width_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postio").join("terminal.ini");
        let dragged = TerminalState {
            reader_columns: Some(61),
        };
        dragged.save_to(&path).unwrap();
        assert_eq!(TerminalState::load_from(&path), dragged);
    }

    #[test]
    fn nonsense_is_the_default_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("terminal.ini");
        for text in [
            "",
            "[Terminal]\nreader_columns=wide\n",
            "[Terminal]\nreader_columns=99999\n",
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(
                TerminalState::load_from(&path),
                TerminalState::default(),
                "{text:?}"
            );
        }
    }
}
