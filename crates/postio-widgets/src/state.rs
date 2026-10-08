//! Window state that survives a restart: where the file lives, and the
//! window's own size.
//!
//! Where the user dragged a divider is not configuration — nobody wants to
//! hand-edit it in `config.toml`, and it is not something to round-trip
//! through the mail database. It is view state, it belongs to the view
//! layer, and it lives in `$XDG_STATE_HOME/postio/window.ini` as a plain key
//! file, which both desktop apps read and write.
//!
//! Everything here is best-effort by design. A missing, unreadable or
//! nonsensical file means the window opens at its default size rather than
//! failing to open at all: losing a size is a shrug, refusing to start over
//! one is a bug.
//!
//! [`Geometry`] is the part every window has: its size and whether it was
//! maximised. An app with more to remember (a window's dividers)
//! keeps its own keys in the same file and reads and writes these through
//! [`Geometry::read`] and [`Geometry::write`], so one file holds all of it
//! and neither writer drops the other's keys.

use std::path::{Path, PathBuf};

use gtk::glib;

/// The key-file group the window's own keys live under.
pub const GROUP: &str = "Window";

/// The widest a stored dimension may be before it is treated as corrupt.
///
/// Displays get bigger; this only has to be absurd, not tight. It exists so a
/// truncated write or a hand-edit cannot open a window nobody can reach.
pub const SANE_MAX: i32 = 32_000;

/// A window's size and whether it was maximised.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry {
    /// Width in logical pixels.
    pub width: i32,
    /// Height in logical pixels.
    pub height: i32,
    /// Whether the window was maximised when it was last closed.
    pub maximized: bool,
}

impl Geometry {
    /// A window of `width` by `height`, not maximised.
    pub const fn new(width: i32, height: i32) -> Self {
        Geometry {
            width,
            height,
            maximized: false,
        }
    }

    /// The saved geometry, or `fallback` for anything missing or out of
    /// range, key by key.
    pub fn load(fallback: Geometry) -> Self {
        Self::load_from(&path(), fallback)
    }

    /// As [`load`](Self::load), from a path you name.
    pub fn load_from(path: &Path, fallback: Geometry) -> Self {
        let key_file = glib::KeyFile::new();
        if key_file
            .load_from_file(path, glib::KeyFileFlags::NONE)
            .is_err()
        {
            return fallback;
        }
        Self::read(&key_file, fallback)
    }

    /// The geometry in `key_file`, or `fallback` for any key missing or out
    /// of range.
    pub fn read(key_file: &glib::KeyFile, fallback: Geometry) -> Self {
        Geometry {
            width: length(key_file, "width", fallback.width),
            height: length(key_file, "height", fallback.height),
            maximized: key_file
                .boolean(GROUP, "maximized")
                .unwrap_or(fallback.maximized),
        }
    }

    /// Put the geometry into `key_file`, leaving every other key alone.
    pub fn write(&self, key_file: &glib::KeyFile) {
        key_file.set_integer(GROUP, "width", self.width);
        key_file.set_integer(GROUP, "height", self.height);
        key_file.set_boolean(GROUP, "maximized", self.maximized);
    }

    /// Write the geometry out, creating the state directory if it is missing.
    pub fn save(&self) -> Result<(), glib::Error> {
        self.save_to(&path())
    }

    /// As [`save`](Self::save), to a path you name. What the file already
    /// holds besides the geometry is kept.
    pub fn save_to(&self, path: &Path) -> Result<(), glib::Error> {
        let key_file = open_for_writing(path)?;
        self.write(&key_file);
        key_file.save_to_file(path)
    }
}

/// A dimension, or `default` when it is missing or not a plausible length.
///
/// A zero-width pane or a window wider than any display is not a
/// preference, it is a corrupt file.
pub fn length(key_file: &glib::KeyFile, key: &str, default: i32) -> i32 {
    match key_file.integer(GROUP, key) {
        Ok(value) if (1..=SANE_MAX).contains(&value) => value,
        _ => default,
    }
}

/// The state file as it stands, ready to be written back with more in it.
///
/// Loads what is already there first: the file carries groups its writers
/// do not share, and a fresh `KeyFile` would silently drop them. A missing
/// or unreadable file is fine — there is nothing to preserve yet. The
/// directory is created.
fn open_for_writing(path: &Path) -> Result<glib::KeyFile, glib::Error> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            glib::Error::new(
                glib::FileError::Failed,
                &format!("cannot create {}: {error}", parent.display()),
            )
        })?;
    }
    let key_file = glib::KeyFile::new();
    let _ = key_file.load_from_file(path, glib::KeyFileFlags::NONE);
    Ok(key_file)
}

/// `$XDG_STATE_HOME/postio/window.ini`.
pub fn path() -> PathBuf {
    state_dir().join("postio").join("window.ini")
}

/// `$XDG_STATE_HOME`, falling back to `~/.local/state` per the XDG Base
/// Directory spec.
///
/// Not `glib::user_state_dir()`: GLib caches that function's result on its
/// first call in the process and never re-reads the environment after, so a
/// test that sets `$XDG_STATE_HOME` to a scratch directory only isolates
/// itself if it is the very first thing in the binary to ask GLib for a
/// state directory — every test after the first real one silently writes
/// into the developer's actual `~/.local/state/postio/`, #324 found. Read
/// directly from `std::env` instead, which has no such cache.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_STATE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    glib::home_dir().join(".local").join("state")
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: Geometry = Geometry::new(1440, 900);

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("postio-state-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("window.ini")
    }

    #[test]
    fn a_size_survives_a_round_trip() {
        let path = scratch("round-trip");
        let saved = Geometry {
            width: 1600,
            height: 900,
            maximized: true,
        };
        saved.save_to(&path).expect("the state should write");
        assert_eq!(Geometry::load_from(&path, DEFAULT), saved);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let path = scratch("missing").with_file_name("nothing-here.ini");
        assert_eq!(Geometry::load_from(&path, DEFAULT), DEFAULT);
    }

    #[test]
    fn a_corrupt_file_falls_back_rather_than_failing() {
        let path = scratch("corrupt");
        std::fs::write(&path, b"this is not a key file at all\x00\x01").unwrap();
        assert_eq!(Geometry::load_from(&path, DEFAULT), DEFAULT);
    }

    #[test]
    fn nonsense_dimensions_are_rejected_field_by_field() {
        let path = scratch("nonsense");
        std::fs::write(
            &path,
            "[Window]\nwidth=0\nheight=99999999\nmaximized=perhaps\n",
        )
        .unwrap();
        assert_eq!(Geometry::load_from(&path, DEFAULT), DEFAULT);

        std::fs::write(&path, "[Window]\nwidth=1000\nheight=-4\n").unwrap();
        let state = Geometry::load_from(&path, DEFAULT);
        assert_eq!(state.width, 1000, "one bad key must not drop the good ones");
        assert_eq!(state.height, DEFAULT.height);
    }

    #[test]
    fn the_state_lives_beside_the_other_state_not_in_the_config() {
        let path = path();
        assert!(path.ends_with("postio/window.ini"), "{}", path.display());
        assert!(
            !path.to_string_lossy().contains("/.config/"),
            "view state is not configuration: {}",
            path.display()
        );
    }

    #[test]
    fn saving_the_size_keeps_what_else_the_file_holds() {
        let path = scratch("shared-file");
        std::fs::write(
            &path,
            "[Window]\nlist-width=390\n[Sidebar]\ncollapsed-folders=7\n",
        )
        .unwrap();
        Geometry::new(1024, 700).save_to(&path).unwrap();
        let kept = std::fs::read_to_string(&path).unwrap();
        assert!(kept.contains("list-width=390"), "{kept}");
        assert!(kept.contains("collapsed-folders=7"), "{kept}");
        assert_eq!(
            Geometry::load_from(&path, DEFAULT),
            Geometry::new(1024, 700)
        );
    }
}
