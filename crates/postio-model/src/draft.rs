use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::account::Identity;
use crate::address::EmailAddress;
use crate::attachment::Attachment;
use crate::ids::{AccountId, DraftId, IdentityId, LabelId, MessageId, RfcMessageId, ThreadId};
use crate::message::{MessageBody, ServerIdentifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DraftKind {
    #[default]
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl DraftKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Reply => "reply",
            Self::ReplyAll => "reply_all",
            Self::Forward => "forward",
        }
    }

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

/// Sending is local-first: the draft goes to `Queued` at once and the UI never waits for SMTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DraftState {
    #[default]
    Editing,
    Queued,
    Sending,
    Sent,
    /// Failed before the payload went (auth, rejected sender/recipient/message, config, retries
    /// exhausted), so "nothing was delivered" is true; the draft is editable again. The
    /// indeterminate case is `Unconfirmed`.
    Failed,
    /// The connection died after the payload began going out, so delivery cannot be told from
    /// failure. Retrying could duplicate mail that cannot be recalled; `Failed` would claim too
    /// much (ADR 0021 Decision 3,
    /// #674).
    /// The next sync of Sent finding the draft's reserved `Message-ID` resolves it to `Sent`
    /// (#461).
    Unconfirmed,
}

impl DraftState {
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

/// Autosaved locally on every change; the sync engine appends it to the Drafts mailbox, which
/// populates `server`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub id: DraftId,
    pub account_id: AccountId,
    pub identity_id: Option<IdentityId>,
    pub kind: DraftKind,
    pub in_reply_to: Option<MessageId>,
    /// Not `in_reply_to`, which a forward leaves empty: it drives threading headers, and a forward
    /// starts a new conversation. This lets the sender fetch an attachment's missing bytes from the
    /// original via `part_id` rather than refuse to send (#1686).
    #[serde(default)]
    pub forwarded_from: Option<MessageId>,
    pub thread_id: Option<ThreadId>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body: MessageBody,
    /// Stored, not derived from `body.html`: turning rich off keeps the marks in case it is turned
    /// back on, which "has an HTML part" cannot express (#1271). Decides what leaves: `text/html`
    /// plus a `text/plain` alternative always, or `text/plain` alone, wrapped and flowed.
    pub rich: bool,
    /// The Markdown typed in the terminal composer, `None` if another frontend saved last. Kept
    /// beside `body` so the terminal reopens exactly what was typed and the text part is sent
    /// `format=fixed` (see [`outgoing::build`](crate::outgoing::build)).
    #[serde(default)]
    pub body_markdown: Option<String>,
    /// Applied to its conversation when sent; a reply starts with its conversation's own.
    #[serde(default)]
    pub labels: Vec<LabelId>,
    /// A `METHOD:REPLY` object as `postio-calendar` writes it, which `outgoing::build` places as
    /// the `text/calendar` alternative. `None` for everything a person writes.
    #[serde(default)]
    pub calendar_reply: Option<String>,
    /// When set, sending sets a reminder on the conversation, due then; if nobody else has written
    /// by then it returns to the top of the Focus inbox as "No reply since".
    #[serde(default)]
    pub remind_at: Option<DateTime<Utc>>,
    /// Owned by `MessageId::UNASSIGNED` until the draft becomes a sent message.
    pub attachments: Vec<Attachment>,
    pub state: DraftState,
    /// Reserved by `DraftRepository::queue_send` in the write that enqueues `Operation::Send`, so
    /// every attempt is the same message (ADR 0021). Cleared on return to `Editing`: an edited
    /// resend is a different message, and a receiver deduplicating on `Message-ID` would drop the
    /// correction.
    pub rfc_message_id: Option<RfcMessageId>,
    pub server: ServerIdentifiers,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Draft {
    pub fn new(account_id: AccountId) -> Self {
        let now = crate::clock::now().with_timezone(&Utc);
        Self {
            id: DraftId::UNASSIGNED,
            account_id,
            identity_id: None,
            kind: DraftKind::New,
            in_reply_to: None,
            forwarded_from: None,
            thread_id: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: String::new(),
            body: MessageBody::default(),
            // Plain by default: a composer that opened rich would decide the shape of the person's
            // mail.
            rich: false,
            body_markdown: None,
            labels: Vec::new(),
            calendar_reply: None,
            remind_at: None,
            attachments: Vec::new(),
            state: DraftState::Editing,
            rfc_message_id: None,
            server: ServerIdentifiers::default(),
            created_at: now,
            updated_at: now,
        }
    }

    /// A header change only; the body is left alone. Re-running `signature::apply` rewrote prose
    /// the user had edited, since `split` finds the RFC 3676 separator, not intent, so no heuristic
    /// avoids destroying work. The signature is inserted only by `start_as`.
    pub fn use_identity(&mut self, identity: &Identity) {
        self.identity_id = Some(identity.id);
    }

    /// For the moment a draft comes into existence, not a later change of mind (`use_identity`):
    /// nothing is typed yet. Replaces rather than appends, so resuming a draft does not stack a
    /// second copy. Plain text only: an HTML signature waits for an HTML composer body
    /// (`postio-z3b.3`).
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

    pub fn has_recipients(&self) -> bool {
        !self.to.is_empty() || !self.cc.is_empty() || !self.bcc.is_empty()
    }

    pub fn is_sendable(&self) -> bool {
        self.has_recipients() && matches!(self.state, DraftState::Editing | DraftState::Failed)
    }

    pub fn all_recipients(&self) -> impl Iterator<Item = &EmailAddress> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    Keep,
    Drop,
}

/// Anything the user typed makes the draft worth keeping (`Esc` never silently discards content).
/// Neither whitespace nor the composer's own signature counts, or every abandoned composer would
/// persist.
pub fn closing(draft: &Draft) -> Closing {
    let body = draft.body.text.as_deref().unwrap_or_default();
    let written = crate::signature::split(body).0;
    if draft.has_recipients() || !draft.subject.trim().is_empty() || !written.trim().is_empty() {
        Closing::Keep
    } else {
        Closing::Drop
    }
}

/// A reply or forward still exactly as it opened has nothing the person wrote and is dropped. Only
/// app-built compositions count: a new message opened with content (a link, an undone send) or a
/// resumed draft is kept.
pub fn closing_since(draft: &Draft, opened: &Draft) -> Closing {
    if closing(draft) == Closing::Drop {
        return Closing::Drop;
    }
    let written = |draft: &Draft| {
        let body = draft.body.text.as_deref().unwrap_or_default();
        crate::signature::split(body).0.trim().to_owned()
    };
    let untouched = opened.kind != DraftKind::New
        && !opened.id.is_assigned()
        && draft.to == opened.to
        && draft.cc == opened.cc
        && draft.bcc == opened.bcc
        && draft.subject == opened.subject
        && draft.attachments.len() == opened.attachments.len()
        && written(draft) == written(opened);
    if untouched {
        Closing::Drop
    } else {
        Closing::Keep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Signature;
    use crate::ids::IdentityId;

    fn opened_reply() -> Draft {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.kind = DraftKind::Reply;
        draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        draft.subject = "Re: Plans".to_owned();
        draft.body.text = Some("\n\nOn 2026-08-26, Ada wrote:\n> hello\n\n-- \nGrace\n".to_owned());
        draft
    }

    #[test]
    fn a_reply_nobody_typed_in_is_dropped() {
        let opened = opened_reply();
        let mut now = opened.clone();
        now.body.text = Some("\n\nOn 2026-08-26, Ada wrote:\n> hello\n".to_owned());
        assert_eq!(closing_since(&now, &opened), Closing::Drop);
        assert_eq!(closing_since(&opened, &opened), Closing::Drop);
    }

    #[test]
    fn a_reply_with_a_word_written_is_kept() {
        let opened = opened_reply();
        let mut now = opened.clone();
        now.body.text = Some("Thanks!\n\nOn 2026-08-26, Ada wrote:\n> hello\n".to_owned());
        assert_eq!(closing_since(&now, &opened), Closing::Keep);
        let mut recipient = opened.clone();
        recipient.cc = vec![EmailAddress::new(None::<String>, "grace@example.net")];
        assert_eq!(closing_since(&recipient, &opened), Closing::Keep);
    }

    #[test]
    fn a_new_message_opened_with_content_is_kept_even_untouched() {
        let mut opened = opened_reply();
        opened.kind = DraftKind::New;
        assert_eq!(closing_since(&opened, &opened), Closing::Keep);
    }

    #[test]
    fn a_saved_draft_resumed_untouched_is_kept() {
        let mut opened = opened_reply();
        opened.id = DraftId::new(4);
        assert_eq!(closing_since(&opened, &opened), Closing::Keep);
    }

    #[test]
    fn a_new_draft_is_dated_by_the_clock_seam() {
        // The queued reply is dated by the frozen clock, not the real day.
        let _turn = crate::clock::TURN.lock().unwrap_or_else(|e| e.into_inner());
        let frozen = chrono::Local::now();
        crate::clock::freeze(frozen);
        std::thread::sleep(std::time::Duration::from_millis(5));
        let draft = Draft::new(AccountId::UNASSIGNED);
        crate::clock::thaw();
        assert_eq!(draft.created_at, frozen.with_timezone(&Utc));
        assert_eq!(draft.updated_at, draft.created_at);
    }

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

        draft.start_as(&ada);
        assert_eq!(
            draft.body.text.as_deref(),
            Some("Looking now.\n\n-- \nAda\n")
        );
    }

    #[test]
    fn changing_identity_never_touches_the_body() {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.start_as(&identity("ada@example.com", Some("Ada")));
        assert_eq!(
            draft.body.text.as_deref(),
            Some("\n\n-- \nAda\n"),
            "a draft still opens carrying its identity's signature (FR-030)"
        );

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
