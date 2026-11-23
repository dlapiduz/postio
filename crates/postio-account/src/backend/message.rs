//! Protocol-neutral results; they stop short of `postio_model::Message` because a backend does not
//! know local ids.

use chrono::{DateTime, Utc};
use postio_model::{
    AccountId, Attachment, BodyState, Disposition, EmailAddress, FlagSet, Mailbox, MailboxId,
    MailboxRole, Message, MessageId, ModSeq, RfcMessageId, Uid, UidValidity,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxFilter {
    /// The IMAP list pattern. `*` is every mailbox at every depth.
    pub pattern: String,
    pub subscribed_only: bool,
}

impl MailboxFilter {
    pub fn all() -> Self {
        Self {
            pattern: "*".to_owned(),
            subscribed_only: false,
        }
    }

    pub fn subscribed() -> Self {
        Self {
            subscribed_only: true,
            ..Self::all()
        }
    }
}

impl Default for MailboxFilter {
    fn default() -> Self {
        Self::all()
    }
}

/// `role` is resolved at the edge from `SPECIAL-USE` attributes and the name; iCloud advertises no
/// attributes for `Sent Messages` or `Deleted Messages`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxSummary {
    pub path: String,
    pub delimiter: Option<char>,
    pub attributes: Vec<String>,
    pub role: MailboxRole,
    pub selectable: bool,
    pub subscribed: bool,
}

impl MailboxSummary {
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

    /// Text, not an id: linking parents needs the whole listing, which is the repository's job.
    pub fn parent_path(&self) -> Option<String> {
        let delimiter = self.delimiter?;
        self.path
            .rsplit_once(delimiter)
            .map(|(parent, _)| parent.to_owned())
            .filter(|parent| !parent.is_empty())
    }

    pub fn depth(&self) -> usize {
        match self.delimiter {
            Some(delimiter) => self.path.matches(delimiter).count(),
            None => 0,
        }
    }

    pub fn has_children(&self) -> bool {
        self.attributes
            .iter()
            .any(|attribute| attribute.eq_ignore_ascii_case("\\HasChildren"))
    }

    /// The parent link is left unset: resolving the hierarchy is the repository's job.
    pub fn into_mailbox(self, account_id: AccountId) -> Mailbox {
        let mut mailbox = Mailbox::new(account_id, self.path, self.delimiter);
        mailbox.role = self.role;
        mailbox.selectable = self.selectable;
        mailbox.subscribed = self.subscribed;
        mailbox
    }
}

/// `ReadOnly` is `EXAMINE`, which does not set `\Recent`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

/// `uid_validity` says whether cached UIDs still mean anything; `highest_mod_seq` says how far
/// behind we are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailboxStatus {
    pub path: String,
    /// Opaque above the seam; a change invalidates every identity minted for the mailbox (#543).
    pub generation: postio_model::Generation,
    pub uid_next: Uid,
    pub exists: u32,
    pub unseen: Option<u32>,
    pub highest_mod_seq: Option<ModSeq>,
    pub permanent_flags: FlagSet,
    pub can_create_keywords: bool,
    pub read_only: bool,
}

/// `references` and `list_id` come from companion `BODY.PEEK[HEADER.FIELDS ..]` items in the same
/// fetch: not in `ENVELOPE`, and JWZ threading needs `references`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Envelope {
    pub date: Option<DateTime<Utc>>,
    pub subject: Option<String>,
    pub from: Vec<EmailAddress>,
    pub sender: Option<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub message_id: Option<RfcMessageId>,
    pub in_reply_to: Option<RfcMessageId>,
    pub references: Vec<RfcMessageId>,
    pub list_id: Option<String>,
}

/// Described by `BODYSTRUCTURE`, so names and sizes are known before any body byte downloads.
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

    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    pub fn with_content_id(mut self, content_id: impl Into<String>) -> Self {
        self.content_id = Some(content_id.into());
        self
    }

    pub fn with_disposition(mut self, disposition: Disposition) -> Self {
        self.disposition = disposition;
        self
    }

    pub fn with_charset(mut self, charset: impl Into<String>) -> Self {
        self.charset = Some(charset.into());
        self
    }

    pub fn with_encoding(mut self, encoding: impl Into<String>) -> Self {
        self.encoding = Some(encoding.into());
        self
    }

    pub fn section(&self) -> &str {
        &self.section
    }

    pub fn mime_type(&self) -> &str {
        &self.mime_type
    }

    pub fn charset(&self) -> Option<&str> {
        self.charset.as_deref()
    }

    pub fn encoding(&self) -> Option<&str> {
        self.encoding.as_deref()
    }

    /// Rebuilds the MIME headers from `BODYSTRUCTURE`, since `BODY[section]` returns none (ADR
    /// 0017).
    /// Writes only what was declared; RFC 2045 defaults are the parser's to apply.
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

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn filename(&self) -> Option<&str> {
        self.filename.as_deref()
    }

    pub fn content_id(&self) -> Option<&str> {
        self.content_id.as_deref()
    }

    pub fn disposition(&self) -> &Disposition {
        &self.disposition
    }

    /// A `text/plain` or `text/html` part with no filename and no `Content-ID`. `inline`
    /// disposition does not disqualify it: senders put it on the message itself (#751).
    pub fn is_body_text(&self) -> bool {
        matches!(self.mime_type.as_str(), "text/plain" | "text/html")
            && self.filename.is_none()
            && self.content_id.is_none()
    }

    pub fn to_attachment(&self, message_id: MessageId) -> Attachment {
        let mut attachment = Attachment::new(message_id, self.mime_type.clone(), self.size);
        attachment.filename = self.filename.clone();
        // Bare: angle brackets of the header value dropped so ids compare against a `cid:` URL
        // (#751).
        attachment.set_content_id(self.content_id.as_deref());
        attachment.disposition = self.disposition.clone();
        attachment.part_id = Some(self.section.clone());
        // Stored now: only here are `BODYSTRUCTURE` and `Attachment` both in hand, else it costs a
        // `[section.MIME]` round trip per part (ADR 0017).
        attachment.part_headers = Some(self.mime_headers());
        attachment
    }
}

/// Flat: consumers need only the body part and the downloadable parts; section strings keep the
/// structure.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyStructure {
    /// The root content type: `multipart/*`, or the single part's own type.
    content_type: String,
    parts: Vec<PartNode>,
}

impl BodyStructure {
    pub fn from_parts<I: IntoIterator<Item = PartNode>>(
        content_type: impl Into<String>,
        parts: I,
    ) -> Self {
        Self {
            content_type: content_type.into().to_ascii_lowercase(),
            parts: parts.into_iter().collect(),
        }
    }

    pub fn content_type(&self) -> &str {
        &self.content_type
    }

    pub fn parts(&self) -> &[PartNode] {
        &self.parts
    }

    pub fn text_part(&self) -> Option<&PartNode> {
        self.parts
            .iter()
            .find(|part| part.mime_type() == "text/plain" && part.is_body_text())
    }

    pub fn html_part(&self) -> Option<&PartNode> {
        self.parts
            .iter()
            .find(|part| part.mime_type() == "text/html" && part.is_body_text())
    }

    pub fn attachments(&self) -> impl Iterator<Item = &PartNode> {
        self.parts.iter().filter(|part| !part.is_body_text())
    }

    pub fn to_attachments(&self, message_id: MessageId) -> Vec<Attachment> {
        self.attachments()
            .map(|part| part.to_attachment(message_id))
            .collect()
    }
}

/// Headers only, so a mailbox can be listed, threaded and searched before any body exists locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedMessage {
    /// Only when the backend guarantees it; ordinary IMAP leaves this absent.
    pub content_identity: Option<postio_model::ContentIdentity>,
    /// What the row stores as `remote_id`, filled by the adapter that fetched it (#543, #544).
    pub remote_id: postio_model::RemoteId,
    pub uid: Uid,
    pub uid_validity: UidValidity,
    pub mod_seq: Option<ModSeq>,
    pub flags: FlagSet,
    pub internal_date: DateTime<Utc>,
    pub size: u64,
    pub envelope: Option<Envelope>,
    pub structure: Option<BodyStructure>,
    /// Only when the fetch asked: `fetch_headers_for_filing` does, `fetch_headers` does not. `None`
    /// means not asked, not known.
    pub promoted: Option<postio_model::promoted::PromotedHeaders>,
}

impl FetchedMessage {
    /// `\Recent` is dropped: it is a per-session server signal and must not reach the database.
    pub fn into_message(self, account_id: AccountId, mailbox_id: MailboxId) -> Message {
        let mut message = Message::new(account_id, mailbox_id, self.internal_date);

        message.flags = self.flags.persistable();
        message.size = self.size;
        message.promoted = self.promoted;
        message.server.uid = Some(self.uid);
        message.server.uid_validity = Some(self.uid_validity);
        message.server.mod_seq = self.mod_seq;
        message.server.remote_id = Some(self.remote_id);
        message.server.content_identity = self.content_identity;

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
            // Only here are `BODYSTRUCTURE` and `Message` both in hand; the backfill names these
            // sections instead of fetching the whole message (ADR 0017).
            message.text_part_id = structure.text_part().map(|part| part.section().to_owned());
            message.text_part_headers = structure.text_part().map(PartNode::mime_headers);
            message.html_part_id = structure.html_part().map(|part| part.section().to_owned());
            message.html_part_headers = structure.html_part().map(PartNode::mime_headers);
            message.attachments = structure.to_attachments(MessageId::UNASSIGNED);
        }

        message
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyPart {
    Whole,
    Headers,
    Text,
    Section(String),
}

impl BodyPart {
    pub fn section(section: impl Into<String>) -> Self {
        Self::Section(section.into())
    }

    pub fn section_spec(&self) -> String {
        match self {
            Self::Whole => String::new(),
            Self::Headers => "HEADER".to_owned(),
            Self::Text => "TEXT".to_owned(),
            Self::Section(section) => section.clone(),
        }
    }
}

/// The bytes went to the caller's `BodySink` as they arrived, keeping a 40 MB attachment off the
/// heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedBody {
    pub remote_id: postio_model::RemoteId,
    pub part: BodyPart,
    pub bytes_written: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendMessage {
    pub raw: Vec<u8>,
    pub flags: FlagSet,
    /// The `INTERNALDATE` to claim; the server's clock is used when absent.
    pub internal_date: Option<DateTime<Utc>>,
}

impl AppendMessage {
    pub fn new(raw: Vec<u8>) -> Self {
        Self {
            raw,
            flags: FlagSet::new(),
            internal_date: None,
        }
    }

    pub fn with_flags(mut self, flags: FlagSet) -> Self {
        self.flags = flags;
        self
    }

    pub fn with_internal_date(mut self, internal_date: DateTime<Utc>) -> Self {
        self.internal_date = Some(internal_date);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagChange {
    Add(FlagSet),
    Remove(FlagSet),
    Replace(FlagSet),
}

impl FlagChange {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagUpdate {
    pub remote_id: postio_model::RemoteId,
    pub flags: FlagSet,
    pub mod_seq: Option<ModSeq>,
}

/// Only known under UIDPLUS; otherwise the destination UID has to be found by searching.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UidMapping {
    pub source: Uid,
    pub destination: Uid,
    pub uid_validity: UidValidity,
    pub destination_remote_id: postio_model::RemoteId,
}

impl UidMapping {
    /// Filled by the adapter, never spelled by the caller (#543).
    pub fn destination_remote_id(&self) -> postio_model::RemoteId {
        self.destination_remote_id.clone()
    }
}

/// Raw: IDLE says that a mailbox changed, not what it contains; run a QRESYNC pull rather than
/// apply these as a diff.
/// `EXPUNGE` carries a sequence number; only QRESYNC servers send `VANISHED` with UIDs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MailboxEvent {
    Exists { count: u32 },
    Expunged { seq: u32 },
    Vanished { uids: Vec<Uid> },
    FlagsChanged { uid: Option<Uid>, flags: FlagSet },
}

/// Runs in every backend before discovery sees the listing, so discovery never re-derives roles
/// from names and hands a look-alike its role back (#943).
pub fn resolve_roles(mailboxes: &mut [MailboxSummary]) {
    resolve_roles_with_known_names(mailboxes, &BTreeMap::new());
}

/// `known_names` settles a tie the alphabet would decide (#959). Order: the server's `SPECIAL-USE`
/// wins; then the provider's own name (iCloud advertises none and has look-alikes like `Sent`/`Sent
/// Messages`, #501, #943); then the shallowest name match, ties alphabetical.
/// `Inbox` is exempt; a user's `[mailboxes]` mapping outranks all of this and is applied afterward.
pub fn resolve_roles_with_known_names(
    mailboxes: &mut [MailboxSummary],
    known_names: &BTreeMap<MailboxRole, String>,
) {
    let mut winners: BTreeMap<MailboxRole, usize> = BTreeMap::new();

    for (index, mailbox) in mailboxes.iter().enumerate() {
        let role = mailbox.role;
        if !role.is_special() || role == MailboxRole::Inbox {
            continue;
        }
        match winners.get(&role) {
            None => {
                winners.insert(role, index);
            }
            Some(&held) => {
                if beats(mailbox, &mailboxes[held], known_names) {
                    winners.insert(role, index);
                }
            }
        }
    }

    for (index, mailbox) in mailboxes.iter_mut().enumerate() {
        let role = mailbox.role;
        if !role.is_special() || role == MailboxRole::Inbox {
            continue;
        }
        if winners.get(&role) != Some(&index) {
            mailbox.role = MailboxRole::Regular;
        }
    }

    // Logged so a provider name that matched nothing is findable when the wrong folder still won
    // (#959).
    for (role, name) in known_names {
        let matched = mailboxes
            .iter()
            .any(|mailbox| mailbox.role == *role && &mailbox.path == name);
        if !matched {
            tracing::debug!(
                role = ?role,
                expected = %name,
                "the provider's own name for this role was not in the listing; \
                 falling back to the shallowest name match"
            );
        }
    }
}

fn beats(
    candidate: &MailboxSummary,
    held: &MailboxSummary,
    known_names: &BTreeMap<MailboxRole, String>,
) -> bool {
    let candidate_declared = declares_role(candidate);
    let held_declared = declares_role(held);
    if candidate_declared != held_declared {
        return candidate_declared;
    }
    if !candidate_declared {
        let candidate_known = matches_known_name(candidate, known_names);
        let held_known = matches_known_name(held, known_names);
        if candidate_known != held_known {
            return candidate_known;
        }
    }
    match candidate.depth().cmp(&held.depth()) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => candidate.path < held.path,
    }
}

fn matches_known_name(
    mailbox: &MailboxSummary,
    known_names: &BTreeMap<MailboxRole, String>,
) -> bool {
    known_names.get(&mailbox.role) == Some(&mailbox.path)
}

fn declares_role(mailbox: &MailboxSummary) -> bool {
    mailbox
        .attributes
        .iter()
        .any(|attribute| MailboxRole::from_special_use(attribute).is_some())
}
