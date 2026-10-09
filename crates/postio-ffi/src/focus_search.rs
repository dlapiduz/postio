//! Focus's search at the boundary (specs/010-focus-search,
//! contracts/ffi-search.md).
//!
//! The dropdown is the controller's (`postio_focus`, ADR 0045): which state
//! it is in, what each row says and what running one does. The Mac draws
//! the `FocusDropdown` it is told and reports what happened -- the field's
//! words and a row run, through the bar's own exports (`focus_bar_typed`,
//! `focus_bar_run`, `focus_bar_tab`), and the dropdown's own keys here:
//! where the arrows rest, ⌥⌫ on a recent search, and ⌘↩.
//!
//! The results view (step 3) is the controller's too: `FocusQuery`,
//! `FocusResults`, `FocusResultsPage`, `FocusResultsCursor` and
//! `FocusLeaveResults` say what to draw, `focus_search_row` reads a row, and
//! a control's change to the query crosses as a `TermEditFfi`. A highlight
//! crosses as runs of words, each marked or not, so no offset is counted in
//! one encoding and drawn in another.
//!
//! Every word is composed in Rust by `postio-ui`; a keycap crosses as the
//! keymap spells it and Swift draws it as every Mac keycap is drawn.

use crate::session::Session;
use crate::settings::KeyHintFfi;

/// Which of the design's states the dropdown is in (§2's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DropdownStateFfi {
    /// Nothing typed: recent, saved, the cheat sheet (screen 01).
    Empty,
    /// Words: top hits, Narrow to, Show all (screen 03).
    Words,
}

impl From<postio_focus::DropdownState> for DropdownStateFfi {
    fn from(state: postio_focus::DropdownState) -> Self {
        match state {
            postio_focus::DropdownState::Words => DropdownStateFfi::Words,
            _ => DropdownStateFfi::Empty,
        }
    }
}

/// How a run of words is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RunStyleFfi {
    /// The row's own face.
    Plain,
    /// Bold.
    Strong,
    /// Monospaced: a query, an operator.
    Mono,
}

impl From<postio_focus::RunStyle> for RunStyleFfi {
    fn from(style: postio_focus::RunStyle) -> Self {
        match style {
            postio_focus::RunStyle::Plain => RunStyleFfi::Plain,
            postio_focus::RunStyle::Strong => RunStyleFfi::Strong,
            postio_focus::RunStyle::Mono => RunStyleFfi::Mono,
        }
    }
}

/// A stretch of words, highlighted where the engine found the query's.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RunFfi {
    /// The words.
    pub text: String,
    /// Drawn with the find highlight.
    pub highlighted: bool,
    /// How it is set.
    pub style: RunStyleFfi,
}

fn runs(runs: Vec<postio_focus::Run>) -> Vec<RunFfi> {
    runs.into_iter()
        .map(|run| RunFfi {
            text: run.text,
            highlighted: run.highlighted,
            style: run.style.into(),
        })
        .collect()
}

/// What a dropdown row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DropdownRowKindFfi {
    /// A search run lately: ↩ runs it again, ⌥⌫ forgets it.
    Recent,
    /// A conversation: ↩ opens it.
    Hit,
    /// "Show all N results".
    ShowAll,
    /// An operator and what it is for.
    CheatSheet,
    /// The plain-English example.
    Example,
}

impl From<postio_focus::DropdownRowKind> for DropdownRowKindFfi {
    fn from(kind: postio_focus::DropdownRowKind) -> Self {
        use postio_focus::DropdownRowKind as Kind;
        match kind {
            Kind::Recent => DropdownRowKindFfi::Recent,
            Kind::Hit => DropdownRowKindFfi::Hit,
            Kind::ShowAll => DropdownRowKindFfi::ShowAll,
            Kind::Example => DropdownRowKindFfi::Example,
            _ => DropdownRowKindFfi::CheatSheet,
        }
    }
}

/// One row of the dropdown.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownRowFfi {
    /// What `focus_bar_run` hands back to run it.
    pub token: u64,
    /// What it is.
    pub kind: DropdownRowKindFfi,
    /// Its title: a query, "Sender · Subject".
    pub title: Vec<RunFfi>,
    /// After the title: a count, a passage.
    pub detail: Vec<RunFfi>,
    /// The folder column: `in:Inbox`.
    pub folder: Option<String>,
    /// The right column: a date, "yesterday".
    pub right: Option<String>,
    /// The key that runs it, as the keymap spells it.
    pub key: Option<String>,
    /// Whether the arrows may rest on it.
    pub selectable: bool,
}

/// A pill: a saved search, or a filter to narrow to.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PillFfi {
    /// What `focus_bar_run` hands back to run it.
    pub token: u64,
    /// The operator, tertiary and monospaced: `from:`.
    pub op: Option<String>,
    /// The saved search's name, or the filter's value.
    pub label: String,
    /// How many it holds.
    pub count: Option<String>,
    /// The key that runs it (`alt+1`), as the keymap spells it.
    pub key: Option<String>,
}

/// One section of the dropdown.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownSectionFfi {
    /// Bold and secondary; empty for the Show all row's.
    pub title: String,
    /// Tertiary, on the right.
    pub note: Option<String>,
    /// The key the note names before its words.
    pub note_key: Option<String>,
    /// Its rows.
    pub rows: Vec<DropdownRowFfi>,
    /// Its pills, in a line after the title.
    pub pills: Vec<PillFfi>,
}

/// The dropdown, whole: everything `FocusDropdown` redraws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownViewFfi {
    /// Which state it is in.
    pub state: DropdownStateFfi,
    /// Top to bottom.
    pub sections: Vec<DropdownSectionFfi>,
    /// The row focused by default: kept while the highlighted row is still
    /// drawn, taken when it is not.
    pub highlight: Option<u64>,
    /// A run moved the highlight here: taken whatever is highlighted.
    pub select: Option<u64>,
    /// The footer's keys, as the keymap spells them.
    pub footer_hints: Vec<KeyHintFfi>,
    /// "48 matches · 38 ms".
    pub footer_count: Option<String>,
}

impl From<postio_focus::DropdownView> for DropdownViewFfi {
    fn from(view: postio_focus::DropdownView) -> Self {
        DropdownViewFfi {
            state: view.state.into(),
            sections: view
                .sections
                .into_iter()
                .map(|section| DropdownSectionFfi {
                    title: section.title,
                    note: section.note,
                    note_key: section.note_key,
                    rows: section
                        .rows
                        .into_iter()
                        .map(|row| DropdownRowFfi {
                            token: row.token,
                            kind: row.kind.into(),
                            title: runs(row.title),
                            detail: runs(row.detail),
                            folder: row.folder,
                            right: row.right,
                            key: row.key,
                            selectable: row.selectable,
                        })
                        .collect(),
                    pills: section
                        .pills
                        .into_iter()
                        .map(|pill| PillFfi {
                            token: pill.token,
                            op: pill.op,
                            label: pill.label,
                            count: pill.count,
                            key: pill.key,
                        })
                        .collect(),
                })
                .collect(),
            highlight: view.highlight,
            select: view.select,
            footer_hints: view
                .hints
                .into_iter()
                .map(|hint| KeyHintFfi {
                    key: hint.key,
                    label: hint.label,
                })
                .collect(),
            footer_count: view.count,
        }
    }
}

// ---------------------------------------------------------------------------
// The results view (spec 010 step 3, screens 06 and 07)
// ---------------------------------------------------------------------------

/// The results' tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ResultsTabFfi {
    /// Conversations.
    Conversations,
    /// Files.
    Files,
    /// People.
    People,
}

impl From<ResultsTabFfi> for postio_search::results::ResultsTab {
    fn from(tab: ResultsTabFfi) -> Self {
        match tab {
            ResultsTabFfi::Conversations => Self::Conversations,
            ResultsTabFfi::Files => Self::Files,
            ResultsTabFfi::People => Self::People,
        }
    }
}

impl From<postio_search::results::ResultsTab> for ResultsTabFfi {
    fn from(tab: postio_search::results::ResultsTab) -> Self {
        use postio_search::results::ResultsTab;
        match tab {
            ResultsTab::Conversations => ResultsTabFfi::Conversations,
            ResultsTab::Files => ResultsTabFfi::Files,
            ResultsTab::People => ResultsTabFfi::People,
        }
    }
}

/// The Sort menu's two orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ConversationOrderFfi {
    /// Ranked, with Top hits first.
    BestMatch,
    /// Newest first, in month groups only.
    Newest,
}

impl From<ConversationOrderFfi> for postio_search::results::ConversationOrder {
    fn from(order: ConversationOrderFfi) -> Self {
        match order {
            ConversationOrderFfi::BestMatch => Self::BestMatch,
            ConversationOrderFfi::Newest => Self::Newest,
        }
    }
}

impl From<postio_search::results::ConversationOrder> for ConversationOrderFfi {
    fn from(order: postio_search::results::ConversationOrder) -> Self {
        match order {
            postio_search::results::ConversationOrder::Newest => ConversationOrderFfi::Newest,
            _ => ConversationOrderFfi::BestMatch,
        }
    }
}

/// A filter bar button (design §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FilterKindFfi {
    /// From ▾.
    From,
    /// To ▾.
    To,
    /// Date ▾.
    Date,
    /// Anywhere ▾ (a folder).
    Anywhere,
    /// Label ▾.
    Label,
    /// The Attachment toggle.
    Attachment,
    /// The Has action toggle.
    HasAction,
    /// The Unread toggle.
    Unread,
}

impl From<postio_ui::search_view::FilterKind> for FilterKindFfi {
    fn from(kind: postio_ui::search_view::FilterKind) -> Self {
        use postio_ui::search_view::FilterKind;
        match kind {
            FilterKind::From => FilterKindFfi::From,
            FilterKind::To => FilterKindFfi::To,
            FilterKind::Date => FilterKindFfi::Date,
            FilterKind::Anywhere => FilterKindFfi::Anywhere,
            FilterKind::Label => FilterKindFfi::Label,
            FilterKind::Attachment => FilterKindFfi::Attachment,
            FilterKind::HasAction => FilterKindFfi::HasAction,
            FilterKind::Unread => FilterKindFfi::Unread,
        }
    }
}

impl From<FilterKindFfi> for postio_ui::search_view::FilterKind {
    fn from(kind: FilterKindFfi) -> Self {
        use postio_ui::search_view::FilterKind;
        match kind {
            FilterKindFfi::From => FilterKind::From,
            FilterKindFfi::To => FilterKind::To,
            FilterKindFfi::Date => FilterKind::Date,
            FilterKindFfi::Anywhere => FilterKind::Anywhere,
            FilterKindFfi::Label => FilterKind::Label,
            FilterKindFfi::Attachment => FilterKind::Attachment,
            FilterKindFfi::HasAction => FilterKind::HasAction,
            FilterKindFfi::Unread => FilterKind::Unread,
        }
    }
}

/// A change to the query from a control: the operator's keyword and value,
/// which Rust spells (D13). Swift never builds query text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TermEditFfi {
    /// Add `field:value` (`-field:value` when `negated`): "from",
    /// "ada@example.com".
    Add {
        /// The operator's keyword.
        field: String,
        /// Its value.
        value: String,
        /// Excluded.
        negated: bool,
    },
    /// A chip's ✕, by its token.
    Remove {
        /// [`QueryChipFfi::token`].
        token: u32,
    },
    /// A toggle button: "has" "attachment", "is" "unread", "has" "action".
    Toggle {
        /// The operator's keyword.
        field: String,
        /// Its value.
        value: String,
    },
    /// A timeline drag over the months `first..=last`, 0 the oldest bar.
    SetMonths {
        /// The first month.
        first: i32,
        /// The last month.
        last: i32,
    },
    /// Keep the words, drop every operator.
    ClearFilters,
}

impl From<TermEditFfi> for postio_focus::TermEdit {
    fn from(edit: TermEditFfi) -> Self {
        let month = |n: i32| u32::try_from(n).unwrap_or(0);
        match edit {
            TermEditFfi::Add {
                field,
                value,
                negated,
            } => postio_focus::TermEdit::Add {
                field,
                value,
                negated,
            },
            TermEditFfi::Remove { token } => postio_focus::TermEdit::Remove { token },
            TermEditFfi::Toggle { field, value } => postio_focus::TermEdit::Toggle { field, value },
            TermEditFfi::SetMonths { first, last } => postio_focus::TermEdit::SetMonths {
                first: month(first),
                last: month(last),
            },
            TermEditFfi::ClearFilters => postio_focus::TermEdit::ClearFilters,
        }
    }
}

/// One operator term, as a chip in the field (design §1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueryChipFfi {
    /// What `TermEditFfi::Remove` names it by.
    pub token: u32,
    /// The operator with its colon, tertiary: "from:".
    pub operator: String,
    /// Its value, in label colour.
    pub value: String,
    /// Struck through.
    pub excluded: bool,
    /// Ringed.
    pub focused: bool,
}

/// One filter button (design §3.2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilterButtonFfi {
    /// Which.
    pub kind: FilterKindFfi,
    /// "From", or "From: Ada Moreno" once applied.
    pub label: String,
    /// Solid.
    pub applied: bool,
    /// Its popover is open: the accent ring.
    pub open: bool,
}

/// The query as the field and the filter bar draw it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueryViewFfi {
    /// The chips, in the query's order.
    pub chips: Vec<QueryChipFfi>,
    /// The plain words, after the chips.
    pub words: String,
    /// "/ to edit".
    pub hint: String,
    /// The filter bar's buttons, left to right.
    pub buttons: Vec<FilterButtonFfi>,
}

impl From<postio_focus::QueryView> for QueryViewFfi {
    fn from(view: postio_focus::QueryView) -> Self {
        QueryViewFfi {
            chips: view
                .chips
                .into_iter()
                .map(|chip| QueryChipFfi {
                    token: chip.token,
                    operator: chip.operator,
                    value: chip.value,
                    excluded: chip.excluded,
                    focused: chip.focused,
                })
                .collect(),
            words: view.words,
            hint: view.hint,
            buttons: view
                .buttons
                .into_iter()
                .map(|button| FilterButtonFfi {
                    kind: button.kind.into(),
                    label: button.label,
                    applied: button.applied,
                    open: button.open,
                })
                .collect(),
        }
    }
}

/// A results tab, with its count.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TabFfi {
    /// Which.
    pub tab: ResultsTabFfi,
    /// "Conversations".
    pub label: String,
    /// "48".
    pub count: String,
    /// The one shown.
    pub selected: bool,
    /// Its key (`cmd+1`), as the keymap spells it.
    pub key: Option<String>,
}

/// One bar of the timeline.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MonthBarFfi {
    /// "Sep".
    pub label: String,
    /// Conversations in it.
    pub conversations: u64,
    /// 0 to 1 of the tallest.
    pub height: f64,
    /// Inside the query's dates: the soft band, the bold label.
    pub selected: bool,
}

// `height` is a share of the tallest bar, never NaN, so equality is total.
impl Eq for MonthBarFfi {}

/// A group header and the rows under it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultGroupFfi {
    /// "Top hits", "September 2026".
    pub title: String,
    /// "9", tertiary after the title; empty for Top hits.
    pub count: String,
    /// "newest first", "why each one ranked is under the sender".
    pub note: Option<String>,
    /// The first row's position.
    pub first: u64,
    /// How many rows.
    pub rows: u64,
    /// Top hits: rows 66 tall, not 58.
    pub top_hits: bool,
    /// What VoiceOver says for the header: "September 2026 · 9".
    pub accessible: String,
}

/// The results view's frame: everything but the rows, which
/// `focus_search_row` reads.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultsViewFfi {
    /// Conversations, Files, People.
    pub tabs: Vec<TabFfi>,
    /// The Sort menu.
    pub order: ConversationOrderFfi,
    /// "48 conversations".
    pub count_line: String,
    /// "12 files · 6 people · last 12 months".
    pub sub_line: String,
    /// The twelve bars, oldest first.
    pub months: Vec<MonthBarFfi>,
    /// The timeline's hint on its right: "Matches by month · drag across
    /// months to narrow", or "Jul – Sep selected · drag to change".
    pub timeline_hint: String,
    /// ⌥←/⌥→ "steps a month", after the hint while a range is selected.
    pub timeline_step: Option<KeyHintFfi>,
    /// The groups, top to bottom.
    pub groups: Vec<ResultGroupFfi>,
    /// How many rows the table has.
    pub rows: u64,
    /// The row with the focus ring.
    pub cursor: Option<u64>,
    /// The footer's keys.
    pub footer_hints: Vec<KeyHintFfi>,
    /// "48 conversations · local index · 41 ms".
    pub footer_right: String,
    /// Conversations checked: the footer is the bulk bar while this is
    /// above zero.
    pub selected: u64,
    /// The bulk bar's verbs and keys.
    pub bulk: Vec<KeyHintFfi>,
    /// The bulk bar's right, "⇧X select all 12", while some but not every
    /// conversation the query matches is checked.
    pub select_all: Option<KeyHintFfi>,
}

fn month_bars(months: Vec<postio_focus::MonthBar>) -> Vec<MonthBarFfi> {
    months
        .into_iter()
        .map(|month| MonthBarFfi {
            label: month.label,
            conversations: month.conversations,
            height: month.height,
            selected: month.selected,
        })
        .collect()
}

fn hints(hints: Vec<postio_ui::hints::Hint>) -> Vec<KeyHintFfi> {
    hints
        .into_iter()
        .map(|hint| KeyHintFfi {
            key: hint.key,
            label: hint.label,
        })
        .collect()
}

impl From<postio_focus::ResultsView> for ResultsViewFfi {
    fn from(view: postio_focus::ResultsView) -> Self {
        ResultsViewFfi {
            tabs: view
                .tabs
                .into_iter()
                .map(|tab| TabFfi {
                    tab: tab.tab.into(),
                    label: tab.label,
                    count: tab.count,
                    selected: tab.selected,
                    key: tab.key,
                })
                .collect(),
            order: view.order.into(),
            count_line: view.count_line,
            sub_line: view.sub_line,
            months: month_bars(view.months),
            timeline_hint: view.timeline_hint,
            timeline_step: view.timeline_step.map(|hint| KeyHintFfi {
                key: hint.key,
                label: hint.label,
            }),
            groups: view
                .groups
                .into_iter()
                .map(|group| ResultGroupFfi {
                    title: group.title,
                    count: group.count,
                    note: group.note,
                    first: group.first,
                    rows: group.rows,
                    top_hits: group.top_hits,
                    accessible: group.accessible,
                })
                .collect(),
            rows: view.rows,
            cursor: view.cursor,
            footer_hints: hints(view.hints),
            footer_right: view.footer,
            selected: view.selected,
            bulk: hints(view.bulk),
            select_all: view.select_all.map(|hint| KeyHintFfi {
                key: hint.key,
                label: hint.label,
            }),
        }
    }
}

/// One row of a list popover (§3.6): a person, a folder or a label.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PopoverRowFfi {
    /// What `focus_search_popover_toggle` names it by.
    pub token: u64,
    /// "Ada Moreno", "Inbox", "Atlas".
    pub title: String,
    /// The address under a person's name, in SF Mono 11.
    pub detail: Option<String>,
    /// A person's avatar: "AM".
    pub initials: Option<String>,
    /// A label's colour, `#rrggbb`.
    pub color: Option<String>,
    /// Conversations among the results it opened on.
    pub count: u64,
    /// Its bar, 0 to 1 of the largest.
    pub share: f64,
    /// Checked: the query holds it.
    pub checked: bool,
    /// Excluded: the query holds `-` it.
    pub excluded: bool,
}

// `share` is a share of the largest count, never NaN.
impl Eq for PopoverRowFfi {}

/// One of the Date popover's presets.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DatePresetFfi {
    /// What `focus_search_date_preset` names it by.
    pub token: u64,
    /// "Last 30 days", "Custom…".
    pub label: String,
    /// Its count; none for Custom….
    pub count: Option<String>,
    /// Ringed: the query's dates are its own.
    pub selected: bool,
}

/// A filter popover, whole (§3.6, screens 08 and 09).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverViewFfi {
    /// The button it hangs from.
    pub kind: FilterKindFfi,
    /// Its search field's placeholder; empty for Date, which has none.
    pub placeholder: String,
    /// What its search field holds.
    pub filter: String,
    /// The rows its field leaves.
    pub rows: Vec<PopoverRowFfi>,
    /// Its footer's keys: Space toggle, ⌥ -click excludes, ↩ apply.
    pub hints: Vec<KeyHintFfi>,
    /// The Date popover's presets, Custom… last.
    pub presets: Vec<DatePresetFfi>,
    /// The Date popover's plain words.
    pub words: String,
    /// What they became: "→ after:2026-07-01".
    pub parsed: Option<String>,
    /// The line under the words.
    pub words_hint: String,
    /// The Date popover's chart, 90 tall: the timeline's bars.
    pub months: Vec<MonthBarFfi>,
    /// "12 of 21".
    pub result: Option<String>,
    /// "Jul – Sep 2026".
    pub range: Option<String>,
}

impl From<postio_focus::PopoverView> for PopoverViewFfi {
    fn from(view: postio_focus::PopoverView) -> Self {
        PopoverViewFfi {
            kind: view.kind.into(),
            placeholder: view.placeholder,
            filter: view.filter,
            rows: view
                .rows
                .into_iter()
                .map(|row| PopoverRowFfi {
                    token: row.token,
                    title: row.title,
                    detail: row.detail,
                    initials: row.initials,
                    color: row.color,
                    count: row.count,
                    share: row.share,
                    checked: row.checked,
                    excluded: row.excluded,
                })
                .collect(),
            hints: hints(view.hints),
            presets: view
                .presets
                .into_iter()
                .map(|preset| DatePresetFfi {
                    token: preset.token,
                    label: preset.label,
                    count: preset.count,
                    selected: preset.selected,
                })
                .collect(),
            words: view.words,
            parsed: view.parsed,
            words_hint: view.words_hint,
            months: month_bars(view.months),
            result: view.result,
            range: view.range,
        }
    }
}

/// One match card in Quick Look (design §3.7).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MatchCardFfi {
    /// The left column's first line: "Body", "Earlier reply", "Subject",
    /// a file's name.
    pub place: String,
    /// Its second line: "Ada · 26 Sep", where in a file; may be empty.
    pub when: String,
    /// The passage, 14/22, its words marked.
    pub passage: Vec<RunFfi>,
    /// The place is a file's name.
    pub file: bool,
}

/// Quick Look, whole (design §3.7, screen 10).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuickLookViewFfi {
    /// "Quick Look", bold in the header.
    pub title: String,
    /// "1 of 12".
    pub position: String,
    /// "j/k" "moves through results while it stays open".
    pub walk: Option<KeyHintFfi>,
    /// The header's buttons, in order: Open, Archive, Close, with keys.
    pub actions: Vec<KeyHintFfi>,
    /// The subject, 22/28 bold, its words marked.
    pub subject: Vec<RunFfi>,
    /// The sender line: the name strong, the address mono, then when and
    /// the thread's size.
    pub sender: Vec<RunFfi>,
    /// "4 matches in this conversation".
    pub matches_line: String,
    /// "]/[" "jump between them".
    pub matches_hint: Option<KeyHintFfi>,
    /// One card per match, oldest first, the subject last.
    pub cards: Vec<MatchCardFfi>,
    /// The ringed card.
    pub current: Option<u32>,
}

impl From<postio_focus::QuickLookView> for QuickLookViewFfi {
    fn from(view: postio_focus::QuickLookView) -> Self {
        let hint = |hint: postio_ui::hints::Hint| KeyHintFfi {
            key: hint.key,
            label: hint.label,
        };
        QuickLookViewFfi {
            title: view.title,
            position: view.position,
            walk: view.walk.map(hint),
            actions: hints(view.actions),
            subject: runs(view.subject),
            sender: runs(view.sender),
            matches_line: view.matches_line,
            matches_hint: view.matches_hint.map(hint),
            cards: view
                .cards
                .into_iter()
                .map(|card| MatchCardFfi {
                    place: card.place,
                    when: card.when,
                    passage: runs(card.passage),
                    file: card.file,
                })
                .collect(),
            current: view.current,
        }
    }
}

/// One result row (design §3.4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultRowFfi {
    /// The message it shows and opens.
    pub id: i64,
    /// Its conversation.
    pub thread: Option<i64>,
    /// Its group, an index into `ResultsViewFfi::groups`.
    pub group: u32,
    /// A top hit: 66 tall, with its reason.
    pub top_hit: bool,
    /// The sender column.
    pub sender: String,
    /// Bold sender, and the unread dot.
    pub unread: bool,
    /// Under the sender on a top hit: "you replied · 3 matches".
    pub reason: Option<String>,
    /// The subject, matched words in the find highlight.
    pub subject: Vec<RunFfi>,
    /// The label pills.
    pub pills: Vec<crate::focus_list::LabelPillFfi>,
    /// The paperclip.
    pub attachments: bool,
    /// The thread count.
    pub count_badge: Option<String>,
    /// "body", "quoted text", "subject", a file's name.
    pub source_tag: String,
    /// The tag is a file's name: italics.
    pub source_is_file: bool,
    /// The passage, matched words in the find highlight; empty until read.
    pub passage: Vec<RunFfi>,
    /// "in:Inbox".
    pub folder: String,
    /// "26 Sep".
    pub date: String,
    /// Checked: the filled box and the selection tint.
    pub checked: bool,
    /// What VoiceOver reads (design §5).
    pub accessible: String,
}

impl From<postio_focus::ResultRow> for ResultRowFfi {
    fn from(row: postio_focus::ResultRow) -> Self {
        ResultRowFfi {
            id: row.message.get(),
            thread: row.thread.map(|thread| thread.get()),
            group: row.group,
            top_hit: row.top_hit,
            sender: row.sender,
            unread: row.unread,
            reason: row.reason,
            subject: runs(row.subject),
            pills: row
                .labels
                .into_iter()
                .map(|pill| crate::focus_list::LabelPillFfi {
                    name: pill.name,
                    color: pill.color,
                })
                .collect(),
            attachments: row.attachments,
            count_badge: row.count_badge,
            source_tag: row.source_tag,
            source_is_file: row.source_is_file,
            passage: runs(row.passage),
            folder: row.folder,
            date: row.date,
            checked: row.checked,
            accessible: row.accessible,
        }
    }
}

/// The results view's chrome, in `postio-ui`'s words (§3.1-3.5): what the
/// Mac draws around the rows that no event carries.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchWordsFfi {
    /// "‹ Inbox": the toolbar's way back.
    pub back: String,
    /// The toolbar's Save search button.
    pub save: String,
    /// The quiet word before the Sort menu.
    pub sort: String,
    /// The Sort menu's ranked order.
    pub best_match: String,
    /// The Sort menu's date order.
    pub newest: String,
    /// The timeline's hint on its right.
    pub timeline_hint: String,
}

/// The results view's chrome words.
#[uniffi::export]
pub fn focus_search_words() -> SearchWordsFfi {
    use postio_ui::search_view as words;
    SearchWordsFfi {
        back: words::BACK_TO_INBOX.to_owned(),
        save: words::SAVE_SEARCH.to_owned(),
        sort: words::SORT.to_owned(),
        best_match: words::BEST_MATCH.to_owned(),
        newest: words::NEWEST.to_owned(),
        timeline_hint: words::TIMELINE_HINT.to_owned(),
    }
}

/// The bulk bar's count while `n` results are checked: "5 selected".
#[uniffi::export]
pub fn focus_search_checked(n: u64) -> String {
    postio_ui::search_view::checked_line(n)
}

#[uniffi::export]
impl Session {
    /// The arrows rest on the dropdown's row `token` now (the highlight is
    /// the toolkit's, 009 FR-004): what ⌥⌫ from a menu forgets.
    pub fn focus_search_highlighted(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::SearchHighlighted(token));
    }

    /// ⌥⌫ on the recent search `token`: forget it.
    pub fn focus_search_forget(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::SearchForget(token));
    }

    /// ⌘↩, or a click on Show all: the main window turns into the results
    /// for what is typed (`FocusQuery`, then `FocusResults`).
    pub fn focus_search_show_all(&self) {
        let _ = self
            .focus_driver()
            .command(postio_core::CommandId::ShowAllResults);
    }

    /// A filter button, a chip's ✕, a popover's check or a timeline drag:
    /// the results' query changes, and `FocusQuery` then `FocusResults`
    /// say how.
    pub fn focus_search_edit(&self, edit: TermEditFfi) {
        self.focus_driver()
            .input(postio_focus::Input::SearchEdit(edit.into()));
    }

    /// A filter button with a popover was pressed: `FocusQuery` rings it
    /// and `FocusPopover` says what to hang from it. A toggle button's kind
    /// opens nothing.
    pub fn focus_search_popover(&self, kind: FilterKindFfi) {
        self.focus_driver()
            .input(postio_focus::Input::SearchPopover(kind.into()));
    }

    /// Space or a click on the open popover's row `token`; `exclude` when
    /// ⌥ was held. The controller decides what that writes.
    pub fn focus_search_popover_toggle(&self, token: u64, exclude: bool) {
        self.focus_driver()
            .input(postio_focus::Input::PopoverToggle { token, exclude });
    }

    /// The popover's own search field ("Filter people in these results").
    pub fn focus_search_popover_filter(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::PopoverFilter(text));
    }

    /// ↩ (`apply`), or Esc and a click away: the popover closes, keeping
    /// its preview or putting back the query it opened on. Not an echo:
    /// the Mac reports the close the toolkit made, and a close the
    /// controller made (`FocusPopover` with none) needs no report.
    pub fn focus_search_popover_done(&self, apply: bool) {
        self.focus_driver()
            .input(postio_focus::Input::PopoverDone { apply });
    }

    /// The Date popover's plain-words field.
    pub fn focus_search_date_words(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::DateWords(text));
    }

    /// The Date popover's preset `token`.
    pub fn focus_search_date_preset(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::DatePreset(token));
    }

    /// A drag across the timeline ended over bars `first..=last`, 0 the
    /// oldest (either order): the query's dates become those months.
    pub fn focus_search_months(&self, first: u32, last: u32) {
        self.focus_driver().input(postio_focus::Input::SearchEdit(
            postio_focus::TermEdit::SetMonths { first, last },
        ));
    }

    /// A results tab picked by a click (⌘1-3 are commands).
    pub fn focus_search_tab(&self, tab: ResultsTabFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsTab(tab.into()));
    }

    /// The Sort menu.
    pub fn focus_search_order(&self, order: ConversationOrderFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsOrder(order.into()));
    }

    /// A click on the result at `position`: the focus ring goes there.
    pub fn focus_search_point(&self, position: u64) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsPoint(position));
    }

    /// How many rows the results table has.
    pub fn focus_search_row_count(&self) -> u64 {
        self.focus_driver().result_count()
    }

    /// The result at `position`, or `None` while its page is on its way:
    /// a miss asks for it, and `FocusResultsPage` says when to read it
    /// again. Synchronous; what the table calls for every visible row.
    pub fn focus_search_row(&self, position: u64) -> Option<ResultRowFfi> {
        self.focus_driver().result_row(position).map(Into::into)
    }

    /// The field's placeholder while it is empty (screen 01).
    pub fn focus_search_placeholder(&self) -> String {
        postio_ui::search_view::PLACEHOLDER.to_owned()
    }
}
