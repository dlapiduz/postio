//! Focus's command bar and folders popover. The controller (`postio_focus`, ADR 0045) decides what
//! typing means and what each line does; the Mac draws what it is told and reports the field's
//! words, a line run by token, and `Tab`.

use crate::mailbox::MailboxRoleFfi;
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BarModeFfi {
    Search,
    Commands,
}

impl From<postio_focus::BarMode> for BarModeFfi {
    fn from(mode: postio_focus::BarMode) -> Self {
        match mode {
            postio_focus::BarMode::Search => BarModeFfi::Search,
            postio_focus::BarMode::Commands => BarModeFfi::Commands,
        }
    }
}

/// A range of the field's characters to select: the chip being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct BarSelectFfi {
    /// The first character selected, counting characters, not bytes.
    pub start: u32,
    pub end: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BarLineKindFfi {
    Heading,
    Hint,
    Command,
    Correspondent,
}

impl From<postio_focus::BarLineKind> for BarLineKindFfi {
    fn from(kind: postio_focus::BarLineKind) -> Self {
        use postio_focus::BarLineKind as Kind;
        match kind {
            Kind::Heading => BarLineKindFfi::Heading,
            Kind::Hint => BarLineKindFfi::Hint,
            Kind::Command => BarLineKindFfi::Command,
            Kind::Correspondent => BarLineKindFfi::Correspondent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarLineFfi {
    pub kind: BarLineKindFfi,
    /// What [`Session::focus_bar_run`] takes to run it. Never reused: a
    /// token from lines since redrawn runs nothing.
    pub token: u64,
    pub title: String,
    pub detail: Option<String>,
    /// The key that does the same, as the keymap spells it (`g i`,
    /// `cmd+k`), or a hint's prefix (`>`, `@`).
    pub key: Option<String>,
    /// The registry command the keycap is for, when it is one's: what the
    /// Mac spells the keycap from.
    pub command: Option<String>,
    pub selectable: bool,
}

impl From<postio_focus::BarLine> for BarLineFfi {
    fn from(line: postio_focus::BarLine) -> Self {
        BarLineFfi {
            kind: line.kind.into(),
            token: line.token,
            title: line.title,
            detail: line.detail,
            key: line.key,
            command: line.command.map(|command| command.to_string()),
            selectable: line.selectable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarViewFfi {
    /// The line under the field: "You typed “…”".
    pub echo: Option<String>,
    pub lines: Vec<BarLineFfi>,
    /// The line the highlight goes to, by token. `None` keeps it where it
    /// is, or puts it on the first selectable line when it is nowhere.
    pub highlight: Option<u64>,
    /// The pinned saved searches' names, in order: the saved row. The
    /// `n`th runs `saved_search_<n>`.
    pub saved: Vec<String>,
}

impl From<postio_focus::BarView> for BarViewFfi {
    fn from(view: postio_focus::BarView) -> Self {
        BarViewFfi {
            echo: view.echo,
            lines: view.lines.into_iter().map(BarLineFfi::from).collect(),
            highlight: view.highlight,
            saved: view.saved,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PlaceMarkFfi {
    Role {
        role: MailboxRoleFfi,
    },
    Dot {
        /// Its stored colour, `#rrggbb`; `None` for one the frontend picks
        /// from the name.
        color: Option<String>,
    },
    /// Filtered's own mark: drawn with Archive's role it read as a second Archive.
    Filtered,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaceEntryFfi {
    /// What [`Session::focus_open_place`] takes to go there. Good until the
    /// places are read again (`FocusPlacesChanged`).
    pub token: u64,
    /// The section it is listed under: "Mailboxes", "Folders", "Labels".
    /// Entries come in section order, so a heading goes wherever this
    /// changes.
    pub section: String,
    pub name: String,
    /// What it says on the right: its conversations, or "186 today".
    pub count: Option<String>,
    pub mark: PlaceMarkFfi,
    pub command: Option<String>,
    pub footer: String,
}

impl PlaceEntryFfi {
    fn of(token: u64, entry: postio_ui::places::Entry) -> Self {
        let footer = postio_ui::places::footer(Some(&entry));
        PlaceEntryFfi {
            token,
            section: entry.section.title().to_owned(),
            mark: match entry.mark {
                _ if entry.command == Some(postio_core::CommandId::GoToFiltered) => {
                    PlaceMarkFfi::Filtered
                }
                postio_ui::places::Mark::Role(role) => PlaceMarkFfi::Role { role: role.into() },
                postio_ui::places::Mark::Dot(color) => PlaceMarkFfi::Dot { color },
            },
            command: entry.go.map(|command| command.to_string()),
            count: entry.count,
            name: entry.name,
            footer,
        }
    }
}

#[uniffi::export]
impl Session {
    /// The bar's field holds `text` now. The same words again change nothing.
    pub fn focus_bar_typed(&self, text: String) {
        self.focus_driver()
            .input(postio_focus::Input::Typed { text });
    }

    /// Run the bar's line with `token`: Return on the highlighted line, or a
    /// click on one. The bar closes itself when the line says so.
    pub fn focus_bar_run(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::BarRun(token));
    }

    /// `Tab` in the bar's field: into the chips, and on to the next.
    /// `false` when there is no chip to step into, and the key is the
    /// toolkit's.
    pub fn focus_bar_tab(&self) -> bool {
        self.focus_driver().bar_tab()
    }

    /// The folders popover's places whose names hold `filter`, case aside,
    /// in the order it lists them. Read now, from the places the last read
    /// found; `FocusPlacesChanged` says when to ask again.
    pub fn focus_places(&self, filter: String) -> Vec<PlaceEntryFfi> {
        self.focus_driver()
            .places(&filter)
            .into_iter()
            .map(|(token, entry)| PlaceEntryFfi::of(token, entry))
            .collect()
    }

    /// Go to the popover's place with `token`: the list shows it
    /// (`FocusPlace`, then the list's own events), or -- Filtered -- its
    /// view opens. The popover closes itself.
    pub fn focus_open_place(&self, token: u64) {
        self.focus_driver()
            .input(postio_focus::Input::OpenPlace(token));
    }

    pub fn focus_places_placeholder(&self) -> String {
        postio_ui::places::FILTER_PLACEHOLDER.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filtered_is_marked_apart_from_archive() {
        let filtered = PlaceEntryFfi::of(1, postio_ui::places::filtered_entry(3));
        assert_eq!(filtered.mark, PlaceMarkFfi::Filtered);
    }
}
