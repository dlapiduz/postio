//! The egress log's vocabulary: proof by request log that nothing leaves the machine unasked (ADR
//! 0003 req 7, ADR 0009 Q6).
//! Defined here because the connecting crates and `postio-storage` cannot depend on each other.
//! A row is ids, counts and outcomes, never content.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::AccountId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EgressSubsystem {
    Imap,
    Smtp,
    Discovery,
    Model,
    Caldav,
}

impl EgressSubsystem {
    /// Stable, because rows outlive binaries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Smtp => "smtp",
            Self::Discovery => "discovery",
            Self::Model => "model",
            Self::Caldav => "caldav",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "imap" => Some(Self::Imap),
            "smtp" => Some(Self::Smtp),
            "discovery" => Some(Self::Discovery),
            "model" => Some(Self::Model),
            "caldav" => Some(Self::Caldav),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EgressOutcome {
    Connected,
    Failed,
}

impl EgressOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connected" => Some(Self::Connected),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressEvent {
    pub at: DateTime<Utc>,
    pub subsystem: EgressSubsystem,
    /// `None` before an account exists, e.g. discovery during onboarding.
    pub account: Option<AccountId>,
    pub host: String,
    pub port: u16,
    pub outcome: EgressOutcome,
}

/// Object-safe and synchronous: called from async transports and a blocking discovery probe, so
/// `record` must return immediately, handing the event to a channel and never a database.
/// `postio-session` owns the persisting implementation.
pub trait EgressSink: Send + Sync {
    fn record(&self, event: EgressEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_round_trip() {
        for subsystem in [
            EgressSubsystem::Imap,
            EgressSubsystem::Smtp,
            EgressSubsystem::Discovery,
            EgressSubsystem::Model,
        ] {
            assert_eq!(EgressSubsystem::parse(subsystem.as_str()), Some(subsystem));
        }
        for outcome in [EgressOutcome::Connected, EgressOutcome::Failed] {
            assert_eq!(EgressOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(EgressSubsystem::parse("carrier-pigeon"), None);
    }
}
