//! Focus's search at the boundary. The dropdown is the controller's (`postio_focus`, ADR 0045); the
//! Mac draws the `FocusDropdown` it is told. A highlight crosses as runs of words, so no offset is
//! counted in one encoding and drawn in another.

use crate::session::Session;
use crate::settings::KeyHintFfi;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DropdownStateFfi {
    Empty,
    Words,
    Prefix,
    Operator,
    PlainEnglish,
}

impl From<postio_focus::DropdownState> for DropdownStateFfi {
    fn from(state: postio_focus::DropdownState) -> Self {
        match state {
            postio_focus::DropdownState::Words => DropdownStateFfi::Words,
            postio_focus::DropdownState::Prefix => DropdownStateFfi::Prefix,
            postio_focus::DropdownState::Operator => DropdownStateFfi::Operator,
            postio_focus::DropdownState::PlainEnglish => DropdownStateFfi::PlainEnglish,
            _ => DropdownStateFfi::Empty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RunStyleFfi {
    Plain,
    Strong,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RunFfi {
    pub text: String,
    pub highlighted: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DropdownRowKindFfi {
    /// A search run lately: ↩ runs it again, ⌥⌫ forgets it.
    Recent,
    Hit,
    ShowAll,
    CheatSheet,
    Example,
    /// A completed word: Return or Tab puts it in the field.
    Word,
    /// A label: Return its chip, Option-Return its exclusion.
    Label,
    List,
    File,
    /// A person: the initials avatar, Return the chip, Option-Return the
    /// exclusion.
    Person,
    Folder,
}

impl From<postio_focus::DropdownRowKind> for DropdownRowKindFfi {
    fn from(kind: postio_focus::DropdownRowKind) -> Self {
        use postio_focus::DropdownRowKind as Kind;
        match kind {
            Kind::Recent => DropdownRowKindFfi::Recent,
            Kind::Hit => DropdownRowKindFfi::Hit,
            Kind::ShowAll => DropdownRowKindFfi::ShowAll,
            Kind::Example => DropdownRowKindFfi::Example,
            Kind::Word => DropdownRowKindFfi::Word,
            Kind::Label => DropdownRowKindFfi::Label,
            Kind::List => DropdownRowKindFfi::List,
            Kind::File => DropdownRowKindFfi::File,
            Kind::Person => DropdownRowKindFfi::Person,
            Kind::Folder => DropdownRowKindFfi::Folder,
            _ => DropdownRowKindFfi::CheatSheet,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownRowFfi {
    pub token: u64,
    pub kind: DropdownRowKindFfi,
    /// Its title: a query, "Sender · Subject".
    pub title: Vec<RunFfi>,
    /// After the title: a count, a passage.
    pub detail: Vec<RunFfi>,
    /// The folder column: `in:Inbox`.
    pub folder: Option<String>,
    /// The right column: a date, "yesterday".
    pub right: Option<String>,
    pub key: Option<String>,
    pub selectable: bool,
    pub initials: Option<String>,
    /// On its right while the arrows rest on it: "↩ adds" (screen 04).
    #[uniffi(default = None)]
    pub highlighted: Option<KeyHintFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnderstoodTileFfi {
    /// The operator with its colon, tertiary ("from:"); empty for a word.
    pub op: String,
    pub value: String,
    /// "from ‘last month’", 10.5 pt under the term.
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PillFfi {
    pub token: u64,
    /// The operator, tertiary and monospaced: `from:`.
    pub op: Option<String>,
    pub label: String,
    pub count: Option<String>,
    /// A saved search that notifies: its quiet badge ("3 new"), never a banner.
    pub fresh: Option<String>,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownSectionFfi {
    /// Bold and secondary; empty for the Show all row's.
    pub title: String,
    pub note: Option<String>,
    pub note_key: Option<String>,
    pub rows: Vec<DropdownRowFfi>,
    pub pills: Vec<PillFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DropdownViewFfi {
    pub state: DropdownStateFfi,
    /// The rest of the best word, tertiary after the caret ("las").
    pub ghost: Option<String>,
    /// In the field before Escape's cap while there is a ghost: "Tab completes".
    #[uniffi(default = None)]
    pub ghost_hint: Option<KeyHintFfi>,
    /// The operator being typed ("from:"), drawn tertiary in the field before its value.
    #[uniffi(default = None)]
    pub operator_prefix: Option<String>,
    pub understood: Vec<UnderstoodTileFfi>,
    pub sections: Vec<DropdownSectionFfi>,
    /// The row focused by default: kept while the highlighted row is still
    /// drawn, taken when it is not.
    pub highlight: Option<u64>,
    /// A run moved the highlight here: taken whatever is highlighted.
    pub select: Option<u64>,
    pub footer_hints: Vec<KeyHintFfi>,
    /// "48 matches · 38 ms".
    pub footer_count: Option<String>,
}

impl From<postio_focus::DropdownView> for DropdownViewFfi {
    fn from(view: postio_focus::DropdownView) -> Self {
        DropdownViewFfi {
            state: view.state.into(),
            ghost: view.ghost,
            ghost_hint: view.ghost_hint.map(key_hint),
            operator_prefix: view.operator,
            understood: view
                .understood
                .into_iter()
                .map(|tile| UnderstoodTileFfi {
                    op: tile.op,
                    value: tile.value,
                    origin: tile.origin,
                })
                .collect(),
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
                            initials: row.initials,
                            highlighted: row.highlighted.map(key_hint),
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
                            fresh: pill.fresh,
                            key: pill.key,
                        })
                        .collect(),
                })
                .collect(),
            highlight: view.highlight,
            select: view.select,
            footer_hints: view.hints.into_iter().map(key_hint).collect(),
            footer_count: view.count,
        }
    }
}

fn key_hint(hint: postio_ui::hints::Hint) -> KeyHintFfi {
    KeyHintFfi {
        key: hint.key,
        label: hint.label,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ResultsTabFfi {
    Conversations,
    Files,
    People,
    Events,
}

impl From<ResultsTabFfi> for postio_search::results::ResultsTab {
    fn from(tab: ResultsTabFfi) -> Self {
        match tab {
            ResultsTabFfi::Conversations => Self::Conversations,
            ResultsTabFfi::Files => Self::Files,
            ResultsTabFfi::People => Self::People,
            ResultsTabFfi::Events => Self::Events,
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
            ResultsTab::Events => ResultsTabFfi::Events,
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FilterKindFfi {
    From,
    To,
    Date,
    Anywhere,
    Label,
    Attachment,
    HasAction,
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

/// A change to the query from a control: the operator's keyword and value, which Rust spells. Swift
/// never builds query text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TermEditFfi {
    /// Add `field:value` (`-field:value` when `negated`): "from",
    /// "ada@example.com".
    Add {
        field: String,
        value: String,
        negated: bool,
    },
    Remove {
        token: u32,
    },
    /// A toggle button: "has" "attachment", "is" "unread", "has" "action".
    Toggle {
        field: String,
        value: String,
    },
    /// A timeline drag over the months `first..=last`, 0 the oldest bar.
    SetMonths {
        first: i32,
        last: i32,
    },
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueryChipFfi {
    pub token: u32,
    /// The operator with its colon, tertiary: "from:".
    pub operator: String,
    pub value: String,
    pub excluded: bool,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilterButtonFfi {
    pub kind: FilterKindFfi,
    /// "From", or "From: Ada Moreno" once applied.
    pub label: String,
    pub applied: bool,
    /// Its popover is open: the accent ring.
    pub open: bool,
    /// The key that opens its popover, as the registry spells it; `None`
    /// for a toggle.
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QueryViewFfi {
    pub chips: Vec<QueryChipFfi>,
    pub words: String,
    /// "/ to edit", or "clears filters" after its key while nothing
    /// matches.
    pub hint: String,
    /// The hint's key as the registry spells it (`cmd+BackSpace`), drawn
    /// as a cap before it; `None` when the hint names its own.
    pub hint_key: Option<String>,
    /// The whole field carries the accent ring: nothing matches.
    pub ringed: bool,
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
            hint_key: view.hint_key,
            ringed: view.ringed,
            buttons: view
                .buttons
                .into_iter()
                .map(|button| FilterButtonFfi {
                    kind: button.kind.into(),
                    label: button.label,
                    applied: button.applied,
                    open: button.open,
                    key: button.key,
                })
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TabFfi {
    pub tab: ResultsTabFfi,
    pub label: String,
    pub count: String,
    pub selected: bool,
    pub key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MonthBarFfi {
    pub label: String,
    pub conversations: u64,
    /// 0 to 1 of the tallest.
    pub height: f64,
    /// Inside the query's dates: the soft band, the bold label.
    pub selected: bool,
    /// `conversations` as said, for VoiceOver: "9", or "9+" when the search stopped at its walk's
    /// cap and every count is a floor.
    #[uniffi(default = "")]
    pub count: String,
}

// `height` is a share of the tallest bar, never NaN, so equality is total.
impl Eq for MonthBarFfi {}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultGroupFfi {
    /// "Top hits", "September 2026".
    pub title: String,
    /// "9", tertiary after the title; empty for Top hits.
    pub count: String,
    /// "newest first", "why each one ranked is under the sender".
    pub note: Option<String>,
    pub first: u64,
    pub rows: u64,
    /// Top hits: rows 66 tall, not 58.
    pub top_hits: bool,
    /// What VoiceOver says for the header: "September 2026 · 9".
    pub accessible: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultsViewFfi {
    pub tabs: Vec<TabFfi>,
    pub order: ConversationOrderFfi,
    /// Whether Sort offers a choice: not on the Files tab, whose cards are
    /// newest first.
    pub sortable: bool,
    /// "48 threads".
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
    pub groups: Vec<ResultGroupFfi>,
    pub rows: u64,
    pub cursor: Option<u64>,
    pub footer_hints: Vec<KeyHintFfi>,
    /// "48 threads · local index · 41 ms".
    pub footer_right: String,
    /// Conversations checked: the footer is the bulk bar while this is
    /// above zero.
    pub selected: u64,
    pub bulk: Vec<KeyHintFfi>,
    /// The bulk bar's right, "⇧X select all 12", while some but not every
    /// conversation the query matches is checked.
    pub select_all: Option<KeyHintFfi>,
    /// The Files tab's header, while it is the tab shown; its cards are
    /// `focus_search_file`'s, `rows` of them.
    pub files: Option<FilesHeaderFfi>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilesHeaderFfi {
    pub title: String,
    /// What is searched inside files, and whether it is done.
    pub note: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FilePreviewFfi {
    Sheet,
    Page,
    Slides,
    Image,
    Text,
}

impl From<postio_focus::FilePreview> for FilePreviewFfi {
    fn from(preview: postio_focus::FilePreview) -> Self {
        match preview {
            postio_focus::FilePreview::Sheet => FilePreviewFfi::Sheet,
            postio_focus::FilePreview::Page => FilePreviewFfi::Page,
            postio_focus::FilePreview::Slides => FilePreviewFfi::Slides,
            postio_focus::FilePreview::Image => FilePreviewFfi::Image,
            postio_focus::FilePreview::Text => FilePreviewFfi::Text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PersonRowFfi {
    /// Their address: what ↩ searches `from:`.
    pub address: String,
    pub name: String,
    pub initials: String,
    /// "3 messages": the matched messages from and to them.
    pub messages: String,
    /// The newest of those: "26 Sep".
    pub last: String,
    pub focused: bool,
    pub accessible: String,
}

impl From<postio_focus::PersonRow> for PersonRowFfi {
    fn from(row: postio_focus::PersonRow) -> Self {
        PersonRowFfi {
            address: row.address,
            name: row.name,
            initials: row.initials,
            messages: row.messages,
            last: row.last,
            focused: row.focused,
            accessible: row.accessible,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileCardFfi {
    pub attachment: i64,
    pub message: i64,
    pub kind: String,
    pub preview: FilePreviewFfi,
    /// Which of the preview's lines is marked, from the top; none when
    /// only the name matched.
    pub marked: Option<u32>,
    pub name: Vec<RunFfi>,
    /// "Ada Moreno · 26 Sep · 48 KB".
    pub meta: String,
    /// "Sheet ‘Q3’, row 3: Total Atlas budget …", matched words in the
    /// find highlight; empty when only the name matched.
    pub line: Vec<RunFfi>,
    /// "in ‘Re: Atlas Q3 budget’".
    pub subject: String,
    pub focused: bool,
    pub accessible: String,
}

impl From<postio_focus::FileCard> for FileCardFfi {
    fn from(card: postio_focus::FileCard) -> Self {
        FileCardFfi {
            attachment: card.attachment.get(),
            message: card.message.get(),
            kind: card.kind,
            preview: card.preview.into(),
            marked: card.marked,
            name: runs(card.name),
            meta: card.meta,
            line: runs(card.line),
            subject: card.subject,
            focused: card.focused,
            accessible: card.accessible,
        }
    }
}

/// A copy of a file for the system: Quick Look on it, or a save panel. The path is in the app's own
/// temporary folder.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileCopyFfi {
    pub path: String,
    pub name: String,
    pub save: bool,
}

impl From<postio_focus::FileCopy> for FileCopyFfi {
    fn from(copy: postio_focus::FileCopy) -> Self {
        FileCopyFfi {
            path: copy.path.to_string_lossy().into_owned(),
            name: copy.name,
            save: copy.purpose == postio_focus::FilePurpose::Save,
        }
    }
}

fn month_bars(months: Vec<postio_focus::MonthBar>) -> Vec<MonthBarFfi> {
    months
        .into_iter()
        .map(|month| MonthBarFfi {
            label: month.label,
            conversations: month.conversations,
            height: month.height,
            selected: month.selected,
            count: month.count,
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
            sortable: view.sortable,
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
            files: view.files.map(|header| FilesHeaderFfi {
                title: header.title,
                note: header.note,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PopoverRowFfi {
    pub token: u64,
    pub title: String,
    pub detail: Option<String>,
    pub initials: Option<String>,
    /// A label's colour, `#rrggbb`.
    pub color: Option<String>,
    pub count: u64,
    /// Its bar, 0 to 1 of the largest.
    pub share: f64,
    pub checked: bool,
    pub excluded: bool,
    /// `count` as drawn on the row's right: "24", or "24+" when the results were capped and every
    /// count is a floor.
    #[uniffi(default = "")]
    pub count_label: String,
}

// `share` is a share of the largest count, never NaN.
impl Eq for PopoverRowFfi {}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DatePresetFfi {
    pub token: u64,
    pub label: String,
    /// Its count; none for Custom….
    pub count: Option<String>,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PopoverViewFfi {
    pub kind: FilterKindFfi,
    /// Its search field's placeholder; empty for Date, which has none.
    pub placeholder: String,
    pub filter: String,
    pub rows: Vec<PopoverRowFfi>,
    /// Its footer's keys: Space toggle, ⌥ -click excludes, ↩ apply.
    pub hints: Vec<KeyHintFfi>,
    pub presets: Vec<DatePresetFfi>,
    pub words: String,
    /// What they became: "→ after:2026-07-01".
    pub parsed: Option<String>,
    pub words_hint: String,
    pub months: Vec<MonthBarFfi>,
    pub result: Option<String>,
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
                    count_label: row.count_label,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SaveViewFfi {
    pub title: String,
    pub name_label: String,
    pub name: String,
    pub chips: Vec<String>,
    pub pin: bool,
    pub pin_label: String,
    pub pin_note: String,
    /// The key it will run on (`alt+3`); `None` past the fourth.
    pub pin_key: Option<String>,
    pub notify: bool,
    pub notify_label: String,
    pub notify_note: String,
    pub rolling: bool,
    pub rolling_label: String,
    /// What each way means; `None` when the query has no date and the
    /// switch is not drawn.
    pub rolling_note: Option<String>,
    pub cancel: String,
    pub save: String,
    pub save_key: Option<String>,
}

impl From<postio_focus::SaveView> for SaveViewFfi {
    fn from(view: postio_focus::SaveView) -> Self {
        SaveViewFfi {
            title: view.title,
            name_label: view.name_label,
            name: view.name,
            chips: view.chips,
            pin: view.pin,
            pin_label: view.pin_label,
            pin_note: view.pin_note,
            pin_key: view.pin_key,
            notify: view.notify,
            notify_label: view.notify_label,
            notify_note: view.notify_note,
            rolling: view.rolling,
            rolling_label: view.rolling_label,
            rolling_note: view.rolling_note,
            cancel: view.cancel,
            save: view.save,
            save_key: view.save_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MatchCardFfi {
    /// The left column's first line: "Body", "Earlier reply", "Subject",
    /// a file's name.
    pub place: String,
    /// Its second line: "Ada · 26 Sep", where in a file; may be empty.
    pub when: String,
    pub passage: Vec<RunFfi>,
    pub file: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct QuickLookViewFfi {
    pub title: String,
    pub position: String,
    pub walk: Option<KeyHintFfi>,
    pub actions: Vec<KeyHintFfi>,
    pub subject: Vec<RunFfi>,
    pub sender: Vec<RunFfi>,
    pub matches_line: String,
    pub matches_hint: Option<KeyHintFfi>,
    /// One card per match, oldest first, the subject last.
    pub cards: Vec<MatchCardFfi>,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelaxationFfi {
    pub number: u32,
    /// The key that runs it (`1`), for its cap; `None` when unbound.
    pub key: Option<String>,
    pub label: String,
    pub query: String,
    pub count: String,
    pub focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoResultsViewFfi {
    pub title: String,
    pub body: String,
    /// The looser searches, most first, at most four; empty while counted.
    pub relaxations: Vec<RelaxationFfi>,
    pub counting: Option<String>,
    pub searched: String,
}

impl From<postio_focus::NoResultsView> for NoResultsViewFfi {
    fn from(view: postio_focus::NoResultsView) -> Self {
        NoResultsViewFfi {
            title: view.title,
            body: view.body,
            relaxations: view
                .relaxations
                .into_iter()
                .map(|way| RelaxationFfi {
                    number: way.number,
                    key: way.key,
                    label: way.label,
                    query: way.query,
                    count: way.count,
                    focused: way.focused,
                })
                .collect(),
            counting: view.counting,
            searched: view.searched,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ResultRowFfi {
    pub id: i64,
    pub thread: Option<i64>,
    /// Its group, an index into `ResultsViewFfi::groups`.
    pub group: u32,
    /// A top hit: 66 tall, with its reason.
    pub top_hit: bool,
    pub sender: String,
    /// The sender's address: what a click on the name asks the popover
    /// about.
    pub from_address: Option<String>,
    pub unread: bool,
    /// Under the sender on a top hit: "you replied · 3 matches".
    pub reason: Option<String>,
    pub subject: Vec<RunFfi>,
    pub pills: Vec<crate::focus_list::LabelPillFfi>,
    pub attachments: bool,
    pub count_badge: Option<String>,
    /// "3 of 6 messages match", in the passage's place, when several of a
    /// thread's messages matched.
    pub summary: Option<String>,
    /// "body", "quoted text", "subject", a file's name.
    pub source_tag: String,
    pub source_is_file: bool,
    /// The passage, matched words in the find highlight; empty until read.
    pub passage: Vec<RunFfi>,
    pub folder: String,
    pub date: String,
    pub checked: bool,
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
            from_address: row.from_address,
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
            summary: row.summary,
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

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SearchWordsFfi {
    pub back: String,
    pub save: String,
    pub sort: String,
    pub best_match: String,
    pub newest: String,
    pub timeline_hint: String,
    pub understood_as: String,
    pub understood_note: String,
    pub query_label: String,
    pub suggestions_label: String,
    pub results_label: String,
    pub files_label: String,
    pub people_label: String,
}

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
        understood_as: words::UNDERSTOOD_AS.to_owned(),
        understood_note: words::UNDERSTOOD_NOTE.to_owned(),
        query_label: words::A11Y_QUERY.to_owned(),
        suggestions_label: words::A11Y_SUGGESTIONS.to_owned(),
        results_label: words::A11Y_RESULTS.to_owned(),
        files_label: words::A11Y_FILES.to_owned(),
        people_label: words::A11Y_PEOPLE.to_owned(),
    }
}

#[uniffi::export]
pub fn focus_search_row_accessible(
    title: String,
    detail: String,
    folder: Option<String>,
    right: Option<String>,
) -> String {
    postio_ui::search_view::dropdown_row_accessible(
        &title,
        &detail,
        folder.as_deref(),
        right.as_deref(),
    )
}

#[uniffi::export]
pub fn focus_search_checked(n: u64) -> String {
    postio_ui::search_view::checked_line(n)
}

#[uniffi::export]
impl Session {
    /// The arrows rest on the dropdown's row `token` now: what ⌥⌫ from a menu forgets.
    pub fn focus_search_highlighted(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::SearchHighlighted(token));
    }

    pub fn focus_search_forget(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::SearchForget(token));
    }

    /// ⌥↩ on the person, label or folder `token`: its chip, excluded.
    pub fn focus_search_exclude(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::SearchExclude(token));
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

    pub fn focus_search_date_words(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::DateWords(text));
    }

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

    pub fn focus_search_tab(&self, tab: ResultsTabFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsTab(tab.into()));
    }

    pub fn focus_search_order(&self, order: ConversationOrderFfi) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsOrder(order.into()));
    }

    pub fn focus_search_point(&self, position: u64) {
        self.focus_driver()
            .input(postio_focus::Input::ResultsPoint(position));
    }

    /// Save ↩ in the Save popover: the results' query is kept in
    /// `config.toml` under `name`, pinned, notifying and with its dates
    /// rolling as the switches say; the popover goes.
    pub fn focus_search_save(&self, name: String, pin: bool, notify: bool, rolling: bool) {
        self.focus_driver()
            .input(postio_focus::Input::SaveSearchAs {
                name,
                pin,
                notify,
                rolling,
            });
    }

    pub fn focus_search_row_count(&self) -> u64 {
        self.focus_driver().result_count()
    }

    /// The result at `position`, or `None` while its page is on its way: a miss asks for it, and
    /// `FocusResultsPage` says when. Synchronous.
    pub fn focus_search_row(&self, position: u64) -> Option<ResultRowFfi> {
        self.focus_driver().result_row(position).map(Into::into)
    }

    /// The Files tab's card at `position`, or `None` past the last. Synchronous.
    pub fn focus_search_file(&self, position: u64) -> Option<FileCardFfi> {
        self.focus_driver().result_file(position).map(Into::into)
    }

    /// The People tab's row at `position`, or `None` past the last. Synchronous.
    pub fn focus_search_person(&self, position: u64) -> Option<PersonRowFfi> {
        self.focus_driver().result_person(position).map(Into::into)
    }

    /// The system's Quick Look on a file, or its save panel, is gone (closed, saved or cancelled):
    /// the copy it was handed is removed. A panel the controller closed (`FocusFileCopy` with none)
    /// needs no report.
    pub fn focus_search_file_done(&self) {
        self.focus_driver().input(postio_focus::Input::FileCopyDone);
    }

    pub fn focus_search_placeholder(&self) -> String {
        postio_ui::search_view::PLACEHOLDER.to_owned()
    }

    pub fn focus_search_resting_placeholder(&self) -> String {
        postio_ui::search_view::RESTING_PLACEHOLDER.to_owned()
    }
}
