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

use serde::{Deserialize, Serialize};

/// How the user gets back from a command that changed something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Nothing to recover from; the command changed no durable state.
    None,
    /// Reversible from the undo stack, and worth an "— Undo" toast
    /// (docs/PRODUCT.md §16: *Archived 12 messages — Undo*).
    Undo,
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
/// through [`available`].
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
}

/// The state [`Requirement`]s are evaluated against.
///
/// A struct rather than bare arguments so a new requirement adds a field
/// instead of changing every call site's signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Availability {
    /// What the mail on screen belongs to.
    pub scope: Scope,
}

impl Requirement {
    /// Whether `state` satisfies this requirement.
    pub fn met_by(self, state: Availability) -> bool {
        match self {
            Requirement::SingleAccount => state.scope.is_single_account(),
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
    /// having the right surface focused. `None` for almost everything.
    pub requires: Option<Requirement>,
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

/// Reading the message list, a thread and a single message: the surfaces where
/// a message action means something.
const MESSAGE_SURFACES: &[Context] = &[Context::List, Context::Thread, Context::Reader];
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
    Context::Thread,
    Context::Reader,
    Context::Composer,
];
/// The surfaces that scroll through a list of messages.
const LIST_SURFACES: &[Context] = &[
    Context::List,
    Context::Thread,
    Context::Reader,
    Context::Search,
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
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::PrevMessage,
        title: "Previous message",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::LastMessage,
        title: "Last message",
        default_binding: "G",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::OpenMessage,
        title: "Open message",
        // `Return` is what config.toml documents; `l` is the canvas's
        // vim-style open, and both reach the same command.
        default_binding: "Return",
        alternate_bindings: &["l", "Right"],
        contexts: ctx(&[Context::List, Context::Thread, Context::Search]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleSelection,
        title: "Toggle selection",
        // Gmail's, and everyone else's since. Muscle memory is worth more
        // here than a mnemonic nobody has.
        default_binding: "x",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        // Changing what an action *would* hit changes no durable state, so
        // there is nothing to undo and nothing to confirm.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionDown,
        title: "Extend selection down",
        default_binding: "J",
        alternate_bindings: &["shift+Down"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionUp,
        title: "Extend selection up",
        default_binding: "K",
        alternate_bindings: &["shift+Up"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SelectAll,
        title: "Select all",
        default_binding: "ctrl+a",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::PrevView,
        title: "Previous view",
        default_binding: "h",
        alternate_bindings: &["Left"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::Thread,
        title: "Show thread",
        default_binding: "t",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List, Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleThreadUnread,
        title: "Unread only",
        // `u` is Undo and `U` is Mark unread in every message surface
        // including this one -- both taken before this command exists, so
        // neither is available to it.
        default_binding: "n",
        alternate_bindings: &[],
        // Only meaningful with a thread column on screen: there is nothing
        // else in the application this filter could apply to.
        contexts: ctx(&[Context::Thread]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleThreadOrder,
        title: "Toggle order",
        default_binding: "o",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Thread]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    // -- Message actions -------------------------------------------------
    CommandSpec {
        id: CommandId::Reply,
        title: "Reply",
        default_binding: "e",
        alternate_bindings: &[],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ReplyAll,
        title: "Reply to all",
        default_binding: "E",
        alternate_bindings: &[],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Forward,
        title: "Forward",
        default_binding: "f",
        alternate_bindings: &[],
        contexts: ctx(REPLY_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Archive,
        title: "Archive",
        default_binding: "a",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        // Sweeping a screenful out of the inbox is exactly the case docs/PRODUCT.md §16
        // wants a toast for.
        destructive: true,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ArchiveThread,
        title: "Archive thread",
        default_binding: "A",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Delete,
        title: "Delete",
        default_binding: "d",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Move,
        title: "Move to…",
        default_binding: "m",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        // A destination is one mailbox in one account, and a unified view
        // spans every enabled account — so there is nowhere for this to mean.
        // Unavailable rather than a no-op: offering it would promise a folder
        // the user was never given the chance to pick (#182, ADR 0005 Q4).
        requires: Some(Requirement::SingleAccount),
    },
    CommandSpec {
        id: CommandId::Flag,
        title: "Flag",
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::MarkUnread,
        title: "Mark unread",
        // `u` belongs to undo (docs/PRODUCT.md §16), so mark-unread is
        // shifted, like the other second-choice actions. The original brief
        // proposed `u` here and lost that argument to the canvas.
        default_binding: "U",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::AddLabel,
        title: "Add label…",
        default_binding: "L",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
    },
    // -- Search ----------------------------------------------------------
    CommandSpec {
        id: CommandId::Search,
        title: "Search",
        default_binding: "/",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SaveSearch,
        title: "Save search as folder",
        // Shares a binding with `save_draft`, which is fine: the two
        // contexts do not overlap, and `ctrl+s` is the "save this" muscle
        // memory in both. See `postio-search`/canvas 2b's "save as folder".
        default_binding: "ctrl+s",
        alternate_bindings: &[],
        // Only reachable with the search box open -- saving needs a query
        // to save, and `Context::Search` is where one exists.
        contexts: Context::Search.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    // -- Compose ---------------------------------------------------------
    CommandSpec {
        id: CommandId::Compose,
        title: "Compose",
        default_binding: "c",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Send,
        title: "Send",
        default_binding: "ctrl+Return",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        // Not destructive — but it is externally visible and irreversible once
        // the queue drains, so it earns an undo-send window rather than a modal.
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "ctrl+s",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::DiscardDraft,
        title: "Discard draft",
        default_binding: "ctrl+d",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        // Typed prose has no other copy anywhere, so this one asks first.
        destructive: true,
        recovery: Recovery::Confirm,
        requires: None,
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "ctrl+shift+a",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::DetachComposer,
        // A toggle, like `toggle_sidebar`, and named for the direction the
        // user has to ask for: in-place is the default and detaching is the
        // opt-in, so "Detach composer" is what someone looking for it in the
        // palette will type. Offered only while composing, which is also the
        // only time the other direction can be reached.
        title: "Detach composer",
        // Not next to `ctrl+d`. Discard is the one composer verb that cannot
        // be undone, and a fat-fingered neighbour of it is a draft gone.
        default_binding: "ctrl+shift+o",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Bold,
        title: "Bold",
        default_binding: "ctrl+b",
        alternate_bindings: &[],
        // ctrl+b is the sidebar everywhere mail is read; the composer is not
        // a message surface, so the convention every editor shares wins here.
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Italic,
        title: "Italic",
        default_binding: "ctrl+i",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::BulletList,
        title: "Bulleted list",
        // The Docs/Gmail convention, and shift dodges nothing here — the
        // digits are free in the composer either way.
        default_binding: "ctrl+shift+8",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::NumberedList,
        title: "Numbered list",
        default_binding: "ctrl+shift+7",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::InsertLink,
        title: "Insert link…",
        // Everywhere else this is ctrl+k, and here ctrl+k is the palette —
        // which is universal or it is not a palette. Shift is the tax.
        default_binding: "ctrl+shift+k",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::QuoteBlock,
        title: "Quote block",
        default_binding: "ctrl+shift+9",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    // -- View and application --------------------------------------------
    CommandSpec {
        id: CommandId::Undo,
        title: "Undo",
        default_binding: "u",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::CommandPalette,
        title: "Command palette",
        default_binding: "ctrl+k",
        alternate_bindings: &[],
        // Universal, or it is not a command palette.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::Settings,
        title: "Settings",
        default_binding: "ctrl+comma",
        alternate_bindings: &[],
        // Universal, like the palette it is an alternative to reaching.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "ctrl+e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleSidebar,
        title: "Toggle sidebar",
        default_binding: "ctrl+b",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::FocusSidebar,
        title: "Focus the folder list",
        // `g` is already the "go to" prefix — `g g` is the first message — so
        // "go to folders" reads as one idiom rather than a second one.
        default_binding: "g f",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::NextFolder,
        title: "Next folder",
        // The same keys the message list moves by, in a context where they
        // mean a different thing. One idiom for "move down", two verbs —
        // rather than reusing `next_message` for something that is not a
        // message, which is how a registry stops meaning anything.
        default_binding: "j",
        alternate_bindings: &["Down"],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::PrevFolder,
        title: "Previous folder",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleFolder,
        title: "Expand or collapse folder",
        // Distinct from `Return`/`l`/`Right`, which the message surfaces use
        // to open something — the folder list already opens on selection
        // (`postio-cfd.2`), so this is a second verb the same key would
        // otherwise be asked to mean two things at once.
        default_binding: "space",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        // Which folders are open is view state, not durable data — nothing
        // here for undo to reach.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Refresh,
        title: "Refresh",
        default_binding: "F5",
        // The canvas' own retry key, for the empty and error states in
        // `postio-gtk::list_state`: "retry now" and "check for new mail
        // now" are the same command from the user's chair.
        alternate_bindings: &["R"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    // -- Parts panel -------------------------------------------------------
    CommandSpec {
        id: CommandId::OpenParts,
        title: "Show message parts",
        default_binding: "p",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::NextPart,
        title: "Next part",
        // The same keys the message list walks by, in a context where they
        // mean a different verb — see `Context::Sidebar`'s `NextFolder` for
        // why that is one idiom rather than reusing `next_message`.
        default_binding: "j",
        alternate_bindings: &["Down"],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::PrevPart,
        title: "Previous part",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::OpenPart,
        title: "Open part",
        default_binding: "Return",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SavePart,
        title: "Save part",
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SaveAllParts,
        title: "Save all parts",
        default_binding: "S",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::OpenPartExternally,
        title: "Open part externally",
        default_binding: "x",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::RenderPartOnce,
        title: "Render part once",
        default_binding: "H",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
    for_context(context).find(|spec| spec.bindings().any(|candidate| candidate == binding))
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
    pub requires: Option<Requirement>,
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
            // An extension has no way to name one yet; when it does, it
            // arrives here rather than at a surface.
            requires: None,
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

/// Whether `spec` is reachable in `context` *and* satisfied by `state`.
///
/// The one place a surface asks "can the user do this right now". Splitting
/// it from [`ActionSpec::available_in`] keeps the context question — which is
/// most of them — free of state nobody else needs.
pub fn available(spec: &ActionSpec, context: Context, state: Availability) -> bool {
    spec.available_in(context) && spec.requires.is_none_or(|need| need.met_by(state))
}

/// Every command reachable in `context` for a view scoped to `scope`.
///
/// What the palette, the cheat sheet and the key hints iterate. [`reachable`]
/// stays the scope-blind form, because `docs/keybindings.md` documents the
/// whole vocabulary rather than one session's state — somebody looking up `m`
/// has to find it whatever is on screen.
pub fn reachable_in(context: Context, scope: Scope) -> impl Iterator<Item = ActionSpec> {
    let state = Availability { scope };
    reachable(context).filter(move |spec| spec.requires.is_none_or(|need| need.met_by(state)))
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

    #[test]
    fn the_table_is_ordered_like_command_id_all() {
        // `get` indexes the table by discriminant; this is what makes that safe.
        assert_eq!(SPECS.len(), CommandId::ALL.len());
        for (spec, id) in SPECS.iter().zip(CommandId::ALL) {
            assert_eq!(spec.id, *id, "registry row out of order at `{id}`");
        }
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
            lookup_binding(Context::List, "l").map(|spec| spec.id),
            Some(CommandId::OpenMessage),
            "alternate bindings resolve too"
        );
        assert_eq!(lookup_binding(Context::Composer, "a"), None);
        assert_eq!(lookup_binding(Context::List, "ctrl+alt+q"), None);
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
