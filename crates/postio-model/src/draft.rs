//! Drafts: messages being composed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::attachment::Attachment;
use crate::ids::{AccountId, DraftId, IdentityId, MessageId, ThreadId};
use crate::message::{MessageBody, ServerIdentifiers};

/// What the user was doing when the draft was started.
///
/// Determines quoting, subject prefixing and how recipients were seeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DraftKind {
    /// A new message.
    #[default]
    New,
    /// A reply to the sender only. Bound to `e` in the keymap.
    Reply,
    /// A reply to everyone.
    ReplyAll,
    /// A forward.
    Forward,
}

impl DraftKind {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reply => "reply",
            Self::ReplyAll => "reply_all",
            Self::Forward => "forward",
        }
    }

    /// The inverse of [`DraftKind::as_str`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "new" => Some(Self::New),
            "reply" => Some(Self::Reply),
            "reply_all" => Some(Self::ReplyAll),
            "forward" => Some(Self::Forward),
            _ => None,
        }
    }
}

/// Where a draft is in its life cycle.
///
/// Sending is local-first like everything else: the draft goes to
/// [`DraftState::Queued`] immediately and the UI never waits for SMTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DraftState {
    /// Being edited, autosaved locally.
    #[default]
    Editing,
    /// Handed to the operation queue for sending.
    Queued,
    /// Currently being submitted.
    Sending,
    /// Accepted by the submission server.
    Sent,
    /// Submission failed; the draft is editable again.
    Failed,
}

impl DraftState {
    /// A stable lowercase identifier, for storage.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editing => "editing",
            Self::Queued => "queued",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    /// The inverse of [`DraftState::as_str`].
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "editing" => Some(Self::Editing),
            "queued" => Some(Self::Queued),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// A message being composed.
///
/// Autosaved locally on every change and appended to the account's Drafts
/// mailbox by the sync engine, which is when `server` becomes populated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// Local id.
    pub id: DraftId,
    /// Account this will be sent from.
    pub account_id: AccountId,
    /// Identity to send as; `None` means the account default.
    pub identity_id: Option<IdentityId>,
    /// What kind of composition this is.
    pub kind: DraftKind,
    /// The local message being replied to or forwarded, when there is one.
    pub in_reply_to: Option<MessageId>,
    /// The thread this draft belongs to, so it can be shown inline.
    pub thread_id: Option<ThreadId>,
    /// `To` recipients.
    pub to: Vec<EmailAddress>,
    /// `Cc` recipients.
    pub cc: Vec<EmailAddress>,
    /// `Bcc` recipients.
    pub bcc: Vec<EmailAddress>,
    /// Subject as typed.
    pub subject: String,
    /// Body being composed.
    pub body: MessageBody,
    /// Attachments added so far. These carry
    /// [`MessageId::UNASSIGNED`](crate::MessageId::UNASSIGNED) as their owner
    /// until the draft becomes a sent message.
    pub attachments: Vec<Attachment>,
    /// Life-cycle state.
    pub state: DraftState,
    /// Server identifiers, once the draft has been appended remotely.
    pub server: ServerIdentifiers,
    /// When composition started.
    pub created_at: DateTime<Utc>,
    /// When the draft was last autosaved.
    pub updated_at: DateTime<Utc>,
}

impl Draft {
    /// Builds an empty draft for `account_id`.
    pub fn new(account_id: AccountId) -> Self {
        let now = Utc::now();
        Self {
            id: DraftId::UNASSIGNED,
            account_id,
            identity_id: None,
            kind: DraftKind::New,
            in_reply_to: None,
            thread_id: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: MessageBody::default(),
            attachments: Vec::new(),
            state: DraftState::Editing,
            server: ServerIdentifiers::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Whether the draft has at least one recipient anywhere.
    pub fn has_recipients(&self) -> bool {
        !self.to.is_empty() || !self.cc.is_empty() || !self.bcc.is_empty()
    }

    /// Whether the draft could be sent as it stands.
    pub fn is_sendable(&self) -> bool {
        self.has_recipients() && matches!(self.state, DraftState::Editing | DraftState::Failed)
    }

    /// Every recipient across `To`, `Cc` and `Bcc`.
    pub fn all_recipients(&self) -> impl Iterator<Item = &EmailAddress> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_kinds_and_states_round_trip_through_their_stored_identifiers() {
        for kind in [
            DraftKind::New,
            DraftKind::Reply,
            DraftKind::ReplyAll,
            DraftKind::Forward,
        ] {
            assert_eq!(DraftKind::from_name(kind.as_str()), Some(kind));
        }
        for state in [
            DraftState::Editing,
            DraftState::Queued,
            DraftState::Sending,
            DraftState::Sent,
            DraftState::Failed,
        ] {
            assert_eq!(DraftState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(DraftKind::from_name("reply-all"), None);
        assert_eq!(DraftState::from_name("draft"), None);
    }
}
