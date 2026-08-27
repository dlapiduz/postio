//! What a backend hands back, in domain terms.
//!
//! Every type here is protocol-neutral by construction: an IMAP `ENVELOPE`, a
//! JMAP `Email/get` response and the mock's canned data all land in the same
//! shapes. That is what makes the [`MailBackend`](super::MailBackend) trait a
//! porting surface rather than an IMAP-shaped hole.
//!
//! The types stop one step short of [`postio_model::Message`] on purpose: a
//! backend does not know an account's local ids, so it reports what the server
//! said and lets [`FetchedMessage::into_message`] bind it to a row.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, BodyState, Disposition, EmailAddress, FlagSet, Mailbox, MailboxId,
    MailboxRole, Message, MessageId, ModSeq, RfcMessageId, Uid, UidValidity,
};

// ---------------------------------------------------------------------------
// Mailboxes
// ---------------------------------------------------------------------------

/// Which mailboxes a `LIST` should return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxFilter {
    /// The IMAP list pattern. `*` is every mailbox at every depth.
    pub pattern: String,
    /// Whether to return only mailboxes the account is subscribed to.
    pub subscribed_only: bool,
}

impl MailboxFilter {
    /// Every mailbox on the server.
    pub fn all() -> Self {
        Self {
            pattern: "*".to_owned(),
            subscribed_only: false,
        }
    }

    /// Only the mailboxes the account is subscribed to.
    pub fn subscribed() -> Self {
        Self {
            subscribed_only: true,
            ..Self::all()
        }
    }

    /// Mailboxes matching an IMAP list pattern.
    pub fn matching(pattern: impl Into<String>) -> Self {
        Self {
            pattern: pattern.into(),
            subscribed_only: false,
        }
    }
}

impl Default for MailboxFilter {
    fn default() -> Self {
        Self::all()
    }
}

/// One row of a `LIST` response.
///
/// `role` is resolved here, at the edge, because that is where both halves of
/// the evidence are: the server's `SPECIAL-USE` attributes and the folder name.
/// iCloud advertises no attributes for `Sent Messages` or `Deleted Messages`,
/// so the name heuristic is not a nicety — see
/// [`MailboxRole::resolve`](postio_model::MailboxRole::resolve).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxSummary {
    /// Full path, hierarchy delimiters included.
    pub path: String,
    /// The hierarchy delimiter the server reported, if any.
    pub delimiter: Option<char>,
    /// The raw attributes, e.g. `\HasChildren`, `\Sent`, `\Noselect`.
    pub attributes: Vec<String>,
    /// What the folder is for.
    pub role: MailboxRole,
    /// Whether the folder can hold messages.
    pub selectable: bool,
    /// Whether the account is subscribed to it.
    pub subscribed: bool,
}

impl MailboxSummary {
    /// Builds a summary, resolving the role from the attributes and the path.
    pub fn new<I, S>(path: impl Into<String>, delimiter: Option<char>, attributes: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let path = path.into();
        let attributes: Vec<String> = attributes.into_iter().map(Into::into).collect();
        let role = MailboxRole::resolve(&attributes, &path);
        let selectable = !attributes
            .iter()
            .any(|attribute| attribute.eq_ignore_ascii_case("\\Noselect"));

        Self {
            path,
            delimiter,
            attributes,
            role,
            selectable,
            subscribed: true,
        }
    }

    /// The leaf name, for display.
    pub fn name(&self) -> &str {
        match self.delimiter {
            Some(delimiter) => self
                .path
                .rsplit(delimiter)
                .next()
                .unwrap_or(&self.path)
                .trim(),
            None => &self.path,
        }
    }

    /// The path of this folder's parent, when it has one.
    ///
    /// Text, not an id: resolving a hierarchy into local parent links needs
    /// the whole listing and the ids it was written under, which is the
    /// repository's job. This is the evidence it works from.
    pub fn parent_path(&self) -> Option<String> {
        let delimiter = self.delimiter?;
        self.path
            .rsplit_once(delimiter)
            .map(|(parent, _)| parent.to_owned())
            .filter(|parent| !parent.is_empty())
    }

    /// How deep the folder sits: `0` at the top level.
    pub fn depth(&self) -> usize {
        match self.delimiter {
            Some(delimiter) => self.path.matches(delimiter).count(),
            None => 0,
        }
    }

    /// Whether the server said this folder has children.
    pub fn has_children(&self) -> bool {
        self.attributes
            .iter()
            .any(|attribute| attribute.eq_ignore_ascii_case("\\HasChildren"))
    }

    /// Binds this summary to an account, producing an unpersisted mailbox.
    ///
    /// The parent link is left unset: resolving a hierarchy needs the whole
    /// listing and the local ids it was written under, which is the
    /// repository's job, not the backend's.
    pub fn into_mailbox(self, account_id: AccountId) -> Mailbox {
        let mut mailbox = Mailbox::new(account_id, self.path, self.delimiter);
        mailbox.role = self.role;
        mailbox.selectable = self.selectable;
        mailbox.subscribed = self.subscribed;
        mailbox
    }
}

/// Whether a mailbox is opened for writing.
///
/// `ReadOnly` is `EXAMINE`: it does not set `\Recent`, which matters when
/// another client is watching the same mailbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    /// `SELECT` — flags and expunges are allowed.
    #[default]
    ReadWrite,
    /// `EXAMINE` — look, do not touch.
    ReadOnly,
}

/// What the server says about a mailbox's contents right now.
///
/// `uid_validity` and `highest_mod_seq` are the two values incremental sync is
/// built on: the first says whether cached UIDs still mean anything, the second
/// says how far behind we are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxStatus {
    /// The mailbox this describes.
    pub path: String,
    /// Generation of the UID space. A change invalidates every cached UID.
    pub uid_validity: UidValidity,
    /// The UID the server will assign to the next message it stores.
    pub uid_next: Uid,
    /// How many messages the mailbox holds.
    pub exists: u32,
    /// How many are unread, when the server volunteered it.
    pub unseen: Option<u32>,
    /// Highest modification sequence, when the server speaks CONDSTORE.
    pub highest_mod_seq: Option<ModSeq>,
    /// The flags the server will store permanently.
    pub permanent_flags: FlagSet,
    /// Whether the server said new keywords may be created (`\*`).
    pub can_create_keywords: bool,
    /// Whether the mailbox was opened read-only.
    pub read_only: bool,
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// The addressing and identity headers of a message.
///
/// `references` is not part of an IMAP `ENVELOPE`; it comes from a companion
/// `BODY.PEEK[HEADER.FIELDS (REFERENCES)]` in the same fetch, because JWZ
/// threading cannot work without it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Envelope {
    /// The `Date` header, as claimed by the sender.
    pub date: Option<DateTime<Utc>>,
    /// `Subject`, verbatim and already decoded.
    pub subject: Option<String>,
    /// `From`.
    pub from: Vec<EmailAddress>,
    /// `Sender`, when it differs from the author.
    pub sender: Option<EmailAddress>,
    /// `Reply-To`.
    pub reply_to: Vec<EmailAddress>,
    /// `To`.
    pub to: Vec<EmailAddress>,
    /// `Cc`.
    pub cc: Vec<EmailAddress>,
    /// `Bcc`.
    pub bcc: Vec<EmailAddress>,
    /// `Message-ID`.
    pub message_id: Option<RfcMessageId>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<RfcMessageId>,
    /// `References`, oldest ancestor first.
    pub references: Vec<RfcMessageId>,
    /// The bracketed identifier out of `List-Id`, when the message carries
    /// one. Not part of an `ENVELOPE` either, for the same reason
    /// `references` is not: a companion `BODY.PEEK[HEADER.FIELDS (LIST-ID)]`
    /// in the same fetch.
    pub list_id: Option<String>,
}

/// One MIME part, as described by the server rather than by parsing bytes.
///
/// This is the whole point of `BODYSTRUCTURE`: attachment names, types and
/// sizes are known before a single body byte is downloaded, so the list can
/// show a paperclip and search can answer `has:attachment` on a mailbox nobody
/// has opened yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartNode {
    section: String,
    mime_type: String,
    charset: Option<String>,
    encoding: Option<String>,
    size: u64,
    filename: Option<String>,
    content_id: Option<String>,
    disposition: Disposition,
}

impl PartNode {
    /// A part at `section` — `1`, `2.1`, and so on.
    pub fn new(section: impl Into<String>, mime_type: impl Into<String>, size: u64) -> Self {
        Self {
            section: section.into(),
            mime_type: mime_type.into().to_ascii_lowercase(),
            charset: None,
            encoding: None,
            size,
            filename: None,
            content_id: None,
            disposition: Disposition::Attachment,
        }
    }

    /// Sets the declared filename.
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Sets the `Content-ID`, for a part referenced from HTML by `cid:`.
    pub fn with_content_id(mut self, content_id: impl Into<String>) -> Self {
        self.content_id = Some(content_id.into());
        self
    }

    /// Sets how the part is meant to be presented.
    pub fn with_disposition(mut self, disposition: Disposition) -> Self {
        self.disposition = disposition;
        self
    }

    /// Sets the declared charset.
    pub fn with_charset(mut self, charset: impl Into<String>) -> Self {
        self.charset = Some(charset.into());
        self
    }

    /// Sets the declared `Content-Transfer-Encoding`.
    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    /// The IMAP section number, e.g. `2.1`.
    pub fn section(&self) -> &str {
        &self.section
    }

    /// The lowercased MIME type, e.g. `application/pdf`.
    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    /// The declared charset, if any.
    pub fn charset(&self) -> Option<&str> {
        self.charset.as_deref()
    }

    /// The declared transfer encoding, if any.
    pub fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    /// The part's MIME header block, rebuilt from what `BODYSTRUCTURE` said.
    ///
    /// `BODY[1.1]` hands back a part's encoded bytes and none of its headers,
    /// so nothing in the response explains whether they are base64, or what
    /// charset they are in. Both were reported at header-sync time; this
    /// renders them back into the two headers a parser needs, so a fetched
    /// section can be turned into a self-contained entity without spending a
    /// second round trip on `BODY[1.1.MIME]` (ADR 0017).
    ///
    /// Only what was actually declared is written. RFC 2045's defaults --
    /// `7bit`, `us-ascii` -- are the parser's to apply, and stating a guess
    /// here would only risk contradicting it.
    pub fn mime_headers(&self) -> String {
        let mut headers = format!("Content-Type: {}", self.mime_type);
        if let Some(charset) = &self.charset {
            headers.push_str(&format!("; charset=\"{charset}\""));
        }
        headers.push_str("\r\n");
        if let Some(encoding) = &self.encoding {
            headers.push_str(&format!("Content-Transfer-Encoding: {encoding}\r\n"));
        }
        headers
    }

    /// The size in bytes the server declared.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The declared filename, if any.
    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    /// The `Content-ID`, if any.
    pub fn content_id(&self) -> Option<&str> {
        self.content_id.as_deref()
    }

    /// How the part is meant to be presented.
    pub fn disposition(&self) -> &Disposition {
        &self.disposition
    }

    /// Whether this part is the message's own text, rather than something
    /// hanging off it.
    ///
    /// A `text/plain` or `text/html` part with no filename and no inline
    /// disposition is the body; the same type *with* a filename is an
    /// attachment that happens to be text, and must not be rendered as the
    /// message.
    pub fn is_body_text(&self) -> bool {
        matches!(self.mime_type.as_str(), "text/plain" | "text/html")
            && self.filename.is_none()
            && self.disposition != Disposition::Inline
    }

    /// Turns this part into attachment metadata with no bytes downloaded.
    pub fn to_attachment(&self, message_id: MessageId) -> Attachment {
        let mut attachment = Attachment::new(message_id, self.mime_type.clone(), self.size);
        attachment.filename = self.filename.clone();
        attachment.content_id = self.content_id.clone();
        attachment.disposition = self.disposition.clone();
        attachment.part_id = Some(self.section.clone());
        // What will explain the bytes when somebody fetches `BODY[<section>]`
        // and gets them back encoded, with no headers of their own (ADR 0017,
        // the payload axis). This is the only moment a `BODYSTRUCTURE` and an
        // `Attachment` are both in hand, so it is here or it is a second
        // round trip for `[<section>.MIME]` per part.
        attachment.part_headers = Some(self.mime_headers());
        attachment
    }
}

/// A message's MIME tree, flattened into the parts a fetch can ask for.
///
/// Flat rather than nested because every consumer wants the same two things —
/// "which part is the body" and "what can be downloaded later" — and neither
/// needs the tree. The section string keeps the structure recoverable.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyStructure {
    /// The message's own content type — `BODYSTRUCTURE`'s root, not any one
    /// part's: `multipart/mixed`, `multipart/alternative`,
    /// `multipart/related`, or a single part's own type when the message is
    /// not multipart at all. This is the row the parts tree hangs off.
    content_type: String,
    parts: Vec<PartNode>,
}

impl BodyStructure {
    /// Builds a structure from the message's own `content_type` and its
    /// `parts`, in document order.
    pub fn from_parts<I: IntoIterator<Item = PartNode>>(
        content_type: impl Into<String>,
        parts: I,
    ) -> Self {
        Self {
            content_type: content_type.into().to_ascii_lowercase(),
            parts: parts.into_iter().collect(),
        }
    }

    /// The message's own content type. See the field doc for what "own"
    /// means here.
    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    /// The parts, in document order.
    pub fn parts(&self) -> &[PartNode] {
        &self.parts
    }

    /// The part to render as the plain-text body, if there is one.
    pub fn text_part(&self) -> Option<&PartNode> {
        self.parts
            .iter()
            .find(|part| part.mime_type() == "text/plain" && part.is_body_text())
    }

    /// The part to render as the HTML body, if there is one.
    pub fn html_part(&self) -> Option<&PartNode> {
        self.parts
            .iter()
            .find(|part| part.mime_type() == "text/html" && part.is_body_text())
    }

    /// Every part that is not the message's own text.
    pub fn attachments(&self) -> impl Iterator<Item = &PartNode> {
        self.parts.iter().filter(|part| !part.is_body_text())
    }

    /// Attachment metadata for `message_id`, with no bytes downloaded.
    pub fn to_attachments(&self, message_id: MessageId) -> Vec<Attachment> {
        self.attachments()
            .map(|part| part.to_attachment(message_id))
            .collect()
    }
}

/// One message as a fetch reported it.
///
/// Headers only: this is what makes an initial sync feel fast. A mailbox can
/// be listed, threaded and searched by header long before a body exists
/// locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedMessage {
    /// The server's UID for this message.
    pub uid: Uid,
    /// The UID generation it was observed under. A `uid` without this is
    /// meaningless.
    pub uid_validity: UidValidity,
    /// `MODSEQ`, when the server speaks CONDSTORE.
    pub mod_seq: Option<ModSeq>,
    /// The flags the server reports, `\Recent` included.
    pub flags: FlagSet,
    /// `INTERNALDATE` — when the server received it. Always known.
    pub internal_date: DateTime<Utc>,
    /// `RFC822.SIZE`.
    pub size: u64,
    /// The addressing headers, when they were asked for.
    pub envelope: Option<Envelope>,
    /// The MIME tree, when `BODYSTRUCTURE` was asked for.
    pub structure: Option<BodyStructure>,
}

impl FetchedMessage {
    /// Binds this fetch result to a local account and mailbox.
    ///
    /// `\Recent` is dropped here rather than at the storage layer: it is a
    /// per-session server signal, and a `Message` that carries it would
    /// serialize one session's accident into the database.
    pub fn into_message(self, account_id: AccountId, mailbox_id: MailboxId) -> Message {
        let mut message = Message::new(account_id, mailbox_id, self.internal_date);

        message.flags = self.flags.persistable();
        message.size = self.size;
        message.server.uid = Some(self.uid);
        message.server.uid_validity = Some(self.uid_validity);
        message.server.mod_seq = self.mod_seq;
        message.server.remote_id = Some(super::identity::remote_id(self.uid_validity, self.uid));

        if let Some(envelope) = self.envelope {
            message.date = envelope.date;
            message.subject = envelope.subject;
            message.from = envelope.from;
            message.sender = envelope.sender;
            message.reply_to = envelope.reply_to;
            message.to = envelope.to;
            message.cc = envelope.cc;
            message.bcc = envelope.bcc;
            message.rfc_message_id = envelope.message_id;
            message.in_reply_to = envelope.in_reply_to;
            message.references = envelope.references;
            message.list_id = envelope.list_id;
            message.sync.body_state = BodyState::HeadersOnly;
        }

        if let Some(structure) = self.structure {
            message.content_type = Some(structure.content_type().to_owned());
            // Where the message keeps its own words. This is the only place a
            // `BODYSTRUCTURE` and a `Message` are both in hand, and the
            // backfill downstream needs to *name* these sections rather than
            // fetch the whole message and sift it (ADR 0017).
            message.text_part_id = structure.text_part().map(|part| part.section().to_owned());
            message.text_part_headers = structure.text_part().map(PartNode::mime_headers);
            message.html_part_id = structure.html_part().map(|part| part.section().to_owned());
            message.html_part_headers = structure.html_part().map(PartNode::mime_headers);
            message.attachments = structure.to_attachments(MessageId::UNASSIGNED);
        }

        message
    }
}

// ---------------------------------------------------------------------------
// Bodies
// ---------------------------------------------------------------------------

/// Which bytes of a message to fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyPart {
    /// The whole RFC 5322 message, headers included.
    Whole,
    /// The header block only.
    Headers,
    /// Everything after the header block.
    Text,
    /// One MIME section, e.g. `2.1` — how an attachment is downloaded.
    Section(String),
}

impl BodyPart {
    /// A specific MIME section.
    pub fn section(section: impl Into<String>) -> Self {
        Self::Section(section.into())
    }

    /// The section specifier as it appears inside `BODY[…]`.
    pub fn section_spec(&self) -> String {
        match self {
            Self::Whole => String::new(),
            Self::Headers => "HEADER".to_owned(),
            Self::Text => "TEXT".to_owned(),
            Self::Section(section) => section.clone(),
        }
    }
}

/// What a body fetch delivered.
///
/// The bytes are not in here: they went to the caller's
/// [`BodySink`](super::BodySink) as they arrived, which is what keeps a 40 MB
/// attachment off the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedBody {
    /// The message the bytes came from.
    pub uid: Uid,
    /// The part that was asked for.
    pub part: BodyPart,
    /// How many bytes reached the sink.
    pub bytes_written: u64,
}

/// A message to upload with `APPEND` — a sent copy, or a saved draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendMessage {
    /// The raw RFC 5322 bytes.
    pub raw: Vec<u8>,
    /// Flags to set on arrival.
    pub flags: FlagSet,
    /// The `INTERNALDATE` to claim; the server's clock is used when absent.
    pub internal_date: Option<DateTime<Utc>>,
}

impl AppendMessage {
    /// A message to append, with no flags and the server's own timestamp.
    pub fn new(raw: Vec<u8>) -> Self {
        Self {
            raw,
            flags: FlagSet::new(),
            internal_date: None,
        }
    }

    /// Sets the flags the message should arrive with.
    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    /// Sets the `INTERNALDATE` to claim.
    pub fn with_internal_date(mut self, internal_date: DateTime<Utc>) -> Self {
        self.internal_date = Some(internal_date);
        self
    }
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// How a `STORE` should change a message's flags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagChange {
    /// `+FLAGS` — add these, leave the rest.
    Add(FlagSet),
    /// `-FLAGS` — remove these, leave the rest.
    Remove(FlagSet),
    /// `FLAGS` — these and nothing else.
    Replace(FlagSet),
}

impl FlagChange {
    /// Applies the change to a flag set, as the server would.
    pub fn apply(&self, flags: &FlagSet) -> FlagSet {
        match self {
            Self::Add(added) => flags.iter().chain(added.iter()).cloned().collect(),
            Self::Remove(removed) => flags
                .iter()
                .filter(|flag| !removed.contains(flag))
                .cloned()
                .collect(),
            Self::Replace(replacement) => replacement.clone(),
        }
    }
}

/// The flags a message carries after a `STORE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagUpdate {
    /// The message that changed.
    pub uid: Uid,
    /// Its flags now, as the server reports them.
    pub flags: FlagSet,
    /// The modification sequence the change landed at, under CONDSTORE.
    pub mod_seq: Option<ModSeq>,
}

/// Where a copied or moved message ended up.
///
/// Only knowable when the server speaks UIDPLUS; without it the destination
/// UID has to be found by searching, which is why
/// [`Capability::UidPlus`](super::Capability::UidPlus) is worth gating on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UidMapping {
    /// The UID in the source mailbox.
    pub source: Uid,
    /// The UID in the destination mailbox.
    pub destination: Uid,
    /// The destination mailbox's UID generation.
    pub uid_validity: UidValidity,
}

impl UidMapping {
    /// The backend-neutral identity of the message where it landed (#543):
    /// what a row that came into being through an append or move stores as
    /// its `remote_id`, spelled by the adapter, never by the caller.
    pub fn destination_remote_id(&self) -> postio_model::RemoteId {
        super::identity::remote_id(self.uid_validity, self.destination)
    }
}

// ---------------------------------------------------------------------------
// Push
// ---------------------------------------------------------------------------

/// Something the server pushed while we were idling.
///
/// Deliberately raw. IDLE tells us *that* a mailbox changed, not what it now
/// contains; the answer is to run a QRESYNC pull, not to apply these as a diff.
/// Note that `EXPUNGE` carries a sequence number, not a UID — only a
/// QRESYNC-capable server sends `VANISHED` with UIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MailboxEvent {
    /// `* n EXISTS` — the mailbox now holds `count` messages.
    Exists {
        /// The new message count.
        count: u32,
    },
    /// `* n EXPUNGE` — a message left, identified by sequence number.
    Expunged {
        /// The sequence number that went away.
        seq: u32,
    },
    /// `* VANISHED …` — messages left, identified by UID.
    Vanished {
        /// The UIDs that went away.
        uids: Vec<Uid>,
    },
    /// `* n FETCH (FLAGS …)` — flags changed on a message.
    FlagsChanged {
        /// The message, when the server volunteered its UID.
        uid: Option<Uid>,
        /// Its flags now.
        flags: FlagSet,
    },
}
