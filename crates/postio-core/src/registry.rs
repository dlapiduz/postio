//! The command registry: one table every surface derives from (keymap, palette, cheat sheet, menus,
//! key hints).
//!
//! Defaults follow Gmail's keys where Postio has the command (docs/PRODUCT.md §8); a destructive
//! command with no `Recovery` fails the tests.

use crate::action::ActionId;
use crate::command::CommandId;
use crate::context::{Context, ContextSet};
use crate::state::Scope;

use postio_config::paths::Platform;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    None,
    /// A command claiming this must be something `UndoStack` can hold: an `UndoKind` exists for it.
    Undo,
    /// For a send: opening the draft cancels it. Not the undo stack, whose ten-minute expiry would
    /// shadow the archive beneath it and answer `u` with "too late" (#1481).
    Window,
    Confirm,
}

/// Data on the row, not a `fn` pointer, so every surface evaluates it through `reachable_in` and it
/// can be tested and printed. A new one is a variant, a line in `Availability`, and an arm in
/// `met_by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    SingleAccount,
    /// The window is on screen before the store is (#1114): a window with no store must not offer
    /// verbs that cannot run, and a key bound to one refuses out loud.
    StoreOpen,
    /// Only the terminal's composer has Markdown text and a preview; the desktop edits rich text in
    /// place.
    Terminal,
    /// A terminal draws text in its own font and colours, so zoom and darkening have nothing to act
    /// on.
    Graphical,
    /// Focus is the desktop app or the terminal. The one keymap reserves these keys in every app;
    /// only Focus offers them.
    Focus,
}

impl Requirement {
    pub const ALL: [Requirement; 5] = [
        Requirement::SingleAccount,
        Requirement::StoreOpen,
        Requirement::Terminal,
        Requirement::Graphical,
        Requirement::Focus,
    ];

    const fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// App requirements are settled at startup; the rest change while it runs.
    pub const fn is_about_the_app(self) -> bool {
        matches!(
            self,
            Requirement::Terminal | Requirement::Graphical | Requirement::Focus
        )
    }
}

/// A set, not an `Option`: `Move` needs both a single account and an open store. A set rather than
/// a closure, as with `ContextSet`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RequirementSet(u8);

impl RequirementSet {
    pub const NONE: RequirementSet = RequirementSet(0);

    pub const fn from_slice(requirements: &[Requirement]) -> RequirementSet {
        let mut bits = 0u8;
        let mut index = 0;
        while index < requirements.len() {
            bits |= requirements[index].bit();
            index += 1;
        }
        RequirementSet(bits)
    }

    pub const fn contains(self, requirement: Requirement) -> bool {
        self.0 & requirement.bit() != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn met_by(self, state: Availability) -> bool {
        Requirement::ALL
            .iter()
            .all(|need| !self.contains(*need) || need.met_by(state))
    }

    /// Asked by the keymap, so a key kept for another app binds nothing here. State is not the
    /// keymap's to judge, so only the requirements about the app are asked.
    pub fn offered_by(self, frontend: Frontend) -> bool {
        let app = Availability {
            frontend,
            ..Availability::open(Scope::Unified)
        };
        self.iter()
            .filter(|need| need.is_about_the_app())
            .all(|need| need.met_by(app))
    }

    pub fn iter(self) -> impl Iterator<Item = Requirement> {
        Requirement::ALL
            .into_iter()
            .filter(move |need| self.contains(*need))
    }
}

/// Every app reads the one registry and keymap; what differs is which commands an app offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frontend {
    Terminal,
    /// The Mac registers as this too.
    Focus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Availability {
    pub scope: Scope,
    /// `false` only between the first frame and the store landing (#1114).
    pub store_open: bool,
    pub frontend: Frontend,
}

impl Availability {
    pub fn open(scope: Scope) -> Availability {
        Availability {
            scope,
            store_open: true,
            frontend: Frontend::Focus,
        }
    }
}

impl Requirement {
    pub fn met_by(self, state: Availability) -> bool {
        match self {
            Requirement::SingleAccount => state.scope.is_single_account(),
            Requirement::StoreOpen => state.store_open,
            Requirement::Terminal => state.frontend == Frontend::Terminal,
            Requirement::Graphical => state.frontend != Frontend::Terminal,
            Requirement::Focus => matches!(state.frontend, Frontend::Focus | Frontend::Terminal),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    pub id: CommandId,
    pub title: &'static str,
    /// The built-in binding, in the untyped syntax the keymap resolver parses
    /// (`"a"`, `"A"`, `"ctrl+k"`, `"g g"`). Overridable via `[keys]`.
    pub default_binding: &'static str,
    /// Secondary bindings for the same command — the arrow keys beside `j`/`k`,
    /// `o` beside `Return`. Not overridable; `[keys]` replaces the primary.
    pub alternate_bindings: &'static [&'static str],
    pub contexts: ContextSet,
    pub destructive: bool,
    /// How the user gets back. Never [`Recovery::None`] when `destructive`.
    pub recovery: Recovery,
    /// Almost everything carries `StoreOpen`; `NONE` is the chrome, which means the same with an
    /// empty window as with a full one.
    pub requires: RequirementSet,
}

impl CommandSpec {
    pub fn available_in(&self, context: Context) -> bool {
        self.contexts.contains(context)
    }

    /// Every binding for this command, the default first.
    ///
    /// The cheat sheet shows the default; the resolver registers all of them.
    pub fn bindings(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.default_binding).chain(self.alternate_bindings.iter().copied())
    }
}

const fn ctx(contexts: &'static [Context]) -> ContextSet {
    ContextSet::from_slice(contexts)
}

const fn needs(requirements: &'static [Requirement]) -> RequirementSet {
    RequirementSet::from_slice(requirements)
}

const MAIL: RequirementSet = needs(&[Requirement::StoreOpen]);

const TERMINAL_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Terminal]);
const GRAPHICAL_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Graphical]);
/// Works on what only Postio Focus has. Every other app keeps the key free and offers nothing on
/// it.
const FOCUS_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Focus]);
const FOCUS_GRAPHICAL_MAIL: RequirementSet = needs(&[
    Requirement::StoreOpen,
    Requirement::Focus,
    Requirement::Graphical,
]);

const CHROME: RequirementSet = RequirementSet::NONE;

const MESSAGE_SURFACES: &[Context] = &[Context::List, Context::Conversation, Context::Reader];
const CAPTURE_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Capture,
];
/// Reply, reply-all and forward must resolve while a draft is open, or the key is swallowed and
/// looks unbound (#426).
const REPLY_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Composer,
];
/// Where extending a row selection means something: `LIST_SURFACES` minus the conversation pane,
/// where `J`/`K` walk the thread's messages instead (#1007).
const SELECTION_SURFACES: &[Context] = &[Context::List, Context::Reader, Context::Search];

const LIST_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Search,
];

/// `LIST_SURFACES` plus the folder list: `g i` must work wherever the keyboard is. Not
/// `ContextSet::ANY`, since `g` is a letter in the composer.
const GO_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Search,
    Context::Sidebar,
    Context::Calendar,
    Context::CalendarWindow,
];

const CALENDAR: &[Context] = &[Context::Calendar, Context::CalendarWindow];

const CONTACTS: &[Context] = &[Context::Contacts];

/// A row states only what differs; `id`, `title`, `default_binding` and `contexts` are always
/// written out.
const BASE: CommandSpec = CommandSpec {
    id: CommandId::NextMessage,
    title: "",
    default_binding: "",
    alternate_bindings: &[],
    contexts: ContextSet::ANY,
    destructive: false,
    recovery: Recovery::None,
    requires: MAIL,
};

static SPECS: &[CommandSpec] = &[
    CommandSpec {
        id: CommandId::NextMessage,
        title: "Next message",
        default_binding: "j",
        alternate_bindings: &["Down"],
        contexts: ctx(LIST_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results)
            .with(Context::Contacts)
            .with(Context::Outline)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::PrevMessage,
        title: "Previous message",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(LIST_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results)
            .with(Context::Contacts)
            .with(Context::Outline)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::FirstMessage,
        title: "First message",
        // A binding string spells a sequence with a space between chords; `gg` would read as a key
        // named "gg".
        default_binding: "g g",
        contexts: ctx(LIST_SURFACES)
            .with(Context::Results)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::LastMessage,
        title: "Last message",
        default_binding: "G",
        contexts: ctx(LIST_SURFACES)
            .with(Context::Results)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::OpenMessage,
        title: "Open message",
        // `Return` is what config.toml documents; `Right` belongs to `expand_thread` in Focus's
        // lists.
        default_binding: "Return",
        alternate_bindings: &["o"],
        contexts: ctx(&[Context::List, Context::Conversation, Context::Search])
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results)
            .with(Context::Outline)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleSelection,
        title: "Toggle selection",
        default_binding: "x",
        contexts: ctx(LIST_SURFACES)
            .with(Context::Results)
            .with(Context::Filtered)
            .with(Context::Contacts),
        // Changing what an action would hit changes no durable state.
        ..BASE
    },
    CommandSpec {
        id: CommandId::ExtendSelectionDown,
        title: "Extend selection down",
        default_binding: "J",
        alternate_bindings: &["shift+Down"],
        // No conversation: inside the pane `J`/`K` walk one thread's messages (#1007).
        contexts: ctx(SELECTION_SURFACES)
            .with(Context::Filtered)
            .with(Context::Contacts),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ExtendSelectionUp,
        title: "Extend selection up",
        default_binding: "K",
        alternate_bindings: &["shift+Up"],
        contexts: ctx(SELECTION_SURFACES)
            .with(Context::Filtered)
            .with(Context::Contacts),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SelectAll,
        title: "Select all",
        // `mod+a` stays for the hand that reaches for it from other applications.
        default_binding: "* a",
        alternate_bindings: &["mod+a", "X"],
        contexts: ctx(LIST_SURFACES)
            .with(Context::Results)
            .with(Context::Filtered)
            .with(Context::Contacts),
        ..BASE
    },
    CommandSpec {
        id: CommandId::PrevView,
        title: "Previous view",
        // `h` is Focus's remind key, and `Left` is `collapse_thread`'s.
        default_binding: "u",
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Back,
        title: "Back",
        default_binding: "Escape",
        contexts: ContextSet::ANY,
        requires: CHROME,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleResultOrder,
        // `alt+o`: the query holds the keyboard, so a bare `O` is a letter in it (#499).
        title: "Toggle result order",
        default_binding: "alt+o",
        contexts: ctx(&[Context::Search]).with(Context::Results),
        ..BASE
    },
    CommandSpec {
        id: CommandId::NextInConversation,
        title: "Next message in thread",
        // `j` walks conversations; these walk the messages of the open one. `]`/`[` are the
        // defaults,
        // and `n`/`p` still work.
        default_binding: "]",
        alternate_bindings: &["n", "alt+Down"],
        contexts: ctx(&[Context::Conversation, Context::Reader]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::PrevInConversation,
        title: "Previous message in thread",
        default_binding: "[",
        alternate_bindings: &["p", "alt+Up"],
        contexts: ctx(&[Context::Conversation, Context::Reader]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleFold,
        // `z` is Gmail's undo, so folding takes `Z`; plain `space` cost the key every reading
        // surface turns pages with (#1402).
        title: "Fold or unfold this message",
        default_binding: "Z",
        contexts: ctx(&[Context::Conversation]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ViewOriginal,
        title: "View original",
        // `mod`, not a literal `ctrl`: it is Command on a Mac (#669), and `platform_bindings.rs`
        // requires
        // the tables to differ only in the primary modifier. Not a bare letter: a rare gesture on a
        // surface of constant verbs.
        default_binding: "mod+o",
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleReaderView,
        title: "Reader view",
        // The composer's `Detach composer` has this key only where the composer is, so the two
        // never meet.
        default_binding: "mod+shift+o",
        // A terminal cannot tell `ctrl+shift+o` from `ctrl+o`, so it needs a key it can send.
        alternate_bindings: &["alt+o"],
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::DarkenMessage,
        title: "Darken this message",
        // `alt+d`: `D` stops digesting a sender under the one keymap. The title reads "Show as
        // sent" while darkened.
        default_binding: "alt+d",
        contexts: ctx(MESSAGE_SURFACES),
        requires: GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SwitchTreatment,
        title: "Show original or app colours",
        // The shifted open: shows the whole message as sent. Gmail binds no `O`.
        default_binding: "O",
        contexts: ctx(&[Context::Reader]),
        // Focus's open message is where the treatments are drawn; the three-pane readers draw
        // reader view and the terminal draws text.
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FindInMessage,
        title: "Find in message",
        default_binding: "mod+f",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Source),
        ..BASE
    },
    CommandSpec {
        id: CommandId::FindNext,
        title: "Next match",
        default_binding: "mod+g",
        alternate_bindings: &["F3"],
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::FindPrevious,
        title: "Previous match",
        // The composer's `mod+shift+g` never meets a message surface: compose takes the reading pane over.
        default_binding: "mod+shift+g",
        alternate_bindings: &["shift+F3"],
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ZoomIn,
        title: "Zoom in",
        // `mod+equal` because `+` is shifted on most layouts.
        default_binding: "mod+plus",
        alternate_bindings: &["mod+equal", "mod+KP_Add"],
        contexts: ctx(MESSAGE_SURFACES),
        requires: GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ZoomOut,
        title: "Zoom out",
        default_binding: "mod+minus",
        alternate_bindings: &["mod+KP_Subtract"],
        contexts: ctx(MESSAGE_SURFACES),
        requires: GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ZoomReset,
        title: "Actual size",
        default_binding: "mod+0",
        alternate_bindings: &["mod+KP_0"],
        contexts: ctx(MESSAGE_SURFACES),
        requires: GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ExpandAll,
        title: "Expand all",
        default_binding: ";",
        alternate_bindings: &["mod+shift+e"],
        contexts: ctx(&[Context::Conversation]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Reply,
        title: "Reply",
        default_binding: "r",
        alternate_bindings: &["mod+r"],
        contexts: ctx(REPLY_SURFACES).with(Context::Results),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ReplyAll,
        title: "Reply to all",
        default_binding: "a",
        alternate_bindings: &["mod+shift+r"],
        contexts: ctx(REPLY_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Forward,
        title: "Forward",
        default_binding: "f",
        alternate_bindings: &["mod+shift+f"],
        contexts: ctx(REPLY_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Archive,
        title: "Archive",
        default_binding: "e",
        alternate_bindings: &["mod+shift+a"],
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Results)
            .with(Context::Digest),
        destructive: true,
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Delete,
        title: "Delete",
        // `d` is Focus's digest key under the one keymap.
        default_binding: "#",
        alternate_bindings: &["Delete"],
        // Filtered too: nothing deletes automatically, but the person may delete what they see
        // there.
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Results)
            .with(Context::Filtered)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        destructive: true,
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Move,
        title: "Move to…",
        default_binding: "v",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        recovery: Recovery::Undo,
        // A unified view has no single account to hold a destination; unavailable beats a no-op
        // (#182, ADR 0005 Q4).
        requires: needs(&[Requirement::SingleAccount, Requirement::StoreOpen]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Flag,
        title: "Flag",
        default_binding: "s",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Filtered),
        recovery: Recovery::Undo,
        // A flag another client set must be clearable from every app.
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleRead,
        title: "Mark read or unread",
        // `ToggleRead` toggles, so both keys reach it: `I` reads an unread message, `U` unreads a
        // read one.
        default_binding: "I",
        alternate_bindings: &["U"],
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Results)
            .with(Context::Filtered),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Snooze,
        title: "Snooze",
        default_binding: "b",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Unsnooze,
        title: "Unsnooze",
        default_binding: "B",
        contexts: ctx(MESSAGE_SURFACES),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::RemindIfNoReply,
        title: "Remind if no reply…",
        default_binding: "h",
        alternate_bindings: &["mod+h"],
        contexts: ctx(REPLY_SURFACES),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::AddLabel,
        title: "Add label…",
        default_binding: "l",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::AcceptInvite,
        title: "Accept invitation",
        default_binding: "y",
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        // Undone inside a short window, like a send, not from the undo stack (research R9).
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DeclineInvite,
        title: "Decline invitation",
        default_binding: "Y",
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DigestRule,
        title: "Digest rule…",
        default_binding: "d",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Digest),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::StopDigestingSender,
        title: "Stop digesting this sender",
        default_binding: "D",
        contexts: ctx(&[Context::Digest, Context::Reader]),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ViewSource,
        title: "Source and verification",
        default_binding: "V",
        contexts: ctx(MESSAGE_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::OpenAttachmentOrLink,
        title: "Open attachment or link…",
        default_binding: "A",
        contexts: ctx(MESSAGE_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DismissMarker,
        title: "Dismiss marker",
        default_binding: "-",
        contexts: ctx(MESSAGE_SURFACES),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::MoreActions,
        title: "More actions",
        default_binding: ".",
        contexts: Context::Reader.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleReadingPane,
        title: "Read beside the list or over it",
        default_binding: "F8",
        contexts: ctx(&[Context::List]),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Search,
        title: "Search",
        default_binding: "/",
        alternate_bindings: &["alt+mod+f"],
        contexts: ctx(GO_SURFACES)
            .with(Context::Results)
            .with(Context::Outline),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveSearch,
        title: "Save search as folder",
        // Shares `mod+s` with `save_draft`: the contexts do not overlap.
        default_binding: "mod+s",
        contexts: Context::Search.as_set().with(Context::Results),
        ..BASE
    },
    CommandSpec {
        id: CommandId::BackToWords,
        title: "Back to words",
        default_binding: "mod+BackSpace",
        // A terminal delivers `ctrl+BackSpace` as plain backspace.
        alternate_bindings: &["alt+BackSpace"],
        // In the results, where nothing matched, the same key clears the filters rather than add a
        // second meaning.
        contexts: Context::Search.as_set().with(Context::Results),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ShowAllResults,
        title: "Show all results",
        // Where `mod+Return` sends in the composer: contexts do not overlap.
        default_binding: "mod+Return",
        contexts: Context::Search.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ForgetRecent,
        title: "Forget recent search",
        // Linux uses Alt+Delete because `alt+BackSpace` is the terminal's back to words
        // (`binding_offered_on`).
        default_binding: "alt+BackSpace",
        alternate_bindings: &["alt+Delete"],
        contexts: Context::Search.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ExcludeSuggestion,
        title: "Exclude suggestion",
        default_binding: "alt+Return",
        contexts: Context::Search.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::HistoryBack,
        title: "Back in history",
        default_binding: "mod+bracketleft",
        contexts: ctx(&[Context::List, Context::Results]),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::HistoryForward,
        title: "Forward in history",
        default_binding: "mod+bracketright",
        contexts: ctx(&[Context::List, Context::Results]),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ResultsConversations,
        title: "Show threads",
        default_binding: "mod+1",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ResultsFiles,
        title: "Show files",
        default_binding: "mod+2",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ResultsPeople,
        title: "Show people",
        default_binding: "mod+3",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilterFrom,
        title: "Filter by sender",
        default_binding: "alt+f",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilterTo,
        title: "Filter by recipient",
        default_binding: "alt+t",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilterDate,
        title: "Filter by date",
        default_binding: "alt+d",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilterAnywhere,
        title: "Filter by folder",
        default_binding: "alt+w",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilterLabel,
        title: "Filter by label",
        default_binding: "alt+l",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::StepRangeBack,
        title: "Earlier month",
        default_binding: "alt+Left",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::StepRangeForward,
        title: "Later month",
        default_binding: "alt+Right",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::QuickLook,
        title: "Quick Look",
        default_binding: "space",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::NextMatch,
        title: "Next match",
        default_binding: "]",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PrevMatch,
        title: "Previous match",
        default_binding: "[",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickRelaxation1,
        title: "Run looser search 1",
        default_binding: "1",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickRelaxation2,
        title: "Run looser search 2",
        default_binding: "2",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickRelaxation3,
        title: "Run looser search 3",
        default_binding: "3",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickRelaxation4,
        title: "Run looser search 4",
        default_binding: "4",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveFile,
        title: "Save file",
        default_binding: "mod+Down",
        contexts: Context::Results.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Compose,
        title: "Compose",
        default_binding: "c",
        alternate_bindings: &["mod+n"],
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Send,
        title: "Send",
        default_binding: "mod+Return",
        // `alt+s` first: a terminal delivers it everywhere, where many take `ctrl+Return` or
        // `alt+Return` for fullscreen.
        alternate_bindings: &["alt+s", "alt+Return"],
        contexts: Context::Composer.as_set(),
        // Not destructive — but it is externally visible and irreversible once
        // the queue drains, so it earns an undo-send window rather than a modal.
        recovery: Recovery::Window,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ScheduleSend,
        title: "Schedule send…",
        // Its own keystroke, not a modifier on Send's: it opens a picker rather than sending.
        default_binding: "mod+shift+Return",
        alternate_bindings: &["alt+S"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "mod+s",
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::DiscardDraft,
        title: "Discard draft",
        default_binding: "mod+d",
        contexts: Context::Composer.as_set(),
        // Typed prose has no other copy anywhere, so this one asks first.
        destructive: true,
        recovery: Recovery::Confirm,
        ..BASE
    },
    CommandSpec {
        id: CommandId::MarkSent,
        title: "Mark as sent",
        // Not palette-only: every command is reachable by keyboard (`command_registry.rs`). Not
        // `mod+shift+s`, which an extension binds in List (#495).
        default_binding: "mod+shift+m",
        alternate_bindings: &["alt+m"],
        contexts: ctx(&[Context::List, Context::Composer]),
        // No `Undo`: this settles a claim about the world, so the correction is to send again
        // (#674;
        // see `Actions::mark_sent`).
        ..BASE
    },
    CommandSpec {
        id: CommandId::RetrySend,
        title: "Retry send",
        // Moved off `mod+shift+r`: macOS gives Reply to all that chord, and Thunderbird's Reply All
        // resolves there on Freedesktop. Retry is Postio's own verb, so it yields.
        default_binding: "mod+shift+y",
        alternate_bindings: &["alt+r"],
        // List for the Outbox and Drafts; Composer because the same draft can be open there with
        // its failure showing (#1487).
        contexts: ctx(&[Context::List, Context::Composer]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::CancelSend,
        title: "Cancel send",
        default_binding: "mod+shift+x",
        alternate_bindings: &["alt+x"],
        contexts: ctx(&[Context::List, Context::Composer]),
        // Refused once the submission is in flight (ADR 0021); asking again is `RetrySend`.
        ..BASE
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "mod+shift+a",
        alternate_bindings: &["alt+a"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::DetachComposer,
        title: "Detach composer",
        // Not next to `ctrl+d`. Discard is the one composer verb that cannot
        // be undone, and a fat-fingered neighbour of it is a draft gone.
        default_binding: "mod+shift+o",
        alternate_bindings: &["alt+o"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::CopyFields,
        title: "Cc and Bcc",
        default_binding: "mod+shift+c",
        alternate_bindings: &["alt+c"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::InsertImage,
        title: "Insert image…",
        default_binding: "mod+shift+g",
        alternate_bindings: &["alt+g"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::EditExternally,
        title: "Edit in external editor",
        // `alt+e` went to `reply_to_latest`; a legacy terminal cannot deliver `ctrl+shift+e`
        // (`terminal::legacy_deliverable`), hence `alt+shift+e`.
        default_binding: "mod+shift+e",
        alternate_bindings: &["alt+shift+e"],
        contexts: Context::Composer.as_set(),
        requires: TERMINAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::TogglePreview,
        title: "Toggle preview",
        default_binding: "mod+shift+p",
        alternate_bindings: &["alt+p"],
        contexts: Context::Composer.as_set(),
        requires: TERMINAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Bold,
        title: "Bold",
        default_binding: "mod+b",
        // ctrl+b is the sidebar everywhere mail is read; the composer is not
        // a message surface, so the convention every editor shares wins here.
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Italic,
        title: "Italic",
        default_binding: "mod+i",
        alternate_bindings: &["alt+i"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Heading1,
        title: "Heading 1",
        // The editors' `mod+alt+<level>` shelf; not `mod+shift+<n>`, which on a Mac is a screenshot.
        // A terminal cannot send `ctrl+alt+1` (`terminal::legacy_deliverable`), so it gets `alt+1`.
        default_binding: "mod+alt+1",
        alternate_bindings: &["alt+1"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Heading2,
        title: "Heading 2",
        default_binding: "mod+alt+2",
        alternate_bindings: &["alt+2"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Heading3,
        title: "Heading 3",
        default_binding: "mod+alt+3",
        alternate_bindings: &["alt+3"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Heading4,
        title: "Heading 4",
        default_binding: "mod+alt+4",
        alternate_bindings: &["alt+4"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Heading5,
        title: "Heading 5",
        default_binding: "mod+alt+5",
        alternate_bindings: &["alt+5"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Heading6,
        title: "Heading 6",
        default_binding: "mod+alt+6",
        alternate_bindings: &["alt+6"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::NormalText,
        title: "Normal text",
        // Docs' paragraph style key, on the heading shelf (spec 014 R7).
        default_binding: "mod+alt+0",
        alternate_bindings: &["alt+0"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::BulletList,
        title: "Bulleted list",
        default_binding: "mod+shift+8",
        alternate_bindings: &["alt+8"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::NumberedList,
        title: "Numbered list",
        default_binding: "mod+shift+7",
        alternate_bindings: &["alt+7"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::InsertLink,
        title: "Insert link…",
        // Everywhere else this is ctrl+k, and here ctrl+k is the palette —
        // which is universal or it is not a palette. Shift is the tax.
        default_binding: "mod+shift+k",
        alternate_bindings: &["alt+k"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::QuoteBlock,
        title: "Quote block",
        default_binding: "mod+shift+9",
        alternate_bindings: &["alt+9"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Strikethrough,
        title: "Strikethrough",
        // Docs' alt+shift+5 is heading 5's terminal key, and Slack's mod+shift+x is Cancel send here (spec 014 R7).
        default_binding: "mod+shift+s",
        alternate_bindings: &["alt+asciitilde"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::InlineCode,
        title: "Inline code",
        // GitHub's.
        default_binding: "mod+e",
        alternate_bindings: &["alt+grave"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::CodeBlock,
        title: "Code block",
        default_binding: "mod+alt+c",
        alternate_bindings: &["alt+C"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::TaskList,
        title: "Task list",
        // Beside the lists' and the quote's mod+shift+7 to 9.
        default_binding: "mod+shift+6",
        alternate_bindings: &["alt+t"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleTask,
        title: "Done or not done",
        default_binding: "mod+alt+x",
        alternate_bindings: &["alt+X"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Indent,
        title: "Indent",
        // Docs' indent keys.
        default_binding: "mod+bracketright",
        alternate_bindings: &["alt+greater"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Outdent,
        title: "Outdent",
        default_binding: "mod+bracketleft",
        alternate_bindings: &["alt+less"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Divider,
        title: "Divider",
        default_binding: "mod+alt+minus",
        alternate_bindings: &["alt+minus"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Table,
        title: "Table",
        // Not mod+alt+t, which is GNOME's terminal.
        default_binding: "mod+shift+t",
        alternate_bindings: &["alt+bar"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ClearFormatting,
        title: "Clear formatting",
        // Docs' clear formatting.
        default_binding: "mod+backslash",
        alternate_bindings: &["alt+backslash"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleMarkdownSource,
        title: "Edit as Markdown or rich text",
        // The terminal's body is Markdown already, so it has nothing to switch to.
        default_binding: "mod+alt+s",
        contexts: Context::Composer.as_set(),
        requires: GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::AttachRemoteImage,
        title: "Download and attach image",
        // Fetches only on this press (spec 014 FR-031).
        default_binding: "mod+alt+i",
        alternate_bindings: &["alt+I"],
        contexts: Context::Composer.as_set(),
        ..BASE
    },
    // -- View and application --------------------------------------------
    CommandSpec {
        id: CommandId::Undo,
        title: "Undo",
        // `mod+z` too; a terminal delivers it as `ctrl+z` and nothing there suspends.
        default_binding: "z",
        alternate_bindings: &["mod+z"],
        // Also the account list: removal is a soft delete with a toast wired to
        // `AccountRepository::restore`,
        // so undo reaches it while the toast is up (#464, ADR 0005 Q6c). Digest, Filtered and
        // Contacts
        // edits are undoable on the global stack, so `z` must work there.
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Accounts)
            .with(Context::Digest)
            .with(Context::Filtered)
            .with(Context::Contacts)
            .with(Context::ContactPopover)
            .with(Context::Results)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        ..BASE
    },
    CommandSpec {
        id: CommandId::CommandPalette,
        title: "Command palette",
        default_binding: "mod+k",
        contexts: ContextSet::ANY,
        requires: CHROME,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CheatSheet,
        title: "Keyboard shortcuts",
        default_binding: "?",
        // Not while composing or searching: there `?` is a character.
        contexts: ctx(MESSAGE_SURFACES),
        requires: CHROME,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Settings,
        title: "Settings",
        default_binding: "mod+comma",
        alternate_bindings: &["alt+comma"],
        contexts: ContextSet::ANY,
        ..BASE
    },
    CommandSpec {
        id: CommandId::AddAccount,
        title: "Add account",
        // Not a letter: a rare act, and every unmodified message-surface key is a verb used dozens
        // of times a session.
        default_binding: "mod+shift+n",
        alternate_bindings: &["alt+n"],
        // Without Contacts, where `mod+shift+n` is New contact.
        contexts: ContextSet::ANY.without(Context::Contacts),
        ..BASE
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "mod+e",
        contexts: ctx(MESSAGE_SURFACES),
        requires: CHROME,
        ..BASE
    },
    CommandSpec {
        id: CommandId::Quit,
        title: "Quit Postio",
        default_binding: "mod+q",
        contexts: ContextSet::ANY,
        requires: CHROME,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ShowImages,
        title: "Show remote images",
        // The bare `i` is the contact popover's, so images take the modified pair.
        default_binding: "alt+i",
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::AlwaysShowImages,
        title: "Always show images from this sender",
        default_binding: "alt+I",
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::Unsubscribe,
        title: "Unsubscribe from this list",
        // Shifted and deliberate: an unsubscribe tells the sender the address is read.
        default_binding: "M",
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Digest)
            .with(Context::Contacts),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ClearListMail,
        title: "Clear what the list sent",
        // `C` for clear: a shifted letter spells a deliberate act, pressed only after `M` and a
        // look at the count (#1953).
        default_binding: "C",
        contexts: ctx(MESSAGE_SURFACES).with(Context::Digest),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToFolders,
        title: "Go to folders",
        default_binding: "g l",
        contexts: ctx(LIST_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToInbox,
        title: "Go to inbox",
        // Targets the role, not a name, so an inbox a provider names otherwise is still where `g i`
        // goes.
        default_binding: "g i",
        contexts: ctx(GO_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToDrafts,
        title: "Go to drafts",
        default_binding: "g d",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToSent,
        title: "Go to sent",
        default_binding: "g t",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToFlagged,
        title: "Go to flagged",
        default_binding: "g s",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToArchive,
        title: "Go to archive",
        default_binding: "g a",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToSnoozed,
        title: "Go to snoozed",
        default_binding: "g b",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToOutbox,
        title: "Go to outbox",
        default_binding: "g o",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToJunk,
        title: "Go to junk",
        default_binding: "g j",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToTrash,
        title: "Go to trash",
        default_binding: "g #",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToFiltered,
        title: "Go to Filtered",
        default_binding: "g f",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToDigestRules,
        title: "Go to digest rules",
        default_binding: "g r",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SavedSearch1,
        title: "Saved search 1",
        default_binding: "alt+1",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SavedSearch2,
        title: "Saved search 2",
        default_binding: "alt+2",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SavedSearch3,
        title: "Saved search 3",
        default_binding: "alt+3",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SavedSearch4,
        title: "Saved search 4",
        default_binding: "alt+4",
        contexts: ctx(GO_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleHasAction,
        title: "Show only what has an action",
        default_binding: "!",
        contexts: ctx(&[Context::List]).with(Context::Results),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleAccountEnabled,
        title: "Enable or disable account",
        default_binding: "Return",
        contexts: ctx(&[Context::Accounts]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::RemoveAccount,
        title: "Remove account",
        default_binding: "Delete",
        contexts: ctx(&[Context::Accounts]),
        destructive: true,
        // Removal is a soft delete with a toast wired to `AccountRepository::restore`, so there is
        // something to undo while the toast is up (#464).
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::UpdateCredential,
        title: "Update account credential",
        default_binding: "c",
        contexts: ctx(&[Context::Accounts]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::RebuildAccountIndex,
        title: "Rebuild search index",
        default_binding: "r",
        contexts: ctx(&[Context::Accounts]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::SetDefaultAccount,
        // "Default", never "primary" or "main": those suggest this account is more the user's
        // (#960).
        title: "Set as default account",
        default_binding: "m",
        contexts: ctx(&[Context::Accounts]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::MapMailboxRole,
        title: "Map mailbox role",
        default_binding: "M",
        contexts: ctx(&[Context::Accounts]),
        recovery: Recovery::Undo,
        ..BASE
    },
    CommandSpec {
        id: CommandId::NextScope,
        title: "Next scope",
        default_binding: "g n",
        contexts: ctx(&[Context::Sidebar, Context::List]),
        // Deliberately not `SingleAccount`: this is the command that *leaves*
        // a single-account scope, so requiring one would switch itself off.
        ..BASE
    },
    CommandSpec {
        id: CommandId::Refresh,
        title: "Refresh",
        // Also the retry for empty and error states: "retry now" and "check now" are one command.
        default_binding: "F5",
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    // -- Reader --------------------------------------------------------
    CommandSpec {
        id: CommandId::ScrollReaderDown,
        title: "Scroll reading pane down",
        default_binding: "Page_Down",
        alternate_bindings: &["space"],
        // The conversation is here too so `Page_Up` and `Page_Down` serve the same surfaces
        // (#1402).
        contexts: ctx(&[Context::List, Context::Reader, Context::Conversation]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::ScrollReaderUp,
        title: "Scroll reading pane up",
        default_binding: "Page_Up",
        alternate_bindings: &["shift+space"],
        contexts: ctx(MESSAGE_SURFACES),
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerChoose1,
        title: "Choose option 1",
        default_binding: "1",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerChoose2,
        title: "Choose option 2",
        default_binding: "2",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerChoose3,
        title: "Choose option 3",
        default_binding: "3",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerChoose4,
        title: "Choose option 4",
        default_binding: "4",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerTypeDate,
        title: "Type a date",
        default_binding: "tab",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerToggle,
        title: "Toggle option",
        default_binding: "space",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PickerConfirm,
        title: "Confirm",
        default_binding: "Return",
        contexts: Context::Picker.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::NextReference,
        title: "Next reference",
        default_binding: "]",
        contexts: Context::Digest.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PrevReference,
        title: "Previous reference",
        default_binding: "[",
        contexts: Context::Digest.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ToggleDigestSummary,
        title: "Summary or messages",
        default_binding: "tab",
        contexts: Context::Digest.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::RestoreFiltered,
        title: "Restore to inbox",
        default_binding: "R",
        contexts: Context::Filtered.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab1,
        title: "Reason 1",
        default_binding: "1",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab2,
        title: "Reason 2",
        default_binding: "2",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab3,
        title: "Reason 3",
        default_binding: "3",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab4,
        title: "Reason 4",
        default_binding: "4",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab5,
        title: "Reason 5",
        default_binding: "5",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab6,
        title: "Reason 6",
        default_binding: "6",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FilteredTab7,
        title: "Reason 7",
        default_binding: "7",
        contexts: Context::Filtered.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SourceTabVerification,
        title: "Verification",
        default_binding: "1",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SourceTabHeaders,
        title: "Headers",
        default_binding: "2",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SourceTabRaw,
        title: "Raw",
        default_binding: "3",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CopySource,
        title: "Copy all",
        default_binding: "mod+c",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveSource,
        title: "Save as .eml…",
        default_binding: "mod+s",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::UnfoldPart,
        title: "Show",
        default_binding: "Return",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::AcceptKey,
        title: "Accept this key",
        default_binding: "a",
        contexts: Context::Source.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ForgetKey,
        title: "Forget this key",
        default_binding: "x",
        contexts: Context::Source.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ImportKey,
        title: "Import key…",
        default_binding: "i",
        contexts: Context::Source.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SweepInbox,
        title: "Filter what is in the inbox…",
        default_binding: "F",
        contexts: Context::List.as_set(),
        destructive: true,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CaptureTask,
        title: "Capture a task…",
        default_binding: "T",
        contexts: ctx(CAPTURE_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CaptureNote,
        title: "Capture a note…",
        default_binding: "N",
        contexts: ctx(CAPTURE_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CaptureChangeProject,
        title: "Change project",
        default_binding: "mod+p",
        contexts: Context::Capture.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CaptureUseSubject,
        title: "Use the subject instead",
        default_binding: "alt+s",
        contexts: Context::Capture.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CaptureWrite,
        title: "Add to the vault",
        default_binding: "mod+Return",
        alternate_bindings: &["alt+Return"],
        contexts: Context::Capture.as_set(),
        // It appends a line to a note on this computer, which the person
        // can delete there; Postio never edits a note beyond appending.
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DigestLikeThis,
        title: "Digest mail like this",
        // Whether the dialog offers it at all depends on `[focus.model]`, which the registry does
        // not
        // read (`ModelFeature::LikeThis`).
        default_binding: "L",
        contexts: Context::List.as_set(),
        requires: FOCUS_MAIL,
        ..BASE
    },
    // Focus's, drawn: the terminal has no Contacts surface.
    CommandSpec {
        id: CommandId::OpenContacts,
        title: "Contacts",
        default_binding: "g c",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactPopover,
        title: "Contact popover",
        default_binding: "i",
        // Not the search bar: `i` there is a letter being typed.
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Results)
            .with(Context::Contacts),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactOpenCard,
        title: "Open card",
        default_binding: "mod+i",
        contexts: Context::ContactPopover.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactAdd,
        title: "Add contact",
        default_binding: "mod+Return",
        contexts: Context::ContactPopover.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactIgnore,
        title: "Ignore sender",
        default_binding: "mod+BackSpace",
        contexts: Context::ContactPopover.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactArrivalPrev,
        title: "Arrival: previous",
        default_binding: "Left",
        contexts: Context::ContactPopover.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactArrivalNext,
        title: "Arrival: next",
        default_binding: "Right",
        contexts: Context::ContactPopover.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactSetArrival,
        title: "Choose where this sender's mail arrives",
        default_binding: "0",
        contexts: Context::ContactPopover.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactShowMail,
        title: "Show mail",
        default_binding: "Return",
        alternate_bindings: &["o"],
        contexts: ctx(CONTACTS).with(Context::ContactPopover),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactCompose,
        title: "Write to",
        default_binding: "c",
        contexts: ctx(CONTACTS).with(Context::ContactPopover),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsFilter,
        title: "Filter contacts",
        default_binding: "/",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsTabPrevious,
        title: "Previous kind tab",
        default_binding: "[",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsTabNext,
        title: "Next kind tab",
        default_binding: "]",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsToggleEveryone,
        title: "Everyone from mail",
        default_binding: "v e",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactJoin,
        title: "Merge",
        // `J` extends the selection being merged and `x` marks it; the id keeps 005's word, "join",
        // while the screen says Merge.
        default_binding: "m",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactAddAddress,
        title: "Add address",
        default_binding: "plus",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactDetachAddress,
        title: "Detach address",
        // Bare `-` rather than `minus`: the Mac draws both as one accelerator and its menu test
        // rejects two spellings.
        default_binding: "-",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactSetPreferred,
        title: "Use this address first",
        // Not `*`, the first key of Select all's `* a`, which would wait for a second key.
        default_binding: "P",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactNew,
        title: "New contact",
        // Contacts is excluded from `add_account`, which holds `mod+shift+n` in every other
        // context.
        default_binding: "mod+shift+n",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactEdit,
        title: "Edit contact",
        default_binding: "mod+e",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactSave,
        title: "Save contact",
        default_binding: "mod+s",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactDelete,
        title: "Delete",
        default_binding: "d",
        alternate_bindings: &["Delete"],
        contexts: ctx(CONTACTS),
        destructive: true,
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ContactRestore,
        title: "Restore contact",
        default_binding: "r",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsToggleDeleted,
        title: "Deleted contacts",
        default_binding: "v d",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsSuggestions,
        title: "Review contacts",
        default_binding: "g w",
        contexts: ctx(GO_SURFACES).with(Context::Contacts),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SuggestionDismiss,
        title: "Not the same person",
        // Not `X`, which is also Select all's alternate.
        default_binding: "n",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactGroupNew,
        title: "New group",
        default_binding: "g n",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactGroupRename,
        title: "Rename group",
        default_binding: "R",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactGroupAdd,
        title: "Add to group",
        default_binding: "l",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactGroupRemove,
        title: "Remove from group",
        default_binding: "L",
        contexts: ctx(CONTACTS),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsImport,
        title: "Import contacts",
        default_binding: "v i",
        contexts: ctx(CONTACTS),
        // Import only adds, and its summary says what it merged; there is no unit to take back.
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsExport,
        title: "Export contacts",
        default_binding: "v x",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ContactsSync,
        title: "Sync contacts",
        default_binding: "v y",
        contexts: ctx(CONTACTS),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    // Drawn as pixels: the terminal app keeps its own thread view, so it offers none and keeps the
    // keys free.
    CommandSpec {
        id: CommandId::ExpandThread,
        title: "Expand thread",
        default_binding: "Right",
        contexts: ctx(&[Context::List, Context::Results]).with(Context::Filtered),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CollapseThread,
        title: "Collapse thread",
        default_binding: "Left",
        contexts: ctx(&[Context::List, Context::Results]).with(Context::Filtered),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FirstInConversation,
        title: "First message in thread",
        default_binding: "{",
        contexts: Context::Reader.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::LatestInConversation,
        title: "Latest message in thread",
        default_binding: "}",
        contexts: Context::Reader.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::FirstUnreadInConversation,
        title: "First unread message in thread",
        default_binding: "g u",
        contexts: ctx(&[Context::Reader, Context::Outline]),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::OutlineFirstUnread,
        title: "First unread message",
        default_binding: "u",
        contexts: Context::Outline.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ThreadOutline,
        title: "Thread outline",
        default_binding: "alt+t",
        contexts: ctx(&[Context::Reader, Context::List]),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ThreadMenu,
        title: "Thread menu",
        default_binding: "alt+period",
        contexts: Context::Reader.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ReplyToLatest,
        title: "Reply to latest message",
        // Not the terminal: there `alt+e` is the composer's external editor.
        default_binding: "alt+e",
        contexts: ctx(&[
            Context::List,
            Context::Reader,
            Context::Results,
            Context::Composer,
        ]),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::MuteThread,
        title: "Mute thread",
        default_binding: "m",
        contexts: ctx(&[Context::List, Context::Reader, Context::Results]),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SplitThread,
        title: "Split into its own thread",
        default_binding: "alt+shift+s",
        contexts: Context::Reader.as_set(),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::MoveThisMessage,
        title: "Move this message",
        default_binding: "alt+v",
        contexts: ctx(&[Context::Reader, Context::List, Context::Results]),
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DeleteThisMessage,
        title: "Delete this message",
        default_binding: "alt+BackSpace",
        alternate_bindings: &["alt+Delete"],
        contexts: ctx(&[Context::Reader, Context::List, Context::Results]),
        destructive: true,
        recovery: Recovery::Undo,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ForwardWhat1,
        title: "Forward this message",
        default_binding: "1",
        contexts: Context::ForwardChooser.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ForwardWhat2,
        title: "Forward the whole thread",
        default_binding: "2",
        contexts: Context::ForwardChooser.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ForwardWhat3,
        title: "Forward the attachments",
        default_binding: "3",
        contexts: Context::ForwardChooser.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ShowNewer,
        title: "Show newer messages",
        // Not the terminal: the reply footer is drawn as pixels.
        default_binding: "mod+shift+Down",
        contexts: Context::Composer.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::ExpandQuoted,
        title: "Expand quoted text",
        default_binding: "mod+shift+q",
        contexts: Context::Composer.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::GoToCalendar,
        title: "Calendar",
        default_binding: "g e",
        contexts: ctx(GO_SURFACES),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CalendarAgenda,
        title: "Agenda",
        default_binding: "1",
        contexts: ctx(CALENDAR),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CalendarPrevious,
        title: "Earlier",
        default_binding: "[",
        contexts: ctx(CALENDAR),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CalendarNext,
        title: "Later",
        default_binding: "]",
        contexts: ctx(CALENDAR),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CalendarToday,
        title: "Today",
        default_binding: "t",
        contexts: ctx(CALENDAR),
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PopOut,
        title: "Open in its own window",
        default_binding: "mod+shift+o",
        contexts: Context::Calendar.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::PopIn,
        title: "Back into the main window",
        default_binding: "mod+shift+o",
        contexts: Context::CalendarWindow.as_set(),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::CloseWindow,
        title: "Close window",
        default_binding: "mod+w",
        contexts: ContextSet::ANY,
        requires: needs(&[Requirement::Graphical]),
        ..BASE
    },
    CommandSpec {
        id: CommandId::NewEvent,
        title: "New event",
        default_binding: "c",
        contexts: ctx(CALENDAR),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::EditEvent,
        title: "Edit event",
        default_binding: "mod+e",
        contexts: ctx(CALENDAR),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::MoveEvent,
        title: "Move to…",
        default_binding: "v",
        contexts: ctx(CALENDAR),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::DuplicateEvent,
        title: "Duplicate",
        // Not `mod+d`, which discards a draft in the composer.
        default_binding: "mod+shift+d",
        alternate_bindings: &["alt+D"],
        contexts: ctx(CALENDAR),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::TentativeInvite,
        title: "Accept tentatively",
        default_binding: "alt+y",
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Calendar)
            .with(Context::CalendarWindow),
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveEvent,
        title: "Save",
        default_binding: "mod+s",
        contexts: ctx(CALENDAR),
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::SaveAndSendEvent,
        title: "Save and send",
        default_binding: "mod+Return",
        alternate_bindings: &["alt+Return"],
        contexts: ctx(CALENDAR),
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
        ..BASE
    },
    CommandSpec {
        id: CommandId::EmailAttendees,
        title: "Email attendees",
        default_binding: "alt+e",
        contexts: ctx(CALENDAR),
        requires: FOCUS_GRAPHICAL_MAIL,
        ..BASE
    },
];

pub fn all() -> impl Iterator<Item = &'static CommandSpec> {
    SPECS.iter()
}

pub fn for_context(context: Context) -> impl Iterator<Item = &'static CommandSpec> {
    SPECS.iter().filter(move |spec| spec.available_in(context))
}

pub fn title_in(id: CommandId, context: Context) -> &'static str {
    let calendar = matches!(context, Context::Calendar | Context::CalendarWindow);
    match id {
        CommandId::OpenMessage if calendar => "Open event",
        CommandId::Delete if calendar => "Delete event",
        CommandId::AcceptInvite if calendar => "Accept",
        CommandId::DeclineInvite if calendar => "Decline",
        CommandId::Search if calendar => "Search events",
        _ => get(id).title,
    }
}

/// The spec for one command. Total: every [`CommandId`] has exactly one row.
pub fn get(id: CommandId) -> &'static CommandSpec {
    let spec = &SPECS[id as usize];
    debug_assert_eq!(spec.id, id, "the registry table is out of order");
    if spec.id == id {
        return spec;
    }
    SPECS
        .iter()
        .find(|spec| spec.id == id)
        .expect("every CommandId has a registry entry")
}

/// User overrides from `[keys]` are applied on top of this, not here.
pub fn lookup_binding(context: Context, binding: &str) -> Option<&'static CommandSpec> {
    lookup_binding_on(context, binding, Platform::host())
}

/// `binding` is a concrete accelerator from a key press, so the candidates are expanded, not it:
/// `ctrl+k` on a Mac must not match a `mod+k` default.
pub fn lookup_binding_on(
    context: Context,
    binding: &str,
    platform: Platform,
) -> Option<&'static CommandSpec> {
    for_context(context).find(|spec| {
        spec.bindings()
            .any(|candidate| match candidate.contains("mod+") {
                true => postio_config::keys::expand_mod(candidate, platform) == binding,
                false => candidate == binding,
            })
    })
}

/// `Some` names the command the rebind would silently shadow in a context the two share (#881).
/// Scoped by context: commands may share a binding in disjoint contexts.
pub fn binding_conflict(
    command: CommandId,
    proposed: &str,
    bindings: &postio_config::KeyBindings,
    platform: Platform,
) -> Option<&'static CommandSpec> {
    let mine = get(command);
    let expanded = postio_config::keys::expand_mod(proposed, platform);
    all().find(|other| {
        other.id != command
            && mine.contexts.intersects(other.contexts)
            && binding_in_force(other, bindings, platform) == expanded
    })
}

/// Built here, not asked of `postio-config`'s `KeyBindings`: it cannot see this registry and
/// answered
/// from a short table, so most commands read as unbound (#1227).
fn binding_in_force(
    spec: &CommandSpec,
    bindings: &postio_config::KeyBindings,
    platform: Platform,
) -> String {
    let raw = bindings
        .overrides()
        .get(spec.id.as_str())
        .map(String::as_str)
        .unwrap_or(spec.default_binding);
    postio_config::keys::expand_mod(raw, platform)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    pub id: ActionId,
    pub title: &'static str,
    pub default_binding: Option<&'static str>,
    pub alternate_bindings: &'static [&'static str],
    pub contexts: ContextSet,
    pub destructive: bool,
    pub recovery: Recovery,
    pub requires: RequirementSet,
}

impl ActionSpec {
    pub fn available_in(&self, context: Context) -> bool {
        self.contexts.contains(context)
    }

    pub fn bindings(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.default_binding
            .into_iter()
            .chain(self.alternate_bindings.iter().copied())
    }
}

impl From<&'static CommandSpec> for ActionSpec {
    fn from(spec: &'static CommandSpec) -> Self {
        ActionSpec {
            id: ActionId::Builtin(spec.id),
            title: spec.title,
            default_binding: Some(spec.default_binding),
            alternate_bindings: spec.alternate_bindings,
            contexts: spec.contexts,
            destructive: spec.destructive,
            recovery: spec.recovery,
            requires: spec.requires,
        }
    }
}

/// `reachable` stays state-blind because `docs/keybindings.md` documents the whole vocabulary.
pub fn reachable(context: Context) -> impl Iterator<Item = ActionSpec> {
    for_context(context).map(ActionSpec::from)
}

/// For a command whose surface a platform's design does not have, so a menu item would be a key
/// that
/// does nothing (#1571, #1573). Every surface asks this, so a platform answers once.
pub fn offered_on(action: ActionId, platform: Platform) -> bool {
    use CommandId as C;
    !matches!(
        (action, platform),
        // The Mac's compose is a window of its own, so there is nothing to detach.
        (ActionId::Builtin(C::DetachComposer), Platform::Apple)
            // The Mac's sidebar lists every account's folders at once; no account strip for `g a`
            // to cycle.
            | (ActionId::Builtin(C::NextScope), Platform::Apple)
            // The Mac reads mail in a web view with no contrast repair, so recolouring would break
            // the contrast floor (#1705).
            | (ActionId::Builtin(C::DarkenMessage), Platform::Apple)
            | (ActionId::Builtin(C::ContactsSync), Platform::Freedesktop)
            // AppKit's own Close takes ⌘W, and the Mac has no Calendar until its PostioKit side.
            | (
                ActionId::Builtin(
                    C::CloseWindow
                        | C::GoToCalendar
                        | C::CalendarAgenda
                        | C::CalendarPrevious
                        | C::CalendarNext
                        | C::CalendarToday
                        | C::PopOut
                        | C::PopIn
                        | C::NewEvent
                        | C::EditEvent
                        | C::MoveEvent
                        | C::DuplicateEvent
                        | C::TentativeInvite
                        | C::SaveEvent
                        | C::SaveAndSendEvent
                        | C::EmailAttendees
                ),
                Platform::Apple
            )
    )
}

/// `alt+BackSpace` is the Mac's `forget_recent` alone: the terminal reads it as back to words, so
/// Linux uses `alt+Delete` (#1825).
pub fn binding_offered_on(action: ActionId, binding: &str, platform: Platform) -> bool {
    !matches!(
        (action, binding, platform),
        (
            ActionId::Builtin(CommandId::BackToWords),
            "alt+BackSpace",
            Platform::Apple
        ) | (
            ActionId::Builtin(CommandId::ForgetRecent),
            "alt+BackSpace",
            Platform::Freedesktop
        ) | (
            ActionId::Builtin(CommandId::ForgetRecent),
            "alt+Delete",
            Platform::Apple
        ) | (
            ActionId::Builtin(CommandId::DeleteThisMessage),
            "alt+BackSpace",
            Platform::Freedesktop
        ) | (
            ActionId::Builtin(CommandId::DeleteThisMessage),
            "alt+Delete",
            Platform::Apple
        )
    )
}

pub fn reachable_in(context: Context, state: Availability) -> impl Iterator<Item = ActionSpec> {
    reachable(context).filter(move |spec| spec.requires.met_by(state))
}

pub fn every_action() -> impl Iterator<Item = ActionSpec> {
    all().map(ActionSpec::from)
}

pub fn spec(id: ActionId) -> Option<ActionSpec> {
    match id {
        ActionId::Builtin(id) => Some(ActionSpec::from(get(id))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Scope;
    use postio_config::paths::Platform;

    const NOT_ON_THE_MAC: [CommandId; 19] = [
        CommandId::DarkenMessage,
        CommandId::DetachComposer,
        CommandId::NextScope,
        CommandId::GoToCalendar,
        CommandId::CalendarAgenda,
        CommandId::CalendarPrevious,
        CommandId::CalendarNext,
        CommandId::CalendarToday,
        CommandId::PopOut,
        CommandId::PopIn,
        CommandId::CloseWindow,
        CommandId::NewEvent,
        CommandId::EditEvent,
        CommandId::MoveEvent,
        CommandId::DuplicateEvent,
        CommandId::TentativeInvite,
        CommandId::SaveEvent,
        CommandId::SaveAndSendEvent,
        CommandId::EmailAttendees,
    ];

    const NOT_ON_LINUX: [CommandId; 1] = [CommandId::ContactsSync];

    #[test]
    fn every_command_is_offered_on_freedesktop() {
        let scoped: Vec<CommandId> = CommandId::ALL
            .iter()
            .copied()
            .filter(|id| !offered_on(ActionId::Builtin(*id), Platform::Freedesktop))
            .collect();
        assert_eq!(
            scoped, NOT_ON_LINUX,
            "a command left Linux's menus without anyone deciding it"
        );
    }

    #[test]
    fn the_mac_is_not_offered_what_its_design_has_no_surface_for() {
        for id in NOT_ON_THE_MAC {
            assert!(
                !offered_on(ActionId::Builtin(id), Platform::Apple),
                "`{id}`"
            );
        }
        let scoped: Vec<CommandId> = CommandId::ALL
            .iter()
            .copied()
            .filter(|id| !offered_on(ActionId::Builtin(*id), Platform::Apple))
            .collect();
        assert_eq!(scoped, NOT_ON_THE_MAC);
    }

    #[test]
    fn the_table_is_ordered_like_command_id_all() {
        // `get` indexes the table by discriminant; this is what makes that safe.
        assert_eq!(SPECS.len(), CommandId::ALL.len());
        for (spec, id) in SPECS.iter().zip(CommandId::ALL) {
            assert_eq!(spec.id, *id, "registry row out of order at `{id}`");
        }
    }

    /// `space` pages and `Z` folds in a conversation (#1402).
    #[test]
    fn space_pages_in_a_conversation_and_shift_z_folds() {
        let keymap = crate::config::Keymap::defaults();
        assert_eq!(
            keymap.command_for(Context::Conversation, "space"),
            Some(CommandId::ScrollReaderDown.into()),
            "`space` must turn the page in a conversation -- the one-document \
             pane has nothing to fold, and this is the key a reading surface \
             is expected to page with"
        );
        assert_eq!(
            keymap.command_for(Context::Conversation, "Z"),
            Some(CommandId::ToggleFold.into()),
            "folding must keep a key: the stacked pane still folds, and \
             taking `space` away without giving it somewhere else would lose \
             a working gesture"
        );
        for key in ["Page_Down", "Page_Up"] {
            assert!(
                keymap.command_for(Context::Conversation, key).is_some(),
                "`{key}` must resolve in a conversation; the two directions \
                 serving different surfaces is what this issue found"
            );
        }
    }

    #[test]
    fn rebinding_over_another_commands_binding_in_a_shared_context_is_a_conflict() {
        let bindings = postio_config::KeyBindings::default();
        let conflict = binding_conflict(
            CommandId::NextMessage,
            "k",
            &bindings,
            Platform::Freedesktop,
        );
        assert_eq!(conflict.map(|spec| spec.id), Some(CommandId::PrevMessage));
    }

    #[test]
    fn the_same_binding_in_disjoint_contexts_is_not_a_conflict() {
        // Reusing a key across disjoint contexts is deliberate (`j` is three commands).
        let bindings = postio_config::KeyBindings::default();
        let conflict = binding_conflict(
            CommandId::NextMessage,
            "mod+i",
            &bindings,
            Platform::Freedesktop,
        );
        assert_eq!(conflict, None);
    }

    #[test]
    fn a_binding_nothing_else_uses_is_not_a_conflict() {
        let bindings = postio_config::KeyBindings::default();
        let conflict = binding_conflict(
            CommandId::NextMessage,
            "ctrl+shift+9",
            &bindings,
            Platform::Freedesktop,
        );
        assert_eq!(conflict, None);
    }

    #[test]
    fn a_command_config_has_no_default_for_still_holds_its_key() {
        // #1227: the check asked a table that lacked `Flag`, so its key read as free.
        let bindings = postio_config::KeyBindings::default();
        assert_eq!(
            binding_conflict(
                CommandId::NextMessage,
                "s",
                &bindings,
                Platform::Freedesktop
            )
            .map(|spec| spec.id),
            Some(CommandId::Flag),
            "`s` is Flag's default and Flag shares a context with NextMessage"
        );
    }

    #[test]
    fn the_check_sees_overrides_not_just_defaults() {
        let mut bindings = postio_config::KeyBindings::default();
        bindings
            .overrides_mut()
            .insert(CommandId::PrevMessage.as_str().to_owned(), "p".to_owned());
        assert_eq!(
            binding_conflict(
                CommandId::NextMessage,
                "k",
                &bindings,
                Platform::Freedesktop
            ),
            None,
            "k is free now that PrevMessage moved off it"
        );
        assert_eq!(
            binding_conflict(
                CommandId::NextMessage,
                "p",
                &bindings,
                Platform::Freedesktop
            )
            .map(|spec| spec.id),
            Some(CommandId::PrevMessage),
            "p is where PrevMessage actually lives now"
        );
    }

    #[test]
    fn every_binding_in_the_table_is_one_the_resolver_can_parse() {
        // A default nobody can press silently costs the command its key; `postio-config` validates
        // only overrides.
        for spec in all() {
            for binding in spec.bindings() {
                assert_eq!(
                    postio_config::keys::binding_problem(binding),
                    None,
                    "`{}` for `{}`",
                    binding,
                    spec.id
                );
            }
        }
    }

    #[test]
    fn a_binding_resolves_back_to_its_command() {
        assert_eq!(
            lookup_binding(Context::List, "e").map(|spec| spec.id),
            Some(CommandId::Archive)
        );
        assert_eq!(
            lookup_binding(Context::List, "o").map(|spec| spec.id),
            Some(CommandId::OpenMessage),
            "alternate bindings resolve too"
        );
        assert_eq!(lookup_binding(Context::Composer, "e"), None);
        assert_eq!(lookup_binding(Context::List, "ctrl+alt+q"), None);
    }

    #[test]
    fn tab_belongs_to_the_pane_that_owns_it() {
        // Panes that own Tab for their own purpose keep it (#494).
        assert_eq!(lookup_binding(Context::Composer, "tab"), None);
        assert_eq!(lookup_binding(Context::Search, "tab"), None);
    }

    type ThreadKey = (
        &'static str,
        &'static str,
        &'static [&'static str],
        &'static [Context],
    );

    const THREAD_KEYS: &[ThreadKey] = &[
        (
            "expand_thread",
            "Right",
            &[],
            &[Context::List, Context::Results, Context::Filtered],
        ),
        (
            "collapse_thread",
            "Left",
            &[],
            &[Context::List, Context::Results, Context::Filtered],
        ),
        (
            "next_in_thread",
            "]",
            &["n", "alt+Down"],
            &[Context::Conversation, Context::Reader],
        ),
        (
            "prev_in_thread",
            "[",
            &["p", "alt+Up"],
            &[Context::Conversation, Context::Reader],
        ),
        ("first_in_thread", "{", &[], &[Context::Reader]),
        ("latest_in_thread", "}", &[], &[Context::Reader]),
        (
            "first_unread_in_thread",
            "g u",
            &[],
            &[Context::Reader, Context::Outline],
        ),
        ("outline_first_unread", "u", &[], &[Context::Outline]),
        (
            "thread_outline",
            "alt+t",
            &[],
            &[Context::Reader, Context::List],
        ),
        ("thread_menu", "alt+period", &[], &[Context::Reader]),
        (
            "reply_to_latest",
            "alt+e",
            &[],
            &[
                Context::List,
                Context::Reader,
                Context::Results,
                Context::Composer,
            ],
        ),
        (
            "mute_thread",
            "m",
            &[],
            &[Context::List, Context::Reader, Context::Results],
        ),
        ("split_thread", "alt+shift+s", &[], &[Context::Reader]),
        (
            "move_this_message",
            "alt+v",
            &[],
            &[Context::Reader, Context::List, Context::Results],
        ),
        (
            "delete_this_message",
            "alt+BackSpace",
            &["alt+Delete"],
            &[Context::Reader, Context::List, Context::Results],
        ),
        ("forward_what_1", "1", &[], &[Context::ForwardChooser]),
        ("forward_what_2", "2", &[], &[Context::ForwardChooser]),
        ("forward_what_3", "3", &[], &[Context::ForwardChooser]),
        ("show_newer", "mod+shift+Down", &[], &[Context::Composer]),
        ("expand_quoted", "mod+shift+q", &[], &[Context::Composer]),
    ];

    #[test]
    fn each_thread_command_has_its_key_and_its_contexts() {
        let mut problems = Vec::new();
        for (name, key, alternates, contexts) in THREAD_KEYS {
            let Ok(id) = name.parse::<CommandId>() else {
                problems.push(format!("`{name}` is not a command"));
                continue;
            };
            let spec = get(id);
            if spec.default_binding != *key {
                problems.push(format!("`{name}` is on `{}`", spec.default_binding));
            }
            if spec.alternate_bindings != *alternates {
                problems.push(format!("`{name}` alternates {:?}", spec.alternate_bindings));
            }
            for context in Context::ALL {
                if spec.available_in(*context) != contexts.contains(context) {
                    problems.push(format!(
                        "`{name}` reachable in {context} is {}",
                        !contexts.contains(context)
                    ));
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn right_and_left_expand_and_collapse_where_they_used_to_open_and_go_back() {
        for context in [Context::List, Context::Results, Context::Filtered] {
            assert_eq!(
                lookup_binding(context, "Right").map(|spec| spec.id),
                Some(CommandId::ExpandThread),
                "Right in {context}"
            );
            assert_eq!(
                lookup_binding(context, "Left").map(|spec| spec.id),
                Some(CommandId::CollapseThread),
                "Left in {context}"
            );
        }
        assert!(
            !get(CommandId::OpenMessage)
                .bindings()
                .any(|key| key == "Right")
        );
        assert!(!get(CommandId::PrevView).bindings().any(|key| key == "Left"));
        assert_eq!(
            lookup_binding(Context::List, "Return").map(|spec| spec.id),
            Some(CommandId::OpenMessage)
        );
        assert_eq!(
            lookup_binding(Context::List, "o").map(|spec| spec.id),
            Some(CommandId::OpenMessage)
        );
    }

    #[test]
    fn archive_thread_is_gone() {
        assert!("archive_thread".parse::<CommandId>().is_err());
    }

    #[test]
    fn no_two_commands_share_a_key_in_any_context() {
        let mut problems = Vec::new();
        for (frontend, platform) in [
            (Frontend::Terminal, Platform::Freedesktop),
            (Frontend::Focus, Platform::Freedesktop),
            (Frontend::Focus, Platform::Apple),
        ] {
            let keymap =
                crate::config::Keymap::resolve_on(&postio_config::KeyBindings::default(), platform);
            let state = Availability {
                frontend,
                ..Availability::open(Scope::Account(postio_model::AccountId::new(1)))
            };
            for context in Context::ALL {
                let mut seen: std::collections::BTreeMap<String, CommandId> =
                    std::collections::BTreeMap::new();
                for action in reachable_in(*context, state) {
                    let Some(id) = action.id.builtin() else {
                        continue;
                    };
                    for binding in keymap.bindings(id) {
                        if let Some(other) = seen.insert(binding.clone(), id)
                            && other != id
                        {
                            problems.push(format!(
                                "{frontend:?} on {platform:?}, {context}: `{binding}` is both `{other}` and `{id}`"
                            ));
                        }
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn delete_this_message_leaves_alt_backspace_to_the_terminals_back_to_words() {
        let id = ActionId::Builtin(CommandId::DeleteThisMessage);
        assert!(binding_offered_on(id, "alt+BackSpace", Platform::Apple));
        assert!(!binding_offered_on(
            id,
            "alt+BackSpace",
            Platform::Freedesktop
        ));
        assert!(binding_offered_on(id, "alt+Delete", Platform::Freedesktop));
        assert!(!binding_offered_on(id, "alt+Delete", Platform::Apple));
    }

    #[test]
    fn reply_to_latest_does_not_take_the_composers_external_editor_key() {
        assert_eq!(
            lookup_binding(Context::Composer, "alt+e").map(|spec| spec.id),
            Some(CommandId::ReplyToLatest)
        );
        assert_eq!(
            get(CommandId::EditExternally).default_binding,
            "mod+shift+e"
        );
        assert_eq!(
            get(CommandId::EditExternally).alternate_bindings,
            ["alt+shift+e"],
            "the terminal's editor key must stay one a legacy terminal delivers"
        );
    }

    #[test]
    fn the_forward_chooser_and_the_outline_have_contexts_of_their_own() {
        for key in ["1", "2", "3"] {
            assert_ne!(
                lookup_binding(Context::ForwardChooser, key).map(|spec| spec.id),
                lookup_binding(Context::Picker, key).map(|spec| spec.id),
            );
        }
        assert_eq!(
            lookup_binding(Context::Outline, "/").map(|spec| spec.id),
            Some(CommandId::Search)
        );
        assert_eq!(
            lookup_binding(Context::Outline, "u").map(|spec| spec.id),
            Some(CommandId::OutlineFirstUnread)
        );
        assert_eq!(
            lookup_binding(Context::Reader, "u").map(|spec| spec.id),
            Some(CommandId::PrevView)
        );
    }

    #[test]
    fn titles_read_as_palette_rows() {
        for spec in all() {
            let first = spec.title.chars().next().expect("non-empty title");
            assert!(
                first.is_uppercase(),
                "`{}` should be Sentence case for the palette",
                spec.title
            );
            assert!(
                !spec.title.ends_with('.'),
                "`{}` ends in a period",
                spec.title
            );
        }
    }

    /// The vocabulary says Thread, never Conversation, in both title and id (#2171).
    #[test]
    fn no_command_says_conversation() {
        for spec in all() {
            for word in [spec.title, spec.id.as_str()] {
                assert!(
                    !word.to_lowercase().contains("conversation"),
                    "`{word}` says conversation; Postio says thread"
                );
            }
        }
    }

    // -- spec 014: compose formatting, the now phase (`data-model.md`) ----

    /// Every **Now** row of spec 014's registry table: id, title, keys.
    const FORMATTING_NOW: [(CommandId, &str, &str, &[&str]); 19] = [
        (CommandId::Heading1, "Heading 1", "mod+alt+1", &["alt+1"]),
        (CommandId::Heading2, "Heading 2", "mod+alt+2", &["alt+2"]),
        (CommandId::Heading3, "Heading 3", "mod+alt+3", &["alt+3"]),
        (CommandId::Heading4, "Heading 4", "mod+alt+4", &["alt+4"]),
        (CommandId::Heading5, "Heading 5", "mod+alt+5", &["alt+5"]),
        (CommandId::Heading6, "Heading 6", "mod+alt+6", &["alt+6"]),
        (
            CommandId::NormalText,
            "Normal text",
            "mod+alt+0",
            &["alt+0"],
        ),
        (
            CommandId::Strikethrough,
            "Strikethrough",
            "mod+shift+s",
            &["alt+asciitilde"],
        ),
        (
            CommandId::InlineCode,
            "Inline code",
            "mod+e",
            &["alt+grave"],
        ),
        (CommandId::CodeBlock, "Code block", "mod+alt+c", &["alt+C"]),
        (CommandId::TaskList, "Task list", "mod+shift+6", &["alt+t"]),
        (
            CommandId::ToggleTask,
            "Done or not done",
            "mod+alt+x",
            &["alt+X"],
        ),
        (
            CommandId::Indent,
            "Indent",
            "mod+bracketright",
            &["alt+greater"],
        ),
        (
            CommandId::Outdent,
            "Outdent",
            "mod+bracketleft",
            &["alt+less"],
        ),
        (
            CommandId::Divider,
            "Divider",
            "mod+alt+minus",
            &["alt+minus"],
        ),
        (CommandId::Table, "Table", "mod+shift+t", &["alt+bar"]),
        (
            CommandId::ClearFormatting,
            "Clear formatting",
            "mod+backslash",
            &["alt+backslash"],
        ),
        (
            CommandId::ToggleMarkdownSource,
            "Edit as Markdown or rich text",
            "mod+alt+s",
            &[],
        ),
        (
            CommandId::AttachRemoteImage,
            "Download and attach image",
            "mod+alt+i",
            &["alt+I"],
        ),
    ];

    #[test]
    fn every_now_phase_formatting_command_is_registered_as_the_spec_says() {
        for (id, title, default, alternates) in FORMATTING_NOW {
            let spec = get(id);
            assert_eq!(spec.title, title, "`{id}`");
            assert_eq!(spec.default_binding, default, "`{id}`");
            assert_eq!(spec.alternate_bindings, alternates, "`{id}`");
            assert_eq!(spec.contexts, Context::Composer.as_set(), "`{id}`");
            assert_eq!(spec.recovery, Recovery::None, "`{id}`");
            assert!(!spec.destructive, "`{id}`");
            let expected = if id == CommandId::ToggleMarkdownSource {
                GRAPHICAL_MAIL
            } else {
                MAIL
            };
            assert_eq!(spec.requires, expected, "`{id}`");
        }
    }

    #[test]
    fn no_held_formatting_command_is_registered() {
        // Held for ADR 0039's native composer (spec 014 D8): a row nothing
        // runs is a key that does nothing.
        for held in [
            "code_language",
            "line_break",
            "formatting",
            "paste_markdown",
            "table_row_below",
            "table_delete",
            "picker_choose_0",
            "picker_choose_9",
        ] {
            assert!(held.parse::<CommandId>().is_err(), "`{held}` is registered");
        }
    }
}
