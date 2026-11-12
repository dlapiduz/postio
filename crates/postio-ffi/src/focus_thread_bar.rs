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
