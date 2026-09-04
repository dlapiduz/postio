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
const MESSAGE_SURFACES: &[Context] = &[Context::List, Context::Conversation, Context::Reader];
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
/// The three panes bare Tab cycles between, and only those.
///
/// Deliberately not `LIST_SURFACES`: `Search` is in that one, and the search
/// field owns Tab for its refine chips. A cycle that resolved there would
/// take Tab away from a pane that is using it (#494).
const PANE_SURFACES: &[Context] = &[
    Context::Sidebar,
    Context::List,
    Context::Conversation,
    Context::Reader,
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
        contexts: ctx(&[Context::List, Context::Conversation, Context::Search]),
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
        // `LIST_SURFACES` minus the conversation: `J` walks the open
        // conversation's messages there (#1007), and there is no row
        // selection to extend while the keyboard is inside the pane.
        // `shift+Down` still reaches this everywhere it ever did.
        contexts: ctx(SELECTION_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::SelectAll,
        title: "Select all",
        default_binding: "mod+a",
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
        id: CommandId::ToggleResultOrder,
        // The same title and key as the thread's own toggle, deliberately:
        // "the order of what I am looking at" is one idea, and `o` means it
        // in both places (#499).
        title: "Toggle result order",
        default_binding: "o",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Search]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    // -- Message actions -------------------------------------------------
    CommandSpec {
        id: CommandId::NextInConversation,
        title: "Next message in conversation",
        // Shifted `j`, because it is the same verb one level in: `j` walks
        // the list of conversations, `J` walks the messages of the one that
        // is open. The pair `a`/`A` already means "this, and this whole
        // thread" -- the shift is the level, not a different action.
        default_binding: "J",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::PrevInConversation,
        title: "Previous message in conversation",
        default_binding: "K",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleFold,
        title: "Fold or unfold this message",
        default_binding: "space",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        // How much of a conversation is open is view state, not durable
        // data -- nothing here for undo to reach.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ViewOriginal,
        title: "View original",
        // `mod+o`, not a bare letter: it is a rare gesture on a surface
        // where every bare letter is already a verb people use constantly,
        // and reader view is the default rather than something to escape.
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::ExpandAll,
        title: "Expand all",
        // `o` was the drill-in column's order toggle until #1003 retired it,
        // which is what makes this letter available. Shifted, because it acts
        // on the whole conversation -- the same relationship `a`/`A` already
        // has between a message and its thread.
        default_binding: "O",
        alternate_bindings: &[],
        // Only where there is a conversation to expand. Offering it on the
        // list would be a key that does nothing most of the time.
        contexts: ctx(&[Context::Conversation]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
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
        id: CommandId::Snooze,
        title: "Snooze",
        // `b`, matching the mnemonic every other snooze-shaped mail client
        // already trained a person on.
        default_binding: "b",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
    },
    CommandSpec {
        id: CommandId::Unsnooze,
        title: "Unsnooze",
        default_binding: "B",
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
        default_binding: "mod+s",
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
        default_binding: "mod+Return",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        // Not destructive — but it is externally visible and irreversible once
        // the queue drains, so it earns an undo-send window rather than a modal.
        destructive: false,
        recovery: Recovery::Undo,
        requires: None,
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
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        // Opening the picker commits nothing; `Recovery::Undo` belongs to
        // whichever time the user picks, exactly as it does for `Send`.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "mod+s",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        requires: None,
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
        alternate_bindings: &[],
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "mod+shift+a",
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
        default_binding: "mod+shift+o",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::Italic,
        title: "Italic",
        default_binding: "mod+i",
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
        default_binding: "mod+shift+8",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::NumberedList,
        title: "Numbered list",
        default_binding: "mod+shift+7",
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
        default_binding: "mod+shift+k",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::QuoteBlock,
        title: "Quote block",
        default_binding: "mod+shift+9",
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
        // Plus the account list. #464 built account removal as a soft delete
        // with a toast wired straight to AccountRepository::restore rather
        // than through the global stack, and said so because Remove was not a
        // command then. Registering it with Recovery::Undo makes that a
        // declaration, and a declaration nothing backs from the keyboard is
        // what ADR 0005 keeps refusing to ship -- so `u` reaches the toast
        // while it is up. Context-local state, context-local binding; the
        // global stack is untouched (ADR 0005 Q6c).
        contexts: ctx(MESSAGE_SURFACES).with(Context::Accounts),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        default_binding: "mod+comma",
        alternate_bindings: &[],
        // Universal, like the palette it is an alternative to reaching.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: None,
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
        alternate_bindings: &[],
        // The same reach `Settings` has, for the reason ADR 0012 Q1 gives:
        // adding an account is a setting, and the folder list is where the
        // account will eventually appear.
        contexts: ContextSet::ANY,
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "mod+e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ToggleSidebar,
        title: "Toggle sidebar",
        default_binding: "mod+b",
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
        id: CommandId::CyclePane,
        title: "Next pane",
        // The top-level meaning of bare Tab, which had none: it was not a
        // command at all, so what it did was whatever GTK's native focus
        // chain produced -- "sometimes it changes panes, sometimes it
        // changes items within a pane" (#494).
        //
        // Rebindable like everything else here. The panes that own Tab for
        // their own purpose keep first claim on it: they are not in
        // `PANE_SURFACES`, so this never resolves there.
        default_binding: "tab",
        alternate_bindings: &[],
        contexts: ctx(PANE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::CyclePaneBack,
        title: "Previous pane",
        default_binding: "shift+tab",
        alternate_bindings: &[],
        contexts: ctx(PANE_SURFACES),
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
        id: CommandId::RenameSavedSearch,
        title: "Rename saved search",
        // Free in `Context::Sidebar`: none of the message surfaces' bindings
        // reach here, since `Sidebar` is not one of `MESSAGE_SURFACES`.
        default_binding: "r",
        alternate_bindings: &[],
        // Only meaningful with a saved search focused, not a folder — the
        // guard is defensive the same way `ToggleThreadUnread`'s is, since
        // the registry already keeps this to the one context both kinds of
        // row share (#455).
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::MoveSavedSearchUp,
        title: "Move saved search up",
        // The same physical key `PrevFolder`'s alternate binding sits on,
        // with Shift held to move the row instead of the cursor — the usual
        // "hold Shift to reorder" idiom rather than a second unrelated key.
        default_binding: "shift+Up",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        // A reorder destroys nothing; moving it back is the same action
        // once more, same as the mouse menu's version of this verb.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::MoveSavedSearchDown,
        title: "Move saved search down",
        default_binding: "shift+Down",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Sidebar]),
        destructive: false,
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::DeleteSavedSearch,
        title: "Delete saved search",
        default_binding: "d",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Sidebar]),
        // Deleting a saved search is a config-file edit with no undo stack
        // to reach (see `postio-gtk::config::request_delete`'s doc comment),
        // so like `DiscardDraft` this asks first rather than offering undo.
        destructive: true,
        recovery: Recovery::Confirm,
        requires: None,
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
        requires: None,
    },
    CommandSpec {
        id: CommandId::RemoveAccount,
        title: "Remove account",
        default_binding: "d",
        alternate_bindings: &[],
        contexts: ctx(&[Context::Accounts]),
        destructive: true,
        // Unlike DeleteSavedSearch, which is a config edit with no undo stack
        // to reach: #464 built removal as a soft delete with a toast wired to
        // AccountRepository::restore, and reaped at the next start. So there
        // is something to undo for as long as the toast is up, and declaring
        // it here is what the registry enforces a keyboard path for.
        recovery: Recovery::Undo,
        requires: None,
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
        requires: None,
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
        contexts: ctx(&[Context::List, Context::Reader]),
        destructive: false,
        // What the pane is scrolled to is view state, not durable data —
        // nothing here for undo to reach.
        recovery: Recovery::None,
        requires: None,
    },
    CommandSpec {
        id: CommandId::ScrollReaderUp,
        title: "Scroll reading pane up",
        default_binding: "Page_Up",
        alternate_bindings: &["shift+space"],
        contexts: ctx(MESSAGE_SURFACES),
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
    fn tab_cycles_the_panes_from_every_pane_it_cycles_through() {
        // #494, reported directly: "tab, shift+tab, ctrl+tab are
        // inconsistent, sometimes it changes panes, sometimes it changes
        // items within a pane. I need an easy way to go from the sidebar to
        // the message list to the preview pane."
        //
        // Bare Tab had no entry in the table at all, so its top-level meaning
        // was whatever GTK's native focus chain happened to produce. A
        // binding that resolves from the sidebar but not the reader would
        // cycle you out and strand you, so every pane in the cycle is
        // asserted rather than one of them.
        for context in [
            Context::Sidebar,
            Context::List,
            Context::Conversation,
            Context::Reader,
        ] {
            assert_eq!(
                lookup_binding(context, "tab").map(|spec| spec.id),
                Some(CommandId::CyclePane),
                "Tab does not cycle panes from {context:?}"
            );
            assert_eq!(
                lookup_binding(context, "shift+tab").map(|spec| spec.id),
                Some(CommandId::CyclePaneBack),
                "Shift+Tab does not cycle back from {context:?}"
            );
        }

        // The panes that own Tab for their own purpose keep it. A refine
        // chip, a recipient-completion popover and the finder are all
        // correctly consuming Tab, and #494 says so explicitly: those local
        // overrides are legitimate and must not regress.
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
