//! The thread bar at the boundary: `postio_ui::thread_bar` words the bar, kicker, older banner and
//! Reply to row; each crosses as an event when the window opens and on every step.

use postio_ui::thread_bar::{BannerView, Chip, ThreadBarView};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ThreadChipFfi {
    Message {
        index: u32,
        initials: String,
        /// "Ben · Thu".
        label: String,
        current: bool,
        unread: bool,
        matched: bool,
    },
    More {
        count: u32,
        from: u32,
        to: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadBarFfi {
    pub title: String,
    /// "6 · 2 new".
    pub count_line: String,
    /// The chips, left to right. Empty means a thread of one: draw no bar.
    pub chips: Vec<ThreadChipFfi>,
    pub menu_label: String,
    /// "Message 4 of 6": where the message on screen is in its thread, as
    /// the kicker says it without its keys. The window's title line says
    /// this while a thread walk is up. Empty when no chip is on screen.
    pub place: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadBannerFfi {
    pub text: String,
    pub action: String,
    /// The button's key, as the keymap has it: "}".
    pub key: Option<String>,
}

impl From<ThreadBarView> for ThreadBarFfi {
    fn from(view: ThreadBarView) -> Self {
        // The bar is a thread of two or more: its chips run to the last
        // member (a trailing "more" chip counts up to its `to`), and one is
        // the message on screen.
        let count = view
            .chips
            .iter()
            .map(|chip| match chip {
                Chip::Message { index, .. } => *index + 1,
                Chip::More { to, .. } => *to + 1,
            })
            .max();
        let current = view.chips.iter().find_map(|chip| match chip {
            Chip::Message {
                index,
                current: true,
                ..
            } => Some(*index),
            _ => None,
        });
        let place = match (current, count) {
            (Some(index), Some(count)) => {
                postio_ui::thread_bar::place(index as usize, count as usize)
            }
            _ => String::new(),
        };
        ThreadBarFfi {
            place,
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

use postio_ui::thread_outline::{MenuEntry, MenuGroup, OutlineRow, OutlineView, ThreadMenuView};

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuEntryFfi {
    pub command: String,
    pub title: String,
    pub keycap: Option<String>,
    pub note: Option<String>,
    /// Drawn dim and not pickable when false.
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuGroupFfi {
    /// "Whole thread", "This message only".
    pub title: String,
    pub entries: Vec<ThreadMenuEntryFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadMenuFfi {
    pub groups: Vec<ThreadMenuGroupFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlineRowFfi {
    pub index: u32,
    /// Its number as the person counts.
    pub number: u32,
    pub message: i64,
    pub initials: String,
    pub name: String,
    pub bold: bool,
    pub line: String,
    pub date: String,
    pub unread: bool,
    pub current: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlinePersonFfi {
    pub name: String,
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutlineFfi {
    pub rows: Vec<OutlineRowFfi>,
    pub people: Vec<OutlinePersonFfi>,
    pub filter: String,
    pub person: Option<String>,
    /// The row the keyboard is on, among `rows`.
    pub cursor: u32,
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
