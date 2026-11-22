//! Threads in Focus's list: a thread row and its children cross with the words they draw, composed
//! by `postio_ui`; ids as `i64`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ChevronFfi {
    Collapsed,
    Expanded,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ThreadRowFfi {
    /// "Lena, Ben, Ana, you".
    pub participants: String,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChildHintFfi {
    pub keys: Vec<String>,
    /// What they act on: "this one".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChildRowFfi {
    pub initials: String,
    /// "Ben Ortiz", "You", or "Draft - you".
    pub sender: String,
    /// Unread: drawn bold, with an accent dot.
    pub bold: bool,
    /// "to Ana" for a reply to an older message; "not sent" on a draft.
    pub tag: Option<String>,
    pub line: String,
    pub placeholder: bool,
    pub attachment: bool,
    pub time: String,
    pub draft: bool,
    pub hint: Option<ChildHintFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FooterKeyFfi {
    pub cap: String,
    /// What it does: "reply to this".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FooterViewFfi {
    /// "Message 4 of 6 · Ben Adeyemi".
    pub title: String,
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

use crate::RunFfi;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ResultChildFfi {
    Match {
        message: i64,
        number: u32,
        /// "message 5", "message 5 · file".
        tag: String,
        /// The words around the match, the matched ones marked; empty
        /// while no passage is cut.
        passage: Vec<RunFfi>,
        /// Who wrote it: "Tomás Reyes", "You".
        sender: String,
        initials: String,
        time: String,
    },
    Gap {
        /// "Message 4 doesn't match · Ben, 16 Sep".
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultSpliceFfi {
    pub anchor: u64,
    pub children: Vec<ResultChildFfi>,
    pub focused: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MatchBannerFfi {
    /// "‘rate limit headers’ · match 1 of 3 in this thread (messages 3, 5 and 6)".
    pub text: String,
    pub next: String,
    pub next_key: Option<String>,
    pub back: String,
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
                sender,
                initials,
                time,
            } => ResultChildFfi::Match {
                sender: sender.clone(),
                initials: initials.clone(),
                time: time.clone(),
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
