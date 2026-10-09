//! Focus's search at the boundary (specs/010-focus-search,
//! contracts/ffi-search.md).
//!
//! The dropdown is the controller's (`postio_focus`, ADR 0045): which state
//! it is in, what each row says and what running one does. The Mac draws
//! the `FocusDropdown` it is told and reports what happened -- the field's
//! words and a row run, through the bar's own exports (`focus_bar_typed`,
//! `focus_bar_run`, `focus_bar_tab`), and the dropdown's own keys here:
//! where the arrows rest (which asks for a person's latest), ⌥⌫ on a
//! recent search, ⌥↩ on a person, label or folder, and ⌘↩.
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
    /// One to three letters: the ghost and suggestions (screen 02).
    Prefix,
    /// An operator's value: people, labels or folders (screen 04). The
    /// field sets its text in SF Mono 14.
    Operator,
    /// A sentence lowered into operators: "Understood as" (screen 05).
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
    /// A completed word: Return or Tab puts it in the field.
    Word,
    /// A label: Return its chip, Option-Return its exclusion.
    Label,
    /// A mailing list.
    List,
    /// The files whose names match.
    File,
    /// A person: the initials avatar, Return the chip, Option-Return the
    /// exclusion.
    Person,
    /// A folder, for `in:`.
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
    /// A person's initials, for the round avatar.
    pub initials: Option<String>,
}

/// One tile of the "Understood as" bar (screen 05).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnderstoodTileFfi {
    /// The operator with its colon, tertiary ("from:"); empty for a word.
    pub op: String,
    /// Its value, or the word.
    pub value: String,
    /// "from ‘last month’", 10.5 pt under the term.
    pub origin: String,
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
    /// A saved search that notifies: its quiet badge ("3 new"), never a
    /// banner (D15).
    pub fresh: Option<String>,
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
    /// The rest of the best word, tertiary after the caret ("las").
    pub ghost: Option<String>,
    /// The "Understood as" bar's tiles, in plain English.
    pub understood: Vec<UnderstoodTileFfi>,
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
            ghost: view.ghost,
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
    /// "/ to edit", or "clears filters" after its key while nothing
    /// matches.
    pub hint: String,
    /// The hint's key as the registry spells it (`cmd+BackSpace`), drawn
    /// as a cap before it; `None` when the hint names its own.
    pub hint_key: Option<String>,
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
            hint_key: view.hint_key,
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
    /// The Files tab's header, while it is the tab shown; its cards are
    /// `focus_search_file`'s, `rows` of them.
    pub files: Option<FilesHeaderFfi>,
}

/// The Files tab's header over the grid (design §3.8).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FilesHeaderFfi {
    /// "Files whose name or contents match".
    pub title: String,
    /// What is searched inside files, and whether it is done.
    pub note: String,
}

/// How a file card's preview is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FilePreviewFfi {
    /// A sheet's grid.
    Sheet,
    /// A page's lines.
    Page,
    /// A slide.
    Slides,
    /// A picture.
    Image,
    /// Lines of text.
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

/// One card of the Files tab (design §3.8, screen 11).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileCardFfi {
    /// The file.
    pub attachment: i64,
    /// The message it came in.
    pub message: i64,
    /// The type tile: "XLSX".
    pub kind: String,
    /// How the preview is drawn.
    pub preview: FilePreviewFfi,
    /// Which of the preview's lines is marked, from the top; none when
    /// only the name matched.
    pub marked: Option<u32>,
    /// The name, matched words in the find highlight.
    pub name: Vec<RunFfi>,
    /// "Ada Moreno · 26 Sep · 48 KB".
    pub meta: String,
    /// "Sheet ‘Q3’, row 3: Total Atlas budget …", matched words in the
    /// find highlight; empty when only the name matched.
    pub line: Vec<RunFfi>,
    /// "in ‘Re: Atlas Q3 budget’".
    pub subject: String,
    /// The focus ring is on it.
    pub focused: bool,
    /// What VoiceOver reads.
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

/// A copy of a file for the system: Quick Look on it, or a save panel
/// (FR-053). The path is in the app's own temporary folder.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FileCopyFfi {
    /// The copy.
    pub path: String,
    /// The file's own name, for a save panel.
    pub name: String,
    /// A save panel, rather than Quick Look.
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
            files: view.files.map(|header| FilesHeaderFfi {
                title: header.title,
                note: header.note,
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

/// The Save popover (design §3.9, screen 12): every word it draws and the
/// switches as they start. 364 wide, from the toolbar's Save search.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SaveViewFfi {
    /// "Save as a saved search".
    pub title: String,
    /// "Name", over the field.
    pub name_label: String,
    /// The name offered, prefilled: "Atlas budget from Ada".
    pub name: String,
    /// The terms as small read-only chips: "from:Ada Moreno".
    pub chips: Vec<String>,
    /// Pin to saved searches, as it starts.
    pub pin: bool,
    /// "Pin to saved searches".
    pub pin_label: String,
    /// "Appears at the top of search as", before `pin_key`.
    pub pin_note: String,
    /// The key it will run on (`alt+3`); `None` past the fourth.
    pub pin_key: Option<String>,
    /// Notify when new mail matches, as it starts.
    pub notify: bool,
    /// "Notify when new mail matches".
    pub notify_label: String,
    /// "A quiet badge, not a banner".
    pub notify_note: String,
    /// Keep the date rolling, as it starts.
    pub rolling: bool,
    /// "Keep the date rolling".
    pub rolling_label: String,
    /// What each way means; `None` when the query has no date and the
    /// switch is not drawn.
    pub rolling_note: Option<String>,
    /// "Cancel".
    pub cancel: String,
    /// "Save".
    pub save: String,
    /// Save's keycap (`Return`).
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

/// One looser search on the no-results page (design §3.10).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RelaxationFfi {
    /// Its number, from 1.
    pub number: u32,
    /// The key that runs it (`1`), for its cap; `None` when unbound.
    pub key: Option<String>,
    /// What it changes: "Remove “before March”".
    pub label: String,
    /// The query it runs, drawn monospaced.
    pub query: String,
    /// "4 conversations".
    pub count: String,
    /// Ringed: Return runs it.
    pub focused: bool,
}

/// The page a search that found nothing shows in the rows' place (design
/// §3.10, screen 13).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoResultsViewFfi {
    /// "Nothing matches all four filters".
    pub title: String,
    /// The sentence under it.
    pub body: String,
    /// The looser searches, most first, at most four; empty while counted.
    pub relaxations: Vec<RelaxationFfi>,
    /// "Counting looser searches…" while their counts are on their way.
    pub counting: Option<String>,
    /// "Searched all 18,204 messages on this Mac."
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
    /// The plain-English bar's title (screen 05).
    pub understood_as: String,
    /// Its note on the right.
    pub understood_note: String,
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
        understood_as: words::UNDERSTOOD_AS.to_owned(),
        understood_note: words::UNDERSTOOD_NOTE.to_owned(),
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

    /// ⌥↩ on the person, label or folder `token`: its chip,
    /// excluded (spec 010 US7).
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

    /// The Files tab's card at `position`, or `None` past the last.
    /// Synchronous; what the grid calls for every visible card.
    pub fn focus_search_file(&self, position: u64) -> Option<FileCardFfi> {
        self.focus_driver().result_file(position).map(Into::into)
    }

    /// The system's Quick Look on a file, or its save panel, is gone --
    /// closed by the person, or the save done or cancelled: the copy it
    /// was handed is removed (FR-053). A panel the controller closed
    /// (`FocusFileCopy` with none) needs no report.
    pub fn focus_search_file_done(&self) {
        self.focus_driver().input(postio_focus::Input::FileCopyDone);
    }

    /// The field's placeholder while it is empty (screen 01).
    pub fn focus_search_placeholder(&self) -> String {
        postio_ui::search_view::PLACEHOLDER.to_owned()
    }
}
