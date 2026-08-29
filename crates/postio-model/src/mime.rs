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
//! That promise is enforced rather than merely stated. [`mail_parser`] can
//! unwind on malformed input — #277 found an assertion in its multipart walk
//! that a 144-byte message reaches — so [`try_parse`] runs it inside
//! [`std::panic::catch_unwind`] and [`parse`] reports a contained failure as
//! an empty message. These bytes arrive from a server during sync, chosen by
//! whoever sent the mail, so the guarantee has to hold against hostile input
//! and not only against untidy input.
//!
//! Use [`try_parse`] where the difference between "nothing parsed" and
//! "nothing was there" is something a person will read: the reading pane says
//! *this message has no body* for the second, and that is a lie about the
//! first.
//!
//! **The signal is weaker than it looks, and the reason is worth knowing.**
//! The assertion #277 found is a `debug_assert!`, so it fires in this
//! workspace's dev and test profiles and in the `-Cdebug-assertions` build
//! `cargo fuzz` makes — and *not* in a release build, where it compiles out
//! and the parser returns a thin but ordinary-looking message instead. So
//! `try_parse` reports `Err` for that input under test and `Ok` in a shipped
//! binary. It is still the right seam for a real `panic!` in a future version
//! of the dependency; it is not a way to detect this particular bug in
//! production. The crash that *was* unconditional was ours — see
//! [`part_paths`].
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
    /// The bracketed identifier out of `List-Id` (RFC 2919), when the
    /// message carries one — the fact that lets the list be detected with
    /// no configuration, rather than by matching an address by hand.
    pub list_id: Option<String>,
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
    /// Whether the `text/plain` part's own `Content-Type` declared
    /// `format=flowed` (RFC 3676): its lines are soft-wrapped prose, not
    /// breaks the sender typed on purpose. `false` for a message with no
    /// text part at all, same as any other fact this parser cannot find.
    ///
    /// Recorded so a later reply or forward can tell `body.text` apart from
    /// ordinary plain text and undo the wrapping instead of showing a
    /// soft-wrapped sentence as three lines the sender never typed (#456).
    /// `postio-model` cannot depend on `postio-body` (see that crate's
    /// `outgoing` module docs), so the flag crosses the boundary as a
    /// plain `bool` — the crate that does own `format=flowed` unwrapping
    /// reads it from here.
    pub text_is_flowed: bool,
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
        message.list_id = self.list_id;
        message.from = self.from;
        message.sender = self.sender;
        message.reply_to = self.reply_to;
        message.to = self.to;
        message.cc = self.cc;
        message.bcc = self.bcc;
        message.subject = self.subject;
        message.date = self.date;
        message.body = self.body;
        message.text_is_flowed = self.text_is_flowed;
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

/// Raw bytes the parser could not make a message of at all.
///
/// Not "a message with missing fields" — that is the ordinary case and
/// [`ParsedMessage`] represents it with `None`s. This is the parser giving up
/// entirely, which today means it panicked and [`try_parse`] caught it.
///
/// Carries nothing. What went wrong is a fact about a dependency's internals,
/// not something a caller can act on differently, and the one thing a caller
/// *does* need — "do not tell the user to wait for this" — is the whole
/// message of the type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unparseable;

impl std::fmt::Display for Unparseable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the message could not be parsed")
    }
}

impl std::error::Error for Unparseable {}

/// Parses raw RFC 5322 bytes, body and attachments included.
///
/// Infallible: see the [module docs](self). A message that cannot be parsed at
/// all yields an empty [`ParsedMessage`] carrying only its `size`, which is
/// the same shape as a message with nothing in it — use [`try_parse`] when the
/// difference matters, as the reading pane's does.
pub fn parse(raw: &[u8]) -> ParsedMessage {
    try_parse(raw).unwrap_or_else(|_| empty(raw, false))
}

/// Parses only the header block, buffering no body and no attachment bytes.
///
/// This is what the initial sync uses: headers newest-first, bodies backfilled
/// later. The result's [`ParsedMessage::body_state`] is
/// [`BodyState::HeadersOnly`], so a [`Message`] built from it says so.
pub fn parse_headers(raw: &[u8]) -> ParsedMessage {
    try_parse_headers(raw).unwrap_or_else(|_| empty(raw, true))
}

/// As [`parse`], but says so when nothing could be parsed.
///
/// # Why this exists (#277)
///
/// The module has always documented [`parse`] as never panicking, and it did
/// panic: `mail-parser` 0.11.8 aborts with "Invalid part ID, could not find
/// multipart" on a malformed multipart, and the unwind came straight out of
/// here. That is not a cosmetic contract violation — this runs on bytes the
/// server handed us, during sync, before anyone has opened anything, so the
/// input is chosen by whoever sent the mail and the crash lands on arrival.
/// A message that kills the client on sight is re-delivered by every sync.
///
/// The containment is [`std::panic::catch_unwind`] rather than validation:
/// the panic is inside a dependency's own state machine, and no shape of
/// pre-check here could predict which inputs reach it. Validation would be
/// guessing; catching is a fact.
///
/// Callers that only need a message use [`parse`]. Callers that must tell
/// "nothing parsed" from "nothing was there" — the reading pane, which
/// otherwise says "this message has no body" about a parser bug — use this.
pub fn try_parse(raw: &[u8]) -> Result<ParsedMessage, Unparseable> {
    contain(|| parse_inner(raw, false))
}

/// As [`parse_headers`], but says so when nothing could be parsed.
pub fn try_parse_headers(raw: &[u8]) -> Result<ParsedMessage, Unparseable> {
    contain(|| parse_inner(raw, true))
}

/// Decodes RFC 2047 encoded words in a raw header value that never goes
/// through [`parse`] — an IMAP `ENVELOPE`'s subject or a display name,
/// which arrive as isolated field values well before any body reaches this
/// module. [`parse`] already decodes these correctly wherever they sit
/// inside a full header block; this gives the same decoding to a value
/// with no header block around it.
///
/// Whatever `mail_parser` cannot make sense of — a truncated encoded word,
/// bytes that are not valid in the charset they declare — comes back as
/// whatever raw text survived, the same "show what arrived" promise
/// [`parse`] keeps rather than an error. A completely unreadable result
/// falls back to a lossy UTF-8 decode of the original bytes.
///
/// `mail_parser`'s decoder is hostile-input code (see [`try_parse`]'s own
/// docs on `#277`), reached here on bytes an IMAP server chose, so this is
/// wrapped in [`catch_unwind`](contain) too.
pub fn decode_header_text(raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }
    // `parse_unstructured` reads until an unfolded `\n`; with none in the
    // input it falls off the end and reports `HeaderValue::Empty` rather
    // than whatever it collected, so one is appended purely as a terminator.
    let mut terminated = Vec::with_capacity(raw.len() + 1);
    terminated.extend_from_slice(raw);
    terminated.push(b'\n');

    let decoded = std::panic::catch_unwind(|| {
        mail_parser::parsers::MessageStream::new(&terminated)
            .parse_unstructured()
            .as_text()
            .map(str::to_string)
    });
    match decoded {
        Ok(Some(text)) => text,
        Ok(None) | Err(_) => String::from_utf8_lossy(raw).into_owned(),
    }
}

/// The bracketed identifier out of a `List-Id` header value — RFC 2919's
/// `"Display Name" <list-id>` — so a mailing list is recognized by its
/// stable id and not by the display name a moderator can rename at will.
/// Some senders omit the display name and send only the bracket, or omit
/// the bracket too; either way whatever is left after trimming is the id.
/// `None` once nothing is left to keep.
///
/// This is text-processing only — no `mail_parser` call, so no panic
/// surface — and is shared with `postio-imap`, whose `ENVELOPE` fetch reads
/// `List-Id` as its own isolated header value the same way it reads the
/// subject.
pub fn list_id_from_text(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let id = match (raw.find('<'), raw.rfind('>')) {
        (Some(start), Some(end)) if end > start => &raw[start + 1..end],
        _ => raw,
    };
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// Run `parse` and turn an unwind into an [`Unparseable`].
///
/// `AssertUnwindSafe` is honest here rather than a shrug: the closure borrows
/// only the input slice and returns an owned [`ParsedMessage`], so there is no
/// state that a half-finished parse could leave visibly broken. `MessageParser`
/// is constructed inside and dropped with the unwind.
///
/// The panic still reaches the process's panic hook on its way past, so a
/// contained failure is visible on stderr rather than silent. That is
/// deliberate — the message is the dependency's own text and names no mail —
/// but it means a caller that expects this should log the outcome itself, at
/// a level that says "handled".
fn contain<F>(parse: F) -> Result<ParsedMessage, Unparseable>
where
    F: FnOnce() -> ParsedMessage,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse)).map_err(|_| Unparseable)
}

/// What a caller gets when nothing could be parsed: the size, and nothing
/// claimed that was not read.
///
/// `size` is still the truth — those bytes did arrive — and everything else
/// stays at its default, which is the same "we do not know" the ordinary
/// parser produces for a field it could not fill.
fn empty(raw: &[u8], headers_only: bool) -> ParsedMessage {
    ParsedMessage {
        size: raw.len() as u64,
        body_state: if headers_only {
            BodyState::HeadersOnly
        } else {
            BodyState::Full
        },
        ..ParsedMessage::default()
    }
}

fn parse_inner(raw: &[u8], headers_only: bool) -> ParsedMessage {
    let mut message = ParsedMessage {
        size: raw.len() as u64,
        body_state: if headers_only {
            BodyState::HeadersOnly
        } else {
            BodyState::Full
        },
        ..ParsedMessage::default()
    };

    // `mail_parser` can unwind, and this function promises it does not (#277).
    //
    // A malformed multipart reaches a `debug_assert!` in `mail-parser`'s part
    // walk — "Invalid part ID, could not find multipart", `message.rs:485` in
    // 0.11.8 — and before this guard the unwind went straight out of `parse`.
    // These bytes come off the socket during sync, before anyone opens
    // anything, so the input is chosen by whoever sent the mail. A parser
    // documented infallible has to be infallible against that, and against
    // whatever the next such bug in a dependency turns out to be.
    //
    // `AssertUnwindSafe` is honest here rather than a shrug: nothing crosses
    // the boundary. The parser is constructed inside the closure, `raw` is a
    // shared slice this function does not mutate, and on the unwind path
    // every partial result is dropped and `message` is returned exactly as it
    // was built above — a true `size`, an empty everything else, which is the
    // "yields whatever could be recovered" the module docs promise.
    //
    // The containment used to be here, around the parser call alone. It moved
    // out to `contain`, which wraps this whole function: catching here means
    // `parse_inner` always returns a value, so `try_parse` can only ever say
    // `Ok` and the reading pane loses the one signal that lets it say "this
    // did not parse" rather than "this has no body". Wrapping the outside
    // also covers the mapping below, not just the dependency.
    //
    // Still deliberately silent about the failure at this layer: logging it
    // would mean a `tracing` dependency on the crate the whole workspace waits
    // on to compile, which CLAUDE.md guards for the reason ADR 0004 Q1 and
    // ADR 0007 give. The callers that care log it — see `postio-sync`'s
    // backfill and `postio-app`'s reading pane.
    let parser = MessageParser::default();
    let parsed = if headers_only {
        parser.parse_headers(raw)
    } else {
        parser.parse(raw)
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
    message.list_id = addresses(source.list_id().as_address())
        .into_iter()
        .next()
        .and_then(|address| list_id_from_text(&address.address));

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

    let text_part = source.text_bodies().find_map(|part| match &part.body {
        PartType::Text(text) => Some((part, text.to_string())),
        _ => None,
    });
    message.text_is_flowed = text_part.as_ref().is_some_and(|(part, _)| {
        part.content_type()
            .and_then(|content_type| content_type.attribute("format"))
            .is_some_and(|format| format.eq_ignore_ascii_case("flowed"))
    });
    message.body = MessageBody {
        text: text_part.map(|(_, text)| text),
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

/// The decoded bytes of one MIME entity — a header block, a blank line, and a
/// body — with any transfer encoding undone.
///
/// # What this is for
///
/// ADR 0017's payload axis. `BODY[2.1]` returns a part's *encoded* bytes and
/// none of its headers, so nothing in the response says whether they are
/// base64. `BODYSTRUCTURE` reported the type and the encoding at header-sync
/// time and they are kept on the row
/// ([`Attachment::part_headers`](crate::Attachment::part_headers)), so
/// prepending them rebuilds an entity this can decode — and an attachment's
/// bytes are content-addressed on the *decoded* form, which is what makes two
/// messages carrying the same file share one blob.
///
/// [`parse`] is the wrong tool for this: it answers about a *message* — its
/// text bodies, its attachments, its headers — and a lone payload part is
/// none of those. This answers the one question a payload fetch has.
///
/// `None` when the bytes are not an entity at all, or carry no body. Same
/// contract as [`try_parse`]: hostile input yields an answer rather than a
/// panic, and never a fragment the caller could mistake for a file.
pub fn decode_entity(raw: &[u8]) -> Option<Vec<u8>> {
    // Contained for the reason [`parse`] is contained (#277): these bytes came
    // off a socket and were chosen by whoever sent the mail.
    //
    // `AssertUnwindSafe` is honest here for the same reason too — the parser
    // is built inside the closure, `raw` is a shared slice nothing mutates,
    // and on the unwind path every partial result is dropped.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let parsed = MessageParser::default().parse(raw)?;
        // Part zero is the entity's root. A payload is a leaf by
        // construction: `BODYSTRUCTURE` named a section, and a section is one
        // part.
        let contents = parsed.parts.first()?.contents();
        (!contents.is_empty()).then(|| contents.to_vec())
    }))
    .ok()
    .flatten()
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
/// # Why this walks with an explicit stack (#277)
///
/// It recursed, once per level of nesting, over a tree built entirely from the
/// message. Nesting is free for a sender — `multipart/mixed` inside
/// `multipart/mixed`, as deep as they care to type — so a crafted message
/// overflowed the stack here.
///
/// That was the serious half of #277, and the half with no mitigations. The
/// `mail_parser` unwind this module contains is a `debug_assert!` and so is
/// absent from a release build; this was unconditional. And a stack overflow
/// is a `SIGSEGV` rather than an unwind, so the [`catch_unwind`](try_parse)
/// that makes [`parse`] infallible cannot touch it, and no caller can.
///
/// An explicit worklist rather than a depth limit, because a limit is a number
/// somebody has to be right about — too low and a legitimately baroque
/// forwarded thread loses its attachments, too high and the crash is still
/// reachable. Iteration has no such number. The heap it uses is bounded by a
/// message that is already in memory.
///
/// Found by the `parse_message` fuzz target from #147, which could only reach
/// it once the unwind stopped firing first on the same inputs.
fn part_paths(source: &MpMessage<'_>) -> HashMap<MessagePartId, String> {
    let mut paths = HashMap::new();
    // (part, the path of that part). The root has no path of its own: its
    // children are numbered from 1, which is what IMAP calls them.
    let mut pending: Vec<(MessagePartId, Option<String>)> = vec![(0, None)];

    while let Some((id, prefix)) = pending.pop() {
        let Some(part) = source.parts.get(id as usize) else {
            continue;
        };
        match &part.body {
            PartType::Multipart(children) => {
                for (index, child) in children.iter().enumerate() {
                    let path = match prefix.as_deref() {
                        Some(prefix) => format!("{prefix}.{}", index + 1),
                        None => (index + 1).to_string(),
                    };
                    pending.push((*child, Some(path)));
                }
            }
            _ => {
                paths.insert(id, prefix.unwrap_or_else(|| "1".to_owned()));
            }
        }
    }
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
