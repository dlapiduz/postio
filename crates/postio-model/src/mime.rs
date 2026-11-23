//! Turns raw RFC 5322 bytes into the domain model. `parse` is infallible: real mail has bad
//! charsets, bare LF and truncated multiparts, and a client that
//! refuses a broken message is worse than one that shows what arrived. `mail_parser` can unwind on
//! hostile input (#277), so `try_parse` contains it with `catch_unwind`.
//! Nothing here touches the blob store; `parse_headers` buffers no attachment payloads.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use mail_parser::{
    AddressList, Encoding, HeaderForm, Message as MpMessage, MessageParser, MessagePart, PartFlags,
    PartId, PartKind, PartRole, TextList,
};

use crate::address::EmailAddress;
use crate::attachment::{Attachment, Disposition};
use crate::headers::Headers;
use crate::ids::{AccountId, MailboxId, MessageId, RfcMessageId};
use crate::message::{BodyState, Message, MessageBody};

/// Fills a message-list row's second line at the design canvas widths; short so storing one per
/// message keeps the list's database small.
pub const PREVIEW_CHARS: usize = 200;

/// Carries the decoded bytes, which the caller moves into the blob store and drops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPart {
    pub attachment: Attachment,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedMessage {
    pub headers: Headers,
    pub rfc_message_id: Option<RfcMessageId>,
    pub in_reply_to: Option<RfcMessageId>,
    pub references: Vec<RfcMessageId>,
    /// From `List-Id` (RFC 2919); detects lists with no configuration.
    pub list_id: Option<String>,
    // POSTIO-CONSENT: detection only, never sent — see `Message::read_receipt_requested`.
    pub read_receipt_requested: bool,
    pub from: Vec<EmailAddress>,
    pub sender: Option<EmailAddress>,
    pub reply_to: Vec<EmailAddress>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub body: MessageBody,
    /// Whether the `text/plain` part declared `format=flowed` (RFC 3676), so a later reply can undo
    /// the wrapping (#456).
    /// A plain `bool` because `postio-model` cannot depend on `postio-body`, which owns the
    /// unwrapping.
    pub text_is_flowed: bool,
    pub preview: Option<String>,
    pub parts: Vec<ParsedPart>,
    pub size: u64,
    pub body_state: BodyState,
    /// Not an error, but worth logging when a user reports mojibake.
    pub encoding_problems: bool,
}

impl ParsedMessage {
    pub fn attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.parts.iter().map(|part| &part.attachment)
    }

    /// `received_at` is the server's delivery time (IMAP `INTERNALDATE`), the list's sort key; the
    /// `Date` header is kept separately.
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
        // Promoted headers are known from any header block, but not from bytes that had none.
        message.promoted = (!self.headers.is_empty())
            .then(|| crate::promoted::PromotedHeaders::from_headers(&self.headers));
        message.headers = self.headers;
        message.sync.body_state = self.body_state;

        message
    }
}

/// The parser gave up entirely (it panicked and `try_parse` caught it), unlike a message with
/// missing fields, which `ParsedMessage` represents with `None`s. Carries nothing: callers only
/// need to know not to tell the user to wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Unparseable;

impl std::fmt::Display for Unparseable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the message could not be parsed")
    }
}

impl std::error::Error for Unparseable {}

/// Bumped whenever a change makes `parse` yield something different for bytes it already saw. Raw
/// bytes are not kept (ADR 0020), so the store stamps each body with its version
/// (`messages.body_parsed_with`) and backfill fetches again the rows an older version got wrong. 1:
/// lenient quoted-printable. 2: `mail-parser` 1.0, reading around damaged transfer encodings. 3: a
/// delimiter line holds only whitespace after the boundary (`patches/mail-parser/0001`, #899), and
/// a guessed structure carries the caveat (#287).
pub const PARSER_VERSION: u32 = 3;

/// Infallible; a message that cannot be parsed yields an empty `ParsedMessage` with only its
/// `size`, the same shape as a message with nothing in it. Use `try_parse` when the difference
/// matters.
pub fn parse(raw: &[u8]) -> ParsedMessage {
    try_parse(raw).unwrap_or_else(|_| empty(raw, false))
}

/// Buffers no body or attachment bytes; `body_state` is `HeadersOnly`. Used by the initial sync
/// (newest-first headers, bodies backfilled later).
pub fn parse_headers(raw: &[u8]) -> ParsedMessage {
    try_parse_headers(raw).unwrap_or_else(|_| empty(raw, true))
}

/// As `parse`, but says so when nothing could be parsed (#277): `mail-parser` 0.11.8 aborted on a
/// malformed multipart, and this runs on bytes a sender chose, during sync, so a crash would be
/// re-delivered by every sync. Caught with `catch_unwind`, not validated, since no pre-check can
/// predict which inputs reach the dependency. The reading pane uses it to tell "nothing parsed"
/// from "no body".
pub fn try_parse(raw: &[u8]) -> Result<ParsedMessage, Unparseable> {
    contain(|| parse_inner(raw, false))
}

pub fn try_parse_headers(raw: &[u8]) -> Result<ParsedMessage, Unparseable> {
    contain(|| parse_inner(raw, true))
}

/// Decodes RFC 2047 encoded words in an isolated header value (an IMAP `ENVELOPE` subject or
/// display name). Unreadable content comes back as surviving raw text, falling back to a lossy
/// UTF-8 decode. Contained like `try_parse`, as the decoder is hostile-input code (#277).
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

/// One line: runs of line breaks and control characters fold to a single space. An encoded word can
/// decode to `=0D=0A`, which no unfolding removes, and a forged `Bcc:` line appeared under a
/// subject (#1770). Whitespace touching a run joins it.
pub fn one_line(text: &str) -> String {
    let breaks = |c: char| c.is_control() || c == '\u{2028}' || c == '\u{2029}';
    if !text.chars().any(breaks) {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_run = false;
    for c in text.chars() {
        if breaks(c) {
            while out.ends_with(char::is_whitespace) {
                out.pop();
            }
            in_run = true;
        } else if in_run && c.is_whitespace() {
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

/// A non-ASCII filename rarely travels as plain `filename=`: RFC 2231 splits it (`filename*0*`) and
/// many clients put an
/// RFC 2047 word in `name=`. The pairs are put back into one `Content-Type` and parsed by
/// `mail_parser`, contained like `parse` (#1686).
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

/// The bracketed `List-Id` identifier (RFC 2919), so a list is recognized by its stable id, not a
/// renameable display name;
/// without brackets, what remains after trimming is the id. `None` once nothing is left. Text only,
/// shared with `postio-account`'s `ENVELOPE` fetch.
pub fn list_id_from_text(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let id = first_angle_bracketed(raw).unwrap_or(raw);
    let id = id.trim();
    (!id.is_empty()).then(|| id.to_owned())
}

/// RFC 2919 §3 allows a quoted phrase holding `<` and `>`; the first identifier wins, not
/// everything up to the last `>`.
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

/// `AssertUnwindSafe` is sound: the closure borrows only the input and returns an owned value. The
/// panic still reaches the hook, so a contained failure shows on stderr; a caller expecting it
/// should log the outcome as "handled".
fn contain<F>(parse: F) -> Result<ParsedMessage, Unparseable>
where
    F: FnOnce() -> ParsedMessage,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(parse)).map_err(|_| Unparseable)
}

/// `size` is still true (the bytes arrived); everything else is default, the same "we do not know"
/// as an unfilled field.
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

    // `mail_parser` can unwind, which this promises not to (#277). Containment is in `contain`,
    // wrapping this whole function so `try_parse` can report `Err`. Silent here: logging would put
    // `tracing` in the crate the workspace waits on (ADR 0004 Q1).
    let parser = MessageParser::default();
    let parsed = if headers_only {
        parser.parse_headers(raw)
    } else {
        parser.parse(raw)
    };

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
        // RFC 2046 §5.1.1: a multipart with an unusable boundary has no parts and is read as the
        // entity's own content;
        // `text_is_flowed` stays `false` since that part has no `format` attribute.
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

    // The parser's HTML-to-text feeds the snippet only: an HTML-only message needs a preview, but
    // converted text is not good enough to store.
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
        // Parser attachments plus inline media (the reader resolves `cid:` through this list). A
        // `multipart/*` part is a container, not a file (#900).
        .filter(|part| matches!(part.role(), PartRole::Attachment | PartRole::MediaBody))
        .filter(|part| !is_multipart_type(part))
        .filter(|part| {
            named_html_alternative
                .as_ref()
                .is_none_or(|html| html.id() != part.id())
        })
        .map(|part| parsed_part(&part, &paths))
        .collect();
    // The only signal that the words on screen differ from the words sent, so it carries every way
    // a body can arrive altered (#901, #287).
    message.encoding_problems = transfer_encoding_problem(&source)
        || decoded_lossily(raw, &message.body)
        || structure_guessed(&source);

    message
}

/// RFC 8621 `parseStructure` treats a named HTML alternative as an attachment and lets plain text
/// stand as the HTML body; Postio shows the sender's HTML and does not offer it again as a file. A
/// part disposed `attachment` stays one.
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

/// `mail-parser` 1.0 recovers from structures 0.11 could not walk and flags the part (#287).
/// `FALLBACK_DELIMITER` and `LIMIT_REACHED` mean content was lost. `MISSING_DELIMITER` and
/// `UNTERMINATED` lose nothing, so they are not flagged: a caveat on mail that reads in full
/// teaches people to ignore it.
fn structure_guessed(source: &MpMessage<'_>) -> bool {
    source.parts().any(|part| {
        part.flags()
            .intersects(PartFlags::FALLBACK_DELIMITER | PartFlags::LIMIT_REACHED)
    })
}

/// RFC 2045 §6.4: an unknown encoding (private `x-` tokens included) is `application/octet-stream`;
/// Postio shows the octets as text, which beats an empty body but is a guess, so it is flagged.
/// Transfer layer only: a charset mismatch is `decoded_lossily`'s.
fn transfer_encoding_problem(source: &MpMessage<'_>) -> bool {
    source.parts().any(|part| {
        if part.is_multipart() || part.is_message() {
            return false;
        }
        !part.has_known_transfer_encoding()
            || (part.encoding() != Encoding::None && !part.decoded_checked().1.is_empty())
    })
}

/// Whether charset decoding lost octets (U+FFFD), which is unrecoverable once in the stored body,
/// index and quoted replies. `is_encoding_problem` is about the transfer encoding and is false
/// either way on a charset mismatch (#901). The raw bytes are the control: a U+FFFD the sender sent
/// is theirs, so a message with both reads clean, the safe direction to be wrong in.
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

/// Raw text is kept, not the parser's interpretation: it backs "view source" and a later reparse.
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

/// Each fold (CRLF plus whitespace) collapses to a single space (RFC 5322).
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

/// For the payload axis (ADR 0017): `BODY[2.1]` returns encoded bytes with no headers, so the
/// `BODYSTRUCTURE` headers kept on the row are prepended to rebuild an entity. Attachments are
/// content-addressed on the decoded form, so identical files share a blob. `None` when the bytes
/// are not an entity or carry no body. Contained like `try_parse`.
pub fn decode_entity(raw: &[u8]) -> Option<Vec<u8>> {
    // Contained for the reason `parse` is (#277): these bytes were chosen by whoever sent the mail.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let parsed = MessageParser::default().parse(raw)?;
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

/// Rejects what is not a `Message-ID` (empty `<>`, unterminated angle-addr, bare token): threading
/// edges built on them only match other garbage.
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

/// Groups are flattened to their members (the group name stays in the raw headers); an entry with a
/// display name but no addr-spec is dropped.
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

/// Drops anything that reads as an HTML tag; some senders put markup in `text/plain`. Not a
/// sanitizer: `postio-model` may not depend on `ammonia` or `html5ever` (ADR 0004), and the preview
/// is drawn as a label. A tag is `<`, optional `/`, an ASCII letter, name characters, then
/// whitespace, `/` or `>`, which spares `<https://example.com>` and `<ada@example.com>`.
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

/// Unwraps markdown links, so a snippet is words rather than syntax: bulk senders run HTML through
/// a converter. `[label](url)` becomes `label` and `[ ](url)` becomes nothing. Only the link form,
/// only in the snippet; anything not a well-formed link is left as typed.
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

pub fn preview(text: &str) -> Option<String> {
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
        while out.chars().count() + 1 > PREVIEW_CHARS {
            out.pop();
        }
        out.push('\u{2026}');
    }
    Some(out)
}

fn is_multipart_type(part: &MessagePart<'_>) -> bool {
    part.content_type()
        .is_some_and(|content_type| content_type.ctype().eq_ignore_ascii_case("multipart"))
}

/// RFC 2046 §5.1.1: a multipart with a missing, empty or unrecognisable boundary "must be treated
/// as text/plain".
/// `mail_parser` leaves it as one `multipart/*` part holding the entity's own content, which can
/// only be the root.
/// RFC 2046 defines no charset for multipart, so a `Binary` root decodes as UTF-8 with replacement.
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

/// Maps every leaf part to its IMAP body-section path (`2.1`), which a lazy fetch of one attachment
/// asks the server for: children number from 1 and nesting extends the path with a dot. Walks an
/// explicit stack: recursion overflowed on crafted deep nesting, a `SIGSEGV` that `catch_unwind`
/// cannot catch, and a depth limit would be a number someone must get right (#277).
fn part_paths(source: &MpMessage<'_>) -> HashMap<PartId, String> {
    let mut paths = HashMap::new();
    // The root has no path of its own: its children are numbered from 1, as IMAP does.
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
    // Stored bare so it compares directly against the `cid:` URL in an HTML body; the IMAP header
    // sync reaches the same normaliser (#751).
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

/// With no `Content-Disposition`, a part with a `Content-ID` is almost always an image the HTML
/// refers to, so inline;
/// anything else is an attachment, which stays visible in the list either way.
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

/// Where each leaf part's body sits in `raw`, in byte order and never overlapping: what a view of
/// the source folds. Empty when the bytes do not parse. Contained like `try_parse` (#277).
pub fn leaf_bodies(raw: &[u8]) -> Vec<LeafBody> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let Some(parsed) = MessageParser::default().parse(raw) else {
            return Vec::new();
        };
        let mut out: Vec<LeafBody> = parsed
            .parts()
            .filter(|part| !part.is_multipart() && !part.is_message())
            .filter_map(|part| {
                let range = part.offset_body() as usize..part.offset_end() as usize;
                (range.start < range.end && range.end <= raw.len()).then(|| LeafBody {
                    range,
                    encoding: part
                        .content_transfer_encoding()
                        .unwrap_or("7bit")
                        .to_ascii_lowercase(),
                    name: part.attachment_name().map(str::to_owned),
                })
            })
            .collect();
        out.sort_by_key(|leaf| leaf.range.start);
        let mut end = 0;
        out.retain(|leaf| {
            let kept = leaf.range.start >= end;
            if kept {
                end = leaf.range.end;
            }
            kept
        });
        out
    }))
    .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafBody {
    pub range: std::ops::Range<usize>,
    /// The `Content-Transfer-Encoding` as written, lower-cased; `7bit` when absent.
    pub encoding: String,
    pub name: Option<String>,
}

#[cfg(test)]
mod containment {
    use super::*;

    #[test]
    fn a_panic_inside_the_parse_is_reported_as_unparseable() {
        let outcome = contain(|| panic!("a dependency bug"));
        assert_eq!(outcome, Err(Unparseable));
    }

    #[test]
    fn leaf_bodies_are_the_bytes_between_a_parts_headers_and_its_end() {
        let raw = b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\
            Content-Type: text/plain\r\n\r\nhello\r\n--b\r\n\
            Content-Type: application/octet-stream; name=x.bin\r\n\
            Content-Transfer-Encoding: BASE64\r\n\r\naGk=\r\n--b--\r\n";
        let leaves = leaf_bodies(raw);
        let seen: Vec<(&[u8], &str, Option<&str>)> = leaves
            .iter()
            .map(|leaf| {
                (
                    &raw[leaf.range.clone()],
                    leaf.encoding.as_str(),
                    leaf.name.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                (&b"hello"[..], "7bit", None),
                (&b"aGk="[..], "base64", Some("x.bin"))
            ]
        );
        assert!(leaf_bodies(b"").is_empty());
    }

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

    /// The control: a bracketed URL or address is ordinary plain text and may be a row's only
    /// content.
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

    #[test]
    fn without_tags_drops_a_closing_tag_and_only_a_tag() {
        assert_eq!(without_tags("a</b>c"), "ac");
        assert_eq!(without_tags("<i>x</i>y"), "xy");
        assert_eq!(without_tags("3 < 4 and <3"), "3 < 4 and <3");
        assert_eq!(without_tags("a <"), "a <");
    }

    #[test]
    fn without_tags_ends_an_attributed_tag_at_its_own_close() {
        assert_eq!(without_tags("<p class=\"x\">hi</p>"), "hi");
        assert_eq!(without_tags("<br/>then"), "then");
        assert_eq!(without_tags("<a href=\"u\">t</a> u"), "t u");
        assert_eq!(without_tags("<p class=\"x\" oops"), "<p class=\"x\" oops");
    }
}

#[cfg(test)]
mod message_ids {
    use super::*;

    #[test]
    fn a_message_id_needs_every_one_of_its_four_conditions() {
        assert!(message_id("<a@example.com>").is_some());
        assert!(message_id("<>").is_none(), "empty");
        assert!(message_id("<no-at-sign>").is_none(), "no @");
        assert!(message_id("<a b@example.com>").is_none(), "whitespace");
        assert!(message_id("<a@<b>>").is_none(), "a bracket inside");
        assert!(message_id("a@example.com").is_some());
    }
}

#[cfg(test)]
mod a_snippet_is_words_not_syntax {
    use super::without_markdown_links;

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
