//! `ActionId`: a built-in `CommandId` or an interned extension id.
//!
//! A well-formed namespaced id parses before anything registers it, so `[keys]` bindings resolve at
//! startup.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::command::{CommandId, UnknownCommand};

/// Separates an extension's namespace from its name; never appears in a built-in id.
pub const NAMESPACE_SEPARATOR: char = ':';

/// The interned id of an extension command, namespaced like `mcp:summarise-thread`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExtId(u32);

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
    /// `None` unless `id` is namespaced. Interning is naming, not registration.
    pub fn intern(id: &str) -> Option<ExtId> {
        if !is_namespaced(id) {
            return None;
        }
        let mut interner = interner().lock().unwrap_or_else(|error| error.into_inner());
        if let Some(&existing) = interner.index.get(id) {
            return Some(ExtId(existing));
        }
        // Leaked on purpose: bounded by the ids the process names, and a freeable id could dangle
        // in a keymap.
        let leaked: &'static str = Box::leak(id.to_owned().into_boxed_str());
        let handle = interner.ids.len() as u32;
        interner.ids.push(leaked);
        interner.index.insert(leaked, handle);
        Some(ExtId(handle))
    }

    pub fn as_str(self) -> &'static str {
        let interner = interner().lock().unwrap_or_else(|error| error.into_inner());
        interner.ids[self.0 as usize]
    }

    pub fn namespace(self) -> &'static str {
        let id = self.as_str();
        id.split_once(NAMESPACE_SEPARATOR)
            .map(|(namespace, _)| namespace)
            .unwrap_or(id)
    }
}

/// Both halves non-empty and exactly one separator.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionId {
    Builtin(CommandId),
    Ext(ExtId),
}

impl ActionId {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionId::Builtin(id) => id.as_str(),
            ActionId::Ext(id) => id.as_str(),
        }
    }

    /// `None` for an extension.
    pub fn builtin(self) -> Option<CommandId> {
        match self {
            ActionId::Builtin(id) => Some(id),
            ActionId::Ext(_) => None,
        }
    }

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

    /// The separator decides, not a lookup, so the answer is the same before and after
    /// registration.
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
