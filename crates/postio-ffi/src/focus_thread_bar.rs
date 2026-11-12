//! The thread bar at the boundary (spec 013 US3, T047).
//!
//! The controller walks the thread; `postio_ui::thread_bar` words the bar,
//! the kicker, the older banner and the Reply to row. Each crosses as an
//! event when the window opens and on every step, and Swift draws what it is
//! told.

use postio_ui::thread_bar::{BannerView, Chip, ThreadBarView};

/// One chip of the bar.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ThreadChipFfi {
    /// A message: an avatar over "Name · time".
    Message {
        /// Its place among the members, from 0.
        index: u32,
        /// The avatar's letters.
        initials: String,
        /// "Ben · Thu".
        label: String,
        /// The message on screen: ringed.
        current: bool,
        /// Not read yet: dotted.
        unread: bool,
        /// A search matched it.
        matched: bool,
    },
    /// "… N more".
    More {
        /// How many it stands for.
        count: u32,
        /// The first member it stands for.
        from: u32,
        /// The last member it stands for.
        to: u32,
    },
}

/// The bar: the label column, the chips and the button.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadBarFfi {
    /// "Thread".
    pub title: String,
    /// "6 · 2 new".
    pub count_line: String,
    /// The chips, left to right. Empty means a thread of one: draw no bar.
    pub chips: Vec<ThreadChipFfi>,
    /// The button on the right.
    pub menu_label: String,
}

/// A quiet banner: words and one button.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadBannerFfi {
    /// What it says.
    pub text: String,
    /// The button: "Latest".
    pub action: String,
    /// The button's key, as the keymap has it: "}".
    pub key: Option<String>,
}

impl From<ThreadBarView> for ThreadBarFfi {
    fn from(view: ThreadBarView) -> Self {
        ThreadBarFfi {
            title: view.title,
            count_line: view.count_line,
            chips: view
                .chips
                .into_iter()
                .map(|chip| match chip {
                    Chip::Message {
                        index,
                        initials,
                        label,
                        current,
                        unread,
                        matched,
                    } => ThreadChipFfi::Message {
                        index,
                        initials,
                        label,
                        current,
                        unread,
                        matched,
                    },
                    Chip::More { count, from, to } => ThreadChipFfi::More { count, from, to },
                })
                .collect(),
            menu_label: view.menu_label,
        }
    }
}

impl From<BannerView> for ThreadBannerFfi {
    fn from(banner: BannerView) -> Self {
        ThreadBannerFfi {
            text: banner.text,
            action: banner.action,
            key: banner.key,
        }
    }
}

// -- Thread menu and outline (spec 013 US5) ----------------------------------

use postio_ui::thread_outline::{MenuEntry, MenuGroup, OutlineRow, OutlineView, ThreadMenuView};

/// One entry of Thread ▾'s menu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuEntryFfi {
    /// The command it runs, by its id; sent back to pick it.
    pub command: String,
    /// What it says.
    pub title: String,
    /// The keycap, as the live keymap spells it.
    pub keycap: Option<String>,
    /// The sentence under the title.
    pub note: Option<String>,
    /// Drawn dim and not pickable when false.
    pub enabled: bool,
}

/// One titled group of the menu.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuGroupFfi {
    /// "Whole thread", "This message only".
    pub title: String,
    /// Its entries, in order.
    pub entries: Vec<ThreadMenuEntryFfi>,
}

/// Thread ▾'s menu: two groups.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuFfi {
    /// The groups, top to bottom.
    pub groups: Vec<ThreadMenuGroupFfi>,
}

/// One message of the outline.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlineRowFfi {
    /// Its place among the thread's members, from 0.
    pub index: u32,
    /// Its number as the person counts.
    pub number: u32,
    /// The message.
    pub message: i64,
    /// The avatar's letters.
    pub initials: String,
    /// Who wrote it.
    pub name: String,
    /// Drawn bold: not read yet.
    pub bold: bool,
    /// Its first line.
    pub line: String,
    /// When.
    pub date: String,
    /// The unread dot.
    pub unread: bool,
    /// The message on screen: ringed.
    pub current: bool,
}

/// A person chip with how many messages they wrote.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlinePersonFfi {
    /// The name.
    pub name: String,
    /// Their messages.
    pub count: u32,
}

/// The outline popover, whole.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlineFfi {
    /// The rows the filter leaves.
    pub rows: Vec<OutlineRowFfi>,
    /// Person chips over the whole thread.
    pub people: Vec<OutlinePersonFfi>,
    /// The field's text.
    pub filter: String,
    /// The chip chosen.
    pub person: Option<String>,
    /// The row the keyboard is on, among `rows`.
    pub cursor: u32,
    /// The footer's `u` keycap.
    pub first_unread_key: Option<String>,
}

impl From<MenuEntry> for ThreadMenuEntryFfi {
    fn from(entry: MenuEntry) -> Self {
        ThreadMenuEntryFfi {
            command: entry.command.to_string(),
            title: entry.title,
            keycap: entry.keycap,
            note: entry.note,
            enabled: entry.enabled,
        }
    }
}

impl From<MenuGroup> for ThreadMenuGroupFfi {
    fn from(group: MenuGroup) -> Self {
        ThreadMenuGroupFfi {
            title: group.title,
            entries: group.entries.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<ThreadMenuView> for ThreadMenuFfi {
    fn from(view: ThreadMenuView) -> Self {
        ThreadMenuFfi {
            groups: view.groups.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<OutlineRow> for OutlineRowFfi {
    fn from(row: OutlineRow) -> Self {
        OutlineRowFfi {
            index: row.index as u32,
            number: row.number,
            message: row.message.get(),
            initials: row.initials,
            name: row.name,
            bold: row.bold,
            line: row.line,
            date: row.date,
            unread: row.unread,
            current: row.current,
        }
    }
}

impl From<OutlineView> for OutlineFfi {
    fn from(view: OutlineView) -> Self {
        OutlineFfi {
            rows: view.rows.into_iter().map(Into::into).collect(),
            people: view
                .people
                .into_iter()
                .map(|(name, count)| OutlinePersonFfi { name, count })
                .collect(),
            filter: view.filter,
            person: view.person,
            cursor: view.cursor as u32,
            first_unread_key: view.first_unread_key,
        }
    }
}
