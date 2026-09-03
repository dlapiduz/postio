//! Where the keyboard is pointing: which surface owns the next keystroke.
//!
//! A context is *not* a widget — `postio-core` knows nothing about widgets. It
//! is the coarse mode the user is in, and it is what makes one key mean two
//! things without ambiguity: `Escape` leaves the search field in
//! [`Context::Search`] and closes the composer in [`Context::Composer`].
//!
//! Every [`CommandSpec`](crate::CommandSpec) carries the set of contexts it is
//! meaningful in, which is what the palette and the `?` cheat sheet filter on.
//!
//! Availability is not key routing. A command being available in
//! [`Context::Search`] says the user can reach it there; whether a bare letter
//! key reaches it, or is swallowed as typed text by a focused entry, is the
//! keymap resolver's decision in `postio-gtk`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The surface that owns the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Context {
    /// The message list: rows, selection, bulk actions.
    List,
    /// A thread drilled into from the list.
    Thread,
    /// The reading pane showing one message.
    Reader,
    /// Compose: the reading pane by default, or a window of its own on ask.
    ///
    /// One context either way. The composer is the same widget reparented, so
    /// a binding that works in the pane works in the detached window without
    /// a second context to keep in step -- see `CommandId::DetachComposer`.
    Composer,
    /// The search field and its results.
    Search,
    /// The `Ctrl+K` command palette overlay.
    Palette,
    /// The folder list, once the keyboard is in it.
    ///
    /// A real context rather than a focus flag, because that is what makes
    /// the folder commands reachable from the palette and printable in the
    /// cheat sheet without either of them learning about the sidebar. Before
    /// it existed there was no way to change mailbox without the mouse.
    Sidebar,
    /// The parts panel: a message's MIME tree, walkable from the keyboard.
    ///
    /// Also a real context rather than a focus flag, and for the same
    /// reason `Sidebar` had to become one: without it, `j` in the panel
    /// reached the window's own resolver first and moved the message
    /// selection instead of walking the tree — see `postio-14b`.
    Parts,
    /// The account list in settings, once the keyboard is in it.
    ///
    /// Scoped to the `accounts_list` widget and named for it, the way
    /// `Sidebar` and `Parts` are — deliberately *not* a `Context::Settings`
    /// spanning the whole panel. That panel also holds a `GtkTextView` of the
    /// literal `config.toml`, and a context named for the panel would put
    /// bare-letter bindings live while somebody types TOML: `d` removing an
    /// account instead of inserting a `d`. Scoping to the list closes that by
    /// construction rather than by remembering (ADR 0005 Q6c, #471).
    Accounts,
    /// The keybinding list in settings, once the keyboard is in it.
    ///
    /// Scoped to the rebind list widget rather than the whole panel, for
    /// the exact reason [`Context::Accounts`] already gives: `[keys]`'s own
    /// escape hatch is a `GtkTextView` over raw TOML, and a bare-letter
    /// binding must not fire while someone is typing there (#881).
    Keys,
}

impl Context {
    /// Every context, in a stable order. The cheat sheet renders in this order.
    pub const ALL: &'static [Context] = &[
        Context::List,
        Context::Thread,
        Context::Reader,
        Context::Composer,
        Context::Search,
        Context::Palette,
        Context::Sidebar,
        Context::Parts,
        // At the end, so the `?` sheet grows a section rather than reordering
        // the ones people have learned (ADR 0005 Q6c).
        Context::Accounts,
        Context::Keys,
    ];

    /// The stable serialized name, matching the `Deserialize` spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Context::List => "list",
            Context::Thread => "thread",
            Context::Reader => "reader",
            Context::Composer => "composer",
            Context::Search => "search",
            Context::Palette => "palette",
            Context::Sidebar => "sidebar",
            Context::Parts => "parts",
            Context::Accounts => "accounts",
            Context::Keys => "keys",
        }
    }

    /// This context on its own, as a set.
    pub const fn as_set(self) -> ContextSet {
        ContextSet::of(self)
    }

    const fn bit(self) -> u16 {
        // `u16`, not `u8`: the eight original contexts filled a byte exactly,
        // and `Accounts` made the ninth. That was not a silent overflow --
        // `ContextSet::ANY` is const-evaluated, so it was a compile error and
        // could never have reached a running build (#471). The room here is
        // for the next one; `every_context_fits_the_set` is what says when
        // this needs widening again.
        1 << (self as u16)
    }
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The error from parsing an unknown context name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownContext(String);

impl UnknownContext {
    /// The text that did not name a context.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UnknownContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown context `{}`", self.0)
    }
}

impl std::error::Error for UnknownContext {}

impl FromStr for Context {
    type Err = UnknownContext;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Context::ALL
            .iter()
            .copied()
            .find(|context| context.as_str() == text)
            .ok_or_else(|| UnknownContext(text.to_owned()))
    }
}

/// The set of contexts a command is available in — its context predicate.
///
/// A set rather than a closure on purpose: a predicate you can only *call* can
/// answer "is this command available here?" but not "what is available here?",
/// and the palette and cheat sheet need the second question answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextSet(u16);

impl ContextSet {
    /// The empty set. No command may keep this — see the registry tests.
    pub const EMPTY: ContextSet = ContextSet(0);

    /// Every context, for commands like the palette that are always reachable.
    ///
    /// Derived from [`Context::ALL`] rather than written as a literal. It was
    /// `0b0011_1111`, and adding a seventh context silently left it meaning
    /// "the first six" — a command declared reachable everywhere would have
    /// been unreachable in the new one, which is the kind of bug that shows up
    /// as a key that does nothing in one pane.
    pub const ANY: ContextSet = {
        let mut bits = 0u16;
        let mut index = 0;
        while index < Context::ALL.len() {
            bits |= Context::ALL[index].bit();
            index += 1;
        }
        ContextSet(bits)
    };

    /// A set holding exactly one context.
    pub const fn of(context: Context) -> ContextSet {
        ContextSet(context.bit())
    }

    /// This set plus one more context, usable in a `const` table.
    ///
    /// For the entries that are "one of the named groups, and also somewhere
    /// else" -- `Undo` over the message surfaces plus the account list. The
    /// alternative is a second slice constant named for a single use, which
    /// reads worse at the call site than the sentence it is standing in for.
    pub const fn with(self, context: Context) -> ContextSet {
        ContextSet(self.0 | context.bit())
    }

    /// A set built from a slice, usable in a `const` table.
    pub const fn from_slice(contexts: &[Context]) -> ContextSet {
        let mut bits = 0u16;
        let mut index = 0;
        while index < contexts.len() {
            bits |= contexts[index].bit();
            index += 1;
        }
        ContextSet(bits)
    }

    /// Whether `context` is in the set — the predicate itself.
    pub const fn contains(self, context: Context) -> bool {
        self.0 & context.bit() != 0
    }

    /// Whether the set holds no context at all.
    /// Whether the two sets share a context.
    ///
    /// Two commands can carry the same binding as long as this is false: `a`
    /// archives in the list and types a letter in the composer.
    pub const fn intersects(self, other: ContextSet) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether this set names no context at all.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The contexts in the set, in [`Context::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = Context> {
        Context::ALL
            .iter()
            .copied()
            .filter(move |context| self.contains(*context))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_holds_what_it_was_built_from() {
        let set = ContextSet::from_slice(&[Context::List, Context::Reader]);
        assert!(set.contains(Context::List));
        assert!(set.contains(Context::Reader));
        assert!(!set.contains(Context::Composer));
        assert_eq!(set.iter().count(), 2);
    }

    #[test]
    fn any_holds_every_context_and_empty_holds_none() {
        for context in Context::ALL {
            assert!(ContextSet::ANY.contains(*context));
            assert!(!ContextSet::EMPTY.contains(*context));
        }
        assert!(ContextSet::EMPTY.is_empty());
        assert_eq!(ContextSet::ANY.iter().count(), Context::ALL.len());
    }

    #[test]
    fn every_context_fits_the_set() {
        assert!(
            Context::ALL.len() <= ContextSet(0).0.count_zeros() as usize,
            "{} contexts will not fit a {}-bit ContextSet. Widen the integer \
             behind it -- the eight original contexts filled a u8 exactly and \
             Accounts made the ninth, which is the last time this happened.",
            Context::ALL.len(),
            ContextSet(0).0.count_zeros()
        );
    }

    #[test]
    fn every_context_has_a_distinct_bit() {
        let mut bits = 0u16;
        for context in Context::ALL {
            assert_eq!(bits & context.bit(), 0, "{context} reuses a bit");
            bits |= context.bit();
        }
        assert_eq!(ContextSet(bits), ContextSet::ANY);
    }
}
