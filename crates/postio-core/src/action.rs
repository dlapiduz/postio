//! Identifiers wide enough for commands this build has never heard of.
//!
//! [`CommandId`] is a closed enum, and deliberately stays one — see
//! `docs/decisions/0002-extensible-command-vocabulary.md`. It is fieldless, so
//! `registry::get` can be `SPECS[id as usize]`; it is `Copy`, so it is passed
//! by value in a few hundred places; and it is exhaustively matched, so a
//! built-in command cannot be silently unhandled.
//!
//! Extensions need none of that and cannot have it anyway: an MCP tool or an
//! AI action is not known at compile time, so there is nothing to match
//! exhaustively and no index to address. What they *do* need is to be named,
//! bound to a key, listed in the palette and shown in the cheat sheet — all of
//! which happen one layer above `CommandId`.
//!
//! That layer is [`ActionId`]. The registry, the keymap, the palette and the
//! cheat sheet deal in it; dispatch and `Command::default_for` keep dealing in
//! `CommandId`, because a built-in is statically known to have a handler and
//! an extension is not. They are equal where it matters to the *user* and
//! distinguishable where it matters to the *compiler*.
//!
//! # Why ids are interned
//!
//! [`ExtId`] is a `u32` handle into an append-only table of leaked strings, so
//! it is `Copy` and cheap to compare, and an `ActionId` stays as small and as
//! copyable as the `CommandId` it sits beside. Leaking is the right shape
//! here rather than a concession: registrations happen at startup, are bounded
//! by the number of extensions loaded, and last as long as the process. An id
//! that could be freed would be an id that could dangle in a keymap.
//!
//! # Parsing does not depend on registration
//!
//! This is what makes `[keys]` work at all. `config.toml` is read and resolved
//! at startup; extensions register later. If `"mcp:summarise"` only parsed
//! once its command existed, every binding naming an extension would resolve
//! to nothing and silently do nothing.
//!
//! So interning is independent of registration: a well-formed namespaced id
//! always parses, whether or not anything has claimed it. The keymap binds the
//! id at startup and it starts reaching a command the moment one registers.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::command::{CommandId, UnknownCommand};

/// What separates an extension's namespace from its command name.
///
/// Built-in ids are `snake_case` with no punctuation beyond `_` — a test in
/// `command.rs` enforces exactly that — so this character is what tells the
/// two vocabularies apart, and it can never appear in a built-in id.
pub const NAMESPACE_SEPARATOR: char = ':';

/// The interned id of a command that is not built in.
///
/// Namespaced: `"mcp:summarise-thread"`, `"user:file-to-receipts"`. The
/// namespace keeps built-in ids collision-free forever and makes provenance
/// visible in the palette and in a log line — which matters more for a
/// command the user did not type than for one they did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtId(u32);

/// The interner: every id ever seen, and the index of each.
///
/// Append-only, so an `ExtId` is valid for the life of the process and
/// `as_str` needs no lock beyond the read.
struct Interner {
    ids: Vec<&'static str>,
    index: HashMap<&'static str, u32>,
}

fn interner() -> &'static Mutex<Interner> {
    static INTERNER: OnceLock<Mutex<Interner>> = OnceLock::new();
    INTERNER.get_or_init(|| {
        Mutex::new(Interner {
            ids: Vec::new(),
            index: HashMap::new(),
        })
    })
}

impl ExtId {
    /// Intern `id`, or return the handle it already has.
    ///
    /// `None` when `id` is not namespaced, which is the one thing that makes
    /// an extension id well-formed. Note what this does *not* check: whether
    /// anything has registered a command under it. Interning is naming, not
    /// registration — see the module note on why `[keys]` depends on that.
    pub fn intern(id: &str) -> Option<ExtId> {
        if !is_namespaced(id) {
            return None;
        }
        let mut interner = interner().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(&existing) = interner.index.get(id) {
            return Some(ExtId(existing));
        }
        // Leaked deliberately: see the module note. Bounded by the number of
        // distinct ids the process ever names.
        let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
        let handle = interner.ids.len() as u32;
        interner.ids.push(leaked);
        interner.index.insert(leaked, handle);
        Some(ExtId(handle))
    }

    /// The namespaced string this id was interned from.
    pub fn as_str(self) -> &'static str {
        let interner = interner().lock().unwrap_or_else(|error| error.into_inner());
        interner.ids[self.0 as usize]
    }

    /// The part before the separator: who the command came from.
    pub fn namespace(self) -> &'static str {
        let id = self.as_str();
        id.split_once(NAMESPACE_SEPARATOR)
            .map(|(namespace, _)| namespace)
            .unwrap_or(id)
    }
}

/// Whether `id` is a well-formed namespaced extension id.
///
/// Both halves must be non-empty, and there must be exactly one separator: a
/// bare `"mcp:"` names nobody and `"a:b:c"` has no single obvious namespace.
fn is_namespaced(id: &str) -> bool {
    match id.split_once(NAMESPACE_SEPARATOR) {
        Some((namespace, name)) => {
            !namespace.is_empty() && !name.is_empty() && !name.contains(NAMESPACE_SEPARATOR)
        }
        None => false,
    }
}

impl fmt::Display for ExtId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Anything the user can invoke: a built-in command or a registered extension.
///
/// This is the vocabulary the *surfaces* speak — the keymap, the palette, the
/// cheat sheet, the key hints. Dispatch does not: see the module note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionId {
    /// A command compiled into this build.
    Builtin(CommandId),
    /// A command registered at runtime.
    Ext(ExtId),
}

impl ActionId {
    /// The stable string id, as `[keys]` in `config.toml` spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            ActionId::Builtin(id) => id.as_str(),
            ActionId::Ext(id) => id.as_str(),
        }
    }

    /// The built-in this names, if it is one.
    ///
    /// The usual way back to the closed vocabulary: a surface that can only
    /// act on built-ins asks for this and ignores `None`.
    pub fn builtin(self) -> Option<CommandId> {
        match self {
            ActionId::Builtin(id) => Some(id),
            ActionId::Ext(_) => None,
        }
    }

    /// Whether this is an extension command rather than a built-in.
    pub fn is_ext(self) -> bool {
        matches!(self, ActionId::Ext(_))
    }
}

impl From<CommandId> for ActionId {
    fn from(id: CommandId) -> Self {
        ActionId::Builtin(id)
    }
}

impl From<ExtId> for ActionId {
    fn from(id: ExtId) -> Self {
        ActionId::Ext(id)
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ActionId {
    type Err = UnknownCommand;

    /// A namespaced id is an extension; anything else must be a built-in.
    ///
    /// The separator decides, not a lookup, so this answers the same way
    /// before and after an extension registers. An id that is namespaced but
    /// malformed — `"mcp:"` — is not an extension id and is reported as
    /// unknown rather than interned.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.contains(NAMESPACE_SEPARATOR) {
            return ExtId::intern(text)
                .map(ActionId::Ext)
                .ok_or_else(|| UnknownCommand::new(text));
        }
        text.parse::<CommandId>().map(ActionId::Builtin)
    }
}

impl Serialize for ActionId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ActionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interning_the_same_id_twice_gives_the_same_handle() {
        let first = ExtId::intern("mcp:same").expect("well-formed");
        let second = ExtId::intern("mcp:same").expect("well-formed");
        assert_eq!(first, second);
        assert_eq!(first.as_str(), "mcp:same");
    }

    #[test]
    fn an_id_must_have_a_namespace_and_a_name() {
        assert!(ExtId::intern("summarise").is_none(), "no namespace");
        assert!(ExtId::intern("mcp:").is_none(), "no name");
        assert!(ExtId::intern(":summarise").is_none(), "no namespace");
        assert!(ExtId::intern("a:b:c").is_none(), "no single namespace");
        assert!(ExtId::intern("mcp:summarise").is_some());
    }

    #[test]
    fn a_built_in_id_parses_to_a_built_in() {
        assert_eq!(
            "archive".parse::<ActionId>().expect("a built-in"),
            ActionId::Builtin(CommandId::Archive)
        );
        assert_eq!(
            ActionId::Builtin(CommandId::Archive).builtin(),
            Some(CommandId::Archive)
        );
    }

    #[test]
    fn parsing_does_not_depend_on_registration() {
        // The property `[keys]` rests on: an id written in a config file
        // resolves at startup, long before anything registers a command for
        // it. See the module note.
        let parsed = "mcp:never-registered"
            .parse::<ActionId>()
            .expect("a namespaced id always parses");
        assert!(parsed.is_ext());
        assert_eq!(parsed.as_str(), "mcp:never-registered");
        assert_eq!(
            parsed,
            "mcp:never-registered".parse::<ActionId>().expect("again"),
            "and parses to the same id every time, or a keymap could not \
             match a binding against it"
        );
    }

    #[test]
    fn a_misspelt_built_in_is_still_unknown() {
        assert!("arhcive".parse::<ActionId>().is_err());
        assert!("mcp:".parse::<ActionId>().is_err(), "malformed, not an id");
    }

    #[test]
    fn the_namespace_is_the_provenance() {
        let id = ExtId::intern("mcp:summarise").expect("well-formed");
        assert_eq!(id.namespace(), "mcp");
    }

    #[test]
    fn an_action_id_round_trips_through_its_string() {
        for id in [
            ActionId::Builtin(CommandId::Archive),
            ActionId::Ext(ExtId::intern("user:thing").expect("well-formed")),
        ] {
            assert_eq!(id.as_str().parse::<ActionId>().expect("round trip"), id);
        }
    }
}
