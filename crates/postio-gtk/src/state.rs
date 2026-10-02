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

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gtk::glib;
use postio_model::ids::MailboxId;
use postio_widgets::state::{GROUP, Geometry, length};

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
        let flag = |key: &str, default: bool| key_file.boolean(GROUP, key).unwrap_or(default);
        let geometry = Geometry::read(
            &key_file,
            Geometry {
                width: fallback.width,
                height: fallback.height,
                maximized: fallback.maximized,
            },
        );

        WindowState {
            width: geometry.width,
            height: geometry.height,
            maximized: geometry.maximized,
            sidebar_width: length(&key_file, "sidebar-width", fallback.sidebar_width),
            list_width: length(&key_file, "list-width", fallback.list_width),
            sidebar_visible: flag("sidebar-visible", fallback.sidebar_visible),
        }
    }

    /// Write the state out, creating the state directory if it is missing.
    pub fn save(&self) -> Result<(), glib::Error> {
        self.save_to(&Self::path())
    }

    /// As [`save`](Self::save), to a path you name.
    pub fn save_to(&self, path: &Path) -> Result<(), glib::Error> {
        // The file also carries the `[Sidebar]` group `SidebarState` owns,
        // and a fresh `KeyFile` here would silently drop it.
        let key_file = postio_widgets::state::open_for_writing(path)?;
        Geometry {
            width: self.width,
            height: self.height,
            maximized: self.maximized,
        }
        .write(&key_file);
        key_file.set_integer(GROUP, "sidebar-width", self.sidebar_width);
        key_file.set_integer(GROUP, "list-width", self.list_width);
        key_file.set_boolean(GROUP, "sidebar-visible", self.sidebar_visible);
        key_file.save_to_file(path)
    }

    /// `$XDG_STATE_HOME/postio/window.ini`.
    pub fn path() -> PathBuf {
        postio_widgets::state::path()
    }
}

/// The `[Sidebar]` group of `$XDG_STATE_HOME/postio/window.ini` (#324):
/// which folders the tree is showing collapsed.
///
/// A separate group in [`WindowState`]'s own file — "beside the other
/// state, not in the config" applies just as much to a folder's disclosure
/// as it does to a dragged divider — rather than a file of its own, so
/// there is still exactly one place view state lives.
///
/// Named for what is *collapsed*, not what is expanded: an account nobody
/// has touched has an empty set, and an empty set of collapsed folders is a
/// fully open tree — the same folders a flat list already showed before
/// #324, just correctly nested now, rather than a wall of closed rows on
/// first run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SidebarState {
    /// Folders the user has closed. Order carries no meaning.
    pub collapsed_folders: HashSet<MailboxId>,
}

/// The `[Sidebar]` group's own key file group name.
const SIDEBAR_GROUP: &str = "Sidebar";

impl SidebarState {
    /// Read the saved state, falling back to [`Default`] for anything
    /// missing, unreadable or unparsable.
    pub fn load() -> Self {
        Self::load_from(&WindowState::path())
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
        let Ok(raw) = key_file.value(SIDEBAR_GROUP, "collapsed-folders") else {
            return Self::default();
        };
        let collapsed_folders = raw
            .split(',')
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse::<i64>().ok())
            .map(MailboxId::new)
            .collect();
        SidebarState { collapsed_folders }
    }

    /// Write the state out, creating the state directory if it is missing.
    pub fn save(&self) -> Result<(), glib::Error> {
        self.save_to(&WindowState::path())
    }

    /// As [`save`](Self::save), to a path you name.
    pub fn save_to(&self, path: &Path) -> Result<(), glib::Error> {
        // As `WindowState::save_to`: keep the `[Window]` group this file
        // also carries across a sidebar-only save.
        let key_file = postio_widgets::state::open_for_writing(path)?;
        let mut ids: Vec<i64> = self.collapsed_folders.iter().map(|id| id.get()).collect();
        ids.sort_unstable();
        let joined = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        key_file.set_string(SIDEBAR_GROUP, "collapsed-folders", &joined);
        key_file.save_to_file(path)
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
    fn a_fresh_account_has_nothing_collapsed() {
        assert_eq!(SidebarState::default().collapsed_folders, HashSet::new());
        let path = scratch("sidebar-missing").with_file_name("nothing-here.ini");
        assert_eq!(SidebarState::load_from(&path), SidebarState::default());
    }

    #[test]
    fn collapsed_folders_survive_a_round_trip() {
        let path = scratch("sidebar-round-trip");
        let saved = SidebarState {
            collapsed_folders: HashSet::from([MailboxId::new(3), MailboxId::new(41)]),
        };
        saved.save_to(&path).expect("the state should write");
        assert_eq!(SidebarState::load_from(&path), saved);
    }

    /// #324: `SidebarState` and `WindowState` write the same file. Neither
    /// may clobber the other's group — see the `load_from_file` calls at the
    /// top of both `save_to` methods.
    #[test]
    fn window_and_sidebar_state_share_the_file_without_clobbering_each_other() {
        let path = scratch("shared-file");

        let window = WindowState {
            width: 1500,
            height: 950,
            maximized: true,
            sidebar_width: 230,
            list_width: 390,
            sidebar_visible: false,
        };
        window.save_to(&path).unwrap();

        let sidebar = SidebarState {
            collapsed_folders: HashSet::from([MailboxId::new(7)]),
        };
        sidebar.save_to(&path).unwrap();

        assert_eq!(
            WindowState::load_from(&path),
            window,
            "the sidebar's save must not have dropped the window group"
        );
        assert_eq!(SidebarState::load_from(&path), sidebar);

        // And the other order.
        let window2 = WindowState {
            width: 1024,
            ..window
        };
        window2.save_to(&path).unwrap();
        assert_eq!(
            SidebarState::load_from(&path),
            sidebar,
            "the window's save must not have dropped the sidebar group"
        );
        assert_eq!(WindowState::load_from(&path), window2);
    }
}
