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
    /// "muted · 1" on a muted thread.
    pub muted_badge: Option<String>,
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

/// One keycap and what it does, in the footer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FooterKeyFfi {
    /// The keycap, a short glyph as the live keymap spells it.
    pub cap: String,
    /// What it does: "reply to this".
    pub label: String,
}

/// The footer under the list: the scope in words, the keys that matter
/// there, and one sentence on what acts on what.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FooterViewFfi {
    /// "Message 4 of 6 · Ben Adeyemi".
    pub title: String,
    /// Keycap and label pairs, in drawing order.
    pub keys: Vec<FooterKeyFfi>,
    /// The right-hand sentence, none on a row that stands for one message.
    pub note: Option<String>,
}

impl From<postio_ui::scope::FooterView> for FooterViewFfi {
    fn from(view: postio_ui::scope::FooterView) -> Self {
        FooterViewFfi {
            title: view.title,
            keys: view
                .keys
                .into_iter()
                .map(|(cap, label)| FooterKeyFfi { cap, label })
                .collect(),
            note: view.note,
        }
    }
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
            muted_badge: view.muted_badge.clone(),
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

// -- A search in threads (spec 013 US6) ---------------------------------------

use crate::RunFfi;

/// One line under an expanded result: a message that matched, or the run of
/// messages between matches that did not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ResultChildFfi {
    /// A message that matched.
    Match {
        /// The message.
        message: i64,
        /// Its place in the thread, from 1.
        number: u32,
        /// "message 5", "message 5 · file".
        tag: String,
        /// The words around the match, the matched ones marked; empty
        /// while no passage is cut.
        passage: Vec<RunFfi>,
    },
    /// Messages that did not match: a 30-point line to read.
    Gap {
        /// "Message 4 doesn't match · Ben, Thu".
        text: String,
    },
}

/// The children drawn after a result's row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultSpliceFfi {
    /// The expanded result's position among the results.
    pub anchor: u64,
    /// The lines, oldest message first.
    pub children: Vec<ResultChildFfi>,
    /// The child with the focus ring, an index into `children`.
    pub focused: Option<u32>,
}

/// The banner of a window opened from a search.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MatchBannerFfi {
    /// "‘rate limit headers’ · match 1 of 3 in this thread (messages 3, 5 and 6)".
    pub text: String,
    /// The first button: "Next".
    pub next: String,
    /// Its key, as the keymap has it.
    pub next_key: Option<String>,
    /// The second: "Back to results".
    pub back: String,
    /// Its key, as the keymap has it.
    pub back_key: Option<String>,
}

impl From<postio_ui::thread_bar::MatchBannerView> for MatchBannerFfi {
    fn from(view: postio_ui::thread_bar::MatchBannerView) -> Self {
        MatchBannerFfi {
            text: view.text,
            next: view.next,
            next_key: view.next_key,
            back: view.back,
            back_key: view.back_key,
        }
    }
}

/// `passage` as runs: the byte ranges it names marked, an ellipsis where
/// text was cut.
fn passage_runs(passage: &postio_search::passage::Passage) -> Vec<RunFfi> {
    let plain = |text: &str| RunFfi {
        text: text.to_owned(),
        highlighted: false,
        style: crate::RunStyleFfi::Plain,
    };
    let mut runs = Vec::new();
    if passage.elided_start {
        runs.push(plain("\u{2026}"));
    }
    let text = &passage.text;
    let mut at = 0;
    for range in &passage.ranges {
        let (start, end) = (range.start, range.end);
        if start < at
            || end > text.len()
            || !text.is_char_boundary(start)
            || !text.is_char_boundary(end)
        {
            continue;
        }
        if start > at {
            runs.push(plain(&text[at..start]));
        }
        runs.push(RunFfi {
            highlighted: true,
            ..plain(&text[start..end])
        });
        at = end;
    }
    if at < text.len() {
        runs.push(plain(&text[at..]));
    }
    if passage.elided_end {
        runs.push(plain("\u{2026}"));
    }
    runs
}

impl From<&postio_ui::search_view::ResultChild> for ResultChildFfi {
    fn from(child: &postio_ui::search_view::ResultChild) -> Self {
        match child {
            postio_ui::search_view::ResultChild::Match {
                message,
                number,
                tag,
                passage,
            } => ResultChildFfi::Match {
                message: message.get(),
                number: *number,
                tag: tag.clone(),
                passage: passage.as_ref().map(passage_runs).unwrap_or_default(),
            },
            gap @ postio_ui::search_view::ResultChild::Gap { .. } => ResultChildFfi::Gap {
                text: postio_ui::search_view::gap_line(gap),
            },
        }
    }
}

impl From<&postio_focus::ResultSplice> for ResultSpliceFfi {
    fn from(splice: &postio_focus::ResultSplice) -> Self {
        ResultSpliceFfi {
            anchor: splice.anchor,
            children: splice.children.iter().map(Into::into).collect(),
            focused: splice.focused,
        }
    }
}
