//! The command registry: one table, every surface.
//!
//! spec.md §8 asks that every command have a keyboard shortcut, a
//! command-palette entry and an accessible UI action. Three hand-maintained
//! lists would drift apart within a release; one enumerable table cannot. The
//! keymap, the `Ctrl+K` palette, the `?` cheat sheet, the right-click context
//! menu and the key hints on the focused row are all *derived* from
//! [`all()`] and [`for_context()`].
//!
//! # Where the bindings come from
//!
//! The design canvas, not spec.md §8 — the canvas is newer and CLAUDE.md says
//! it wins: `e` reply, `a` archive, `A` archive thread, `u` undo, `t` thread.
//! The ids and their defaults are the same vocabulary `postio-config`'s
//! `DEFAULT_BINDINGS` fixed, so `[keys]` overrides land on the right command.
//! Everything here is a *default*; the user's `[keys]` wins at resolve time.
//!
//! # Destructive commands
//!
//! spec.md §1 requires that destructive operations be confirmed or undoable.
//! [`CommandSpec::destructive`] and [`CommandSpec::recovery`] make that
//! machine-checkable rather than a review habit: a destructive command with no
//! [`Recovery`] fails the test suite.

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
    /// (spec.md §16: *Archived 12 messages — Undo*).
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
    },
    CommandSpec {
        id: CommandId::PrevMessage,
        title: "Previous message",
        default_binding: "k",
        alternate_bindings: &["Up"],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::LastMessage,
        title: "Last message",
        default_binding: "G",
        alternate_bindings: &[],
        contexts: ctx(LIST_SURFACES),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::PrevView,
        title: "Previous view",
        default_binding: "h",
        alternate_bindings: &["Left"],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::Thread,
        title: "Show thread",
        default_binding: "t",
        alternate_bindings: &[],
        contexts: ctx(&[Context::List, Context::Reader]),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::ReplyAll,
        title: "Reply to all",
        default_binding: "E",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
    },
    CommandSpec {
        id: CommandId::Forward,
        title: "Forward",
        default_binding: "f",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
    },
    CommandSpec {
        id: CommandId::Archive,
        title: "Archive",
        default_binding: "a",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        // Sweeping a screenful out of the inbox is exactly the case spec.md §16
        // wants a toast for.
        destructive: true,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::ArchiveThread,
        title: "Archive thread",
        default_binding: "A",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::Delete,
        title: "Delete",
        default_binding: "d",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: true,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::Move,
        title: "Move to…",
        default_binding: "m",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::Flag,
        title: "Flag",
        default_binding: "s",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::MarkUnread,
        title: "Mark unread",
        // spec.md §8 wanted `u`, but the canvas gives `u` to undo and the
        // canvas wins; shifted, like the other second-choice actions.
        default_binding: "U",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
    },
    CommandSpec {
        id: CommandId::AddLabel,
        title: "Add label…",
        default_binding: "L",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::Undo,
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
    },
    CommandSpec {
        id: CommandId::SaveDraft,
        title: "Save draft",
        default_binding: "ctrl+s",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::AttachFile,
        title: "Attach file…",
        default_binding: "ctrl+shift+a",
        alternate_bindings: &[],
        contexts: Context::Composer.as_set(),
        destructive: false,
        recovery: Recovery::None,
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
    },
    CommandSpec {
        id: CommandId::EditConfig,
        title: "Edit configuration",
        default_binding: "ctrl+e",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
    },
    CommandSpec {
        id: CommandId::ToggleSidebar,
        title: "Toggle sidebar",
        default_binding: "ctrl+b",
        alternate_bindings: &[],
        contexts: ctx(MESSAGE_SURFACES),
        destructive: false,
        recovery: Recovery::None,
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
