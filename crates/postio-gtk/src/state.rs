//! Window state that survives a restart.
//!
//! Where the user dragged a divider is not configuration — it is not something
//! anybody wants to hand-edit in `config.toml`, and it is not something Postio
//! should ask `postio-core` to round-trip through the mail database. It is
//! view state, it belongs to the view layer, and it lives in
//! `$XDG_STATE_HOME/postio/window.ini` as a plain key file.
//!
//! Everything here is best-effort by design. A missing, unreadable or
//! nonsensical file means the window opens at the canvas' own proportions
//! rather than failing to open at all: losing a divider position is a shrug,
//! refusing to start over one is a bug.

use std::path::{Path, PathBuf};

use gtk::glib;

/// The key-file group everything lives under.
const GROUP: &str = "Window";

/// The widest a stored dimension may be before it is treated as corrupt.
///
/// Displays get bigger; this only has to be absurd, not tight. It exists so a
/// truncated write or a hand-edit cannot open a window nobody can reach.
const SANE_MAX: i32 = 32_000;

/// The geometry and pane proportions a window reopens with.
///
/// The defaults are canvas 1b: a 1120px board with a 212px sidebar and a 404px
/// message list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowState {
    /// Window width in logical pixels.
    pub width: i32,
    /// Window height in logical pixels.
    pub height: i32,
    /// Whether the window was maximized when it was last closed.
    pub maximized: bool,
    /// Where the sidebar / content divider sat.
    pub sidebar_width: i32,
    /// Where the list / reader divider sat.
    pub list_width: i32,
    /// Whether the sidebar was showing.
    pub sidebar_visible: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        WindowState {
            width: crate::window::DEFAULT_SIZE.0,
            height: crate::window::DEFAULT_SIZE.1,
            maximized: false,
            sidebar_width: crate::shell::SIDEBAR_WIDTH,
            list_width: crate::shell::LIST_WIDTH,
            sidebar_visible: true,
        }
    }
}

impl WindowState {
    /// Read the saved state, falling back to [`Default`] for anything missing
    /// or out of range.
    pub fn load() -> Self {
        Self::load_from(&Self::path())
    }

    /// As [`load`](Self::load), from a path you name.
    pub fn load_from(path: &Path) -> Self {
        let key_file = glib::KeyFile::new();
        if key_file
            .load_from_file(path, glib::KeyFileFlags::NONE)
            .is_err()
        {
            return Self::default();
        }

        let fallback = Self::default();
        let length = |key: &str, default: i32| match key_file.integer(GROUP, key) {
            // A zero-width pane or a window wider than any display is not a
            // preference, it is a corrupt file.
            Ok(value) if (1..=SANE_MAX).contains(&value) => value,
            _ => default,
        };
        let flag = |key: &str, default: bool| key_file.boolean(GROUP, key).unwrap_or(default);

        WindowState {
            width: length("width", fallback.width),
            height: length("height", fallback.height),
            maximized: flag("maximized", fallback.maximized),
            sidebar_width: length("sidebar-width", fallback.sidebar_width),
            list_width: length("list-width", fallback.list_width),
            sidebar_visible: flag("sidebar-visible", fallback.sidebar_visible),
        }
    }

    /// Write the state out, creating the state directory if it is missing.
    pub fn save(&self) -> Result<(), glib::Error> {
        self.save_to(&Self::path())
    }

    /// As [`save`](Self::save), to a path you name.
    pub fn save_to(&self, path: &Path) -> Result<(), glib::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                glib::Error::new(
                    glib::FileError::Failed,
                    &format!("cannot create {}: {error}", parent.display()),
                )
            })?;
        }

        let key_file = glib::KeyFile::new();
        key_file.set_integer(GROUP, "width", self.width);
        key_file.set_integer(GROUP, "height", self.height);
        key_file.set_boolean(GROUP, "maximized", self.maximized);
        key_file.set_integer(GROUP, "sidebar-width", self.sidebar_width);
        key_file.set_integer(GROUP, "list-width", self.list_width);
        key_file.set_boolean(GROUP, "sidebar-visible", self.sidebar_visible);
        key_file.save_to_file(path)
    }

    /// `$XDG_STATE_HOME/postio/window.ini`.
    pub fn path() -> PathBuf {
        glib::user_state_dir().join("postio").join("window.ini")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("postio-state-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("window.ini")
    }

    #[test]
    fn the_defaults_are_the_canvas_proportions() {
        let state = WindowState::default();
        assert_eq!((state.width, state.height), crate::window::DEFAULT_SIZE);
        assert_eq!(state.sidebar_width, crate::shell::SIDEBAR_WIDTH);
        assert_eq!(state.list_width, crate::shell::LIST_WIDTH);
        assert!(state.sidebar_visible);
    }

    #[test]
    fn a_dragged_divider_survives_a_round_trip() {
        let path = scratch("round-trip");
        let saved = WindowState {
            width: 1600,
            height: 900,
            maximized: true,
            sidebar_width: 240,
            list_width: 380,
            sidebar_visible: false,
        };
        saved.save_to(&path).expect("the state should write");
        assert_eq!(WindowState::load_from(&path), saved);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = scratch("missing").with_file_name("nothing-here.ini");
        assert_eq!(WindowState::load_from(&path), WindowState::default());
    }

    #[test]
    fn a_corrupt_file_falls_back_rather_than_failing() {
        let path = scratch("corrupt");
        std::fs::write(&path, b"this is not a key file at all\x00\x01").unwrap();
        assert_eq!(WindowState::load_from(&path), WindowState::default());
    }

    #[test]
    fn nonsense_dimensions_are_rejected_field_by_field() {
        let path = scratch("nonsense");
        std::fs::write(
            &path,
            "[Window]\n\
             width=0\n\
             height=99999999\n\
             sidebar-width=-40\n\
             list-width=380\n\
             maximized=perhaps\n",
        )
        .unwrap();

        let state = WindowState::load_from(&path);
        let fallback = WindowState::default();
        assert_eq!(
            state.width, fallback.width,
            "a zero-width window is corrupt"
        );
        assert_eq!(state.height, fallback.height, "wider than any display");
        assert_eq!(state.sidebar_width, fallback.sidebar_width);
        assert_eq!(
            state.maximized, fallback.maximized,
            "`perhaps` is not a bool"
        );
        assert_eq!(
            state.list_width, 380,
            "one bad key must not throw away the good ones"
        );
    }

    #[test]
    fn the_state_lives_beside_the_other_state_not_in_the_config() {
        let path = WindowState::path();
        assert!(path.ends_with("postio/window.ini"), "{}", path.display());
        assert!(
            !path.to_string_lossy().contains("/.config/"),
            "view state is not configuration: {}",
            path.display()
        );
    }
}
