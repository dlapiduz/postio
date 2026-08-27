//! What leaves this machine: the vocabulary of the egress log (#151).
//!
//! Postio's privacy posture is one sentence — *nothing leaves this machine
//! that the user did not ask for* — and three documents promise it will be
//! **proven with a request log rather than asserted** (CLAUDE.md's privacy
//! section, ADR 0003 requirement 7, ADR 0009 Q6). These types are that log's
//! vocabulary, defined here because the crates that open connections
//! (`postio-imap`, `postio-smtp`) and the crate that stores rows
//! (`postio-storage`) may not depend on each other; this one is beneath
//! both.
//!
//! **A row is ids, counts and outcomes — never content.** The same rule
//! every log in this workspace follows (`ARCHITECTURE.md` §11): "Postio
//! opened a connection to imap.example.com:993 and it succeeded" is the
//! auditable fact, and nothing here can carry a byte of anyone's mail.

use chrono::{DateTime, Utc};

use crate::ids::AccountId;

/// Which part of Postio opened the connection.
///
/// ADR 0009 Q6 extends this with AI providers when that subsystem lands;
/// OAuth's token endpoints join when #2's flow does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressSubsystem {
    /// The IMAP sync engine.
    Imap,
    /// The SMTP send path.
    Smtp,
    /// Account discovery: autoconfig lookups and server probes.
    Discovery,
}

impl EgressSubsystem {
    /// The stored spelling, stable because rows outlive binaries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Smtp => "smtp",
            Self::Discovery => "discovery",
        }
    }

    /// The inverse of [`as_str`](Self::as_str), for reading rows back.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "imap" => Some(Self::Imap),
            "smtp" => Some(Self::Smtp),
            "discovery" => Some(Self::Discovery),
            _ => None,
        }
    }
}

/// How the connection attempt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EgressOutcome {
    /// A TCP connection was established.
    Connected,
    /// The attempt failed — refused, unreachable, or timed out.
    Failed,
}

impl EgressOutcome {
    /// The stored spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Failed => "failed",
        }
    }

    /// The inverse of [`as_str`](Self::as_str).
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "connected" => Some(Self::Connected),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// One outbound connection, as the log records it.
#[derive(Debug, Clone, PartialEq)]
pub struct EgressEvent {
    /// When the attempt was made.
    pub at: DateTime<Utc>,
    /// Who made it.
    pub subsystem: EgressSubsystem,
    /// The account it was made for, or `None` before one exists —
    /// discovery during onboarding probes servers for an account that has
    /// not been created yet.
    pub account: Option<AccountId>,
    /// Where it went.
    pub host: String,
    /// And on which port.
    pub port: u16,
    /// How it ended.
    pub outcome: EgressOutcome,
}

/// Where connectors report the connections they open.
///
/// Object-safe and synchronous: it is called from async transports and from
/// a blocking discovery probe, so an implementation must return immediately
/// — hand the event to a channel, never to a database on the caller's
/// thread. `postio-session` owns the implementation that persists;
/// connectors are handed the trait and know nothing else.
pub trait EgressSink: Send + Sync {
    /// Record one connection attempt.
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
        ] {
            assert_eq!(EgressSubsystem::parse(subsystem.as_str()), Some(subsystem));
        }
        for outcome in [EgressOutcome::Connected, EgressOutcome::Failed] {
            assert_eq!(EgressOutcome::parse(outcome.as_str()), Some(outcome));
        }
        assert_eq!(EgressSubsystem::parse("carrier-pigeon"), None);
    }
}
