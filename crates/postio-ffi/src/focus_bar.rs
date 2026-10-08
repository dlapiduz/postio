//! Focus's command bar and folders popover at the boundary
//! (specs/009-focus-macos T082, for the Mac's T084-T086).
//!
//! The bar is the controller's (`postio_focus`, ADR 0045): what typing
//! means, which lines it offers, what each says and what running one does.
//! The Mac draws what it is told and reports what happened -- the field's
//! words ([`Session::focus_bar_typed`]), a line run by its token
//! ([`Session::focus_bar_run`]), `Tab` ([`Session::focus_bar_tab`]) -- and
//! the bar opens and closes through the surface stack like every other
//! surface (`FocusOpenBar`, `FocusCloseSurface { kind: Bar }`,
//! `focus_surface_opened`/`focus_surface_closed`).
//!
//! The folders popover lists [`Session::focus_places`], read in one request
//! when it opens (`FocusOpenPlaces`, then `FocusPlacesChanged` once the read
//! lands), and goes to the one chosen with [`Session::focus_open_place`].

use crate::mailbox::MailboxRoleFfi;
use crate::session::Session;

/// How the bar was opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BarModeFfi {
    /// `/`: search, places and commands, blended.
    Search,
    /// `mod+k`: commands, with `>` typed (C24).
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
    /// One past the last.
    pub end: u32,
}

/// What kind of line the bar draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BarLineKindFfi {
    /// A section heading ("Commands", "Go to", "Conversations · 3 matches").
    Heading,
    /// A line of the empty bar saying what typing does.
    Hint,
    /// A command, with its key.
    Command,
    /// A place to go: `in:Receipts`.
    Place,
    /// "Search mail for …".
    Search,
    /// "Search instead for “word”".
    Instead,
    /// "Sorted by relevance", which switches the order.
    Order,
    /// A correspondent `@` offered.
    Correspondent,
    /// A message: a search hit, or a folder's conversation.
    Message,
}

impl From<postio_focus::BarLineKind> for BarLineKindFfi {
    fn from(kind: postio_focus::BarLineKind) -> Self {
        use postio_focus::BarLineKind as Kind;
        match kind {
            Kind::Heading => BarLineKindFfi::Heading,
            Kind::Hint => BarLineKindFfi::Hint,
            Kind::Command => BarLineKindFfi::Command,
            Kind::Place => BarLineKindFfi::Place,
            Kind::Search => BarLineKindFfi::Search,
            Kind::Instead => BarLineKindFfi::Instead,
            Kind::Order => BarLineKindFfi::Order,
            Kind::Correspondent => BarLineKindFfi::Correspondent,
            Kind::Message => BarLineKindFfi::Message,
        }
    }
}

/// One line of the bar, with everything it draws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarLineFfi {
    /// What kind of line.
    pub kind: BarLineKindFfi,
    /// What [`Session::focus_bar_run`] takes to run it. Never reused: a
    /// token from lines since redrawn runs nothing.
    pub token: u64,
    /// The title: a command's name, `in:Receipts`, a hit's subject.
    pub title: String,
    /// The dimmer detail beside it, or a hit's first line.
    pub detail: Option<String>,
    /// The key that does the same, as the keymap spells it (`g i`,
    /// `cmd+k`), or a hint's prefix (`>`, `@`).
    pub key: Option<String>,
    /// The registry command the keycap is for, when it is one's: what the
    /// Mac spells the keycap from.
    pub command: Option<String>,
    /// Whether the arrows may rest on it and Return run it.
    pub selectable: bool,
    /// A message's sender.
    pub sender: Option<String>,
    /// Where a message is: `in:Archive`, or where a digest holds it, and
    /// under it the account when there are several. One line each.
    pub wheres: Vec<String>,
    /// A message's time column: "16:02", "Tue", "12 Sep".
    pub time: Option<String>,
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
            sender: line.sender,
            wheres: line.wheres,
            time: line.time,
        }
    }
}

/// The bar, whole: everything `FocusBarLines` redraws.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarViewFfi {
    /// The heading over a folder's conversations (`in:`): "Receipts ·
    /// folder · 3 conversations · newest first".
    pub heading: Option<String>,
    /// The line under the field: "You typed “…”", and while a chip is
    /// edited, which one and the keys that move on and go back.
    pub echo: Option<String>,
    /// The chips the words were lowered to, drawn before the field.
    pub chips: Vec<String>,
    /// The chip being edited after `Tab`, drawn as such.
    pub editing: Option<u32>,
    /// The lines, top to bottom.
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
            heading: view.heading,
            echo: view.echo,
            chips: view.chips,
            editing: view.editing,
            lines: view.lines.into_iter().map(BarLineFfi::from).collect(),
            highlight: view.highlight,
            saved: view.saved,
        }
    }
}

/// What sits before a place's name.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PlaceMarkFfi {
    /// A mailbox's icon, by its role.
    Role {
        /// The role.
        role: MailboxRoleFfi,
    },
    /// A label's dot.
    Dot {
        /// Its stored colour, `#rrggbb`; `None` for one the frontend picks
        /// from the name.
        color: Option<String>,
    },
}

/// One place the folders popover lists.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaceEntryFfi {
    /// What [`Session::focus_open_place`] takes to go there. Good until the
    /// places are read again (`FocusPlacesChanged`).
    pub token: u64,
    /// The section it is listed under: "Mailboxes", "Folders", "Labels".
    /// Entries come in section order, so a heading goes wherever this
    /// changes.
    pub section: String,
    /// Its name.
    pub name: String,
    /// What it says on the right: its conversations, or "186 today".
    pub count: Option<String>,
    /// What sits before the name.
    pub mark: PlaceMarkFfi,
    /// The command that goes there directly, whose key its row shows.
    pub command: Option<String>,
    /// The popover's footer while this row is highlighted: what Return does.
    pub footer: String,
}

impl PlaceEntryFfi {
    fn of(token: u64, entry: postio_ui::places::Entry) -> Self {
        let footer = postio_ui::places::footer(Some(&entry));
        PlaceEntryFfi {
            token,
            section: entry.section.title().to_owned(),
            mark: match entry.mark {
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
    /// The bar's field holds `text` now. Said on every change; the same
    /// words again change nothing, so setting the field from `FocusOpenBar`
    /// and hearing it back is harmless.
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

    /// What the popover's filter says before anything is typed.
    pub fn focus_places_placeholder(&self) -> String {
        postio_ui::places::FILTER_PLACEHOLDER.to_owned()
    }
}
