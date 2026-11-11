//! Threads in Focus's list, at the boundary (spec 013, contracts/
//! frontend-threads.md).
//!
//! A thread row and its children cross with the words they draw, composed
//! by `postio_ui` (`ThreadRowView`, `ChildRowView`): ids as `i64`, nothing
//! left for Swift to word.

/// The chevron in a thread row's gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChevronFfi {
    /// Folded: the children are not shown.
    Collapsed,
    /// Open: the children sit under the row.
    Expanded,
}

/// What a thread row says beyond a conversation row's columns.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadRowFfi {
    /// "Lena, Ben, Ana, you".
    pub participants: String,
    /// The message count, for threads of two or more.
    pub count: Option<String>,
    /// "Ben: Looks good...".
    pub latest_line: String,
    /// "2 new".
    pub new: Option<String>,
    /// The gutter's chevron; `None` for a thread of one.
    pub chevron: Option<ChevronFfi>,
}

/// The keys a focused child shows on its right.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChildHintFfi {
    /// The keycaps, as the live keymap spells them.
    pub keys: Vec<String>,
    /// What they act on: "this one".
    pub label: String,
}

/// One child row of an expanded thread.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChildRowFfi {
    /// The avatar's letters.
    pub initials: String,
    /// "Ben Ortiz", "You", or "Draft - you".
    pub sender: String,
    /// Unread: drawn bold, with an accent dot.
    pub bold: bool,
    /// "to Ana" for a reply to an older message; "not sent" on a draft.
    pub tag: Option<String>,
    /// The message's own words.
    pub line: String,
    /// The line is a stand-in, drawn quiet.
    pub placeholder: bool,
    /// A paperclip.
    pub attachment: bool,
    /// The time, as every row says it.
    pub time: String,
    /// An unsent reply: drawn in italics.
    pub draft: bool,
    /// The keys shown while this row is focused.
    pub hint: Option<ChildHintFfi>,
}

impl From<postio_ui::focus_row::Chevron> for ChevronFfi {
    fn from(chevron: postio_ui::focus_row::Chevron) -> Self {
        match chevron {
            postio_ui::focus_row::Chevron::Collapsed => ChevronFfi::Collapsed,
            postio_ui::focus_row::Chevron::Expanded => ChevronFfi::Expanded,
        }
    }
}

impl From<&postio_ui::focus_row::ThreadRowView> for ThreadRowFfi {
    fn from(view: &postio_ui::focus_row::ThreadRowView) -> Self {
        ThreadRowFfi {
            participants: view.participants.clone(),
            count: view.count.clone(),
            latest_line: view.latest_line.clone(),
            new: view.new.clone(),
            chevron: view.chevron.map(Into::into),
        }
    }
}

impl From<&postio_ui::thread_list::ChildRowView> for ChildRowFfi {
    fn from(view: &postio_ui::thread_list::ChildRowView) -> Self {
        ChildRowFfi {
            initials: view.initials.clone(),
            sender: view.sender.clone(),
            bold: view.bold,
            tag: view.tag.clone(),
            line: view.line.clone(),
            placeholder: view.placeholder,
            attachment: view.attachment,
            time: view.time.clone(),
            draft: view.draft,
            hint: view.focused_hint.as_ref().map(|hint| ChildHintFfi {
                keys: hint.keys.clone(),
                label: hint.label.clone(),
            }),
        }
    }
}
