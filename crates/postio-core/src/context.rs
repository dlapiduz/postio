//! Which surface owns the next keystroke: a coarse mode, not a widget.
//!
//! Availability is not key routing: whether a bare letter reaches a command or is swallowed by a
//! focused entry is the frontend resolver's call.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Context {
    List,
    /// Was `Thread` before the conversation moved into the reading pane (#1003); its keys walk
    /// messages inside the stack.
    Conversation,
    Reader,
    /// One context either way: the composer is the same widget reparented
    /// (`CommandId::DetachComposer`).
    Composer,
    Search,
    Palette,
    /// A real context, not a focus flag, so folder commands reach the palette and cheat sheet.
    Sidebar,
    /// A real context so `j` in the panel is not taken by the window's resolver first
    /// (`postio-14b`).
    Parts,
    /// Scoped to the account list, not the whole settings panel, so bare-letter bindings are not
    /// live while someone types TOML (ADR 0005 Q6c, #471).
    Accounts,
    /// Scoped to the rebind list, for the reason `Accounts` gives (#881).
    Keys,
    /// Its own context, not layered over the list: a letter falling through would act on the mail
    /// underneath.
    Picker,
    Digest,
    Filtered,
    /// Its own context, for the picker's reason: its keys mean nothing to the list underneath.
    Capture,
    /// Not `Search`'s (the field is not focused, so `a` archives a result) nor `List`'s (`mod+1`
    /// picks a tab).
    Results,
    /// Scoped to the list and card, so bare letters do not fire while typing in the filter or
    /// editor.
    /// One context for every row kind; a command acts on the focused row's kind.
    Contacts,
    ContactPopover,
    /// Its own context: `u` means "first unread" here but `prev_view` in the reader beneath.
    Outline,
    /// Not the picker's context, which already gives each digit to `picker_choose_n`.
    ForwardChooser,
    Source,
}

impl Context {
    /// Every context, in a stable order. The cheat sheet renders in this order.
    pub const ALL: &'static [Context] = &[
        Context::List,
        Context::Conversation,
        Context::Reader,
        Context::Composer,
        Context::Search,
        Context::Palette,
        Context::Sidebar,
        Context::Parts,
        // At the end, so the `?` sheet grows a section rather than reordering the ones people
        // learned (ADR 0005 Q6c).
        Context::Accounts,
        Context::Keys,
        Context::Picker,
        Context::Digest,
        Context::Filtered,
        Context::Capture,
        Context::Results,
        Context::Contacts,
        Context::ContactPopover,
        Context::Outline,
        Context::ForwardChooser,
        Context::Source,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Context::List => "list",
            Context::Conversation => "conversation",
            Context::Reader => "reader",
            Context::Composer => "composer",
            Context::Search => "search",
            Context::Palette => "palette",
            Context::Sidebar => "sidebar",
            Context::Parts => "parts",
            Context::Accounts => "accounts",
            Context::Keys => "keys",
            Context::Picker => "picker",
            Context::Digest => "digest",
            Context::Filtered => "filtered",
            Context::Capture => "capture",
            Context::Results => "results",
            Context::Contacts => "contacts",
            Context::ContactPopover => "contact_popover",
            Context::Outline => "outline",
            Context::ForwardChooser => "forward_chooser",
            Context::Source => "source",
        }
    }

    pub const fn as_set(self) -> ContextSet {
        ContextSet::of(self)
    }

    const fn bit(self) -> u32 {
        // `u32` so adding contexts is not a silent overflow; `every_context_fits_the_set` says when
        // to widen.
        1 << (self as u32)
    }
}

impl fmt::Display for Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownContext(String);

impl UnknownContext {
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

/// A set rather than a closure: the palette and cheat sheet need to ask what is available here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextSet(u32);

impl ContextSet {
    /// The empty set. No command may keep this — see the registry tests.
    pub const EMPTY: ContextSet = ContextSet(0);

    /// Derived from `Context::ALL`, so a new context cannot be left out of "everywhere".
    pub const ANY: ContextSet = {
        let mut bits = 0u32;
        let mut index = 0;
        while index < Context::ALL.len() {
            bits |= Context::ALL[index].bit();
            index += 1;
        }
        ContextSet(bits)
    };

    pub const fn of(context: Context) -> ContextSet {
        ContextSet(context.bit())
    }

    pub const fn with(self, context: Context) -> ContextSet {
        ContextSet(self.0 | context.bit())
    }

    /// "Everywhere but there", for the one command whose key another surface takes (`add_account`'s
    /// `mod+shift+n` is New contact in Contacts).
    pub const fn without(self, context: Context) -> ContextSet {
        ContextSet(self.0 & !context.bit())
    }

    pub const fn from_slice(contexts: &[Context]) -> ContextSet {
        let mut bits = 0u32;
        let mut index = 0;
        while index < contexts.len() {
            bits |= contexts[index].bit();
            index += 1;
        }
        ContextSet(bits)
    }

    pub const fn contains(self, context: Context) -> bool {
        self.0 & context.bit() != 0
    }

    /// Whether the two sets share a context; two commands may share a binding when this is false.
    pub const fn intersects(self, other: ContextSet) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

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
        let mut bits = 0u32;
        for context in Context::ALL {
            assert_eq!(bits & context.bit(), 0, "{context} reuses a bit");
            bits |= context.bit();
        }
        assert_eq!(ContextSet(bits), ContextSet::ANY);
    }
}
