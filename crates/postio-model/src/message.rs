//! Messages: the centre of the domain model.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::attachment::Attachment;
use crate::flag::FlagSet;
use crate::headers::Headers;
use crate::ids::{
    AccountId, BlobId, LabelId, MailboxId, MessageId, ModSeq, RfcMessageId, ThreadId, Uid,
    UidValidity,
};
use crate::subject::normalize_subject;

/// The text and rich forms of a body.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MessageBody {
    /// `text/plain` form.
    pub text: Option<String>,
    /// `text/html` form.
    pub html: Option<String>,
}

impl MessageBody {
    /// Whether neither form is present.
    pub fn is_empty(&self) -> bool {
        self.text.as_ref().is_none_or(|text| text.is_empty())
            && self.html.as_ref().is_none_or(|html| html.is_empty())
    }
}

/// How much of a message's content has been fetched.
///
/// The sync engine fetches headers newest-first and backfills bodies lazily, so
/// a message can be listed, searched by header and threaded long before its body
/// exists locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum BodyState {
    /// Nothing beyond what a mailbox listing gave us.
    #[default]
    NotFetched,
    /// Envelope and headers only.
    HeadersOnly,
    /// Some body parts fetched, others still remote.
    Partial,
    /// Every part of the message is available locally.
    Full,
}

impl BodyState {
    /// Whether a renderable body exists locally.
    pub fn has_body(self) -> bool {
        matches!(self, Self::Partial | Self::Full)
    }
}

/// Identifiers assigned by the server, not by Postio.
///
/// Deliberately protocol-neutral. For IMAP, `uid`/`uid_validity`/`mod_seq` are
/// `UID`, `UIDVALIDITY` and `MODSEQ`; another protocol may populate only
/// `remote_id`. A `uid` is meaningless without the `uid_validity` it was seen
/// under.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerIdentifiers {
    /// Server message number within its mailbox.
    pub uid: Option<Uid>,
    /// Generation of the UID space this `uid` belongs to.
    pub uid_validity: Option<UidValidity>,
    /// Modification sequence at which this message was last seen changed.
    pub mod_seq: Option<ModSeq>,
    /// An opaque protocol-specific id, for backends that have one.
    pub remote_id: Option<String>,
}

impl ServerIdentifiers {
    /// Whether the server has assigned this message an identity at all — false
    /// for a message composed locally and not yet uploaded.
    pub fn is_known_to_server(&self) -> bool {
        self.uid.is_some() || self.remote_id.is_some()
    }
}

/// What Postio knows locally about a message's synchronization.
///
/// Every mutating action is local-first: the local row changes, an operation is
/// enqueued, the UI repaints. These fields are what tells the sync engine the
/// local row is ahead of the server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocalSyncState {
    /// How much content is available locally.
    pub body_state: BodyState,
    /// Local flag changes not yet pushed.
    pub flags_dirty: bool,
    /// Whether operations for this message are still in the queue.
    pub has_pending_operations: bool,
    /// Hidden locally pending a remote delete or move.
    pub deleted_locally: bool,
    /// When this row last agreed with the server.
    pub last_synced_at: Option<DateTime<Utc>>,
}

impl LocalSyncState {
    /// Whether the local row has nothing outstanding to push.
    pub fn is_clean(&self) -> bool {
        !self.flags_dirty && !self.has_pending_operations && !self.deleted_locally
    }
}

/// A mail message, as Postio models it.
///
/// Independent of IMAP and of any other protocol: a backend translates into this
/// type and never the other way round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Local id. [`MessageId::UNASSIGNED`] until the row is written.
    pub id: MessageId,
    /// Owning account.
    pub account_id: AccountId,
    /// Mailbox this copy lives in.
    pub mailbox_id: MailboxId,
    /// Thread this message was grouped into, once threading has run.
    pub thread_id: Option<ThreadId>,

    /// RFC 5322 `Message-ID`. Absent when the sender omitted it.
    pub rfc_message_id: Option<RfcMessageId>,
    /// RFC 5322 `In-Reply-To`.
    pub in_reply_to: Option<RfcMessageId>,
    /// RFC 5322 `References`, oldest ancestor first.
    pub references: Vec<RfcMessageId>,

    /// `From`. A list because RFC 5322 permits more than one author.
    pub from: Vec<EmailAddress>,
    /// `Sender`, when it differs from the author.
    pub sender: Option<EmailAddress>,
    /// `Reply-To`.
    pub reply_to: Vec<EmailAddress>,
    /// `To`.
    pub to: Vec<EmailAddress>,
    /// `Cc`.
    pub cc: Vec<EmailAddress>,
    /// `Bcc`. Only ever populated on messages Postio itself sent.
    pub bcc: Vec<EmailAddress>,

    /// `Subject`, verbatim.
    pub subject: Option<String>,
    /// The `Date` header, as claimed by the sender.
    pub date: Option<DateTime<Utc>>,
    /// When the server received it. Always known; this is the sort key.
    pub received_at: DateTime<Utc>,

    /// Body text and HTML, present once `sync.body_state.has_body()`.
    pub body: MessageBody,
    /// A short plain-text snippet for the message list.
    pub preview: Option<String>,
    /// Attachment metadata; the bytes may not be local yet.
    pub attachments: Vec<Attachment>,

    /// Flags and keywords.
    pub flags: FlagSet,
    /// Labels applied to this message.
    pub labels: Vec<LabelId>,
    /// Size in bytes as reported by the server.
    pub size: u64,
    /// Full header block, preserved for display and for later reparsing.
    pub headers: Headers,

    /// Identifiers assigned by the server.
    pub server: ServerIdentifiers,
    /// Local synchronization state.
    pub sync: LocalSyncState,
    /// Blob store key for the raw RFC 5322 bytes, once downloaded.
    pub raw_blob_id: Option<BlobId>,
}

impl Message {
    /// Builds an empty, unpersisted message in `mailbox_id`.
    pub fn new(account_id: AccountId, mailbox_id: MailboxId, received_at: DateTime<Utc>) -> Self {
        Self {
            id: MessageId::UNASSIGNED,
            account_id,
            mailbox_id,
            thread_id: None,
            rfc_message_id: None,
            in_reply_to: None,
            references: Vec::new(),
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
            preview: None,
            attachments: Vec::new(),
            flags: FlagSet::new(),
            labels: Vec::new(),
            size: 0,
            headers: Headers::new(),
            server: ServerIdentifiers::default(),
            sync: LocalSyncState::default(),
            raw_blob_id: None,
        }
    }

    /// Whether this message has been written to the database.
    pub fn is_persisted(&self) -> bool {
        self.id.is_assigned()
    }

    /// Whether the message declares any attachment or inline part.
    pub fn has_attachments(&self) -> bool {
        !self.attachments.is_empty()
    }

    /// The date to sort and display by: the sender's `Date` when present, else
    /// the server's receive time.
    pub fn best_date(&self) -> DateTime<Utc> {
        self.date.unwrap_or(self.received_at)
    }

    /// The subject with reply and forward prefixes stripped, for threading and
    /// for grouping in the UI.
    pub fn normalized_subject(&self) -> String {
        normalize_subject(self.subject.as_deref().unwrap_or_default())
    }

    /// Every ancestor `Message-ID` this message claims, oldest first.
    ///
    /// This is the input JWZ threading walks: `References` in order, with
    /// `In-Reply-To` appended when it is not already the last link. Nothing is
    /// deduplicated beyond that — the threading pass owns that policy.
    pub fn reference_chain(&self) -> impl Iterator<Item = &RfcMessageId> {
        let tail = match &self.in_reply_to {
            Some(in_reply_to) if self.references.last() != Some(in_reply_to) => Some(in_reply_to),
            _ => None,
        };
        self.references.iter().chain(tail)
    }

    /// The address to attribute the message to in the UI.
    pub fn primary_from(&self) -> Option<&EmailAddress> {
        self.from.first().or(self.sender.as_ref())
    }

    /// Every recipient across `To`, `Cc` and `Bcc`, in that order.
    pub fn all_recipients(&self) -> impl Iterator<Item = &EmailAddress> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }
}
