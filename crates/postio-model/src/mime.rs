//! Turning raw RFC 5322 bytes into the domain model.
//!
//! Everything downstream — reading, search indexing, threading, attachments —
//! starts here, and the input is not under our control: real mail arrives with
//! mislabelled charsets, unterminated encoded words, bare LF line endings and
//! multiparts that were truncated in transit. So [`parse`] is **infallible**. It
//! never returns an error and never panics; a message it cannot understand
//! yields whatever could be recovered, and the fields it could not fill stay
//! `None`. A mail client that refuses to show a broken message is worse than one
//! that shows what arrived.
//!
//! # Why this lives in `postio-model`
//!
//! CLAUDE.md keeps this crate free of storage and protocol dependencies, and
//! [`mail_parser`] is neither: it is a pure format parser with no I/O, no
//! sockets and no state. Putting the mapping here is what lets `postio-imap`
//! (which must not depend on `postio-storage`) turn a `FETCH` response into a
//! [`Message`] — see the architecture diagram in CLAUDE.md.
//!
//! # What it does not do
//!
//! Nothing here touches the blob store. [`ParsedPart::content`] hands back the
//! decoded bytes and the caller decides where they go; the raw message itself is
//! the caller's to keep, because it is the caller who knows whether this is a
//! background backfill or a message being opened. [`parse_headers`] exists for
//! exactly that split: the sync engine fetches headers newest-first and
//! backfills bodies later, and parsing headers buffers no attachment payloads.
//!
//! ```
//! use postio_model::{mime, AccountId, MailboxId};
//! use chrono::Utc;
//!
//! let raw = b"From: Ada <ada@example.com>\r\nSubject: Hello\r\n\r\nBody text\r\n";
//! let parsed = mime::parse(raw);
//!
//! assert_eq!(parsed.subject.as_deref(), Some("Hello"));
//! let message = parsed.into_message(AccountId::new(1), MailboxId::new(1), Utc::now());
//! assert_eq!(message.body.text.as_deref(), Some("Body text\r\n"));
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use mail_parser::{
    Address as MpAddress, HeaderValue, Message as MpMessage, MessageParser, MessagePart,
    MessagePartId, MimeHeaders, PartType,
};

use crate::address::EmailAddress;
use crate::attachment::{Attachment, Disposition};
use crate::headers::Headers;
use crate::ids::{AccountId, MailboxId, MessageId, RfcMessageId};
use crate::message::{BodyState, Message, MessageBody};

/// How many characters of body text a [`ParsedMessage::preview`] keeps.
///
/// Enough to fill the second line of a message-list row at the widths the
/// design canvas uses, and short enough that storing one per message does not
/// grow the database the list has to page through.
pub const PREVIEW_CHARS: usize = 200;

/// One MIME part that is not part of the body: an attachment or an inline part.
///
/// Carries the metadata the list and search need *and* the decoded bytes, which
/// the caller is expected to move into the blob store and then drop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPart {
    /// Metadata, ready to persist. Its `blob_id` is `None`: nothing has been
    /// stored yet.
    pub attachment: Attachment,
    /// The part's decoded bytes, with any transfer encoding already undone.
    pub content: Vec<u8>,
}

/// A message that has been parsed out of raw RFC 5322 bytes.
///
/// Everything here is owned, so it outlives the buffer it was parsed from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedMessage {
    /// Every top-level header field, in wire order, unfolded, duplicates kept.
    pub headers: Headers,
    /// `Message-ID`, when the sender sent a usable one.
    pub rfc_message_id: Option<RfcMessageId>,
    /// `In-Reply-To`.
    pub in_reply_to: Option<RfcMessageId>,
    /// `References`, oldest ancestor first, unusable entries dropped.
    pub references: Vec<RfcMessageId>,
    /// `From`, with any address group flattened.
    pub from: Vec<EmailAddress>,
    /// `Sender`.
    pub sender: Option<EmailAddress>,
    /// `Reply-To`.
    pub reply_to: Vec<EmailAddress>,
    /// `To`.
    pub to: Vec<EmailAddress>,
    /// `Cc`.
    pub cc: Vec<EmailAddress>,
    /// `Bcc`.
    pub bcc: Vec<EmailAddress>,
    /// `Subject`, with RFC 2047 encoded words decoded.
    pub subject: Option<String>,
    /// The `Date` header, as claimed by the sender.
    pub date: Option<DateTime<Utc>>,
    /// The body, decoded from its charset and transfer encoding.
    pub body: MessageBody,
    /// A flattened snippet of the body for the message list.
    pub preview: Option<String>,
    /// Attachments and inline parts, with their bytes.
    pub parts: Vec<ParsedPart>,
    /// Length of the raw message in bytes.
    pub size: u64,
    /// [`BodyState::Full`] after [`parse`], [`BodyState::HeadersOnly`] after
    /// [`parse_headers`].
    pub body_state: BodyState,
    /// Whether any part declared an encoding the parser had to guess around.
    ///
    /// Not an error — the content is still there — but a signal worth logging
    /// when a user reports mojibake.
    pub encoding_problems: bool,
}

impl ParsedMessage {
    /// The metadata of every attachment and inline part, without their bytes.
    pub fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.parts.iter().map(|part| &part.attachment)
    }

    /// Builds an unpersisted [`Message`] in `mailbox_id`.
    ///
    /// `received_at` is the server's delivery time (IMAP `INTERNALDATE`), which
    /// is the list's sort key: the `Date` header is whatever the sender claimed
    /// and is kept separately.
    pub fn into_message(
        self,
        account_id: AccountId,
        mailbox_id: MailboxId,
        received_at: DateTime<Utc>,
    ) -> Message {
        let mut message = Message::new(account_id, mailbox_id, received_at);

        message.rfc_message_id = self.rfc_message_id;
        message.in_reply_to = self.in_reply_to;
        message.references = self.references;
        message.from = self.from;
        message.sender = self.sender;
        message.reply_to = self.reply_to;
        message.to = self.to;
        message.cc = self.cc;
        message.bcc = self.bcc;
        message.subject = self.subject;
        message.date = self.date;
        message.body = self.body;
        message.preview = self.preview;
        message.attachments = self
            .parts
            .into_iter()
            .map(|part| Attachment {
                message_id: MessageId::UNASSIGNED,
                ..part.attachment
            })
            .collect();
        message.size = self.size;
        message.headers = self.headers;
        message.sync.body_state = self.body_state;

        message
    }
}

/// Parses raw RFC 5322 bytes, body and attachments included.
///
/// Infallible: see the [module docs](self).
pub fn parse(raw: &[u8]) -> ParsedMessage {
    parse_inner(raw, false)
}

/// Parses only the header block, buffering no body and no attachment bytes.
///
/// This is what the initial sync uses: headers newest-first, bodies backfilled
/// later. The result's [`ParsedMessage::body_state`] is
/// [`BodyState::HeadersOnly`], so a [`Message`] built from it says so.
pub fn parse_headers(raw: &[u8]) -> ParsedMessage {
    parse_inner(raw, true)
}

fn parse_inner(raw: &[u8], headers_only: bool) -> ParsedMessage {
    let parser = MessageParser::default();
    let parsed = if headers_only {
        parser.parse_headers(raw)
    } else {
        parser.parse(raw)
    };

    let mut message = ParsedMessage {
        size: raw.len() as u64,
        body_state: if headers_only {
            BodyState::HeadersOnly
        } else {
            BodyState::Full
        },
        ..ParsedMessage::default()
    };

    // `None` means not even a header block could be found — an empty buffer, or
    // bytes that are not mail at all. Everything stays empty; the size is still
    // true and the caller still gets a row it can show.
    let Some(source) = parsed else {
        return message;
    };

    message.headers = collect_headers(&source);
    message.rfc_message_id = source.message_id().and_then(message_id);
    message.in_reply_to = message_ids(source.in_reply_to()).pop();
    message.references = message_ids(source.references());

    message.from = addresses(source.from());
    message.sender = addresses(source.sender()).into_iter().next();
    message.reply_to = addresses(source.reply_to());
    message.to = addresses(source.to());
    message.cc = addresses(source.cc());
    message.bcc = addresses(source.bcc());

    message.subject = source
        .subject()
        .map(str::trim)
        .filter(|subject| !subject.is_empty())
        .map(ToOwned::to_owned);
    message.date = source
        .date()
        .filter(|date| date.is_valid())
        .and_then(|date| DateTime::from_timestamp(date.to_timestamp(), 0));

    if headers_only {
        return message;
    }

    message.body = MessageBody {
        text: source.text_bodies().find_map(|part| match &part.body {
            PartType::Text(text) => Some(text.to_string()),
            _ => None,
        }),
        html: source.html_bodies().find_map(|part| match &part.body {
            PartType::Html(html) => Some(html.to_string()),
            _ => None,
        }),
    };
    // Falls back to the parser's HTML-to-text rendering when there is no
    // text/plain part: an HTML-only message still needs a list snippet, even
    // though a converted body is not good enough to *store* as the text body.
    message.preview = message
        .body
        .text
        .as_deref()
        .map(std::borrow::Cow::Borrowed)
        .or_else(|| source.body_text(0))
        .as_deref()
        .and_then(preview);

    let paths = part_paths(&source);
    message.parts = source
        .attachments
        .iter()
        .filter_map(|id| {
            let part = source.parts.get(*id as usize)?;
            Some(parsed_part(*id, part, &paths))
        })
        .collect();
    message.encoding_problems = source.parts.iter().any(|part| part.is_encoding_problem);

    message
}

/// Every top-level header, unfolded, in wire order.
///
/// The raw text is kept rather than the parser's interpretation: this block is
/// what the reader shows under "view source" and what a later Postio reparses
/// when it learns to understand a header this one ignores.
fn collect_headers(source: &MpMessage<'_>) -> Headers {
    source
        .headers_raw()
        .map(|(name, value)| crate::headers::Header::new(name, unfold(value)))
        .collect()
}

/// Joins a folded header value back into one line.
///
/// RFC 5322 folding is a CRLF followed by whitespace, and the whitespace is
/// part of the value; collapsing each fold to a single space is what every
/// display of a `Received` chain or a long `Subject` wants.
fn unfold(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_fold = false;
    for character in value.chars() {
        match character {
            '\r' | '\n' => in_fold = true,
            ' ' | '\t' if in_fold => {}
            _ => {
                if in_fold {
                    out.push(' ');
                    in_fold = false;
                }
                out.push(character);
            }
        }
    }
    out.trim().to_owned()
}

/// Normalizes one `Message-ID`, rejecting the spellings that are not one.
///
/// The corpus has a `References` header carrying an empty `<>`, an unterminated
/// angle-addr and a bare token; none of those identify a message, and letting
/// them through would give the threading pass edges that can only ever match
/// other garbage.
fn message_id(raw: &str) -> Option<RfcMessageId> {
    let candidate = RfcMessageId::new(raw);
    let inner = candidate.without_brackets();
    let usable = !inner.is_empty()
        && inner.contains('@')
        && !inner.contains(char::is_whitespace)
        && !inner.contains(['<', '>']);
    usable.then_some(candidate)
}

fn message_ids(value: &HeaderValue<'_>) -> Vec<RfcMessageId> {
    match value {
        HeaderValue::Text(text) => message_id(text).into_iter().collect(),
        HeaderValue::TextList(list) => list.iter().filter_map(|text| message_id(text)).collect(),
        _ => Vec::new(),
    }
}

/// Flattens an address header into the model's list of addresses.
///
/// Groups (`Team: a@example.com, b@example.com;`) are flattened to their
/// members: Postio shows and replies to people, and the group name is still in
/// the raw header block for anyone who wants it. An entry with a display name
/// but no addr-spec is dropped — there is nothing there to send to.
fn addresses(value: Option<&MpAddress<'_>>) -> Vec<EmailAddress> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .iter()
        .filter_map(|address| {
            let addr = address.address()?.trim();
            if addr.is_empty() {
                return None;
            }
            let name = address
                .name()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToOwned::to_owned);
            Some(EmailAddress::new(name, addr))
        })
        .collect()
}

/// Flattens body text into a single-line snippet of at most [`PREVIEW_CHARS`].
fn preview(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut truncated = false;
    for word in text.split_whitespace() {
        let separator = usize::from(!out.is_empty());
        if out.chars().count() + separator + word.chars().count() > PREVIEW_CHARS {
            truncated = true;
            break;
        }
        if separator == 1 {
            out.push(' ');
        }
        out.push_str(word);
    }
    if out.is_empty() {
        return None;
    }
    if truncated {
        // Trim back far enough that the ellipsis still fits the budget.
        while out.chars().count() + 1 > PREVIEW_CHARS {
            out.pop();
        }
        out.push('\u{2026}');
    }
    Some(out)
}

/// Maps every leaf part to its MIME part path (`2.1`), the way IMAP numbers
/// body sections.
///
/// This is what a lazy fetch of one attachment asks the server for, so it has
/// to be the server's numbering and not the parser's flat part index: children
/// of a multipart are numbered from 1, and a nested multipart extends the path
/// with a dot.
fn part_paths(source: &MpMessage<'_>) -> HashMap<MessagePartId, String> {
    fn walk(
        source: &MpMessage<'_>,
        id: MessagePartId,
        prefix: Option<&str>,
        out: &mut HashMap<MessagePartId, String>,
    ) {
        let Some(part) = source.parts.get(id as usize) else {
            return;
        };
        match &part.body {
            PartType::Multipart(children) => {
                for (index, child) in children.iter().enumerate() {
                    let path = match prefix {
                        Some(prefix) => format!("{prefix}.{}", index + 1),
                        None => (index + 1).to_string(),
                    };
                    walk(source, *child, Some(&path), out);
                }
            }
            _ => {
                out.insert(id, prefix.unwrap_or("1").to_owned());
            }
        }
    }

    let mut paths = HashMap::new();
    walk(source, 0, None, &mut paths);
    paths
}

fn parsed_part(
    id: MessagePartId,
    part: &MessagePart<'_>,
    paths: &HashMap<MessagePartId, String>,
) -> ParsedPart {
    let content = part.contents().to_vec();

    let mut attachment =
        Attachment::new(MessageId::UNASSIGNED, mime_type(part), content.len() as u64);
    attachment.filename = part
        .attachment_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned);
    attachment.content_id = part.content_id().map(|id| {
        // Stored bare, so it compares directly against the `cid:` URL in an
        // HTML body rather than after stripping brackets at every use.
        id.trim()
            .trim_start_matches('<')
            .trim_end_matches('>')
            .to_owned()
    });
    attachment.disposition = disposition(part);
    attachment.part_id = paths.get(&id).cloned();

    ParsedPart {
        attachment,
        content,
    }
}

/// The part's media type, lowercased, with RFC 2045's defaults filled in.
fn mime_type(part: &MessagePart<'_>) -> String {
    if let Some(content_type) = part.content_type() {
        let ctype = content_type.ctype().to_ascii_lowercase();
        return match content_type.subtype() {
            Some(subtype) => format!("{ctype}/{}", subtype.to_ascii_lowercase()),
            None => ctype,
        };
    }
    match &part.body {
        PartType::Text(_) => "text/plain".to_owned(),
        PartType::Html(_) => "text/html".to_owned(),
        PartType::Message(_) => "message/rfc822".to_owned(),
        _ => "application/octet-stream".to_owned(),
    }
}

/// How the sender said the part should be presented.
///
/// With no `Content-Disposition` at all, a part that carries a `Content-ID` is
/// almost always an image the HTML refers to, so it is treated as inline;
/// anything else is an attachment, which is the conservative choice because it
/// stays visible in the attachment list either way.
fn disposition(part: &MessagePart<'_>) -> Disposition {
    match part.content_disposition() {
        Some(content_disposition) => {
            let raw = content_disposition.ctype();
            match raw.to_ascii_lowercase().as_str() {
                "inline" => Disposition::Inline,
                "attachment" => Disposition::Attachment,
                _ => Disposition::Other(raw.to_owned()),
            }
        }
        None if part.content_id().is_some() => Disposition::Inline,
        None => match &part.body {
            PartType::InlineBinary(_) => Disposition::Inline,
            _ => Disposition::Attachment,
        },
    }
}
