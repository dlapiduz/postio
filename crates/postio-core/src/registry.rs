//! The command registry: one table, every surface.
//!
//! docs/PRODUCT.md §8 asks that every command have a keyboard shortcut, a
//! command-palette entry and an accessible UI action. Three hand-maintained
//! lists would drift apart within a release; one enumerable table cannot. The
//! keymap, the `Ctrl+K` palette, the `?` cheat sheet, the right-click context
//! menu and the key hints on the focused row are all *derived* from
//! [`all()`] and [`for_context()`].
//!
//! # Where the bindings come from
//!
//! The design canvas: `e` reply, `a` archive, `A` archive thread, `u` undo,
//! `t` thread. The original brief proposed `r` for reply; the canvas was newer
//! and won, and docs/PRODUCT.md §8 records the resolution rather than the
//! argument.
//! The ids and their defaults are the same vocabulary `postio-config`'s
//! `DEFAULT_BINDINGS` fixed, so `[keys]` overrides land on the right command.
//! Everything here is a *default*; the user's `[keys]` wins at resolve time.
//!
//! # Destructive commands
//!
//! docs/PRODUCT.md §1 requires that destructive operations be confirmed or undoable.
//! [`CommandSpec::destructive`] and [`CommandSpec::recovery`] make that
//! machine-checkable rather than a review habit: a destructive command with no
//! [`Recovery`] fails the test suite.

use std::fmt;
use std::sync::{OnceLock, RwLock};

use crate::action::{ActionId, ExtId};
use crate::command::CommandId;
use crate::context::{Context, ContextSet};
use crate::state::Scope;

use postio_config::paths::Platform;
use serde::{Deserialize, Serialize};

/// How the user gets back from a command that changed something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Nothing to recover from; the command changed no durable state.
    None,
    /// Reversible from the undo stack, and worth an "— Undo" toast
    /// (docs/PRODUCT.md §16: *Archived 12 messages — Undo*).
    ///
    /// `u` works, and that is the load-bearing half: a command claiming this
    /// must be something [`crate::undo::UndoStack`] can actually hold,
    /// which means a `UndoKind` exists for it.
    Undo,
    /// Reversible for a limited time, through its own affordance rather than
    /// the undo stack (#1481).
    ///
    /// A send is the case this exists for. It *is* reversible — the draft
    /// sits in the queue and opening it cancels the send — and it is not
    /// reversible from the undo stack, which takes message operations with a
    /// ten-minute expiry.
    ///
    /// Those two numbers are why this is a separate answer rather than
    /// `Undo`. A send's window is however long the drainer takes, which is
    /// seconds; the stack's is ten minutes. An entry recorded there would
    /// outlive what it can act on, sit at the top of the stack shadowing the
    /// archive beneath it, and answer `u` with "too late" — leaving the
    /// person unsure whether the archive they meant to undo had been
    /// consumed. `Recovery::Undo` for a send was not merely unimplemented; it
    /// was the wrong promise.
    ///
    /// The affordance is the toast's own "Undo", live only while the window
    /// is, which is what every client that offers undo-send does.
    Window,
    /// Irreversible enough to ask first.
    Confirm,
}

/// A condition on *state* that a command needs in order to mean anything.
///
/// [`Context`] answers "which surface has focus", which is all most commands
/// need. `Move` is the first that needs more: it has to name a destination,
/// and in [`Scope::Unified`] there is no single account to name one in. ADR
/// 0005's consequences asked for that to be settled once rather than
/// special-cased at every surface, so it is data on the row — the same shape
/// the rest of this table already uses — and every surface evaluates it
/// through [`reachable_in`], which asks [`Requirement::met_by`] per row.
///
/// **The shape for the next one:** add a variant here, give it a line in
/// [`Availability`], and answer it in [`Requirement::met_by`]. Nothing at a
/// surface changes; the palette, cheat sheet and key hints pick it up because
/// they all go through [`reachable_in`]. Resist a `fn` pointer: a predicate
/// that is data can be tested, printed in a failure message, and read by
/// somebody who is not holding the registry in their head.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The view has to be one account's, because the command needs somewhere
    /// in *that* account to put something.
    SingleAccount,
    /// The local store has to be open, because the command reads or writes
    /// mail.
    ///
    /// Postio's window is on screen before the store is (#1114): the keyring
    /// read, the schema migrations and the search-index rebuild all happen
    /// behind a window that already exists, and on a real install that has
    /// been twelve seconds. **A window with no store must not offer verbs
    /// that cannot run** — the palette and the cheat sheet simply do not list
    /// them, which is how they already treat anything unavailable, and a key
    /// bound to one refuses out loud rather than being swallowed.
    StoreOpen,
    /// The frontend has to be the terminal, because the command works on
    /// what only its composer has: Markdown text an editor can open, and a
    /// preview of what it becomes. The desktop's composer edits rich text in
    /// place, so it does not offer these (spec 005, open question 4).
    Terminal,
    /// The frontend has to draw the message as pixels, because the command
    /// changes how it is drawn: zoom, and darkening a sheet of paper
    /// (spec 006). A terminal draws text in its own font and colours, and
    /// has nothing for these to act on.
    Graphical,
    /// The frontend has to be Postio Focus -- the desktop app or the terminal,
    /// which is Focus drawn in character cells (spec 007 C29) -- because the
    /// command works on what only Focus has: invitations answered from the
    /// row, the has-action filter, Filtered, digests and reminders
    /// (specs/007-postio-focus research R4). The one keymap reserves their
    /// keys in every app; only Focus offers them.
    Focus,
}

impl Requirement {
    /// Every requirement, in declaration order. What [`RequirementSet`] is
    /// built over.
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

    /// Whether this requirement is about which app is asking -- settled
    /// once, when the app starts -- rather than about its state, which
    /// changes while it runs.
    pub const fn is_about_the_app(self) -> bool {
        matches!(
            self,
            Requirement::Terminal | Requirement::Graphical | Requirement::Focus
        )
    }
}

/// The requirements one command carries — a set, because they compose.
///
/// `Move` is why this is not an `Option`: a destination has to be one folder
/// in one account, *and* there has to be a store holding it. One slot per row
/// could express either and not both, and the row that needed both was the
/// only row that had a requirement at all.
///
/// A set rather than a closure for [`ContextSet`]'s reason: a predicate you
/// can only call answers "is this available here?" and not "what is available
/// here?", and the palette needs the second question answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RequirementSet(u8);

impl RequirementSet {
    /// Nothing required beyond having the right surface focused.
    pub const NONE: RequirementSet = RequirementSet(0);

    /// A set built from a slice, usable in a `const` table.
    pub const fn from_slice(requirements: &[Requirement]) -> RequirementSet {
        let mut bits = 0u8;
        let mut index = 0;
        while index < requirements.len() {
            bits |= requirements[index].bit();
            index += 1;
        }
        RequirementSet(bits)
    }

    /// Whether `requirement` is in the set.
    pub const fn contains(self, requirement: Requirement) -> bool {
        self.0 & requirement.bit() != 0
    }

    /// Whether the set is empty — the ordinary answer for most commands.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `state` satisfies every requirement in the set.
    pub fn met_by(self, state: Availability) -> bool {
        Requirement::ALL
            .iter()
            .all(|need| !self.contains(*need) || need.met_by(state))
    }

    /// Whether `frontend` offers a command with these requirements at all,
    /// whatever state it is in.
    ///
    /// The question a keymap asks when it decides which keys an app binds:
    /// a key the one keymap keeps for another app is bound to nothing here,
    /// rather than reaching a command this app never offers
    /// (specs/007-postio-focus research R4). State is not the keymap's to
    /// judge -- a key bound to something that needs the store says so out
    /// loud while the store is shut (#1114) -- so only the requirements
    /// [about the app](Requirement::is_about_the_app) are asked.
    pub fn offered_by(self, frontend: Frontend) -> bool {
        let app = Availability {
            frontend,
            ..Availability::open(Scope::Unified)
        };
        self.iter()
            .filter(|need| need.is_about_the_app())
            .all(|need| need.met_by(app))
    }

    /// The requirements in the set, for a failure message that has to name
    /// which one was not met.
    pub fn iter(self) -> impl Iterator<Item = Requirement> {
        Requirement::ALL
            .into_iter()
            .filter(move |need| self.contains(*need))
    }
}

/// Which Postio app is asking (specs/007-postio-focus research R4).
///
/// Every app reads the one registry, and the one keymap gives every command
/// the same key in each; what differs is which commands an app offers at
/// all, and that is a [`Requirement`] evaluated against this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Frontend {
    /// The terminal app (spec 005).
    Terminal,
    /// Postio, the desktop app: Focus (spec 007). The Mac registers as this
    /// too (specs/009-focus-macos R5).
    Focus,
}

/// The state [`Requirement`]s are evaluated against.
///
/// A struct rather than bare arguments so a new requirement adds a field
/// instead of changing every call site's signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Availability {
    /// What the mail on screen belongs to.
    pub scope: Scope,
    /// Whether the local store is open behind this window.
    ///
    /// `false` only between the first frame and the store landing — a window
    /// Postio presents before it has opened anything, so that a slow keyring
    /// read or a long migration is a window that says what it is waiting for
    /// rather than no window at all (#1114).
    pub store_open: bool,
    /// Which app is asking.
    pub frontend: Frontend,
}

impl Availability {
    /// The ordinary state: this scope, with the mail open behind it, in the
    /// desktop app.
    ///
    /// What every surface that has been fed is in, and what a test asserting
    /// about scope alone means. Another app sets
    /// [`frontend`](Self::frontend) over this.
    pub fn open(scope: Scope) -> Availability {
        Availability {
            scope,
            store_open: true,
            frontend: Frontend::Focus,
        }
    }
}

impl Requirement {
    /// Whether `state` satisfies this requirement.
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

/// One row of the registry: everything every surface needs about a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSpec {
    /// The stable id. `[keys]` in `config.toml` names this string.
    pub id: CommandId,
    /// The human-readable title, as the palette and cheat sheet show it.
    pub title: &'static str,
    /// The built-in binding, in the untyped syntax the keymap resolver parses
    /// (`"a"`, `"A"`, `"ctrl+k"`, `"g g"`). Overridable via `[keys]`.
    pub default_binding: &'static str,
    /// Secondary bindings for the same command — the arrow keys beside `j`/`k`,
    /// `l` beside `Return`. Not overridable; `[keys]` replaces the primary.
    pub alternate_bindings: &'static [&'static str],
    /// The contexts this command is meaningful in — its context predicate.
    pub contexts: ContextSet,
    /// Whether the command destroys something the user would have to rebuild.
    pub destructive: bool,
    /// How the user gets back. Never [`Recovery::None`] when `destructive`.
    pub recovery: Recovery,
    /// What the *state* must be for this command to mean anything, beyond
    /// having the right surface focused.
    ///
    /// Almost everything carries [`Requirement::StoreOpen`], because almost
    /// everything reads or writes mail; [`RequirementSet::NONE`] is the
    /// chrome — the palette, the cheat sheet, `Esc`, and where the keyboard
    /// is — which means the same thing with an empty window as with a full
    /// one.
    pub requires: RequirementSet,
}

impl CommandSpec {
    /// The context predicate: whether this command is reachable in `context`.
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

/// What a row needs beyond its context. Spelled short because almost every
/// row carries one.
const fn needs(requirements: &'static [Requirement]) -> RequirementSet {
    RequirementSet::from_slice(requirements)
}

/// Reads or writes mail, which is very nearly everything.
const MAIL: RequirementSet = needs(&[Requirement::StoreOpen]);

/// Works on the terminal composer's Markdown, which only it has.
const TERMINAL_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Terminal]);
/// Mail drawn as pixels: zoom and darken (spec 006).
const GRAPHICAL_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Graphical]);
/// Works on what only Postio Focus has: invitations from the row, the
/// has-action filter, Filtered, digests and reminders (specs/007-postio-focus
/// research R4). Every other app keeps the key free and offers nothing on it.
const FOCUS_MAIL: RequirementSet = needs(&[Requirement::StoreOpen, Requirement::Focus]);
/// A Focus verb that changes how a message is drawn as pixels, so the
/// terminal, which is Focus too, does not offer it.
const FOCUS_GRAPHICAL_MAIL: RequirementSet = needs(&[
    Requirement::StoreOpen,
    Requirement::Focus,
    Requirement::Graphical,
]);

/// Chrome: it means the same thing with an empty window as with a full one.
const CHROME: RequirementSet = RequirementSet::NONE;

/// Reading the message list, a thread and a single message: the surfaces where
/// a message action means something.
const MESSAGE_SURFACES: &[Context] = &[Context::List, Context::Conversation, Context::Reader];
/// Where the capture sheet's `t` and `n` work: from a message, and in the
/// sheet itself, where they switch it between a task and a note.
const CAPTURE_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Capture,
];
/// `MESSAGE_SURFACES` plus the composer.
///
/// Reply, reply-all and forward have to *resolve* while a draft is already
/// open, or the key is swallowed before `Composer::dispatch` ever sees it and
/// pressing it looks identical to nothing being bound at all (#426).
/// Availability is not success: the composer still refuses to replace an
/// in-progress draft, it just gets the chance to say so instead of staying
/// silent.
const REPLY_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Composer,
];
/// The surfaces that scroll through a list of messages.
/// Where extending a *row* selection means something.
///
/// [`LIST_SURFACES`] minus the conversation pane. Inside a conversation the
/// keyboard is walking one thread's messages, not a list of threads, so
/// there is nothing for `J`/`K` to extend — which is exactly what frees them
/// for the walk itself (#1007).
const SELECTION_SURFACES: &[Context] = &[Context::List, Context::Reader, Context::Search];

const LIST_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Search,
];

/// [`LIST_SURFACES`] plus the folder list: everywhere a person could want to
/// be somewhere else.
///
/// The destinations use this rather than `LIST_SURFACES`, because standing in
/// the folder list is the *most* likely moment to want another folder, and a
/// `g i` that works in the message list and not beside it is a key that
/// appears broken depending on where the keyboard happens to be. Not
/// `ContextSet::ANY`: `g` is a letter in the composer.
const GO_SURFACES: &[Context] = &[
    Context::List,
    Context::Conversation,
    Context::Reader,
    Context::Search,
    Context::Sidebar,
];

/// The registry itself. Ordered like [`CommandId::ALL`]; the cheat sheet reads
/// it top to bottom.
static SPECS: &[CommandSpec] = &[
    // -- Navigation ------------------------------------------------------
    CommandSpec {
        id: CommandId::NextMessage,
        title: "Next message",
        default_binding: "j",
        alternate_bindings: &["Down"],
        // And Focus's Filtered view and digest window, whose rows are
        // walked as the list's are (screen 21's footer).
        contexts: ctx(LIST_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::PrevMessage,
        title: "Previous message",
        default_binding: "k",
        alternate_bindings: &["Up"],
        // And Focus's Filtered view and digest window, whose rows are
        // walked as the list's are (screen 21's footer).
        contexts: ctx(LIST_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::FirstMessage,
        title: "First message",
        // The canvas writes this `gg`, vim-style, but a binding *string* spells
        // a sequence with a space between the chords — that is the syntax both
        // `postio-config`'s validator and the keymap resolver parse, and `gg`
        // would be read as a key named "gg", which no keyboard has.
        default_binding: "g g",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::LastMessage,
        title: "Last message",
        default_binding: "G",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::OpenMessage,
        title: "Open message",
        // `Return` is what config.toml documents, and `Right` goes one level
        // in the way `Left` comes back out. `l` was the vim-style open until
        // the one keymap gave it to labels (specs/007-postio-focus
        // contracts/keymap.md).
        default_binding: "Return",
        alternate_bindings: &["Right"],
        // And a row of Focus's Filtered view or digest window.
        contexts: ctx(&[Context::List, Context::Conversation, Context::Search])
            .with(Context::Filtered)
            .with(Context::Digest)
            .with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleSelection,
        title: "Toggle selection",
        // Gmail's, and everyone else's since. Muscle memory is worth more
        // here than a mnemonic nobody has.
        default_binding: "x",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES).with(Context::Results),
        destructive: false,
        // Changing what an action *would* hit changes no durable state, so
        // there is nothing to undo and nothing to confirm.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionDown,
        title: "Extend selection down",
        default_binding: "J",
        alternate_bindings: &["shift+Down"],
        // `LIST_SURFACES` minus the conversation: there is no row selection
        // to extend while the keyboard is inside the pane, which walks one
        // thread's messages on `]` and `[` (#1007). `shift+Down` still
        // reaches this everywhere it ever did.
        contexts: ctx(SELECTION_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionUp,
        title: "Extend selection up",
        default_binding: "K",
        alternate_bindings: &["shift+Up"],
        // See `ExtendSelectionDown`.
        contexts: ctx(SELECTION_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SelectAll,
        title: "Select all",
        // Shifted `x`, the key that selects one row: the same verb, for all
        // of them (specs/007-postio-focus contracts/keymap.md). `mod+a`
        // stays, for the hand that reaches for it from every other
        // application.
        default_binding: "X",
        alternate_bindings: &["mod+a"],
        contexts: ctx(LIST_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::PrevView,
        title: "Previous view",
        // `Left`, the way back out that `Right` goes in. `h` is Focus's
        // remind key under the one keymap.
        default_binding: "Left",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Back,
        title: "Back",
        default_binding: "Escape",
        alternate_bindings: &[],
        // Escape always means "get me out of here", in every context.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: CHROME,
    },
    CommandSpec {
        id: CommandId::ToggleResultOrder,
        // The same title as the thread's own toggle, deliberately: "the
        // order of what I am looking at" is one idea (#499). `alt+o`, because
        // the query holds the keyboard and a bare `O` is a letter in it.
        title: "Toggle result order",
        default_binding: "alt+o",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Search]).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    // -- Message actions -------------------------------------------------
    CommandSpec {
        id: CommandId::NextInConversation,
        title: "Next message in conversation",
        // `]` and `[`: `j` walks the list of conversations, and these walk
        // the messages of the one that is open -- in the macOS pane, and in
        // Focus's reading dialog, which steps through the thread
        // (specs/007-postio-focus contracts/keymap.md). `J`/`K` extend the
        // list's selection.
        default_binding: "]",
        alternate_bindings: &["alt+Down"],
        contexts: ctx(&[Context::Conversation, Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::PrevInConversation,
        title: "Previous message in conversation",
        default_binding: "[",
        alternate_bindings: &["alt+Up"],
        contexts: ctx(&[Context::Conversation, Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleFold,
        // `z`, not the `space` canvas turn 8a gave it. That trade was made
        // for a **stack**, where folding is the gesture the surface is for;
        // FR-013 (#1389) leaves the one-document pane nothing to fold, so
        // `space` there bought nothing and cost the key every reading surface
        // turns pages with. The maintainer settled it the other way (#1402).
        //
        // Folding keeps a key rather than losing one: the stacked pane still
        // folds. `z` is free across the table and is where a vim user already
        // looks -- `za` toggles a fold, and the whole family lives under `z`.
        title: "Fold or unfold this message",
        default_binding: "z",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        // How much of a conversation is open is view state, not durable
        // data -- nothing here for undo to reach.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ViewOriginal,
        title: "View original",
        // `mod+o`, not a bare letter: it is a rare gesture on a surface
        // where every bare letter is already a verb people use constantly.
        // Since spec 006 FR-031 it is the way back from reader view rather
        // than out of a default.
        //
        // `mod`, not a literal `ctrl` -- the canvas writes it `C-o`, which
        // means the primary accelerator, and that is Command on a Mac (#669).
        // A literal `ctrl` here would also break the invariant
        // `platform_bindings.rs` checks: that the two tables differ nowhere
        // *but* the primary modifier.
        default_binding: "mod+o",
        alternate_bindings: &[],
        // Wherever a message is drawn. A no-op when nothing is reduced, so
        // it costs nothing to offer everywhere mail is read rather than
        // making the key's meaning depend on what happens to be on screen.
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleReaderView,
        title: "Reader view",
        // Paired with `View original`'s `mod+o`. The composer's `Detach
        // composer` has this key only where the composer is, and compose
        // takes the reading pane over, so the two never meet.
        default_binding: "mod+shift+o",
        // A terminal cannot tell `ctrl+shift+o` from `ctrl+o`, so it needs a
        // key it can send; `alt+o` is the composer's convention for exactly
        // this, and the composer's own `alt+o` never meets a message surface.
        alternate_bindings: &["alt+o"],
        // Wherever `View original` is: the two are one control's two halves.
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::DarkenMessage,
        title: "Darken this message",
        // `alt+d`: `D` stops digesting a sender under the one keymap
        // (specs/007-postio-focus contracts/keymap.md). The title reads "Show
        // as sent" while the message is darkened -- the command is its own
        // undo.
        default_binding: "alt+d",
        alternate_bindings: &[],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::SwitchTreatment,
        title: "Show original or app colours",
        // `⇧O` (the design handoff, SPEC.md section 7): `o` opens an
        // attachment or a link, and the shifted letter does the larger thing
        // to the same message -- shows the whole of it as sent. Free in the
        // reader: `O` expands a conversation and orders search results, and
        // neither is a reader surface.
        default_binding: "O",
        alternate_bindings: &[],
        // The open message only: the list has no body to draw either way.
        contexts: ctx(&[Context::Reader]),
        destructive: false,
        // How a message is drawn is view state; "Always for this sender"
        // is a setting the line beside the body offers to undo.
        recovery: Recovery::None,
        // Focus's open message is the surface the two treatments are drawn
        // on (T211, T212); the three-pane readers draw reader view instead and
        // the terminal draws text in its own colours.
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::FindInMessage,
        title: "Find in message",
        // The platform's convention, unbound until now.
        default_binding: "mod+f",
        alternate_bindings: &[],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::FindNext,
        title: "Next match",
        // `mod+g` and `F3` are both what every reader uses for the next match.
        default_binding: "mod+g",
        alternate_bindings: &["F3"],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::FindPrevious,
        title: "Previous match",
        // The composer's `mod+shift+g` never meets a message surface: compose takes the reading pane over.
        default_binding: "mod+shift+g",
        alternate_bindings: &["shift+F3"],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ZoomIn,
        title: "Zoom in",
        // `mod+equal` because `+` is shifted on most layouts.
        default_binding: "mod+plus",
        alternate_bindings: &["mod+equal", "mod+KP_Add"],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ZoomOut,
        title: "Zoom out",
        // The platform's convention.
        default_binding: "mod+minus",
        alternate_bindings: &["mod+KP_Subtract"],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ZoomReset,
        title: "Actual size",
        // The platform's convention.
        default_binding: "mod+0",
        alternate_bindings: &["mod+KP_0"],
        // Wherever `View original` is (spec 006 contracts/registry-commands).
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ExpandAll,
        title: "Expand all",
        // `o` was the drill-in column's order toggle until #1003 retired it,
        // which is what makes this letter available. Shifted, because it acts
        // on the whole conversation -- the same relationship `a`/`A` already
        // has between a message and its thread.
        default_binding: "O",
        alternate_bindings: &["mod+shift+e"],
        // Only where there is a conversation to expand. Offering it on the
        // list would be a key that does nothing most of the time.
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Reply,
        title: "Reply",
        default_binding: "e",
        alternate_bindings: &["mod+r"],
        contexts: ctx(REPLY_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ReplyAll,
        title: "Reply to all",
        default_binding: "E",
        alternate_bindings: &["mod+shift+r"],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Forward,
        title: "Forward",
        default_binding: "f",
        alternate_bindings: &["mod+shift+f"],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Archive,
        title: "Archive",
        default_binding: "a",
        alternate_bindings: &["mod+shift+a"],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        // Sweeping a screenful out of the inbox is exactly the case docs/PRODUCT.md §16
        // wants a toast for.
        destructive: true,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ArchiveThread,
        title: "Archive thread",
        default_binding: "A",
        alternate_bindings: &[],
        // In a digest it archives the whole digest: "archive everything this
        // row stands for" (specs/007-postio-focus research R4).
        contexts: ctx(MESSAGE_SURFACES).with(Context::Digest),
        destructive: true,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Delete,
        title: "Delete",
        // The key that says it. `d` is Focus's digest key under the one
        // keymap, and "delete" has one key everywhere it is offered.
        default_binding: "Delete",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        destructive: true,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Move,
        title: "Move to…",
        default_binding: "m",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::Undo,
        // A destination is one mailbox in one account, and a unified view
        // spans every enabled account — so there is nowhere for this to mean.
        // Unavailable rather than a no-op: offering it would promise a folder
        // the user was never given the chance to pick (#182, ADR 0005 Q4).
        requires: needs(&[Requirement::SingleAccount, Requirement::StoreOpen]),
    },
    CommandSpec {
        id: CommandId::Flag,
        title: "Flag",
        // `*`, a star: `s` snoozes under the one keymap
        // (specs/007-postio-focus contracts/keymap.md).
        default_binding: "*",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        // Every app offers it, Focus on `*` with no mark on the row (spec
        // C13): a flag another client set has to be clearable from here.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleRead,
        title: "Mark read or unread",
        // `r` for read, one key for both directions: the verb toggles
        // (`Command::ToggleRead` with no state). `U` unsubscribes under the
        // one keymap (specs/007-postio-focus contracts/keymap.md).
        default_binding: "r",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Snooze,
        title: "Snooze",
        // `s`, the one keymap's snooze (specs/007-postio-focus
        // contracts/keymap.md); `B` still unsnoozes.
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Unsnooze,
        title: "Unsnooze",
        default_binding: "B",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::RemindIfNoReply,
        // The ellipsis says a picker opens, as it does for `Schedule send…`.
        title: "Remind if no reply…",
        default_binding: "h",
        // The composer's key, where `h` is a letter being typed.
        alternate_bindings: &["mod+h"],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::AddLabel,
        title: "Add label…",
        // `l` for label, unshifted now that it no longer opens a message
        // (specs/007-postio-focus contracts/keymap.md).
        default_binding: "l",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    // -- Search ----------------------------------------------------------
    CommandSpec {
        id: CommandId::AcceptInvite,
        title: "Accept invitation",
        // `y` for yes, and its shift for no.
        default_binding: "y",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        // The reply is on its way once a short window closes, the way a send
        // is (research R9), so it is undone inside the window and not from
        // the undo stack.
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::DeclineInvite,
        title: "Decline invitation",
        default_binding: "Y",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Window,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::DigestRule,
        title: "Digest rule…",
        default_binding: "d",
        alternate_bindings: &[],
        // From a message, a new rule for its sender; in a digest, that
        // digest's own rule and cadence.
        contexts: ctx(MESSAGE_SURFACES).with(Context::Digest),
        destructive: false,
        // It opens the rule dialog, whose Create is the act, and a rule is
        // removed where rules are listed (`g d`).
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::StopDigestingSender,
        title: "Stop digesting this sender",
        default_binding: "D",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Digest, Context::Reader]),
        destructive: false,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::ViewSource,
        title: "View source",
        // Focus's for now. The other apps adopt it with a source view of
        // their own, and the key is kept free for them.
        default_binding: "v",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::OpenAttachmentOrLink,
        title: "Open attachment or link…",
        default_binding: "o",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::DismissMarker,
        title: "Dismiss marker",
        // `-`, taking the marker off: no app binds it in a message surface,
        // and the one keymap's enumeration holds it so (specs/007-postio-focus
        // T118; contracts/keymap.md names no key for it).
        default_binding: "-",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::MoreActions,
        title: "More actions",
        // `.`, the open message's More: the verbs a narrow dialog folds out
        // of its action row (specs/007-postio-focus T206). Unbound
        // elsewhere, in every app.
        default_binding: ".",
        alternate_bindings: &[],
        contexts: Context::Reader.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleReadingPane,
        title: "Read beside the list or over it",
        // F8, the key Evolution and Thunderbird give their message pane
        // (specs/007-postio-focus T232). The List context, which the
        // Reader falls back to, so it moves an open message too.
        default_binding: "F8",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List]),
        destructive: false,
        // A view preference, written to config.toml; pressed again, back.
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::Search,
        title: "Search",
        default_binding: "/",
        alternate_bindings: &["alt+mod+f"],
        // The go-to surfaces, for the go-to reason: the folder list is one
        // pane over, and nobody checks which pane has the keyboard before
        // reaching for search.
        contexts: ctx(GO_SURFACES).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SaveSearch,
        title: "Save search as folder",
        // Shares a binding with `save_draft`, which is fine: the two
        // contexts do not overlap, and `ctrl+s` is the "save this" muscle
        // memory in both. See `postio-search`/canvas 2b's "save as folder".
        default_binding: "mod+s",
        alternate_bindings: &[],
        // Only reachable with the search box open -- saving needs a query
        // to save, and `Context::Search` is where one exists.
        contexts: Context::Search.as_set().with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    // -- Compose ---------------------------------------------------------
    CommandSpec {
        id: CommandId::BackToWords,
        title: "Back to words",
        // Beside the word-erasing `mod+BackSpace` every text field knows:
        // this takes the chips back to the words they were typed as.
        default_binding: "mod+BackSpace",
        // A terminal delivers `ctrl+BackSpace` as plain backspace, so the
        // terminal's key is `alt+BackSpace`.
        alternate_bindings: &["alt+BackSpace"],
        // In the Mac's results, where nothing matched, the same key clears
        // the filters and keeps the words (spec 010 D24): there are no
        // words to go back to, and a second command on the key would be
        // two meanings in one place.
        contexts: Context::Search.as_set().with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    // Spec 010: the Mac's search dropdown, drawn as pixels (so not the
    // terminal's). Neither is offered on Linux until it adopts the dropdown
    // (`offered_on`, D25).
    CommandSpec {
        id: CommandId::ShowAllResults,
        title: "Show all results",
        // Where `mod+Return` sends in the composer: contexts do not overlap.
        default_binding: "mod+Return",
        alternate_bindings: &[],
        contexts: Context::Search.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ForgetRecent,
        title: "Forget recent search",
        // The design names the Mac's Option-Delete (D23).
        default_binding: "alt+BackSpace",
        alternate_bindings: &[],
        contexts: Context::Search.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // Spec 010 step 8 (US7): ↩ on a person, label or folder makes its
    // chip, ⌥↩ the excluded chip. Where `alt+Return` sends in a terminal's
    // composer and writes a capture: contexts do not overlap.
    CommandSpec {
        id: CommandId::ExcludeSuggestion,
        title: "Exclude suggestion",
        default_binding: "alt+Return",
        alternate_bindings: &[],
        contexts: Context::Search.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // Spec 010 step 3: the Mac's results view (R10), a mode of the main
    // window with its own context. History moves between it and the inbox
    // the way a browser's back and forward do -- from either, so the inbox
    // names it too -- and ⌘1-3 pick its tabs.
    // None is offered on Linux until it adopts the results view (D25).
    CommandSpec {
        id: CommandId::HistoryBack,
        title: "Back",
        default_binding: "mod+bracketleft",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List, Context::Results]),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::HistoryForward,
        title: "Forward",
        default_binding: "mod+bracketright",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List, Context::Results]),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ResultsConversations,
        title: "Show conversations",
        default_binding: "mod+1",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ResultsFiles,
        title: "Show files",
        default_binding: "mod+2",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::ResultsPeople,
        title: "Show people",
        default_binding: "mod+3",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // ⌥←/⌥→ step the timeline's range by a month (spec 010 FR-023): the
    // results' own, and the Mac's until Linux adopts them (D25).
    CommandSpec {
        id: CommandId::StepRangeBack,
        title: "Earlier month",
        default_binding: "alt+Left",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::StepRangeForward,
        title: "Later month",
        default_binding: "alt+Right",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // Space looks inside a result without opening it, and ]/[ walk its
    // matches while it is open (spec 010 US4, FR-028, design §3.7): the
    // results' own, and the Mac's until Linux adopts them (D25). The same
    // brackets walk a thread's messages and a digest's references, each in
    // its own context.
    CommandSpec {
        id: CommandId::QuickLook,
        title: "Quick Look",
        default_binding: "space",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::NextMatch,
        title: "Next match",
        default_binding: "]",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::PrevMatch,
        title: "Previous match",
        default_binding: "[",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // A search that found nothing offers up to four looser ones, numbered
    // (spec 010 US6, FR-030, design §3.10): the results' own, and the
    // Mac's until Linux adopts them (D25). The same digits pick a picker's
    // options and Filtered's tabs, each in its own context.
    CommandSpec {
        id: CommandId::PickRelaxation1,
        title: "Run looser search 1",
        default_binding: "1",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::PickRelaxation2,
        title: "Run looser search 2",
        default_binding: "2",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::PickRelaxation3,
        title: "Run looser search 3",
        default_binding: "3",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::PickRelaxation4,
        title: "Run looser search 4",
        default_binding: "4",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    // ⌘↓ saves the file under the ring on the Files tab (spec 010 FR-031,
    // design §3.8): Finder's "open" chord, for the file a card stands for.
    // The results' own, and the Mac's until Linux adopts them (D25).
    CommandSpec {
        id: CommandId::SaveFile,
        title: "Save file",
        default_binding: "mod+Down",
        alternate_bindings: &[],
        contexts: Context::Results.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_GRAPHICAL_MAIL,
    },
    CommandSpec {
        id: CommandId::Compose,
        title: "Compose",
        default_binding: "c",
        alternate_bindings: &["mod+n"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Send,
        title: "Send",
        // `ctrl+Return`, and `cmd+Return` on the Mac, where the menu draws
        // it as `⌘↩` (spec 007, decision C28).
        default_binding: "mod+Return",
        // `alt+s` before `alt+Return`: a terminal delivers it everywhere,
        // where many take `ctrl+Return` or `alt+Return` for their own
        // fullscreen.
        alternate_bindings: &["alt+s", "alt+Return"],
        contexts: Context::Composer.as_set(),
        // Not destructive — but it is externally visible and irreversible once
        // the queue drains, so it earns an undo-send window rather than a modal.
        destructive: false,
        recovery: Recovery::Window,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ScheduleSend,
        // The ellipsis matches `Attach file…`: neither command finishes on
        // its own, both open a picker the keystroke or palette row cannot
        // resolve a payload for.
        title: "Schedule send…",
        // Beside `ctrl+Return`, not sharing it: this opens the picker rather
        // than sending, so it earns its own keystroke rather than a modifier
        // on Send's.
        default_binding: "mod+shift+Return",
        alternate_bindings: &["alt+S"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        // Opening the picker commits nothing; `Recovery::Undo` belongs to
        // whichever time the user picks, exactly as it does for `Send`.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "mod+s",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::DiscardDraft,
        title: "Discard draft",
        default_binding: "mod+d",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        // Typed prose has no other copy anywhere, so this one asks first.
        destructive: true,
        recovery: Recovery::Confirm,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::MarkSent,
        title: "Mark as sent",
        // #674 called for palette-only, and this table cannot: PRODUCT.md §8
        // says every command is reachable by keyboard, and
        // `command_registry.rs` asserts it. So it gets a real binding.
        //
        // `mod+shift+m` for "mark", not the `mod+shift+s` this first took:
        // that one is spoken for in the List context by an extension in
        // `gtk_extension_commands.rs`, and a built-in quietly winning a key
        // an extension asked for is a conflict that shows up as the
        // extension's binding vanishing from the palette rather than as an
        // error. #495's landing caught it.
        default_binding: "mod+shift+m",
        alternate_bindings: &["alt+m"],
        contexts: ctx(&[Context::List, Context::Composer]),
        // It settles a question rather than destroying anything: the mail is
        // either already delivered or it is not, and this changes only what
        // Postio claims to know.
        destructive: false,
        // #674 asked for `Undo`. An inverse would have to be a second
        // registry command -- with its own binding, under PRODUCT.md §8 --
        // invented for something no user reaches for. And undo is the wrong
        // instrument: this settles a claim about the world rather than
        // changing it, so the correction for a wrong answer is to send the
        // message again, which is a real act. See `Actions::mark_sent`.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::RetrySend,
        title: "Retry send",
        // `mod+shift+y`, beside `mod+shift+m` for the same family: a message
        // that left the composer and did not arrive.
        //
        // **It was `mod+shift+r`, and that was free when it was chosen.** It
        // stopped being free the moment the macOS frontend gave every
        // menu-shaped verb a chord to show: Reply to all takes `mod+shift+r`
        // there because that is the accelerator every Mac mail client uses
        // for it, and `⇧⌘R` doing something else in Postio would be Postio
        // being wrong about the platform rather than opinionated. On
        // Freedesktop the same string resolves to `ctrl+shift+r`, which is
        // Thunderbird's Reply All, so the convention holds on both.
        //
        // Retrying a send is Postio's own verb with no convention to honour
        // and no client to agree with, and it is reached from a banner on the
        // failed message far more often than from the keyboard -- so when two
        // verbs want one chord, this is the one that moves. `y` is free
        // across the whole table.
        default_binding: "mod+shift+y",
        alternate_bindings: &["alt+r"],
        // List, because the Outbox and Drafts are lists and that is where a
        // stopped send is looked at. Composer, because the same draft can be
        // open there with its failure showing (#1487).
        contexts: ctx(&[Context::List, Context::Composer]),
        // It puts a message back on its way rather than destroying one, and
        // the thing it acts on is already not arriving.
        destructive: false,
        // The inverse is `CancelSend`, which is a real command a person can
        // reach rather than an invented one -- so unlike `MarkSent` this does
        // have a way back, and it is the command below.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::CancelSend,
        title: "Cancel send",
        default_binding: "mod+shift+x",
        alternate_bindings: &["alt+x"],
        contexts: ctx(&[Context::List, Context::Composer]),
        // It stops something from happening rather than losing anything: the
        // draft is left editable, which is the state it came from. Opening a
        // queued draft has done exactly this since #433, silently; this is
        // the same act with a name.
        destructive: false,
        // Asking again is `RetrySend`, and the draft is still there either
        // way. Refused outright once the submission is in flight, which is a
        // rejection rather than something to undo (ADR 0021).
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "mod+shift+a",
        alternate_bindings: &["alt+a"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::DetachComposer,
        // A toggle, and named for the direction the
        // user has to ask for: in-place is the default and detaching is the
        // opt-in, so "Detach composer" is what someone looking for it in the
        // palette will type. Offered only while composing, which is also the
        // only time the other direction can be reached.
        title: "Detach composer",
        // Not next to `ctrl+d`. Discard is the one composer verb that cannot
        // be undone, and a fat-fingered neighbour of it is a draft gone.
        default_binding: "mod+shift+o",
        alternate_bindings: &["alt+o"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::CopyFields,
        // Named for the pair rather than for `+ Cc`, because the button is
        // only the way in and this verb is also the way out. "Cc and Bcc" is
        // what someone hunting the palette for a Bcc field will type.
        title: "Cc and Bcc",
        // The `mod+shift+<letter>` shelf every secondary composer verb sits
        // on, and `c` for the field it names -- which is also what other mail
        // clients bind. `mod+c` is copy and stays copy.
        default_binding: "mod+shift+c",
        alternate_bindings: &["alt+c"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        // Nothing durable changes: this raises and lowers two rows, and it
        // refuses to lower them while they hold anything. There is nothing to
        // take back.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::InsertImage,
        // "Insert image…" rather than "Attach image": the ellipsis says a
        // chooser opens, and the verb is what keeps it distinct from
        // `attach_file` in a palette where both are one search away (FR-049).
        title: "Insert image…",
        // Beside `insert_link` on the `mod+shift+<letter>` shelf, because
        // they are the two verbs that put something *into* the text.
        default_binding: "mod+shift+g",
        alternate_bindings: &["alt+g"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        // The editor's own undo takes it back out, like any other edit.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::EditExternally,
        title: "Edit in external editor",
        // `e` for editor, on the composer's `mod+shift+<letter>` shelf;
        // plain `mod+e` is Edit config everywhere.
        default_binding: "mod+shift+e",
        alternate_bindings: &["alt+e"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        // The body comes back as the editor saved it; the composer's own
        // undo takes the change back.
        recovery: Recovery::None,
        requires: TERMINAL_MAIL,
    },
    CommandSpec {
        id: CommandId::TogglePreview,
        title: "Toggle preview",
        default_binding: "mod+shift+p",
        alternate_bindings: &["alt+p"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: TERMINAL_MAIL,
    },
    CommandSpec {
        id: CommandId::Bold,
        title: "Bold",
        default_binding: "mod+b",
        alternate_bindings: &[],
        // ctrl+b is the sidebar everywhere mail is read; the composer is not
        // a message surface, so the convention every editor shares wins here.
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Italic,
        title: "Italic",
        default_binding: "mod+i",
        alternate_bindings: &["alt+i"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::BulletList,
        title: "Bulleted list",
        // The Docs/Gmail convention, and shift dodges nothing here — the
        // digits are free in the composer either way.
        default_binding: "mod+shift+8",
        alternate_bindings: &["alt+8"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::NumberedList,
        title: "Numbered list",
        default_binding: "mod+shift+7",
        alternate_bindings: &["alt+7"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::InsertLink,
        title: "Insert link…",
        // Everywhere else this is ctrl+k, and here ctrl+k is the palette —
        // which is universal or it is not a palette. Shift is the tax.
        default_binding: "mod+shift+k",
        alternate_bindings: &["alt+k"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::QuoteBlock,
        title: "Quote block",
        default_binding: "mod+shift+9",
        alternate_bindings: &["alt+9"],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    // -- View and application --------------------------------------------
    CommandSpec {
        id: CommandId::Undo,
        title: "Undo",
        // `mod+z`, the undo every other application has taught
        // (specs/007-postio-focus contracts/keymap.md). A terminal delivers
        // it as `ctrl+z`, and nothing there suspends.
        default_binding: "mod+z",
        alternate_bindings: &[],
        // Plus the account list. #464 built account removal as a soft delete
        // with a toast wired straight to AccountRepository::restore rather
        // than through the global stack, and said so because Remove was not a
        // command then. Registering it with Recovery::Undo makes that a
        // declaration, and a declaration nothing backs from the keyboard is
        // what ADR 0005 keeps refusing to ship -- so undo reaches the toast
        // while it is up. Context-local state, context-local binding; the
        // global stack is untouched (ADR 0005 Q6c). And Focus's digest and
        // Filtered: archiving a whole digest and a restore from Filtered are
        // each one undoable action (FR-125, contracts/keymap.md).
        contexts: ctx(MESSAGE_SURFACES)
            .with(Context::Accounts)
            .with(Context::Digest)
            .with(Context::Filtered),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::CommandPalette,
        title: "Command palette",
        default_binding: "mod+k",
        alternate_bindings: &[],
        // Universal, or it is not a command palette.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: CHROME,
    },
    CommandSpec {
        id: CommandId::CheatSheet,
        title: "Keyboard shortcuts",
        default_binding: "?",
        alternate_bindings: &[],
        // Not while composing or searching: there `?` is a character.
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: CHROME,
    },
    CommandSpec {
        id: CommandId::Settings,
        title: "Settings",
        default_binding: "mod+comma",
        alternate_bindings: &["alt+comma"],
        // Universal, like the palette it is an alternative to reaching.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::AddAccount,
        title: "Add account",
        // Not a letter: this is a rare, deliberate act, and every unmodified
        // key in the message surfaces is spoken for by something done dozens
        // of times a session. `n` for "new" is the idiom, and `Ctrl+Shift+N`
        // is where the desktop already puts "a new one of the thing this
        // application is about".
        default_binding: "mod+shift+n",
        alternate_bindings: &["alt+n"],
        // The same reach `Settings` has, for the reason ADR 0012 Q1 gives:
        // adding an account is a setting, and the folder list is where the
        // account will eventually appear.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "mod+e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: CHROME,
    },
    CommandSpec {
        id: CommandId::Quit,
        title: "Quit Postio",
        default_binding: "mod+q",
        // Close window, everywhere on the desktop: Postio has one window,
        // so closing it is quitting (spec 007 T216).
        alternate_bindings: &["mod+w"],
        // Universal, and chrome: quitting means the same with an empty window
        // as with a full one.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: CHROME,
    },
    CommandSpec {
        id: CommandId::ShowImages,
        title: "Show remote images",
        // A sequence under `i` for images: once, or always from this sender.
        default_binding: "i i",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::AlwaysShowImages,
        title: "Always show images from this sender",
        default_binding: "i a",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Unsubscribe,
        title: "Unsubscribe from this list",
        // Shifted and deliberate: an unsubscribe tells the sender the address
        // is read, so it is never one stray keystroke away. `U`, and in a
        // digest too (specs/007-postio-focus contracts/keymap.md).
        default_binding: "U",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES).with(Context::Digest),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToFolders,
        title: "Go to folders",
        // `g` is already the "go to" prefix — `g g` is the first message — so
        // "go to folders" reads as one idiom rather than a second one. Focus
        // opens its folders popover. `g f` is Focus's Filtered.
        default_binding: "g o",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToInbox,
        title: "Go to inbox",
        // `g` is already this app's "go to" prefix -- `g g` is the first
        // message, `g o` the folders -- so a destination reads as the
        // same idiom rather than a second one. `i` for inbox, which is what every mail client on the web binds it to.
        //
        // Targets the *role*, not a name: an inbox a provider calls something
        // else, or names in another language, is still where `g i` goes.
        default_binding: "g i",
        alternate_bindings: &[],
        // The surfaces a person is standing on when they want to be somewhere
        // else -- the folder list included. Not the composer, where `g` is a
        // letter being typed.
        // And back from Focus's Filtered view and digest window (screen
        // 21's footer: "g i inbox").
        contexts: ctx(GO_SURFACES)
            .with(Context::Filtered)
            .with(Context::Digest),
        destructive: false,
        // Going somewhere destroys nothing, so there is nothing to get back.
        recovery: Recovery::None,
        // The store, like every other way of moving between folders: there
        // are no folders to go to without one.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToDrafts,
        title: "Go to drafts",
        // `g` is already this app's "go to" prefix -- `g g` is the first
        // message, `g o` the folders -- so a destination reads as the
        // same idiom rather than a second one. `t`, since `g d` is Focus's digest rules (specs/007-postio-focus contracts/keymap.md).
        //
        // Targets the *role*, not a name: an inbox a provider calls something
        // else, or names in another language, is still where `g i` goes.
        default_binding: "g t",
        alternate_bindings: &[],
        // The surfaces a person is standing on when they want to be somewhere
        // else -- the folder list included. Not the composer, where `g` is a
        // letter being typed.
        contexts: ctx(GO_SURFACES),
        destructive: false,
        // Going somewhere destroys nothing, so there is nothing to get back.
        recovery: Recovery::None,
        // The store, like every other way of moving between folders: there
        // are no folders to go to without one.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToSent,
        title: "Go to sent",
        // `g` is already this app's "go to" prefix -- `g g` is the first
        // message, `g o` the folders -- so a destination reads as the
        // same idiom rather than a second one. `s` for sent, now that the drafts have `t` (specs/007-postio-focus contracts/keymap.md).
        //
        // Targets the *role*, not a name: an inbox a provider calls something
        // else, or names in another language, is still where `g i` goes.
        default_binding: "g s",
        alternate_bindings: &[],
        // The surfaces a person is standing on when they want to be somewhere
        // else -- the folder list included. Not the composer, where `g` is a
        // letter being typed.
        contexts: ctx(GO_SURFACES),
        destructive: false,
        // Going somewhere destroys nothing, so there is nothing to get back.
        recovery: Recovery::None,
        // The store, like every other way of moving between folders: there
        // are no folders to go to without one.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToFlagged,
        title: "Go to flagged",
        // `g` is already this app's "go to" prefix -- `g g` is the first
        // message, `g o` the folders -- so a destination reads as the
        // same idiom rather than a second one. `*`, a star, the key flagging has (specs/007-postio-focus contracts/keymap.md), and the sidebar says Flagged (docs/PRODUCT.md).
        //
        // Targets the *role*, not a name: an inbox a provider calls something
        // else, or names in another language, is still where `g i` goes.
        default_binding: "g *",
        alternate_bindings: &[],
        // The surfaces a person is standing on when they want to be somewhere
        // else -- the folder list included. Not the composer, where `g` is a
        // letter being typed.
        contexts: ctx(GO_SURFACES),
        destructive: false,
        // Going somewhere destroys nothing, so there is nothing to get back.
        recovery: Recovery::None,
        // The store, like every other way of moving between folders: there
        // are no folders to go to without one.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToArchive,
        title: "Go to archive",
        // `r` for the archive, which had no letter until the one keymap
        // (specs/007-postio-focus contracts/keymap.md). A role, like the rest.
        default_binding: "g r",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToSnoozed,
        title: "Go to snoozed",
        // `z`, the sleeping letter.
        default_binding: "g z",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::GoToOutbox,
        title: "Go to outbox",
        // `b`, the box that holds what is on its way out: `o` is the folders.
        default_binding: "g b",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::GoToJunk,
        title: "Go to junk",
        default_binding: "g j",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::GoToTrash,
        title: "Go to trash",
        // `#`, the key that deletes, as `*` is the key that flags.
        default_binding: "g #",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::GoToFiltered,
        title: "Go to Filtered",
        // What Focus filtered out of the inbox, and why.
        default_binding: "g f",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::GoToDigestRules,
        title: "Go to digest rules",
        default_binding: "g d",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::SavedSearch1,
        title: "Saved search 1",
        // The pinned `[saved_searches]` entries, in their order: a saved search is a
        // place a person goes, so the four come with the destinations.
        default_binding: "alt+1",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SavedSearch2,
        title: "Saved search 2",
        default_binding: "alt+2",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SavedSearch3,
        title: "Saved search 3",
        default_binding: "alt+3",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SavedSearch4,
        title: "Saved search 4",
        default_binding: "alt+4",
        alternate_bindings: &[],
        contexts: ctx(GO_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleHasAction,
        title: "Show only what has an action",
        // Pressed again, everything is back. The inbox list is where the
        // filter is, and where its toggle is drawn (FR-017).
        default_binding: "!",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List]).with(Context::Results),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleAccountEnabled,
        title: "Enable or disable account",
        default_binding: "Return",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: false,
        // Pressing it again is the reversal, so there is nothing for the undo
        // stack to hold (ADR 0005 Q6c).
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::RemoveAccount,
        title: "Remove account",
        // `Delete`, the key the message verb has: "delete" has one key.
        default_binding: "Delete",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: true,
        // Unlike a config-file edit, which has no undo stack to reach: #464
        // built removal as a soft delete with a toast wired to
        // AccountRepository::restore, and reaped at the next start. So there
        // is something to undo for as long as the toast is up, and declaring
        // it here is what the registry enforces a keyboard path for.
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::UpdateCredential,
        title: "Update account credential",
        // `c` for credential. ADR 0005 Q6c wanted this one palette-only, on
        // the grounds that "ten commands already have none" -- but none do,
        // and PRODUCT.md §8 makes a shortcut a structural requirement that
        // `every_command_has_an_id_a_title_and_a_default_binding` enforces.
        // The ADR's actual point was discoverability, which the palette entry
        // gives it either way; the exemption was the part resting on a wrong
        // count. Nothing is shadowed: Compose's `c` is scoped to the message
        // surfaces, and this context layers over Global alone.
        default_binding: "c",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::RebuildAccountIndex,
        title: "Rebuild search index",
        // `r` for rebuild. Free within `Context::Accounts` -- the other three
        // rows here use `Return`, `d` and `c`, and `Refresh`'s own `R` is
        // scoped to the message surfaces, not this one.
        default_binding: "r",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: false,
        // Rewriting a derived table -- postio_session::reindex_account's own
        // doc explains why there is nothing here for undo to reach.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::SetDefaultAccount,
        // Says what it does rather than asserting a status (#960). "Primary"
        // invites the reading the decision rules out -- that this account is
        // more the user's than the other one -- so the word is "default"
        // everywhere, never "primary" and never "main".
        title: "Set as default account",
        // `m` for "make default". Free within `Context::Accounts`: the other
        // four rows here use `Return`, `d`, `c` and `r`, and `Move`'s `m` is
        // scoped to the message surfaces, which this context does not layer
        // over.
        default_binding: "m",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: false,
        // On `ToggleAccountEnabled`'s stated precedent: the reversal is the
        // same key on another row, so there is nothing for the undo stack to
        // hold. Nothing is lost either -- the previous holder is still there,
        // unmarked.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::MapMailboxRole,
        title: "Map mailbox role",
        // `M` for map. This branch was cut when `m` was free and #960 took it
        // for `SetDefaultAccount` in the meantime; shift is how this app
        // spells the neighbour of a letter already spoken for (`a`/`A`,
        // `j`/`J`), so the mnemonic survives the collision. `Move`'s own `m`
        // is scoped to the message surfaces and this context layers over
        // Global alone, so nothing is shadowed either way.
        default_binding: "M",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: false,
        // The previous mapping is the inverse, and a wrong pick costs one
        // keystroke rather than a dialog (ADR 0035).
        recovery: Recovery::Undo,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::NextScope,
        title: "Next scope",
        // `g` is already the app's "go to" prefix (`g g`, `g f`), and this is
        // the same gesture aimed at an account rather than a row or a folder.
        default_binding: "g a",
        alternate_bindings: &[],
        // Reachable from the surfaces a scope actually changes -- the folder
        // list it re-roots and the message list it re-fills. Not from the
        // composer or the reader, where the mail on screen is already chosen.
        contexts: ctx(&[Context::Sidebar, Context::List]),
        destructive: false,
        // Which accounts are in view is view state, like which folders are
        // expanded. Nothing durable for undo to reach.
        recovery: Recovery::None,
        // Deliberately not `SingleAccount`: this is the command that *leaves*
        // a single-account scope, so requiring one would switch itself off.
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::Refresh,
        title: "Refresh",
        // Also the retry for the empty and error states: "retry now" and
        // "check for new mail now" are the same command from the user's
        // chair. `R` was its second key
        // until the one keymap gave it to restoring from Filtered.
        default_binding: "F5",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    // -- Reader --------------------------------------------------------
    CommandSpec {
        id: CommandId::ScrollReaderDown,
        title: "Scroll reading pane down",
        default_binding: "Page_Down",
        // `Space` reads a page and moves on in most mail and feed readers;
        // offered alongside `Page_Down` rather than instead of it; see
        // `ScrollReaderUp` for why the shifted form is its pair rather than
        // a binding of its own.
        alternate_bindings: &["space"],
        // Not in the conversation pane, where `space` folds the focused
        // message instead (canvas turn 8a, #1007). A real trade rather than
        // a free one: a long message inside a stack loses its page-turn key
        // and keeps `Page_Down`. The canvas is explicit, and folding is the
        // gesture a stack is *for* -- scrolling is what the scrollbar and
        // the wheel already do.
        //
        // The conversation is here now, and `space` with it (#1402).
        // `ScrollReaderUp` always served `MESSAGE_SURFACES` while this row
        // did not, so `Page_Up` resolved in a thread and `Page_Down` did
        // not -- an asymmetry nothing ever argued for. The two directions
        // serve the same surfaces.
        contexts: ctx(&[Context::List, Context::Reader, Context::Conversation]),
        destructive: false,
        // What the pane is scrolled to is view state, not durable data —
        // nothing here for undo to reach.
        recovery: Recovery::None,
        requires: MAIL,
    },
    CommandSpec {
        id: CommandId::ScrollReaderUp,
        title: "Scroll reading pane up",
        default_binding: "Page_Up",
        alternate_bindings: &["shift+space"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: MAIL,
    },
    // -- Pickers (specs/007-postio-focus) ---------------------------------
    CommandSpec {
        id: CommandId::PickerChoose1,
        title: "Choose option 1",
        // A picker's options are numbered, and a digit takes one.
        default_binding: "1",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerChoose2,
        title: "Choose option 2",
        default_binding: "2",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerChoose3,
        title: "Choose option 3",
        default_binding: "3",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerChoose4,
        title: "Choose option 4",
        default_binding: "4",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerTypeDate,
        title: "Type a date",
        default_binding: "tab",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerToggle,
        title: "Toggle option",
        default_binding: "space",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PickerConfirm,
        title: "Confirm",
        default_binding: "Return",
        alternate_bindings: &[],
        contexts: Context::Picker.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    // -- Digests (specs/007-postio-focus milestone 2) ----------------------
    CommandSpec {
        id: CommandId::NextReference,
        title: "Next reference",
        default_binding: "]",
        alternate_bindings: &[],
        contexts: Context::Digest.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::PrevReference,
        title: "Previous reference",
        default_binding: "[",
        alternate_bindings: &[],
        contexts: Context::Digest.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::ToggleDigestSummary,
        title: "Summary or messages",
        default_binding: "tab",
        alternate_bindings: &[],
        contexts: Context::Digest.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    // -- Filtered (specs/007-postio-focus) ---------------------------------
    CommandSpec {
        id: CommandId::RestoreFiltered,
        title: "Restore to inbox",
        // And never filter the sender again (FR-116).
        default_binding: "R",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab1,
        title: "Reason 1",
        // Filtered's reason tabs, numbered like a picker's options.
        default_binding: "1",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab2,
        title: "Reason 2",
        default_binding: "2",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab3,
        title: "Reason 3",
        default_binding: "3",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab4,
        title: "Reason 4",
        default_binding: "4",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab5,
        title: "Reason 5",
        default_binding: "5",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab6,
        title: "Reason 6",
        default_binding: "6",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::FilteredTab7,
        title: "Reason 7",
        default_binding: "7",
        alternate_bindings: &[],
        contexts: Context::Filtered.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::SweepInbox,
        title: "Filter what is in the inbox…",
        // `F` for filter, on the inbox list: no app binds it there, and the
        // one keymap's enumeration holds it so (specs/007-postio-focus T128;
        // contracts/keymap.md names no key for it). The ellipsis says the
        // count comes first.
        default_binding: "F",
        alternate_bindings: &[],
        contexts: Context::List.as_set(),
        // It files mail away, many at a time: one undo takes it back.
        destructive: true,
        recovery: Recovery::Undo,
        requires: FOCUS_MAIL,
    },
    // -- Obsidian capture (specs/007-postio-focus milestone 3, T158) --------
    CommandSpec {
        id: CommandId::CaptureTask,
        // The ellipsis says a sheet opens before anything is written.
        title: "Capture a task…",
        default_binding: "t",
        alternate_bindings: &[],
        // From a message, and inside the sheet, where its Task toggle shows
        // the same key (screen 25).
        contexts: ctx(CAPTURE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::CaptureNote,
        title: "Capture a note…",
        default_binding: "n",
        alternate_bindings: &[],
        contexts: ctx(CAPTURE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::CaptureChangeProject,
        title: "Change project",
        default_binding: "mod+p",
        alternate_bindings: &[],
        contexts: Context::Capture.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::CaptureUseSubject,
        title: "Use the subject instead",
        default_binding: "alt+s",
        alternate_bindings: &[],
        contexts: Context::Capture.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::CaptureWrite,
        title: "Add to the vault",
        default_binding: "mod+Return",
        // The composer's `send` alternate, for a terminal that cannot send
        // `ctrl+Return`.
        alternate_bindings: &["alt+Return"],
        contexts: Context::Capture.as_set(),
        // It appends a line to a note on this computer, which the person
        // can delete there; Postio never edits a note beyond appending.
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
    CommandSpec {
        id: CommandId::DigestLikeThis,
        title: "Digest mail like this",
        // Additive (specs/007-postio-focus T155): a rule-dialog control,
        // reached from a message the way `d` (`DigestRule`) is; contracts/
        // keymap.md names no key for it, the same as `SweepInbox` above.
        // It reads mail (the candidate queries and their preview), so
        // `FOCUS_MAIL` -- `Requirement::Focus` plus the store being open --
        // the same as `DigestRule`'s. Whether it is offered *at all* is a
        // further, narrower check the dialog makes on its own
        // (`config.focus.model_for(ModelFeature::LikeThis)`), because that
        // depends on `[focus.model]`, which the registry does not read.
        default_binding: "L",
        alternate_bindings: &[],
        contexts: Context::List.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: FOCUS_MAIL,
    },
];

/// Every command, in cheat-sheet order.
///
/// This is the enumeration the palette and the cheat sheet are built from.
pub fn all() -> impl Iterator<Item = &'static CommandSpec> {
    SPECS.iter()
}

/// The commands reachable in `context`, in cheat-sheet order.
pub fn for_context(context: Context) -> impl Iterator<Item = &'static CommandSpec> {
    SPECS.iter().filter(move |spec| spec.available_in(context))
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

/// The command bound to `binding` in `context`, if any.
///
/// A convenience for the keymap resolver's *default* map; user overrides from
/// `[keys]` are applied on top of this, not here.
pub fn lookup_binding(context: Context, binding: &str) -> Option<&'static CommandSpec> {
    lookup_binding_on(context, binding, Platform::host())
}

/// [`lookup_binding`] for a named platform.
///
/// `binding` is a concrete accelerator — it came from a key press — while the
/// registry stores `mod+…` tokens, so the *candidates* are what get expanded.
/// Doing it the other way round would be wrong: there is nothing to resolve on
/// the pressed side, and `ctrl+k` on a Mac must not match a `mod+k` default.
pub fn lookup_binding_on(
    context: Context,
    binding: &str,
    platform: Platform,
) -> Option<&'static CommandSpec> {
    for_context(context).find(|spec| {
        spec.bindings().any(|candidate| {
            // Only the tokens allocate; most bindings are plain keys like `j`.
            match candidate.contains("mod+") {
                true => postio_config::keys::expand_mod(candidate, platform) == binding,
                false => candidate == binding,
            }
        })
    })
}

/// Whether rebinding `command` to `proposed` (already syntax-checked) would
/// collide with another command's *currently effective* binding — default
/// or override, whichever `bindings` resolves to — in a context the two
/// share. `None` means the rebind is free to take everywhere `command`
/// itself is reachable; `Some` names the command it would silently shadow
/// (#881: the capture widget surfaces this rather than overwriting).
///
/// Scoped by context on purpose: two commands may validly share a binding
/// in disjoint contexts (`a` archives in [`Context::List`], something else
/// entirely in [`Context::Composer`]), the same reason `postio-ui`'s own
/// `KeyContext::chain` keeps contexts from falling through into each other.
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

/// The key a command actually answers to: the user's override if `[keys]` set
/// one, otherwise the registry's own default.
///
/// It has to be built here rather than asked of `KeyBindings`, and that is the
/// whole of #1227. `postio-config` cannot see this registry — the dependency
/// runs the other way, for `expand_mod` — so its `binding_on` could only
/// answer from a short table of its own, which listed 23 of the 79 commands
/// here. Every one of the other 56 read as unbound: `delete`, `send`,
/// `mark_unread` and, the one that surfaced it, `flag`.
///
/// The registry is the single source of truth the module doc has always
/// claimed. Asking it directly is what makes that true.
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

// ---------------------------------------------------------------------------
// Extension commands
// ---------------------------------------------------------------------------

/// A command an extension asks the registry to add.
///
/// The owned counterpart of [`CommandSpec`], because nothing about a command
/// loaded at runtime is `'static` at the call site. `destructive` and
/// `recovery` are mandatory rather than defaulted for the reason
/// [`register`] rejects the bad pair: an unrecoverable action nobody typed is
/// the failure this registry exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtCommand {
    /// The namespaced id: `"mcp:summarise-thread"`.
    pub id: String,
    /// The title the palette and cheat sheet show.
    pub title: String,
    /// The binding to ask for, in the same untyped syntax `[keys]` uses.
    /// `None` means palette-only, which is a perfectly good answer.
    pub default_binding: Option<String>,
    /// Secondary bindings, as [`CommandSpec::alternate_bindings`].
    pub alternate_bindings: Vec<String>,
    /// The contexts this command is meaningful in.
    pub contexts: ContextSet,
    /// Whether it destroys something the user would have to rebuild.
    pub destructive: bool,
    /// How the user gets back. Never [`Recovery::None`] when `destructive`.
    pub recovery: Recovery,
}

/// Why a registration was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    /// `destructive` with [`Recovery::None`].
    ///
    /// The invariant `tests/command_registry.rs` asserts over the built-in
    /// table, moved into the door because a table that grows at runtime
    /// cannot be checked by a test over its literal.
    UnrecoverableDestructive,
    /// The id is not `namespace:name`, or it collides with the built-in
    /// vocabulary, which never contains the separator.
    NotNamespaced,
    /// Something already registered that id. Two commands with one id is the
    /// same wiring bug as two handlers for one id.
    AlreadyRegistered,
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistrationError::UnrecoverableDestructive => {
                f.write_str("a destructive command must offer undo or confirmation")
            }
            RegistrationError::NotNamespaced => {
                f.write_str("an extension command id must be `namespace:name`")
            }
            RegistrationError::AlreadyRegistered => {
                f.write_str("that command id is already registered")
            }
        }
    }
}

impl std::error::Error for RegistrationError {}

/// One registered extension command, with its strings leaked to `'static`.
///
/// Leaked rather than `Cow`: see `docs/decisions/0002`. It keeps
/// [`CommandSpec`] `Copy` and untouched, and registrations are append-only and
/// bounded by the number of extensions loaded, so their strings have exactly
/// the lifetime of the ids they sit beside.
#[derive(Debug, Clone, Copy)]
struct ExtSpec {
    id: ExtId,
    title: &'static str,
    default_binding: Option<&'static str>,
    alternate_bindings: &'static [&'static str],
    contexts: ContextSet,
    destructive: bool,
    recovery: Recovery,
}

fn extensions() -> &'static RwLock<Vec<ExtSpec>> {
    static EXTENSIONS: OnceLock<RwLock<Vec<ExtSpec>>> = OnceLock::new();
    EXTENSIONS.get_or_init(|| RwLock::new(Vec::new()))
}

fn read_extensions() -> std::sync::RwLockReadGuard<'static, Vec<ExtSpec>> {
    extensions()
        .read()
        .unwrap_or_else(|error| error.into_inner())
}

/// Add a command to the vocabulary at runtime.
///
/// This is the whole extension door: MCP tools, AI actions and anything else
/// loaded after compilation come through here and are thereafter reachable
/// from the palette, the cheat sheet and `[keys]` on the same footing as a
/// built-in. `ARCHITECTURE.md` §2 — a command that is not in the registry
/// does not exist — is why an extension mechanism must register rather than
/// bypass.
///
/// # Errors
///
/// See [`RegistrationError`]. All three are wiring bugs in the caller rather
/// than conditions to handle at runtime, but they are returned rather than
/// panicked because the caller may be loading somebody else's plugin.
pub fn register(command: ExtCommand) -> Result<ExtId, RegistrationError> {
    if command.destructive && command.recovery == Recovery::None {
        return Err(RegistrationError::UnrecoverableDestructive);
    }
    let id = ExtId::intern(&command.id).ok_or(RegistrationError::NotNamespaced)?;

    let mut registered = extensions()
        .write()
        .unwrap_or_else(|error| error.into_inner());
    if registered.iter().any(|spec| spec.id == id) {
        return Err(RegistrationError::AlreadyRegistered);
    }
    let alternates: Vec<&'static str> = command
        .alternate_bindings
        .into_iter()
        .map(|binding| &*Box::leak(binding.into_boxed_str()))
        .collect();
    registered.push(ExtSpec {
        id,
        title: Box::leak(command.title.into_boxed_str()),
        default_binding: command
            .default_binding
            .map(|binding| &*Box::leak(binding.into_boxed_str())),
        alternate_bindings: Box::leak(alternates.into_boxed_slice()),
        contexts: command.contexts,
        destructive: command.destructive,
        recovery: command.recovery,
    });
    Ok(id)
}

/// One row of the merged vocabulary: a built-in or a registered extension.
///
/// `Copy`, and every field `&'static`, so this behaves like the
/// [`CommandSpec`] it generalises and costs nothing to hand around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionSpec {
    /// The stable id.
    pub id: ActionId,
    /// The human-readable title, as the palette and cheat sheet show it.
    pub title: &'static str,
    /// The binding asked for, or `None` for palette-only.
    pub default_binding: Option<&'static str>,
    /// Secondary bindings for the same command.
    pub alternate_bindings: &'static [&'static str],
    /// The contexts this command is meaningful in.
    pub contexts: ContextSet,
    /// Whether it destroys something the user would have to rebuild.
    pub destructive: bool,
    /// How the user gets back.
    pub recovery: Recovery,
    /// What the state must be, beyond the focused surface. See
    /// [`Requirement`].
    pub requires: RequirementSet,
}

impl ActionSpec {
    /// The context predicate: whether this command is reachable in `context`.
    pub fn available_in(&self, context: Context) -> bool {
        self.contexts.contains(context)
    }

    /// Every binding for this command, the default first.
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

impl From<ExtSpec> for ActionSpec {
    fn from(spec: ExtSpec) -> Self {
        ActionSpec {
            id: ActionId::Ext(spec.id),
            title: spec.title,
            default_binding: spec.default_binding,
            alternate_bindings: spec.alternate_bindings,
            contexts: spec.contexts,
            destructive: spec.destructive,
            recovery: spec.recovery,
            // Every extension command needs the store, and none of them can
            // say otherwise yet. That is the safe default rather than a gap:
            // an MCP tool or an AI action is a thing done *to mail*, so
            // offering one before there is any would be the same broken
            // promise a built-in would make. A registration that wants to
            // name its own requirements adds a field here rather than a check
            // at a surface.
            requires: MAIL,
        }
    }
}

/// Every command reachable in `context` — built-in and registered alike.
///
/// This is what the palette, the cheat sheet and the key hints iterate, and
/// the reason an extension command is discoverable rather than merely
/// dispatchable. Built-ins come first, in cheat-sheet order, then extensions
/// in registration order: a plugin cannot reorder the vocabulary a user has
/// learned by registering early.
///
/// [`all`] and [`for_context`] deliberately keep meaning *the built-in table*,
/// so `docs/keybindings.md` keeps documenting what ships and the tests that
/// assert over what shipped keep compiling.
pub fn reachable(context: Context) -> impl Iterator<Item = ActionSpec> {
    let extensions: Vec<ActionSpec> = read_extensions()
        .iter()
        .filter(|spec| spec.contexts.contains(context))
        .map(|spec| ActionSpec::from(*spec))
        .collect();
    for_context(context).map(ActionSpec::from).chain(extensions)
}

/// Whether `action` is offered on `platform` at all (#1571, #1573).
///
/// Almost everything is offered everywhere: one vocabulary, one table, and a
/// frontend that has not built a surface yet lists the command as debt rather
/// than pretending it does not exist. This is for the other case -- a command
/// whose surface a platform's **design** does not have, so there is nothing
/// to build and a menu item for it would be a key that does nothing, drawn
/// where everybody looks.
///
/// Asked by everything that shows a command: the menu placement
/// ([`crate::menu::section_on`]), the keymap ([`crate::Keymap::resolve_on`],
/// which gives such a command no key there), and through it the palette, the
/// cheat sheet and the key resolver. So a platform answers "not here" once and
/// every surface agrees. Extensions are offered everywhere; they bring their
/// own surfaces.
///
/// A parameter, not a `cfg`, so either host can assert both answers.
pub fn offered_on(action: ActionId, platform: Platform) -> bool {
    use CommandId as C;
    !matches!(
        (action, platform),
        // Compose on the Mac is a window of its own (canvas 26) and never
        // takes over the reading pane, so there is nothing to detach.
        (ActionId::Builtin(C::DetachComposer), Platform::Apple)
            // The Mac's sidebar lists every account's folders at once, under
            // "On My Mac" (canvas 25): there is no account strip for `g a` to
            // cycle.
            | (ActionId::Builtin(C::NextScope), Platform::Apple)
            // Darkening a sender's design is `postio-render`'s, which
            // recolours a message *and* repairs its text contrast (spec 006
            // FR-012, FR-013a). The Mac reads mail in a web view with no such
            // repair, and recolouring there would break the contrast floor;
            // the maintainer chose to leave it out (2026-10-01, #1705).
            | (ActionId::Builtin(C::DarkenMessage), Platform::Apple)
            // The search dropdown is the Mac's until Linux adopts it
            // (spec 010 D23, D25).
            | (ActionId::Builtin(C::ShowAllResults), Platform::Freedesktop)
            | (ActionId::Builtin(C::ForgetRecent), Platform::Freedesktop)
            | (ActionId::Builtin(C::ExcludeSuggestion), Platform::Freedesktop)
            // And so is the results view (spec 010 D17, D25).
            | (ActionId::Builtin(C::HistoryBack), Platform::Freedesktop)
            | (ActionId::Builtin(C::HistoryForward), Platform::Freedesktop)
            | (ActionId::Builtin(C::ResultsConversations), Platform::Freedesktop)
            | (ActionId::Builtin(C::ResultsFiles), Platform::Freedesktop)
            | (ActionId::Builtin(C::ResultsPeople), Platform::Freedesktop)
            | (ActionId::Builtin(C::StepRangeBack), Platform::Freedesktop)
            | (ActionId::Builtin(C::StepRangeForward), Platform::Freedesktop)
            | (ActionId::Builtin(C::QuickLook), Platform::Freedesktop)
            | (ActionId::Builtin(C::NextMatch), Platform::Freedesktop)
            | (ActionId::Builtin(C::PrevMatch), Platform::Freedesktop)
            | (ActionId::Builtin(C::PickRelaxation1), Platform::Freedesktop)
            | (ActionId::Builtin(C::PickRelaxation2), Platform::Freedesktop)
            | (ActionId::Builtin(C::PickRelaxation3), Platform::Freedesktop)
            | (ActionId::Builtin(C::PickRelaxation4), Platform::Freedesktop)
            | (ActionId::Builtin(C::SaveFile), Platform::Freedesktop)
    )
}

/// Whether `platform` lets `action` take its alternate `binding` (as the
/// registry spells it, before `mod+` is expanded).
///
/// Two alternates are the desktop's and not the Mac's: `quit`'s `mod+w`, and
/// `back_to_words`' `alt+BackSpace`, which the Mac's `forget_recent` takes.
/// GTK's Postio has one window, so closing it is quitting (spec 007 T216);
/// on the Mac ⌘W closes the window in front -- the message, digest or
/// compose window over the list most of all -- through Window › Close, and
/// the key monitor sees a key before any menu does, so a ⌘W resolved to
/// `quit` ended the app (specs/009-focus-macos T105).
pub fn alternate_offered_on(action: ActionId, binding: &str, platform: Platform) -> bool {
    !matches!(
        (action, binding, platform),
        (ActionId::Builtin(CommandId::Quit), "mod+w", Platform::Apple)
            // The terminal's key for `back_to_words` is the Mac's for
            // `forget_recent` (spec 010 D23).
            | (
                ActionId::Builtin(CommandId::BackToWords),
                "alt+BackSpace",
                Platform::Apple
            )
    )
}

/// Every command reachable in `context` for a window in `state`.
///
/// What the palette, the cheat sheet and the key hints iterate. [`reachable`]
/// stays the state-blind form, because `docs/keybindings.md` documents the
/// whole vocabulary rather than one session's state — somebody looking up `m`
/// has to find it whatever is on screen.
pub fn reachable_in(context: Context, state: Availability) -> impl Iterator<Item = ActionSpec> {
    reachable(context).filter(move |spec| spec.requires.met_by(state))
}

/// Every command in the merged vocabulary, in the same order as [`reachable`].
pub fn every_action() -> impl Iterator<Item = ActionSpec> {
    let extensions: Vec<ActionSpec> = read_extensions()
        .iter()
        .map(|spec| ActionSpec::from(*spec))
        .collect();
    all().map(ActionSpec::from).chain(extensions)
}

/// The spec for any action, built-in or registered.
///
/// `None` only for an extension id that has been named but never registered —
/// which is a real state, not a bug: `[keys]` can bind an id before the
/// extension providing it loads. Total for every [`CommandId`], like [`get`].
pub fn spec(id: ActionId) -> Option<ActionSpec> {
    match id {
        ActionId::Builtin(id) => Some(ActionSpec::from(get(id))),
        ActionId::Ext(id) => read_extensions()
            .iter()
            .find(|spec| spec.id == id)
            .map(|spec| ActionSpec::from(*spec)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- what a platform offers ------------------------------------------------

    /// The two commands the Mac's design has no surface for (#1571, #1573).
    const NOT_ON_THE_MAC: [CommandId; 3] = [
        CommandId::DarkenMessage,
        CommandId::DetachComposer,
        CommandId::NextScope,
    ];

    /// What only the Mac's search dropdown and results view draw, until
    /// Linux adopts them (spec 010 D23, D25).
    const NOT_ON_LINUX_YET: [CommandId; 18] = [
        CommandId::ShowAllResults,
        CommandId::ForgetRecent,
        CommandId::ExcludeSuggestion,
        CommandId::HistoryBack,
        CommandId::HistoryForward,
        CommandId::ResultsConversations,
        CommandId::ResultsFiles,
        CommandId::ResultsPeople,
        CommandId::StepRangeBack,
        CommandId::StepRangeForward,
        CommandId::QuickLook,
        CommandId::NextMatch,
        CommandId::PrevMatch,
        CommandId::PickRelaxation1,
        CommandId::PickRelaxation2,
        CommandId::PickRelaxation3,
        CommandId::PickRelaxation4,
        CommandId::SaveFile,
    ];

    #[test]
    fn every_command_is_offered_on_freedesktop() {
        // GTK draws every surface these commands name, so nothing is scoped
        // away there but the Mac's dropdown -- this is the half that guards
        // the GTK build against a Mac decision leaking into it.
        let scoped: Vec<CommandId> = CommandId::ALL
            .iter()
            .copied()
            .filter(|id| !offered_on(ActionId::Builtin(*id), Platform::Freedesktop))
            .collect();
        assert_eq!(
            scoped, NOT_ON_LINUX_YET,
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
        // And nothing else: a list that grew quietly would be commands
        // vanishing from the Mac's menus with nobody deciding it.
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

    /// `space` turns the page in a conversation, and `z` folds (#1402).
    ///
    /// Canvas turn 8a gave `space` to folding, and that trade was made for a
    /// **stack**, where folding is the gesture the surface is for. FR-013
    /// (#1389) leaves the one-document pane nothing to fold, so `space` there
    /// bought nothing and cost the key every reading surface turns pages
    /// with. Maintainer settled it the other way on #1402: `space` pages, and
    /// folding moves to `z` -- free across the table, and where a vim user
    /// already looks for it.
    ///
    /// Asserted from the row that has to respect it, because
    /// `bindings_do_not_collide_within_a_context` can only say the two do not
    /// collide -- not which of them won.
    #[test]
    fn space_pages_in_a_conversation_and_z_folds() {
        let keymap = crate::config::Keymap::defaults();
        assert_eq!(
            keymap.command_for(Context::Conversation, "space"),
            Some(CommandId::ScrollReaderDown.into()),
            "`space` must turn the page in a conversation -- the one-document \
             pane has nothing to fold, and this is the key a reading surface \
             is expected to page with"
        );
        assert_eq!(
            keymap.command_for(Context::Conversation, "z"),
            Some(CommandId::ToggleFold.into()),
            "folding must keep a key: the stacked pane still folds, and \
             taking `space` away without giving it somewhere else would lose \
             a working gesture"
        );
        // `Page_Down` was the asymmetry that started #1402: `ScrollReaderUp`
        // served the conversation and `ScrollReaderDown` did not.
        for key in ["Page_Down", "Page_Up"] {
            assert!(
                keymap.command_for(Context::Conversation, key).is_some(),
                "`{key}` must resolve in a conversation; the two directions \
                 serving different surfaces is what this issue found"
            );
        }
    }

    // -- binding_conflict (#881) --------------------------------------------

    #[test]
    fn rebinding_over_another_commands_binding_in_a_shared_context_is_a_conflict() {
        // Both are List/Thread/Reader/Search commands, so "k" (PrevMessage's
        // own default) is a real collision if NextMessage claims it too.
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
        // Italic is Composer-only; NextMessage never reaches there, so reusing
        // Italic's own binding is not shadowing anything.
        //
        // This used to propose `mod+b` and pass for the wrong reason: `mod+b`
        // was claimed in a context NextMessage shares, a real conflict the
        // check could not see while it asked a table that command was not in
        // (#1227). Reusing a key across disjoint contexts is deliberate here
        // (`j` is three commands, `d` is three more), so the case is worth
        // keeping; it just needs a binding only one command claims.
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
        // #1227: this check used to ask `postio-config`'s `DEFAULT_BINDINGS`,
        // which lists 23 commands out of 79. `Flag` is one of the 56 it never
        // knew about, so proposing its key read as free -- and the settings
        // pane let a rebind silently take `s` (its key then; `*` since the
        // one keymap) away from a command that was using it, with no
        // "Already used by" to stop it.
        let bindings = postio_config::KeyBindings::default();
        assert_eq!(
            binding_conflict(
                CommandId::NextMessage,
                "*",
                &bindings,
                Platform::Freedesktop
            )
            .map(|spec| spec.id),
            Some(CommandId::Flag),
            "`*` is Flag's default and Flag shares a context with NextMessage"
        );
    }

    #[test]
    fn the_check_sees_overrides_not_just_defaults() {
        // PrevMessage's default is "k", but a rebind captured by an earlier
        // session moved it to "p" -- the conflict check has to see the file's
        // own state, not the built-in table.
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
        // A default nobody can press is worse than no default: it silently
        // costs the command its key. `postio-config` only validates the user's
        // overrides, so the built-ins need their own check.
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
            lookup_binding(Context::List, "a").map(|spec| spec.id),
            Some(CommandId::Archive)
        );
        assert_eq!(
            lookup_binding(Context::List, "Right").map(|spec| spec.id),
            Some(CommandId::OpenMessage),
            "alternate bindings resolve too"
        );
        assert_eq!(lookup_binding(Context::Composer, "a"), None);
        assert_eq!(lookup_binding(Context::List, "ctrl+alt+q"), None);
    }

    #[test]
    fn tab_belongs_to_the_pane_that_owns_it() {
        // #494: the panes that own Tab for their own purpose keep it. A
        // refine chip, a recipient-completion popover and the finder are all
        // correctly consuming Tab, and a top-level binding there would take
        // it away. The pane cycle that once claimed bare Tab went with the
        // three-pane app (specs/009-focus-macos R5).
        assert_eq!(lookup_binding(Context::Composer, "tab"), None);
        assert_eq!(lookup_binding(Context::Search, "tab"), None);
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
}
