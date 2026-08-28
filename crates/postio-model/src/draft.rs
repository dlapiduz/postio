//! Drafts: messages being composed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::account::Identity;
use crate::address::EmailAddress;
use crate::attachment::Attachment;
use crate::ids::{AccountId, DraftId, IdentityId, MessageId, RfcMessageId, ThreadId};
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
    /// The `Message-ID` reserved for this send attempt series, once one has
    /// been reserved (ADR 0021).
    ///
    /// `None` while the draft is being edited. `DraftRepository::queue_send`
    /// mints one in the same write that enqueues `Operation::Send`, and
    /// [`outgoing::build`](crate::outgoing::build) uses it instead of
    /// generating one — so every attempt at a queued draft is the *same*
    /// message rather than a fresh one that happens to say the same thing.
    ///
    /// # Why it is cleared when the draft returns to `Editing`
    ///
    /// A person told a send could not be confirmed, who then opens the draft,
    /// changes it and sends again, is composing a **different** message. A
    /// receiver that deduplicates on `Message-ID` would drop the corrected
    /// version in favour of the one that may already have arrived, which is
    /// worse than having no id at all. The id belongs to one attempt series
    /// at one piece of text, not to the row.
    pub rfc_message_id: Option<RfcMessageId>,
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
            rfc_message_id: None,
            server: ServerIdentifiers::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Sends this draft as `identity`, and puts its signature in the body.
    ///
    /// Replaces rather than appends — [`signature::apply`] splits the body at
    /// the RFC 3676 separator first — so switching identity mid-compose swaps
    /// one signature for the other and reopening a saved draft does not stack
    /// a second copy on the first.
    ///
    /// Plain text only in v1. An identity's HTML signature waits for the
    /// composer to have an HTML body to put it in (`postio-z3b.3`).
    ///
    /// [`signature::apply`]: crate::signature::apply
    pub fn use_identity(&mut self, identity: &Identity) {
        self.identity_id = Some(identity.id);
        let signature = identity
            .signature
            .as_ref()
            .map(|signature| signature.text.as_str());
        let body =
            crate::signature::apply(self.body.text.as_deref().unwrap_or_default(), signature);
        self.body.text = (!body.is_empty()).then_some(body);
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
    use crate::account::Signature;
    use crate::ids::IdentityId;

    fn identity(address: &str, signature: Option<&str>) -> Identity {
        let mut identity = Identity::new(
            AccountId::UNASSIGNED,
            EmailAddress::new(None::<String>, address),
        );
        identity.id = IdentityId::new(7);
        identity.signature = signature.map(|text| Signature {
            id: Default::default(),
            name: String::new(),
            text: text.to_owned(),
            html: None,
        });
        identity
    }

    #[test]
    fn using_an_identity_records_it_and_signs_the_body_once() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.body.text = Some("Looking now.".to_owned());

        let ada = identity("ada@example.com", Some("Ada"));
        draft.use_identity(&ada);
        assert_eq!(draft.identity_id, Some(IdentityId::new(7)));
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nAda\n")
        );

        // The override the user made is the draft's, and re-applying it is not
        // a second signature.
        draft.use_identity(&ada);
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nAda\n")
        );

        let grace = identity("grace@example.net", Some("Grace"));
        draft.use_identity(&grace);
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nGrace\n"),
            "switching identity replaces the signature"
        );
    }

    #[test]
    fn an_identity_with_no_signature_leaves_the_body_unsigned() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.use_identity(&identity("ada@example.com", None));
        assert_eq!(draft.body.text, None, "and does not invent an empty body");

        draft.body.text = Some("Looking now.".to_owned());
        draft.use_identity(&identity("ada@example.com", None));
        assert_eq!(draft.body.text.as_deref(), Some("Looking now.\n"));
    }

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
