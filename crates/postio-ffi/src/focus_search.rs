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

    /// ⌘↩, or a click on Show all: the results for what is typed. Until
    /// the results view arrives (spec 010 step 3) it keeps the query among
    /// the recent searches and moves the highlight to the first hit.
    pub fn focus_search_show_all(&self) {
        let _ = self
            .focus_driver()
            .command(postio_core::CommandId::ShowAllResults);
    }

    /// The field's placeholder while it is empty (screen 01).
    pub fn focus_search_placeholder(&self) -> String {
        postio_ui::search_view::PLACEHOLDER.to_owned()
    }
}
