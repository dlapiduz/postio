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
    ///
    /// Honest about what it claims: every failure that reaches here is on the
    /// safe side of the boundary — auth, a rejected sender or recipient, a
    /// rejected message, configuration, or retries exhausted *before* the
    /// payload went — so "nothing was delivered" is true rather than hopeful.
    /// The indeterminate case is [`DraftState::Unconfirmed`].
    Failed,
    /// The connection died mid-submission; whether it arrived is unknowable.
    ///
    /// ADR 0021 Decision 3, #674. Once the payload has begun going out, a
    /// dropped session leaves no way to tell delivery from failure — the
    /// server may have accepted and been unable to say so. Retrying would
    /// deliver a duplicate to somebody else's inbox, which cannot be
    /// recalled; calling it `Failed` would claim more than is known.
    ///
    /// So it stops, and says so somewhere that persists. "Unconfirmed"
    /// rather than "uncertain" or "maybe sent" because it names what is
    /// missing, and stays true the moment the confirmation arrives — the
    /// next sync of Sent finding the draft's reserved `Message-ID` (#461)
    /// resolves it to [`DraftState::Sent`] with nothing asked of the user.
    Unconfirmed,
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
            Self::Unconfirmed => "unconfirmed",
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
            "unconfirmed" => Some(Self::Unconfirmed),
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

    /// Sends this draft as `identity`. Does not touch the body.
    ///
    /// Changing who a draft is from is a header change, and that is all it is
    /// (FR-031). This used to re-run [`signature::apply`], which meant
    /// switching the `From` picker rewrote prose the user had already edited:
    /// [`signature::split`] finds the RFC 3676 separator, not intent, so a
    /// signature someone had rewritten looked exactly like one they had not
    /// and was replaced wholesale. There is no version of that heuristic that
    /// never destroys work, so the body is simply left alone.
    ///
    /// The signature is instead something a draft *starts* with — see
    /// [`Self::start_as`], which is the only place one is inserted, and which
    /// is why there is never a second copy to stack.
    ///
    /// [`signature::apply`]: crate::signature::apply
    /// [`signature::split`]: crate::signature::split
    pub fn use_identity(&mut self, identity: &Identity) {
        self.identity_id = Some(identity.id);
    }

    /// Starts this draft as `identity`: records it and signs the body.
    ///
    /// For the moment a draft comes into existence — a new message, a reply, a
    /// forward — and not for a later change of mind, which is
    /// [`Self::use_identity`]. Signing is safe *here* precisely because
    /// nothing has been typed yet: there is no prose to protect, so an
    /// unchanged wrong signature would be the worse outcome.
    ///
    /// Replaces rather than appends — [`signature::apply`] splits the body at
    /// the RFC 3676 separator first — so calling it twice, as resuming a draft
    /// can, does not stack a second copy on the first (FR-032).
    ///
    /// Plain text only in v1. An identity's HTML signature waits for the
    /// composer to have an HTML body to put it in (`postio-z3b.3`).
    ///
    /// [`signature::apply`]: crate::signature::apply
    pub fn start_as(&mut self, identity: &Identity) {
        self.use_identity(identity);
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
    fn starting_as_an_identity_records_it_and_signs_the_body_once() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.body.text = Some("Looking now.".to_owned());

        let ada = identity("ada@example.com", Some("Ada"));
        draft.start_as(&ada);
        assert_eq!(draft.identity_id, Some(IdentityId::new(7)));
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nAda\n")
        );

        // Resuming a draft can run this again, and must not stack.
        draft.start_as(&ada);
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nAda\n")
        );
    }

    #[test]
    fn changing_identity_never_touches_the_body() {
        // FR-031, and the spec's third clarification. `use_identity` used to
        // re-run `signature::apply` on every call, so switching the From
        // picker rewrote prose the user had already edited. There is no
        // reliable way to tell a signature the user has rewritten from one
        // they have not -- `split` finds the `-- ` separator, not intent --
        // so the only rule that never destroys work is to leave the body
        // alone and let the signature be a thing the draft *started* with.
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.start_as(&identity("ada@example.com", Some("Ada")));
        assert_eq!(
            draft.body.text.as_deref(),
            Some("\n\n-- \nAda\n"),
            "a draft still opens carrying its identity's signature (FR-030)"
        );

        // The user writes, and edits the signature the draft came with.
        draft.body.text = Some("Looking now.\n\n-- \nAda, from the boat\n".to_owned());
        let before = draft.body.text.clone();

        draft.use_identity(&identity("grace@example.net", Some("Grace")));

        assert_eq!(
            draft.identity_id,
            Some(IdentityId::new(7)),
            "the From did change"
        );
        assert_eq!(
            draft.body.text, before,
            "changing who a draft is from must not rewrite what is in it -- \
             the hand-edited signature is the user's prose now"
        );
    }

    #[test]
    fn a_draft_never_carries_two_signatures() {
        // FR-032. `start_as` is the one place a signature is inserted, and
        // calling it twice -- which resuming a draft can do -- must not stack.
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        let ada = identity("ada@example.com", Some("Ada"));
        draft.start_as(&ada);
        draft.start_as(&ada);
        assert_eq!(
            draft
                .body
                .text
                .as_deref()
                .unwrap_or_default()
                .matches("-- \n")
                .count(),
            1,
            "two separators means the recipient sees the signature twice"
        );

        // And switching before anything is typed re-signs rather than stacks,
        // because until the user has written a word there is no prose to
        // protect and an unchanged wrong signature would be worse.
        let mut fresh = Draft::new(AccountId::UNASSIGNED);
        fresh.start_as(&ada);
        fresh.start_as(&identity("grace@example.net", Some("Grace")));
        assert_eq!(fresh.body.text.as_deref(), Some("\n\n-- \nGrace\n"));
    }

    #[test]
    fn an_identity_with_no_signature_leaves_the_body_unsigned() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.start_as(&identity("ada@example.com", None));
        assert_eq!(draft.body.text, None, "and does not invent an empty body");

        draft.body.text = Some("Looking now.".to_owned());
        draft.start_as(&identity("ada@example.com", None));
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
            DraftState::Unconfirmed,
        ] {
            assert_eq!(DraftState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(DraftKind::from_name("reply-all"), None);
        assert_eq!(DraftState::from_name("draft"), None);
    }
}
