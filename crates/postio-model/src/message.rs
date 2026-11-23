use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::attachment::Attachment;
use crate::flag::FlagSet;
use crate::headers::Headers;
use crate::ids::{
    AccountId, BlobId, LabelId, MailboxId, MessageId, ModSeq, RemoteId, RfcMessageId, ThreadId,
    Uid, UidValidity,
};
use crate::subject::normalize_subject;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageBody {
    pub text: Option<String>,
    pub html: Option<String>,
}

impl MessageBody {
    pub fn is_empty(&self) -> bool {
        self.text.as_ref().is_none_or(|text| text.is_empty())
            && self.html.as_ref().is_none_or(|html| html.is_empty())
    }
}

/// Headers are fetched newest-first and bodies backfilled lazily, so a message can be listed,
/// searched by header and threaded before its body exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum BodyState {
    #[default]
    NotFetched,
    HeadersOnly,
    Partial,
    Full,
}

impl BodyState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFetched => "not_fetched",
            Self::HeadersOnly => "headers_only",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "not_fetched" => Some(Self::NotFetched),
            "headers_only" => Some(Self::HeadersOnly),
            "partial" => Some(Self::Partial),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn has_body(self) -> bool {
        matches!(self, Self::Partial | Self::Full)
    }
}

/// Protocol-neutral: a `uid` is meaningless without the `uid_validity` it was seen under; another
/// protocol may populate only `remote_id`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerIdentifiers {
    /// An immutable account-wide content identity explicitly guaranteed by
    /// the backend. Mailbox addressing and RFC Message-ID are not sufficient.
    pub content_identity: Option<ContentIdentity>,
    pub uid: Option<Uid>,
    pub uid_validity: Option<UidValidity>,
    pub mod_seq: Option<ModSeq>,
    /// The server's own name for the message (#543, ADR 0018 Q2); IMAP derives it from generation
    /// and uid.
    pub remote_id: Option<RemoteId>,
}

/// Backend-guaranteed identity of immutable message content within one account.
///
/// The namespace distinguishes protocols' identity spaces. Adapters must leave
/// this absent unless the key identifies the same bytes across mailboxes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentIdentity {
    pub namespace: String,
    pub key: String,
}

impl ContentIdentity {
    /// Records a backend guarantee; it must never be inferred from mail headers.
    pub fn new(namespace: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            key: key.into(),
        }
    }
}

impl ServerIdentifiers {
    /// Whether the server has assigned this message an identity at all — false
    /// for a message composed locally and not yet uploaded.
    pub fn is_known_to_server(&self) -> bool {
        self.uid.is_some() || self.remote_id.is_some()
    }
}

/// The fields that tell the sync engine the local row is ahead of the server (every mutation is
/// local-first).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocalSyncState {
    pub body_state: BodyState,
    pub flags_dirty: bool,
    pub has_pending_operations: bool,
    pub deleted_locally: bool,
    pub last_synced_at: Option<DateTime<Utc>>,
}

impl LocalSyncState {
    pub fn is_clean(&self) -> bool {
        !self.flags_dirty && !self.has_pending_operations && !self.deleted_locally
    }
}

/// Independent of IMAP: a backend translates into this type, never the other way round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub account_id: AccountId,
    pub mailbox_id: MailboxId,
    pub thread_id: Option<ThreadId>,

    pub rfc_message_id: Option<RfcMessageId>,
    pub in_reply_to: Option<RfcMessageId>,
    pub references: Vec<RfcMessageId>,
    /// The bracketed `List-Id` (RFC 2919); lets a list be detected without the user naming it.
    pub list_id: Option<String>,
    // POSTIO-CONSENT: detection only. Postio never sends a read receipt —
    // neither automatically nor on request; this counts how often one was asked, for the privacy
    // pane (#970).
    /// Whether `Disposition-Notification-To` or `Return-Receipt-To` was present; denormalized at
    /// ingest like `list_id`.
    pub read_receipt_requested: bool,
    /// `None` while not known. Known at filing when an incremental sync asked for the three fields,
    /// else from the body's headers: a first sync does not ask, since every enumerated message
    /// would pay for it (ADR 0025).
    #[serde(default)]
    pub promoted: Option<crate::promoted::PromotedHeaders>,

    pub from: Vec<EmailAddress>,
    pub sender: Option<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    /// `Bcc`. Only ever populated on messages Postio itself sent.
    pub bcc: Vec<EmailAddress>,

    pub subject: Option<String>,
    pub date: Option<DateTime<Utc>>,
    /// When the server received it. Always known; this is the sort key.
    pub received_at: DateTime<Utc>,

    pub body: MessageBody,
    /// Whether the text part declared `format=flowed` (RFC 3676): `body.text`'s lines are
    /// soft-wrapped, not typed breaks. Callers quoting it choose between undoing the wrap and
    /// taking lines at face value (#456).
    pub text_is_flowed: bool,
    pub preview: Option<String>,
    pub attachments: Vec<Attachment>,
    /// `BODYSTRUCTURE`'s root type, not any part's. `None` before a structure is parsed.
    pub content_type: Option<String>,
    /// The `BODYSTRUCTURE` section of the plain-text body (`1`, `1.1`), or `None`. Recorded so
    /// backfill can fetch `BODY.PEEK[<section>]` instead of the whole message, attachments included
    /// (ADR 0017).
    pub text_part_id: Option<String>,
    /// That part's MIME headers rebuilt from `BODYSTRUCTURE`: `BODY[1.1]` returns encoded bytes
    /// with no headers, so prepending this lets `mime::parse` decode it without a second round
    /// trip.
    pub text_part_headers: Option<String>,
    pub html_part_id: Option<String>,
    pub html_part_headers: Option<String>,

    pub flags: FlagSet,
    pub labels: Vec<LabelId>,
    /// Local only, like `LocalSyncState::deleted_locally`: no sync pass sets it. A user preference
    /// like `flags`, not sync bookkeeping, so it lives here rather than in `LocalSyncState`.
    pub snoozed_until: Option<DateTime<Utc>>,
    pub size: u64,
    pub headers: Headers,

    pub server: ServerIdentifiers,
    pub sync: LocalSyncState,
    pub raw_blob_id: Option<BlobId>,
}

impl Message {
    pub fn new(account_id: AccountId, mailbox_id: MailboxId, received_at: DateTime<Utc>) -> Self {
        Self {
            id: MessageId::UNASSIGNED,
            account_id,
            mailbox_id,
            thread_id: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            list_id: None,
            read_receipt_requested: false,
            promoted: None,
            from: Vec::new(),
            sender: None,
            reply_to: Vec::new(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: None,
            date: None,
            received_at,
            body: MessageBody::default(),
            text_is_flowed: false,
            preview: None,
            attachments: Vec::new(),
            content_type: None,
            text_part_id: None,
            text_part_headers: None,
            html_part_id: None,
            html_part_headers: None,
            flags: FlagSet::new(),
            labels: Vec::new(),
            snoozed_until: None,
            size: 0,
            headers: Headers::new(),
            server: ServerIdentifiers::default(),
            sync: LocalSyncState::default(),
            raw_blob_id: None,
        }
    }

    pub fn is_persisted(&self) -> bool {
        self.id.is_assigned()
    }

    pub fn has_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }

    pub fn best_date(&self) -> DateTime<Utc> {
        self.date.unwrap_or(self.received_at)
    }

    pub fn normalized_subject(&self) -> String {
        normalize_subject(self.subject.as_deref().unwrap_or_default())
    }

    /// The input JWZ threading walks: `References` in order, with `In-Reply-To` appended when not
    /// already last.
    pub fn reference_chain(&self) -> impl Iterator<Item = &RfcMessageId> {
        let tail = match &self.in_reply_to {
            Some(in_reply_to) if self.references.last() != Some(in_reply_to) => Some(in_reply_to),
            _ => None,
        };
        self.references.iter().chain(tail)
    }

    pub fn primary_from(&self) -> Option<&EmailAddress> {
        self.from.first().or(self.sender.as_ref())
    }

    pub fn all_recipients(&self) -> impl Iterator<Item = &EmailAddress> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_identity_is_its_namespace_and_key_and_survives_serde() {
        let jmap = ContentIdentity::new("jmap-email", "M1");
        assert_eq!(
            (jmap.namespace.as_str(), jmap.key.as_str()),
            ("jmap-email", "M1")
        );
        assert_ne!(jmap, ContentIdentity::new("gmail-message", "M1"));
        let json = serde_json::to_string(&jmap).expect("serialize");
        assert_eq!(
            serde_json::from_str::<ContentIdentity>(&json).expect("deserialize"),
            jmap
        );
    }

    #[test]
    fn every_body_state_round_trips_through_its_stored_identifier() {
        for state in [
            BodyState::NotFetched,
            BodyState::HeadersOnly,
            BodyState::Partial,
            BodyState::Full,
        ] {
            assert_eq!(BodyState::from_name(state.as_str()), Some(state));
        }
        assert_eq!(BodyState::from_name("headers-only"), None);
    }
}
