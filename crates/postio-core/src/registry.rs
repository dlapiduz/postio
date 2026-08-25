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
    /// The scope predicate: where this command has meaning at all.
    ///
    /// [`Context`] answers "where is the keyboard"; this answers "what is on
    /// screen", which is *state* rather than place. Move is the first command
    /// gated on it (ADR 0005 Q4): a unified view is not a mailbox, so
    /// "move to…" from it names no destination tree. The shape for the next
    /// state-conditional command is the same — add a variant naming the state
    /// it needs, gate the spec, and [`reachable_in`] carries it to the
    /// palette, the cheat sheet and the key hints at once.
    pub scope_gate: ScopeGate,
}

/// What [`Scope`](crate::state::Scope) a command needs to mean anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeGate {
    /// Meaningful whatever is on screen. Nearly everything.
    #[default]
    Anywhere,
    /// Needs one real account's mailbox tree on screen: unavailable in the
    /// unified scope, where there is no such tree to name a destination in.
    AccountOnly,
}

impl ScopeGate {
    /// Whether a command with this gate is available in `scope`.
    pub fn allows(self, scope: crate::state::Scope) -> bool {
        match self {
            ScopeGate::Anywhere => true,
            ScopeGate::AccountOnly => scope.account().is_some(),
        }
    }
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::PrevMessage,
        title: "Previous message",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::LastMessage,
        title: "Last message",
        default_binding: "G",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionDown,
        title: "Extend selection down",
        default_binding: "J",
        alternate_bindings: &["shift+Down"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ExtendSelectionUp,
        title: "Extend selection up",
        default_binding: "K",
        alternate_bindings: &["shift+Up"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::SelectAll,
        title: "Select all",
        default_binding: "ctrl+a",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::PrevView,
        title: "Previous view",
        default_binding: "h",
        alternate_bindings: &["Left"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::Thread,
        title: "Show thread",
        default_binding: "t",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List, Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ToggleThreadOrder,
        title: "Toggle order",
        default_binding: "o",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Thread]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    // -- Message actions -------------------------------------------------
    CommandSpec {
        id: CommandId::Reply,
        title: "Reply",
        default_binding: "e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ReplyAll,
        title: "Reply to all",
        default_binding: "E",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::Forward,
        title: "Forward",
        default_binding: "f",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ArchiveThread,
        title: "Archive thread",
        default_binding: "A",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::Delete,
        title: "Delete",
        default_binding: "d",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::Move,
        title: "Move to…",
        default_binding: "m",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        // ADR 0005 Q4: a unified view is not a mailbox, so "move to…" from
        // it names no destination tree. Not a no-op -- unavailable.
        scope_gate: ScopeGate::AccountOnly,
    },
    CommandSpec {
        id: CommandId::Flag,
        title: "Flag",
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::AddLabel,
        title: "Add label…",
        default_binding: "L",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "ctrl+s",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "ctrl+shift+a",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "ctrl+e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::ToggleSidebar,
        title: "Toggle sidebar",
        default_binding: "ctrl+b",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::PrevFolder,
        title: "Previous folder",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
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
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::PrevPart,
        title: "Previous part",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::OpenPart,
        title: "Open part",
        default_binding: "Return",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::SavePart,
        title: "Save part",
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::SaveAllParts,
        title: "Save all parts",
        default_binding: "S",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::OpenPartExternally,
        title: "Open part externally",
        default_binding: "x",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
    },
    CommandSpec {
        id: CommandId::RenderPartOnce,
        title: "Render part once",
        default_binding: "H",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Parts]),
        destructive: false,
        recovery: Recovery::None,
        scope_gate: ScopeGate::Anywhere,
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
    /// Where this command has meaning at all. See [`CommandSpec::scope_gate`].
    pub scope_gate: ScopeGate,
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
            scope_gate: spec.scope_gate,
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
            // Extensions are not scope-aware yet: an extension command shows
            // everywhere its contexts say. The day one needs gating, ExtSpec
            // grows the field and this stops being a constant.
            scope_gate: ScopeGate::Anywhere,
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

/// [`reachable`], further filtered by what is on screen.
///
/// `Context` answers "where is the keyboard"; `scope` answers "what is the
/// window showing", which is state rather than place — Move is gated on it
/// because a unified view is not a mailbox and "move to…" from it names no
/// destination tree (ADR 0005 Q4). Every surface that *offers* commands — the
/// palette, the cheat sheet, the key hints — goes through this, so a gated
/// command is absent everywhere at once rather than absent from whichever
/// surfaces remembered to check.
///
/// [`reachable`] keeps meaning the context-only filter, so callers that have
/// no scope — documentation generators, tests over the shipped vocabulary —
/// keep compiling and keep meaning what they meant.
pub fn reachable_in(
    context: Context,
    scope: crate::state::Scope,
) -> impl Iterator<Item = ActionSpec> {
    reachable(context).filter(move |spec| spec.scope_gate.allows(scope))
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
