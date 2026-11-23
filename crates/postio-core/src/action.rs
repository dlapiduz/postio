//! `ActionId`: what a binding, a menu row or a palette row runs.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::command::{CommandId, UnknownCommand};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionId {
    Builtin(CommandId),
}

impl ActionId {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionId::Builtin(id) => id.as_str(),
        }
    }

    pub fn builtin(self) -> Option<CommandId> {
        match self {
            ActionId::Builtin(id) => Some(id),
        }
    }
}

impl From<CommandId> for ActionId {
    fn from(id: CommandId) -> Self {
        ActionId::Builtin(id)
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ActionId {
    type Err = UnknownCommand;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
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
    fn a_misspelt_built_in_is_still_unknown() {
        assert!("arhcive".parse::<ActionId>().is_err());
        assert!("mcp:x".parse::<ActionId>().is_err(), "no extensions");
    }

    #[test]
    fn an_action_id_round_trips_through_its_string() {
        let id = ActionId::Builtin(CommandId::Archive);
        assert_eq!(id.as_str().parse::<ActionId>().expect("round trip"), id);
    }
}
