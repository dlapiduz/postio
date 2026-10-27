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
//! **The containment is defence, not a known bug.** The assertion #277 found
//! was in `mail-parser` 0.11 and is gone in 1.0 (the 144-byte reproducer now
//! parses and recovers its parts), so no input in this repository makes
//! `try_parse` report `Err` any more; `contain` is tested with a closure that
//! panics instead. It stays because the parser is hostile-input code that a
//! sender drives, and the next such bug should cost one message rather than
//! the sync. The crash that *was* unconditional was ours — see
//! [`part_paths`].
//!
//! # Why this lives in `postio-model`
//!
//! CLAUDE.md keeps this crate free of storage and protocol dependencies, and
//! [`mail_parser`] is neither: it is a pure format parser with no I/O, no
//! sockets and no state. Putting the mapping here is what lets `postio-account`
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
    AddressList, Encoding, HeaderForm, Message as MpMessage, MessageParser, MessagePart, PartId,
    PartKind, PartRole, TextList,
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
    // POSTIO-CONSENT: detection only, never sent — see `Message::read_receipt_requested`.
    /// Whether `Disposition-Notification-To` or `Return-Receipt-To` is
    /// present (#970) — Postio never sends one automatically, so this only
    /// counts how often it was asked.
    pub read_receipt_requested: bool,
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
        message.read_receipt_requested = self.read_receipt_requested;
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
        // What the three promoted headers say is known from any header block
        // (spec 007, research R8) -- but not from bytes that had none, which
        // say nothing either way.
        message.promoted = (!self.headers.is_empty())
            .then(|| crate::promoted::PromotedHeaders::from_headers(&self.headers));
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

/// Which reading of a message [`parse`] produces — bumped whenever a change
/// here makes it yield something different for bytes it already saw.
///
/// A body is fetched once and its raw bytes are not kept (ADR 0020), so a
/// parser fix cannot reach a stored body by re-parsing it. The store stamps
/// every body with the version that wrote it (`messages.body_parsed_with`),
/// and the backfill fetches again the rows an older version got wrong: the
/// ones that carried the decode caveat, an empty body from a failed decode
/// among them. Bumping this is what makes that happen; not bumping it after
/// a fix is how three empty messages stayed empty on a real account
/// (2026-09-14).
///
/// 1: the lenient quoted-printable reading.
///
/// 2: `mail-parser` 1.0, which reads around a damaged transfer encoding
/// (base64 outside its alphabet, `==` in quoted-printable, a final unpadded
/// quantum) instead of handing the part back encoded. Rows that carried the
/// decode caveat under 1 are the ones whose words change, so they are the ones
/// fetched again.
pub const PARSER_VERSION: u32 = 2;

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
/// panic: `mail-parser` 0.11.8 aborted with "Invalid part ID, could not find
/// multipart" on a malformed multipart (fixed upstream in 1.0), and the unwind
/// came straight out of here. That is not a cosmetic contract violation — this runs on bytes the
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
    let decoded = std::panic::catch_unwind(|| {
        HeaderForm::Text
            .parse(raw)
            .value()
            .as_text()
            .map(str::to_string)
    });
    match decoded {
        Ok(Some(text)) => one_line(&text),
        Ok(None) | Err(_) => one_line(&String::from_utf8_lossy(raw)),
    }
}

/// A decoded header value as a reader may draw it: one line, every run of
/// line breaks and control characters folded to a single space.
///
/// An encoded word encodes octets, and `=0D=0A` is two of them, so a
/// decoded subject or display name can carry a line break that no
/// unfolding removes. Drawn as it arrived, `Invoice attached=0D=0ABcc:
/// mallory@…` put a forged `Bcc:` line under the subject in the reader
/// (#1770), and an escape or a NUL is no better in a terminal. Every
/// frontend sets these values in a label, so they are folded once, here,
/// for all of them. A run is folded rather than each character so that
/// `\r\n` and the space beside it read as the one gap they stand for.
///
/// What counts: C0 and C1 controls (`char::is_control`: CR, LF, TAB, ESC,
/// NUL, NEL…) and Unicode's line and paragraph separators, which a label
/// breaks on just the same. Whitespace that touches such a run joins it;
/// ordinary spaces elsewhere are left alone.
pub fn one_line(text: &str) -> String {
    let breaks = |c: char| c.is_control() || c == '\u{2028}' || c == '\u{2029}';
    if !text.chars().any(breaks) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if breaks(c) {
            // Whitespace written just before the run belongs to it.
            while out.ends_with(char::is_whitespace) {
                out.pop();
            }
            in_run = true;
        } else if in_run && c.is_whitespace() {
            // So does whitespace just after it.
        } else {
            if in_run && !out.is_empty() {
                out.push(' ');
            }
            in_run = false;
            out.push(c);
        }
    }
    out
}

/// One MIME parameter's value out of a list of undecoded `(name, value)`
/// pairs — an IMAP `BODYSTRUCTURE`'s parameter or disposition list, which a
/// server hands over exactly as the sender wrote them.
///
/// A non-ASCII filename rarely travels as a plain `filename=`: RFC 2231 puts
/// it in `filename*=UTF-8''…`, split across `filename*0*`, `filename*1*`…
/// when long, and many clients put an RFC 2047 encoded word inside a plain
/// `name=` instead. Looking only for the literal `key` found none of those,
/// and the part went nameless (#1686). `mail_parser` already decodes every
/// one of these spellings inside a `Content-Type` header, so the pairs are
/// put back into one and parsed — the same decoder [`parse`] uses, on the
/// same hostile-input terms, contained the same way.
///
/// `None` when no spelling of `key` is present.
pub fn parameter_value(pairs: &[(String, String)], key: &str) -> Option<String> {
    let is_token = |name: &str| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"*-_.".contains(&byte))
    };
    let mut header = String::from("application/octet-stream");
    for (name, value) in pairs {
        if !is_token(name) {
            continue;
        }
        let mut quoted = String::with_capacity(value.len() + 2);
        for c in value.chars().filter(|c| *c != '\r' && *c != '\n') {
            if c == '"' || c == '\\' {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        header.push_str(&format!("; {name}=\"{quoted}\""));
    }

    let parsed = std::panic::catch_unwind(|| {
        HeaderForm::ContentType
            .parse(header.as_bytes())
            .value()
            .as_content_type()
            .and_then(|content_type| {
                content_type
                    .attribute(key)
                    .map(|value| value.trim().to_owned())
            })
    });
    parsed.ok().flatten().filter(|value| !value.is_empty())
}

/// The bracketed identifier out of a `List-Id` header value — RFC 2919's
/// `"Display Name" <list-id>` — so a mailing list is recognized by its
/// stable id and not by the display name a moderator can rename at will.
/// Some senders omit the display name and send only the bracket, or omit
/// the bracket too; either way whatever is left after trimming is the id.
/// `None` once nothing is left to keep.
///
/// This is text-processing only — no `mail_parser` call, so no panic
/// surface — and is shared with `postio-account`, whose `ENVELOPE` fetch reads
/// `List-Id` as its own isolated header value the same way it reads the
/// subject.
pub fn list_id_from_text(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let id = first_angle_bracketed(raw).unwrap_or(raw);
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// The text inside the first `<...>` that is not part of a quoted string.
///
/// RFC 2919 §3 lets the phrase before the bracket be a quoted string, which
/// may hold `<` and `>`; and a value that somehow carries two fields must
/// yield the first identifier, not everything between the first `<` and the
/// last `>`.
fn first_angle_bracketed(raw: &str) -> Option<&str> {
    let mut quoted = false;
    let mut escaped = false;
    let mut start = None;
    for (index, c) in raw.char_indices() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
            continue;
        }
        match (c, start) {
            ('"', None) => quoted = true,
            ('<', None) => start = Some(index + 1),
            ('>', Some(from)) => return Some(&raw[from..index]),
            _ => {}
        }
    }
    None
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
    // on to compile, which CLAUDE.md guards for the reason ADR 0004 Q1 gives.
    // The callers that care log it — see `postio-sync`'s backfill and the
    // reading pane.
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
    message.read_receipt_requested = message.headers.contains("Disposition-Notification-To")
        || message.headers.contains("Return-Receipt-To");
    message.rfc_message_id = source.message_id().and_then(message_id);
    message.in_reply_to = message_ids(source.in_reply_to()).pop();
    message.references = message_ids(source.references());
    message.list_id = addresses(source.list_id())
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
        .map(one_line)
        .map(|subject| subject.trim().to_owned())
        .filter(|subject| !subject.is_empty());
    message.date = source
        .date()
        .filter(|date| date.is_valid())
        .and_then(|date| DateTime::from_timestamp(date.to_timestamp(), 0));

    if headers_only {
        return message;
    }

    let text_part = source
        .text_body()
        .find(|part| matches!(part.kind(), PartKind::Text));
    message.text_is_flowed = text_part.as_ref().is_some_and(|part| {
        part.content_type()
            .and_then(|content_type| content_type.attribute("format"))
            .is_some_and(|format| format.eq_ignore_ascii_case("flowed"))
    });
    let named_html_alternative = named_html_alternative(&source);
    message.body = MessageBody {
        // RFC 2046 §5.1.1: a multipart whose boundary could not be used has
        // no parts to have found a text body among, and is read as this
        // entity's own content instead. `text_is_flowed` is correctly left
        // `false` above -- the fallback part carries no `format` attribute
        // of its own to answer that from.
        text: text_part
            .and_then(|part| part.text())
            .map(std::borrow::Cow::into_owned)
            .or_else(|| multipart_boundary_fallback(&source)),
        html: source
            .html_body()
            .find(|part| matches!(part.kind(), PartKind::Html))
            .or(named_html_alternative)
            .and_then(|part| part.text())
            .map(std::borrow::Cow::into_owned),
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
        .root()
        .parts()
        // Everything offered as a file: the parser's attachments, and the
        // inline media it also shows in the body (an image a `cid:` reference
        // names), which the reader resolves through this list. A
        // `multipart/*` part is a container, not a file: RFC 2046 §5.1.1's
        // fallback above is what happens when one has to be read anyway, and
        // it must not also show up here as a nameless, typeless attachment
        // (#900).
        .filter(|part| matches!(part.role(), PartRole::Attachment | PartRole::MediaBody))
        .filter(|part| !is_multipart_type(part))
        .filter(|part| {
            named_html_alternative
                .as_ref()
                .is_none_or(|html| html.id() != part.id())
        })
        .map(|part| parsed_part(&part, &paths))
        .collect();
    // Three ways a body can arrive as something other than what was sent, and
    // the flag has to carry all of them — it is the only signal Postio has
    // that the words on screen are not the words that were sent (#901).
    message.encoding_problems =
        transfer_encoding_problem(&source) || decoded_lossily(raw, &message.body);

    message
}

/// A `text/html` alternative the parser filed as an attachment only because
/// it carries a name, when the message has no other HTML body.
///
/// RFC 8621 `parseStructure` treats a named HTML alternative as an
/// attachment and lets the plain text stand in as the HTML body; Postio
/// shows the HTML the sender wrote instead (the maintainer's call), and does
/// not offer it again as a file. A part the sender disposed as an
/// `attachment` stays one.
fn named_html_alternative<'m>(source: &'m MpMessage<'m>) -> Option<MessagePart<'m>> {
    if source
        .html_body()
        .any(|part| matches!(part.kind(), PartKind::Html))
    {
        return None;
    }
    source.parts().find(|part| {
        part.is_content_type("text", "html")
            && matches!(part.role(), PartRole::Attachment)
            && part
                .parent()
                .is_some_and(|parent| parent.is_content_type("multipart", "alternative"))
            && !part
                .content_disposition()
                .is_some_and(|disposition| disposition.is_attachment())
    })
}

/// Whether any part's transfer encoding is one this parser cannot undo, or
/// was damaged: base64 outside its alphabet, a quoted-printable escape that
/// encodes nothing.
///
/// RFC 2045 §6.4: an unknown encoding "must be treated as
/// `application/octet-stream`" -- the parser does not know what the octets
/// mean. Postio shows them verbatim as text, which beats an empty body and is
/// still a guess, so it is flagged. `x-` tokens included deliberately: a
/// private encoding is exactly the case §6.4 is written for.
///
/// Only the transfer layer: a charset mismatch is `decoded_lossily`'s.
fn transfer_encoding_problem(source: &MpMessage<'_>) -> bool {
    source.parts().any(|part| {
        if part.is_multipart() || part.is_message() {
            return false;
        }
        !part.has_known_transfer_encoding()
            || (part.encoding() != Encoding::None && !part.decoded_checked().1.is_empty())
    })
}

/// Whether decoding the charset lost octets.
///
/// The mojibake a user actually reports, and the direction of it that cannot
/// be undone: the octets are one charset, the header names another, and what
/// survives the decode is U+FFFD. Once that is in the stored body it is in
/// the search index and in every reply that quotes it — no later pass can
/// recover the bytes, which is why this is worth saying at the time.
///
/// `is_encoding_problem` cannot answer it. That flag is about the *transfer*
/// encoding — base64 outside its alphabet, a truncated part — and it is
/// false in both directions of a charset mismatch, measured on #901.
///
/// The raw bytes are the control. A sender may legitimately send U+FFFD, and
/// flagging every message containing one would make the caveat meaningless
/// on the mail that carries it on purpose; if the replacement character is in
/// the bytes that arrived, it is the sender's. A message carrying both a sent
/// U+FFFD and a lossy part reads as clean, which is the safe direction to be
/// wrong in — it never cries wolf.
fn decoded_lossily(raw: &[u8], body: &crate::MessageBody) -> bool {
    const REPLACEMENT_UTF8: &[u8] = "\u{fffd}".as_bytes();
    let decoded_has_it = [body.text.as_deref(), body.html.as_deref()]
        .into_iter()
        .flatten()
        .any(|text| text.contains('\u{fffd}'));
    decoded_has_it
        && !raw
            .windows(REPLACEMENT_UTF8.len())
            .any(|window| window == REPLACEMENT_UTF8)
}

/// Every top-level header, unfolded, in wire order.
///
/// The raw text is kept rather than the parser's interpretation: this block is
/// what the reader shows under "view source" and what a later Postio reparses
/// when it learns to understand a header this one ignores.
fn collect_headers(source: &MpMessage<'_>) -> Headers {
    source
        .headers()
        .iter()
        .map(|header| {
            crate::headers::Header::new(
                header.raw_name(),
                unfold(&String::from_utf8_lossy(header.raw_value())),
            )
        })
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
        let root = parsed.root_part();
        if root.is_multipart() {
            return None;
        }
        let contents = root.decoded();
        (!contents.is_empty()).then(|| contents.into_owned())
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

fn message_ids(value: Option<TextList<'_>>) -> Vec<RfcMessageId> {
    value
        .map(|list| list.iter().filter_map(message_id).collect())
        .unwrap_or_default()
}

/// Flattens an address header into the model's list of addresses.
///
/// Groups (`Team: a@example.com, b@example.com;`) are flattened to their
/// members: Postio shows and replies to people, and the group name is still in
/// the raw header block for anyone who wants it. An entry with a display name
/// but no addr-spec is dropped — there is nothing there to send to.
fn addresses(value: Option<AddressList<'_>>) -> Vec<EmailAddress> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .mailboxes()
        .filter_map(|address| {
            let addr = address.address()?.trim();
            if addr.is_empty() {
                return None;
            }
            let name = address
                .name()
                .map(one_line)
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty());
            Some(EmailAddress::new(name, addr))
        })
        .collect()
}

/// Drops anything that reads as an HTML tag, leaving everything else.
///
/// The preview is built from the sender's `text/plain` part, and some senders
/// put markup there -- a real list row read
///
/// ```text
/// Eventbrite <hr style="height: 1;border: none;border-top: 1px ...
/// ```
///
/// **Not a general sanitizer, and not allowed to be one.** `postio-model` may
/// not depend on `ammonia` or `html5ever` (`check-crate-boundaries.py`,
/// ADR 0004): the whole workspace waits on this crate to compile. Nothing
/// here is a security control either -- the preview is drawn as a GTK label,
/// never as markup. It is a legibility fix.
///
/// A tag is `<`, an optional `/`, an ASCII letter, then name characters, then
/// whitespace or `/` or `>`. That deliberately spares `<https://example.com>`
/// and `<ada@example.com>`, which are ordinary plain text and appear in
/// previews constantly: after `https` comes a colon, which ends no tag name.
fn without_tags(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains('<') {
        return std::borrow::Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if bytes[at] == b'<'
            && let Some(end) = tag_ends_at(rest)
        {
            at += end;
            continue;
        }
        let character = rest.chars().next().expect("in bounds");
        out.push(character);
        at += character.len_utf8();
    }
    std::borrow::Cow::Owned(out)
}

/// How many bytes the tag starting at the front of `rest` occupies, if it is
/// one.
fn tag_ends_at(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut at = 1;
    if bytes.get(at) == Some(&b'/') {
        at += 1;
    }
    if !bytes.get(at)?.is_ascii_alphabetic() {
        return None;
    }
    while bytes
        .get(at)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'-')
    {
        at += 1;
    }
    match bytes.get(at) {
        Some(b'>') => Some(at + 1),
        Some(b) if b.is_ascii_whitespace() || *b == b'/' => {
            rest[at..].find('>').map(|close| at + close + 1)
        }
        _ => None,
    }
}

/// Unwraps markdown links, so a snippet is words rather than syntax.
///
/// A great many bulk senders write their `text/plain` part by running the
/// HTML through a converter, and markdown is what those converters emit. The
/// list showed the result verbatim:
///
/// ```text
/// [ ](https://app.example.test/parent/home) 9/16/26 - 9/17/26 Overdue...
/// ```
///
/// A label and a URL where four words of the message should be. `[label](url)`
/// becomes `label`, and `[ ](url)` -- the wrapped logo at the top of every one
/// of these -- becomes nothing, which is what it was worth.
///
/// Only the link form, and only in the snippet. A preview is a handful of
/// words to recognise a message by; the body has its own rendering and its own
/// reasons. Anything that is not a well-formed link is left exactly as typed,
/// so prose containing a bracket survives it.
fn without_markdown_links(text: &str) -> std::borrow::Cow<'_, str> {
    if !text.contains("](") {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let (before, from_open) = rest.split_at(open);
        out.push_str(before);
        let Some(close) = from_open.find("](") else {
            out.push_str(from_open);
            return std::borrow::Cow::Owned(out);
        };
        let after_label = &from_open[close + 2..];
        let Some(end) = after_label.find(')') else {
            out.push_str(from_open);
            return std::borrow::Cow::Owned(out);
        };
        out.push_str(&from_open[1..close]);
        rest = &after_label[end + 1..];
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// Flattens body text into a single-line snippet of at most [`PREVIEW_CHARS`].
fn preview(text: &str) -> Option<String> {
    let text = without_tags(text);
    let text = without_markdown_links(text.as_ref());
    let text = text.as_ref();
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

/// Whether `part`'s own `Content-Type` is `multipart/*`.
fn is_multipart_type(part: &MessagePart<'_>) -> bool {
    part.content_type()
        .is_some_and(|content_type| content_type.ctype().eq_ignore_ascii_case("multipart"))
}

/// RFC 2046 §5.1.1's fallback: a multipart entity whose boundary parameter is
/// missing, empty, or unrecognisable "must be treated as text/plain".
///
/// `mail_parser` cannot split such an entity into children — there is no
/// boundary left to split on — so it leaves the whole thing as one part typed
/// `multipart/*`, holding the entity's own content as [`PartKind::Text`] or
/// [`PartKind::Binary`] rather than [`PartKind::Multipart`]'s list of
/// children. That shape can only be the message's own root: an ordinary
/// multipart's root part is always `Multipart`, holding its children —
/// so the root part is the one part this can ever be true of.
///
/// No charset decoding to do here beyond what `mail_parser` already gives:
/// RFC 2046 defines no `charset` parameter for a multipart type — it belongs
/// to `text/*` — so a [`PartKind::Binary`] root decodes as UTF-8 with
/// replacement, the same default `mail_parser` itself falls back to for a
/// `text/plain` part with none declared.
fn multipart_boundary_fallback(source: &MpMessage<'_>) -> Option<String> {
    let root = source.root_part();
    if !is_multipart_type(&root) {
        return None;
    }
    match root.kind() {
        PartKind::Text => root.text().map(std::borrow::Cow::into_owned),
        PartKind::Binary | PartKind::InlineBinary => {
            Some(String::from_utf8_lossy(&root.decoded()).into_owned())
        }
        PartKind::Multipart | PartKind::Html | PartKind::Message(_) => None,
    }
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
fn part_paths(source: &MpMessage<'_>) -> HashMap<PartId, String> {
    let mut paths = HashMap::new();
    // (part, the path of that part). The root has no path of its own: its
    // children are numbered from 1, which is what IMAP calls them.
    let mut pending: Vec<(MessagePart<'_>, Option<String>)> = vec![(source.root_part(), None)];

    while let Some((part, prefix)) = pending.pop() {
        if part.is_multipart() {
            for (index, child) in part.children().enumerate() {
                let path = match prefix.as_deref() {
                    Some(prefix) => format!("{prefix}.{}", index + 1),
                    None => (index + 1).to_string(),
                };
                pending.push((child, Some(path)));
            }
        } else {
            paths.insert(part.id(), prefix.unwrap_or_else(|| "1".to_owned()));
        }
    }
    paths
}

fn parsed_part(part: &MessagePart<'_>, paths: &HashMap<PartId, String>) -> ParsedPart {
    let content = part.decoded().into_owned();

    let mut attachment =
        Attachment::new(MessageId::UNASSIGNED, mime_type(part), content.len() as u64);
    attachment.filename = part
        .attachment_name()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned);
    // Stored bare, so it compares directly against the `cid:` URL in an HTML
    // body rather than after stripping brackets at every use. The IMAP header
    // sync reaches the same normaliser, which is what #751 needed: two ingest
    // paths, one answer.
    attachment.set_content_id(part.content_id());
    attachment.disposition = disposition(part);
    attachment.part_id = paths.get(&part.id()).cloned();

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
    match part.kind() {
        PartKind::Text => "text/plain".to_owned(),
        PartKind::Html => "text/html".to_owned(),
        PartKind::Message(_) => "message/rfc822".to_owned(),
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
        None => match part.kind() {
            PartKind::InlineBinary => Disposition::Inline,
            _ => Disposition::Attachment,
        },
    }
}

#[cfg(test)]
mod containment {
    use super::*;

    /// A panic inside the parse is reported, not propagated: the seam that
    /// keeps `parse` infallible when a future `mail-parser` unwinds.
    #[test]
    fn a_panic_inside_the_parse_is_reported_as_unparseable() {
        let outcome = contain(|| panic!("a dependency bug"));
        assert_eq!(outcome, Err(Unparseable));
    }

    /// And success passes through whole -- the control for the test above.
    #[test]
    fn a_parse_that_succeeds_comes_back_whole() {
        let outcome = contain(|| ParsedMessage {
            size: 7,
            ..ParsedMessage::default()
        });
        assert_eq!(outcome.map(|parsed| parsed.size), Ok(7));
    }
}

#[cfg(test)]
mod preview_is_not_markup {
    use super::*;

    /// A tag in the sender's text part does not reach the list (#1436).
    ///
    /// Both of these are real rows, from a real inbox.
    #[test]
    fn a_tag_in_the_text_part_never_reaches_the_list() {
        let leaked = "<hr style=\"height: 1;border: none;border-top: 1px solid #ccc\">\
                      Eventbrite Order Confirmation";
        let snippet = preview(leaked).expect("a preview");
        assert!(
            !snippet.contains('<') && !snippet.contains("border-top"),
            "markup reached the list row: {snippet:?}"
        );
        assert!(
            snippet.contains("Eventbrite"),
            "and the words survived: {snippet:?}"
        );
    }

    /// The control, and the reason this is not a blunt `<`..`>` strip.
    ///
    /// A bracketed URL or address is ordinary plain text and appears in
    /// previews constantly. Stripping it would lose the only content some
    /// rows have.
    #[test]
    fn a_bracketed_url_or_address_is_not_a_tag() {
        let cloudflare = "You can also view this email as a webpage \
                          <[[https://content.example.com/registrations]]>";
        let snippet = preview(cloudflare).expect("a preview");
        assert!(
            snippet.contains("https://content.example.com"),
            "a bracketed URL was mistaken for markup: {snippet:?}"
        );

        let reply = "On Mon, Ada Lovelace <ada@example.com> wrote:";
        let snippet = preview(reply).expect("a preview");
        assert!(
            snippet.contains("ada@example.com"),
            "a bracketed address was mistaken for markup: {snippet:?}"
        );
    }

    // The first cargo-mutants baseline (#510) left every mutation of
    // `tag_ends_at`'s arithmetic and every `&&` in `message_id` alive: no
    // test told the difference (#1470). These do. Each case is chosen so
    // that exactly one of the surviving mutants gives a different answer.

    #[test]
    fn without_tags_drops_a_closing_tag_and_only_a_tag() {
        // `bytes.get(at) == Some(&b'/')`: with `!=`, a closing tag's slash
        // is not stepped over and `</b>` survives as text.
        assert_eq!(without_tags("a</b>c"), "ac");
        // `at += 1` after the slash: `*=` leaves `at` at 1 and `-=` at 0, so
        // the name check reads the slash or the `<` and refuses the tag.
        assert_eq!(without_tags("<i>x</i>y"), "xy");
        // A `<` that starts no tag is text: a digit, a space, the end.
        assert_eq!(without_tags("3 < 4 and <3"), "3 < 4 and <3");
        assert_eq!(without_tags("a <"), "a <");
    }

    #[test]
    fn without_tags_ends_an_attributed_tag_at_its_own_close() {
        // `at + close + 1`: with `*`, the span is wrong by a factor and
        // either eats text after the tag or leaves part of the tag in.
        assert_eq!(without_tags("<p class=\"x\">hi</p>"), "hi");
        assert_eq!(without_tags("<br/>then"), "then");
        assert_eq!(without_tags("<a href=\"u\">t</a> u"), "t u");
        // An attributed tag that never closes is text, not a swallowed rest.
        assert_eq!(without_tags("<p class=\"x\" oops"), "<p class=\"x\" oops");
    }
}

#[cfg(test)]
mod message_ids {
    use super::*;

    /// Each of the three `&&` in `message_id`, loosened to `||`, lets exactly
    /// one of these through (#1470). A `Message-ID` is what threading keys
    /// on, so what is accepted as one is load-bearing well beyond parsing.
    #[test]
    fn a_message_id_needs_every_one_of_its_four_conditions() {
        assert!(message_id("<a@example.com>").is_some());
        assert!(message_id("<>").is_none(), "empty");
        assert!(message_id("<no-at-sign>").is_none(), "no @");
        assert!(message_id("<a b@example.com>").is_none(), "whitespace");
        assert!(message_id("<a@<b>>").is_none(), "a bracket inside");
        // And a bare token with an @ but no brackets is still one: the
        // brackets are the wrapper, not the identity.
        assert!(message_id("a@example.com").is_some());
    }
}

#[cfg(test)]
mod a_snippet_is_words_not_syntax {
    use super::without_markdown_links;

    /// The shape measured against a real account.
    ///
    /// A school's daily summary, whose `text/plain` part is its HTML run
    /// through a markdown converter. The list row showed the wrapped logo's
    /// empty link and the URL behind it, where the first words of the message
    /// belonged.
    #[test]
    fn a_converters_links_leave_only_their_labels() {
        assert_eq!(
            without_markdown_links("[ ](https://app.example.test/parent/home) 9/16/26 - 9/17/26"),
            "  9/16/26 - 9/17/26",
            "the wrapped logo leaves only the space it labelled itself with"
        );
        assert_eq!(
            without_markdown_links(
                "Assignment.[Bridges Practice #1](https://x.test/a/849) 3:59 pm"
            ),
            "Assignment.Bridges Practice #1 3:59 pm"
        );
        assert_eq!(
            without_markdown_links("[one](http://a.test) and [two](http://b.test)"),
            "one and two"
        );
    }

    /// And the snippet a person actually sees has neither syntax nor URL.
    ///
    /// The end of the chain: `preview` collapses whitespace after this runs,
    /// so the empty label leaves nothing behind either.
    #[test]
    fn the_snippet_reads_as_the_message_starts() {
        let snippet = super::preview(
            "[ ](https://app.example.test/parent/home) 9/16/26 - 9/17/26 Overdue Submissions",
        )
        .expect("a snippet");
        assert_eq!(snippet, "9/16/26 - 9/17/26 Overdue Submissions");
        assert!(!snippet.contains("http"), "no URL in a snippet: {snippet}");
        assert!(!snippet.contains("]("), "and no syntax: {snippet}");
    }

    /// Prose survives unchanged, brackets and all.
    ///
    /// The snippet is the only place this runs, so a false positive costs a
    /// message the words it is recognised by. Nothing that is not a
    /// well-formed link may be touched.
    #[test]
    fn prose_is_left_exactly_as_typed() {
        for prose in [
            "I wrote [sic] in the margin (twice).",
            "The array [1, 2, 3] and the tuple (a, b).",
            "An unclosed [bracket and (parens",
            "See https://example.test/agenda for the agenda.",
            "",
        ] {
            assert_eq!(without_markdown_links(prose), prose, "{prose:?}");
        }
    }
}

/// `parameter_value` reads every spelling a server hands over of a
/// `BODYSTRUCTURE` parameter (#1686).
#[cfg(test)]
mod parameter_value_tests {
    use super::parameter_value;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_plain_parameter_is_read_as_written() {
        let found = parameter_value(&pairs(&[("name", "report.pdf")]), "name");
        assert_eq!(found.as_deref(), Some("report.pdf"));
    }

    #[test]
    fn rfc_2231_continuations_are_joined_and_decoded() {
        let found = parameter_value(
            &pairs(&[
                ("filename*0*", "UTF-8''Qu%C3%A9"),
                ("filename*1*", "bec%20notes.txt"),
            ]),
            "filename",
        );
        assert_eq!(found.as_deref(), Some("Québec notes.txt"));
    }

    #[test]
    fn an_encoded_word_in_a_plain_name_is_decoded() {
        let found = parameter_value(&pairs(&[("name", "=?UTF-8?B?w6l0w6kucGRm?=")]), "name");
        assert_eq!(found.as_deref(), Some("été.pdf"));
    }

    #[test]
    fn quotes_and_backslashes_in_a_value_survive_the_round_trip() {
        let found = parameter_value(&pairs(&[("name", "a \"quoted\" \\ name")]), "name");
        assert_eq!(found.as_deref(), Some("a \"quoted\" \\ name"));
    }

    #[test]
    fn a_name_that_is_not_a_token_cannot_inject_another_parameter() {
        let found = parameter_value(
            &pairs(&[("x; name", "evil.exe"), ("charset", "utf-8")]),
            "name",
        );
        assert_eq!(found, None, "a hostile parameter name was read as `name`");
    }

    #[test]
    fn an_absent_or_empty_key_is_none() {
        assert_eq!(
            parameter_value(&pairs(&[("charset", "utf-8")]), "name"),
            None
        );
        assert_eq!(parameter_value(&pairs(&[("name", "  ")]), "name"), None);
    }
}
