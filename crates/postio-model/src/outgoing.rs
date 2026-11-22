//! The other half of `mime::parse`: turns a composed `Draft` into bytes for `postio-smtp`. Pure
//! format library, no blob store:
//! `OutgoingAttachment` carries bytes the caller resolved. `build` takes the parent `Message`,
//! since a `Draft` carries only its local id.

use chrono::Utc;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MbAddress;
use mail_builder::headers::content_type::ContentType as MbContentType;
use mail_builder::headers::date::Date as MbDate;
use mail_builder::headers::message_id::MessageId as MbMessageId;
use mail_builder::headers::raw::Raw;
use mail_builder::mime::{BodyPart as MbBodyPart, MimePart as MbMimePart, make_boundary};

use crate::account::Identity;
use crate::address::EmailAddress;
use crate::attachment::{Attachment, Disposition};
use crate::draft::Draft;
use crate::ids::RfcMessageId;
use crate::message::Message;

/// RFC 2606 reserves `.invalid`; reached only if an `Identity` was built without a valid `@domain`.
const FALLBACK_DOMAIN: &str = "invalid";

/// `postio-model` has no blob store, so the caller reads the bytes and pairs them with their
/// metadata.
#[derive(Debug, Clone, Copy)]
pub struct OutgoingAttachment<'a> {
    pub attachment: &'a Attachment,
    pub content: &'a [u8],
}

/// The `text/calendar` part an RSVP carries, in the `multipart/alternative` beside the text, where
/// RFC 6047 §2.4 puts an iTIP message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarPart<'a> {
    pub method: CalendarMethod,
    /// UTF-8 with CRLF line endings, as `postio-calendar` writes it.
    pub ics: &'a [u8],
}

/// Postio answers invitations and organises none, so a reply is all it sends; a closed set keeps
/// anything else out of a header parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CalendarMethod {
    Reply,
}

impl CalendarMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            CalendarMethod::Reply => "REPLY",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltMessage {
    pub raw: Vec<u8>,
    pub message_id: RfcMessageId,
}

/// An attachment is embedded inline via `cid:` when it is `Disposition::Inline` with a
/// `content_id`. `in_reply_to`'s `Message-ID`
/// becomes `In-Reply-To` and its chain plus that id becomes `References`; a parent with no
/// `Message-ID` contributes no `In-Reply-To`.
/// The `Message-ID` comes from `Draft::rfc_message_id` when reserved, else fresh (ADR 0021): a
/// `Send` is rebuilt on every
/// drain attempt, so generating one unconditionally made a retried send a distinct message.
/// `calendar` joins the text and HTML in the `multipart/alternative`.
pub fn build(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
    calendar: Option<CalendarPart<'_>>,
) -> BuiltMessage {
    assemble(
        draft,
        identity,
        attachments,
        in_reply_to,
        Bcc::Omit,
        calendar,
    )
}

/// Identical to `build` except it carries `Bcc`: the copy in the user's own Drafts reaches nobody,
/// and dropping `Bcc` there would
/// lose typed recipients. Separate entry points, not a boolean, because the wrong answer discloses
/// a bcc'd address to every recipient.
pub fn build_draft(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
) -> BuiltMessage {
    assemble(
        draft,
        identity,
        attachments,
        in_reply_to,
        Bcc::Include,
        None,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bcc {
    Omit,
    Include,
}

fn assemble(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
    bcc: Bcc,
    calendar: Option<CalendarPart<'_>>,
) -> BuiltMessage {
    let message_id = draft.rfc_message_id.clone().unwrap_or_else(|| {
        generate_message_id(identity.address.domain().unwrap_or(FALLBACK_DOMAIN))
    });

    let mut builder = MessageBuilder::new()
        .message_id(message_id.without_brackets().to_owned())
        .date(MbDate::new(Utc::now().timestamp()))
        .from(mb_address(&identity.address))
        .reply_to(mb_address(identity.effective_reply_to()));

    if let Some(parent) = in_reply_to {
        builder = add_threading_headers(builder, parent);
    }

    if !draft.subject.is_empty() {
        builder = builder.subject(header_text(&draft.subject));
    }
    match recipient_list(&draft.to) {
        Some(to) => builder = builder.to(to),
        // A message whose only recipients are bcc'd would have no `To` and read as broken (a
        // missing `To` is a spam signal),
        // so it gets the empty group `undisclosed-recipients:;`. Only when nothing else names a
        // recipient, and only for
        // `Bcc::Omit`: beside a real `To` it would announce a hidden recipient, and the Drafts copy
        // keeps its real `Bcc`.
        None if bcc == Bcc::Omit && draft.cc.is_empty() && !draft.bcc.is_empty() => {
            // A raw header, not `Address::new_group`, which quotes the name: that is not the
            // spelling clients recognise on sight.
            builder = builder.header("To", Raw::new(UNDISCLOSED_RECIPIENTS));
        }
        None => {}
    }
    if let Some(cc) = recipient_list(&draft.cc) {
        builder = builder.cc(cc);
    }
    // Bcc is written only for the Drafts copy: DATA's bytes reach every envelope recipient, so a
    // Bcc header would hand
    // them the bcc'd list. A bcc'd address is still a `RCPT TO` the caller adds to the envelope.
    if bcc == Bcc::Include
        && let Some(addresses) = recipient_list(&draft.bcc)
    {
        builder = builder.bcc(addresses);
    }
    if let Some(text) = &draft.body.text {
        // The public field, not `.body()`, which is a raw override replacing the whole
        // text/html/attachments assembly;
        // `.text_body()` always spells bare `text/plain`, so this assigns the `format=flowed`
        // header directly.
        builder.text_body = Some(match draft.body_markdown {
            Some(_) => fixed_text_part(text.clone()),
            None => flowed_text_part(text.clone()),
        });
    }
    if let Some(html) = &draft.body.html {
        builder = builder.html_body(html.clone());
    }

    for outgoing in attachments {
        builder = add_attachment(builder, outgoing);
    }
    if let Some(calendar) = calendar {
        builder = with_calendar(builder, calendar);
    }

    let raw = builder
        .write_to_vec()
        .expect("writing RFC 5322 bytes to a Vec<u8> cannot fail");
    BuiltMessage { raw, message_id }
}

/// mail-builder has no place for a third alternative, so this builds the same tree it would with
/// the calendar in
/// the `multipart/alternative` (RFC 6047 §2.4). Without a calendar none of this runs.
fn with_calendar<'x>(
    mut builder: MessageBuilder<'x>,
    calendar: CalendarPart<'_>,
) -> MessageBuilder<'x> {
    let part = MbMimePart::new(
        MbContentType::new("text/calendar")
            .attribute("method", calendar.method.as_str())
            .attribute("charset", "utf-8"),
        MbBodyPart::Text(String::from_utf8_lossy(calendar.ics).into_owned().into()),
    );
    let alternatives: Vec<MbMimePart<'x>> = [builder.text_body.take(), builder.html_body.take()]
        .into_iter()
        .flatten()
        .chain(std::iter::once(part))
        .collect();
    let alternative = MbMimePart::new("multipart/alternative", alternatives);
    builder.body = Some(match builder.attachments.take() {
        Some(attachments) => MbMimePart::new(
            "multipart/mixed",
            std::iter::once(alternative)
                .chain(attachments)
                .collect::<Vec<_>>(),
        ),
        None => alternative,
    });
    builder
}

/// A header value may not contain CR or LF (RFC 5322 §2.2) except as folding; left alone, text
/// after a break becomes a header (#864 reached a `Bcc`).
/// Folded rather than refused: the value can come from received mail (an RFC 2047 word can carry
/// any octet), and refusing would give a hostile sender a veto over your reply.
/// A single space is what unfolding a legitimately folded header produces. Applied in one place,
/// since sanitising at the composer would leave restored drafts, `mailto:`, rules, FFI and MCP
/// open.
fn header_text(value: &str) -> String {
    if !value.contains(['\r', '\n']) {
        return value.to_owned();
    }
    let mut folded = String::with_capacity(value.len());
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\r' && character != '\n' {
            folded.push(character);
            continue;
        }
        while characters
            .peek()
            .is_some_and(|next| matches!(next, '\r' | '\n' | ' ' | '\t'))
        {
            characters.next();
        }
        if !folded.is_empty() && characters.peek().is_some() {
            folded.push(' ');
        }
    }
    folded
}

/// Structured, so folding is wrong: a space inside `<...>` makes a different invalid id. Dropping
/// loses nothing (a malformed id threads
/// nothing) and closes the same injection, which reaches `In-Reply-To` and `References` from one
/// hostile value.
fn usable_message_id(id: &RfcMessageId) -> Option<String> {
    let text = id.without_brackets();
    (!text.contains(['\r', '\n'])).then(|| text.to_owned())
}

/// `References` is what a message threading into `parent` would have claimed as ancestors: its
/// `reference_chain` plus its own `Message-ID`.
fn add_threading_headers<'x>(builder: MessageBuilder<'x>, parent: &Message) -> MessageBuilder<'x> {
    let mut references: Vec<String> = parent
        .reference_chain()
        .filter_map(usable_message_id)
        .collect();

    let mut builder = builder;
    if let Some(parent_id) = parent.rfc_message_id.as_ref().and_then(usable_message_id) {
        builder = builder.in_reply_to(parent_id.clone());
        references.push(parent_id);
    }
    if !references.is_empty() {
        builder = builder.references(MbMessageId::new_list(references.into_iter()));
    }
    builder
}

fn add_attachment<'x>(
    builder: MessageBuilder<'x>,
    outgoing: &OutgoingAttachment<'_>,
) -> MessageBuilder<'x> {
    let mime_type = outgoing.attachment.mime_type.clone();
    let content = outgoing.content.to_vec();

    match (
        &outgoing.attachment.disposition,
        &outgoing.attachment.content_id,
    ) {
        (Disposition::Inline, Some(content_id)) => {
            builder.inline(mime_type, content_id.clone(), content)
        }
        _ => builder.attachment(
            mime_type,
            outgoing.attachment.display_name().to_owned(),
            content,
        ),
    }
}

/// Unconditional, not detected: `postio-model` has no HTML parser or wrapping logic, and
/// `postio_body::render` is the only
/// path that fills `draft.body.text` (#333), so this only has to say so on the wire.
fn flowed_text_part(text: String) -> MbMimePart<'static> {
    MbMimePart::new(
        MbContentType::new("text/plain")
            .attribute("charset", "utf-8")
            .attribute("format", "flowed"),
        MbBodyPart::Text(text.into()),
    )
}

/// Fixed, not flowed: Markdown's line breaks carry meaning (a list item, a code line) and a
/// `format=flowed` reader would join them.
fn fixed_text_part(text: String) -> MbMimePart<'static> {
    MbMimePart::new(
        MbContentType::new("text/plain").attribute("charset", "utf-8"),
        MbBodyPart::Text(text.into()),
    )
}

/// A non-empty address list, or `None`: mail-builder writes a header even for an empty list.
/// The group name a message with nothing to name is addressed to: an RFC 5322 group with no
/// members.
const UNDISCLOSED_RECIPIENTS: &str = "undisclosed-recipients:;";

fn recipient_list(addresses: &[EmailAddress]) -> Option<MbAddress<'static>> {
    if addresses.is_empty() {
        return None;
    }
    Some(MbAddress::new_list(
        addresses.iter().map(mb_address).collect(),
    ))
}

fn mb_address(address: &EmailAddress) -> MbAddress<'static> {
    MbAddress::new_address(address.name.clone(), address.address.clone())
}

/// The same generator `build` falls back to, so whoever reserves an id and whoever writes the
/// header produce one shape (ADR 0021).
/// `domain` is the identity's domain; `FALLBACK_DOMAIN` when it has none.
pub fn reserve_message_id(domain: Option<&str>) -> RfcMessageId {
    generate_message_id(domain.unwrap_or(FALLBACK_DOMAIN))
}

/// Built from `make_boundary` rather than a UUID dependency or this machine's hostname, which
/// mail-builder's own `Message-ID`
/// would carry: nothing leaves the machine that the user did not ask for.
fn generate_message_id(domain: &str) -> RfcMessageId {
    RfcMessageId::new(format!("{}@{domain}", make_boundary(".")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AccountId, IdentityId};
    use crate::mime;

    fn identity(address: &str) -> Identity {
        let mut identity = Identity::new(
            AccountId::UNASSIGNED,
            EmailAddress::new(Some("Ada Lovelace"), address),
        );
        identity.id = IdentityId::new(1);
        identity
    }

    fn draft() -> Draft {
        let mut draft = Draft::new(AccountId::UNASSIGNED);
        draft.to = vec![EmailAddress::new(Some("Grace Hopper"), "grace@example.net")];
        draft.subject = "Tuesday walkthrough notes".to_owned();
        draft.body.text = Some("Looking now.".to_owned());
        draft
    }

    #[test]
    fn a_plain_text_draft_round_trips_through_the_parser() {
        let ada = identity("ada@example.com");
        let built = build(&draft(), &ada, &[], None, None);

        let parsed = mime::parse(&built.raw);
        assert_eq!(parsed.subject.as_deref(), Some("Tuesday walkthrough notes"));
        assert_eq!(parsed.from[0].address, "ada@example.com");
        assert_eq!(parsed.from[0].name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(parsed.to[0].address, "grace@example.net");
        assert_eq!(parsed.body.text.as_deref(), Some("Looking now."));
        assert_eq!(parsed.rfc_message_id, Some(built.message_id));
        assert!(parsed.date.is_some(), "a Date header was generated");
    }

    #[test]
    fn the_text_part_advertises_format_flowed() {
        let ada = identity("ada@example.com");
        let built = build(&draft(), &ada, &[], None, None);
        let raw = String::from_utf8_lossy(&built.raw);

        assert!(
            raw.contains("Content-Type: text/plain;"),
            "the text part should still say what it is: {raw}"
        );
        assert!(
            raw.to_ascii_lowercase().contains(r#"format="flowed""#),
            "the text part should advertise RFC 3676: {raw}"
        );

        let parsed = mime::parse(&built.raw);
        assert_eq!(parsed.body.text.as_deref(), Some("Looking now."));
    }

    #[test]
    fn a_draft_written_in_markdown_sends_its_markdown_as_fixed_text() {
        let markdown = "- one\n- two\n\n    let x = 1;\n".to_owned();
        let mut draft = draft();
        draft.body.text = Some(markdown.clone());
        draft.body.html = Some("<ul><li>one</li><li>two</li></ul>".to_owned());
        draft.body_markdown = Some(markdown.clone());

        let built = build(&draft, &identity("ada@example.com"), &[], None, None);
        let raw = String::from_utf8_lossy(&built.raw).to_ascii_lowercase();

        assert!(
            !raw.contains("flowed"),
            "a markdown text part is fixed: {raw}"
        );
        let parsed = mime::parse(&built.raw);
        let text = parsed.body.text.unwrap_or_default().replace("\r\n", "\n");
        assert_eq!(text, markdown);
        assert!(parsed.body.html.is_some(), "the HTML part still travels");
    }

    #[test]
    fn a_markdown_draft_with_nothing_to_style_is_text_alone() {
        let mut draft = draft();
        draft.body_markdown = Some("Looking now.".to_owned());

        let built = build(&draft, &identity("ada@example.com"), &[], None, None);
        let raw = String::from_utf8_lossy(&built.raw).to_ascii_lowercase();

        assert!(!raw.contains("text/html"), "no HTML part: {raw}");
        assert!(
            !raw.contains("multipart"),
            "one part, not an alternative: {raw}"
        );
        assert!(!raw.contains("flowed"), "{raw}");
    }

    #[test]
    fn a_draft_with_no_reserved_id_gets_a_fresh_one_each_build() {
        let ada = identity("ada@example.com");
        let first = build(&draft(), &ada, &[], None, None);
        let second = build(&draft(), &ada, &[], None, None);
        assert_ne!(first.message_id, second.message_id);
    }

    #[test]
    fn a_reserved_message_id_survives_every_rebuild() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.rfc_message_id = Some(RfcMessageId::new("reserved.once@example.com"));

        let first = build(&draft, &ada, &[], None, None);
        let second = build(&draft, &ada, &[], None, None);

        assert_eq!(first.message_id, second.message_id, "same id, both builds");
        assert_eq!(
            first.message_id,
            RfcMessageId::new("reserved.once@example.com"),
            "and it is the reserved one, not a fresh id that happens to repeat"
        );
        assert_eq!(
            mime::parse(&first.raw).rfc_message_id,
            Some(RfcMessageId::new("reserved.once@example.com")),
        );
    }

    #[test]
    fn the_drafts_copy_carries_the_reserved_id_too() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.rfc_message_id = Some(RfcMessageId::new("reserved.once@example.com"));

        assert_eq!(
            build_draft(&draft, &ada, &[], None).message_id,
            build(&draft, &ada, &[], None, None).message_id,
        );
    }

    #[test]
    fn an_explicit_reply_to_comes_through() {
        let mut ada = identity("ada@example.com");
        ada.reply_to = Some(EmailAddress::new(None::<String>, "replies@example.org"));

        let parsed = mime::parse(&build(&draft(), &ada, &[], None, None).raw);
        assert_eq!(parsed.reply_to[0].address, "replies@example.org");
    }

    #[test]
    fn a_header_value_folds_its_line_breaks_into_single_spaces() {
        assert_eq!(header_text("Tuesday notes"), "Tuesday notes");
        assert_eq!(header_text("two  spaces"), "two  spaces");

        assert_eq!(header_text("a\r\nb"), "a b");
        assert_eq!(header_text("a\nb"), "a b");
        assert_eq!(header_text("a\rb"), "a b");

        assert_eq!(header_text("a\r\n\tb"), "a b");
        assert_eq!(header_text("a\r\n \r\n b"), "a b");

        assert_eq!(header_text("\r\nX-Injected: yes"), "X-Injected: yes");
        assert_eq!(header_text("subject\r\n"), "subject");

        assert_eq!(
            header_text("ok\r\nBcc: eve@example.org"),
            "ok Bcc: eve@example.org"
        );
    }

    #[test]
    fn a_message_id_carrying_a_line_break_is_dropped_rather_than_folded() {
        assert_eq!(
            usable_message_id(&RfcMessageId::new("<a@example.com>")).as_deref(),
            Some("a@example.com")
        );
        assert_eq!(
            usable_message_id(&RfcMessageId::new("a@example.com>\r\nBcc: eve@example.org")),
            None
        );
    }

    #[test]
    fn bcc_recipients_never_appear_in_the_sent_bytes() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let built = build(&draft, &ada, &[], None, None);
        let parsed = mime::parse(&built.raw);

        assert!(
            parsed.bcc.is_empty(),
            "a Bcc header would hand every To/Cc recipient the bcc'd list"
        );
        assert!(
            !String::from_utf8_lossy(&built.raw).contains("quiet@example.com"),
            "the bcc'd address must not appear in the message at all, only in the envelope"
        );
    }

    #[test]
    fn a_bcc_only_message_says_undisclosed_recipients_rather_than_nothing() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.to.clear();
        draft.cc.clear();
        draft.bcc = vec![
            EmailAddress::new(None::<String>, "quiet@example.com"),
            EmailAddress::new(None::<String>, "alsoquiet@example.net"),
        ];

        let built = build(&draft, &ada, &[], None, None);
        let raw = String::from_utf8_lossy(&built.raw).into_owned();
        let parsed = mime::parse(&built.raw);

        assert_eq!(
            parsed.headers.get("To"),
            Some("undisclosed-recipients:;"),
            "a Bcc-only message must still be addressed to something: {raw}"
        );
        for hidden in ["quiet@example.com", "alsoquiet@example.net"] {
            assert!(
                !raw.contains(hidden),
                "{hidden} reached the message body; a bcc'd address travels in \
                 the envelope and nowhere else: {raw}"
            );
        }
        assert!(
            parsed.bcc.is_empty(),
            "a Bcc header would hand every recipient the bcc'd list"
        );
    }

    #[test]
    fn the_drafts_copy_of_a_bcc_only_message_names_its_recipients_instead() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.to.clear();
        draft.cc.clear();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let built = build_draft(&draft, &ada, &[], None);
        let raw = String::from_utf8_lossy(&built.raw).into_owned();

        assert!(
            !raw.contains("undisclosed-recipients"),
            "the Drafts copy names its recipients and must not also claim \
             they are undisclosed: {raw}"
        );
        assert!(
            raw.contains("quiet@example.com"),
            "the Drafts copy must keep the Bcc, or a draft picked up on \
             another client sends to fewer people than was asked: {raw}"
        );
    }

    #[test]
    fn a_message_with_real_recipients_is_never_given_the_placeholder() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let built = build(&draft, &ada, &[], None, None);
        let raw = String::from_utf8_lossy(&built.raw).into_owned();

        assert!(
            !raw.contains("undisclosed-recipients"),
            "a message that names its recipients must not also announce that \
             it has hidden ones: {raw}"
        );
    }

    #[test]
    fn the_drafts_copy_keeps_the_bcc_the_sent_bytes_drop() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let filed = mime::parse(&build_draft(&draft, &ada, &[], None).raw);
        let sent = mime::parse(&build(&draft, &ada, &[], None, None).raw);

        assert_eq!(
            filed.bcc.first().map(|address| address.address.as_str()),
            Some("quiet@example.com")
        );
        assert!(
            sent.bcc.is_empty(),
            "and the two entry points must not have grown into one"
        );
    }

    #[test]
    fn a_draft_with_no_bcc_grows_no_empty_header() {
        let ada = identity("ada@example.com");
        let built = build_draft(&draft(), &ada, &[], None);

        assert!(
            !String::from_utf8_lossy(&built.raw).contains("Bcc"),
            "an empty header is a header a parser still has to explain"
        );
    }

    #[test]
    fn non_ascii_subjects_and_display_names_survive() {
        let ada = identity("jurgen@example.com");
        let mut draft = draft();
        draft.subject = "Gruß aus München".to_owned();
        draft.to = vec![EmailAddress::new(Some("田中 陽子"), "yoko@example.net")];

        let parsed = mime::parse(&build(&draft, &ada, &[], None, None).raw);
        assert_eq!(parsed.subject.as_deref(), Some("Gruß aus München"));
        assert_eq!(parsed.to[0].name.as_deref(), Some("田中 陽子"));
    }

    #[test]
    fn text_and_html_bodies_both_survive_as_multipart_alternative() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.body.html = Some("<p>Looking now.</p>".to_owned());

        let parsed = mime::parse(&build(&draft, &ada, &[], None, None).raw);
        assert_eq!(parsed.body.text.as_deref(), Some("Looking now."));
        assert_eq!(parsed.body.html.as_deref(), Some("<p>Looking now.</p>"));
    }

    #[test]
    fn an_attachment_round_trips_with_its_bytes_and_filename() {
        let ada = identity("ada@example.com");
        let mut file = Attachment::new(crate::ids::MessageId::UNASSIGNED, "application/pdf", 4);
        file.filename = Some("layout.pdf".to_owned());
        let content = b"%PDF".to_vec();

        let built = build(
            &draft(),
            &ada,
            &[OutgoingAttachment {
                attachment: &file,
                content: &content,
            }],
            None,
            None,
        );
        let parsed = mime::parse(&built.raw);

        let attachment = parsed.attachments().next().expect("the attachment");
        assert_eq!(attachment.filename.as_deref(), Some("layout.pdf"));
        assert_eq!(attachment.disposition, crate::Disposition::Attachment);
        let part = &parsed.parts[0];
        assert_eq!(part.content, content);
    }

    #[test]
    fn an_inline_image_round_trips_with_its_content_id_and_is_referenced_from_the_html() {
        let ada = identity("ada@example.com");
        let mut image = Attachment::new(crate::ids::MessageId::UNASSIGNED, "image/png", 4);
        image.disposition = Disposition::Inline;
        image.content_id = Some("logo@postio".to_owned());
        let content = vec![1, 2, 3, 4];

        let mut draft = draft();
        draft.body.html = Some(r#"<img src="cid:logo@postio">"#.to_owned());

        let built = build(
            &draft,
            &ada,
            &[OutgoingAttachment {
                attachment: &image,
                content: &content,
            }],
            None,
            None,
        );
        let parsed = mime::parse(&built.raw);

        let part = parsed.attachments().next().expect("the inline image");
        assert_eq!(part.disposition, crate::Disposition::Inline);
        assert_eq!(part.content_id.as_deref(), Some("logo@postio"));
        assert!(
            parsed
                .body
                .html
                .as_deref()
                .unwrap()
                .contains("cid:logo@postio")
        );
    }

    #[test]
    fn a_draft_with_no_subject_omits_the_header_rather_than_sending_it_empty() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.subject = String::new();

        let parsed = mime::parse(&build(&draft, &ada, &[], None, None).raw);
        assert!(parsed.subject.is_none());
    }

    const REPLY_ICS: &str = "BEGIN:VCALENDAR\r\n\
        PRODID:-//Postio//Postio//EN\r\n\
        VERSION:2.0\r\n\
        METHOD:REPLY\r\n\
        BEGIN:VEVENT\r\n\
        UID:q4-budget-review@example.com\r\n\
        SEQUENCE:0\r\n\
        DTSTAMP:20260928T120000Z\r\n\
        ORGANIZER;CN=\"Ines Okafor\":mailto:ines.okafor@example.com\r\n\
        ATTENDEE;PARTSTAT=ACCEPTED;CN=\"Ada Norwood\":mailto:ada.norwood@example.com\r\n\
        SUMMARY:Q4 budget review\r\n\
        END:VEVENT\r\n\
        END:VCALENDAR\r\n";

    fn rsvp() -> CalendarPart<'static> {
        CalendarPart {
            method: CalendarMethod::Reply,
            ics: REPLY_ICS.as_bytes(),
        }
    }

    fn tree(raw: &[u8]) -> String {
        use mail_parser::{MessageParser, MessagePart};

        fn render(part: MessagePart<'_>) -> String {
            let kind = part.content_type().map_or_else(
                || "text/plain".to_owned(),
                |kind| format!("{}/{}", kind.ctype(), kind.subtype().unwrap_or_default()),
            );
            if part.is_multipart() {
                format!(
                    "{kind}[{}]",
                    part.children().map(render).collect::<Vec<_>>().join(", ")
                )
            } else {
                kind
            }
        }

        let message = MessageParser::default()
            .parse(raw)
            .expect("the built bytes parse");
        render(message.root_part())
    }

    fn calendar_part_of(raw: &[u8]) -> (String, Option<String>, Vec<u8>) {
        use mail_parser::MessageParser;

        let message = MessageParser::default()
            .parse(raw)
            .expect("the built bytes parse");
        let part = message
            .parts()
            .find(|part| {
                part.content_type().is_some_and(|kind| {
                    kind.ctype() == "text" && kind.subtype() == Some("calendar")
                })
            })
            .expect("a text/calendar part");
        let kind = part.content_type().expect("its content type");
        (
            kind.attribute("method").unwrap_or_default().to_owned(),
            kind.attribute("charset").map(str::to_owned),
            part.decoded().to_vec(),
        )
    }

    #[test]
    fn a_reply_joins_the_text_in_a_multipart_alternative() {
        let built = build(
            &draft(),
            &identity("ada@example.com"),
            &[],
            None,
            Some(rsvp()),
        );

        assert_eq!(
            tree(&built.raw),
            "multipart/alternative[text/plain, text/calendar]"
        );
    }

    #[test]
    fn with_html_the_reply_is_the_last_alternative() {
        let mut draft = draft();
        draft.body.html = Some("<p>Looking now.</p>".to_owned());
        let built = build(
            &draft,
            &identity("ada@example.com"),
            &[],
            None,
            Some(rsvp()),
        );

        assert_eq!(
            tree(&built.raw),
            "multipart/alternative[text/plain, text/html, text/calendar]"
        );
    }

    #[test]
    fn with_an_attachment_the_alternative_sits_inside_the_mixed_part() {
        let mut file = Attachment::new(crate::ids::MessageId::UNASSIGNED, "application/pdf", 4);
        file.filename = Some("agenda.pdf".to_owned());
        let content = b"%PDF".to_vec();
        let built = build(
            &draft(),
            &identity("ada@example.com"),
            &[OutgoingAttachment {
                attachment: &file,
                content: &content,
            }],
            None,
            Some(rsvp()),
        );

        assert_eq!(
            tree(&built.raw),
            "multipart/mixed[multipart/alternative[text/plain, text/calendar], application/pdf]"
        );
    }

    #[test]
    fn the_calendar_part_says_its_method_and_carries_the_calendar_unchanged() {
        let built = build(
            &draft(),
            &identity("ada@example.com"),
            &[],
            None,
            Some(rsvp()),
        );
        let (method, charset, content) = calendar_part_of(&built.raw);

        assert_eq!(method, "REPLY");
        assert_eq!(charset.as_deref(), Some("utf-8"));
        assert_eq!(
            String::from_utf8(content).expect("UTF-8"),
            REPLY_ICS,
            "byte for byte, CRLF and all"
        );
    }

    #[test]
    fn a_reply_still_reads_as_its_text_to_a_person() {
        let built = build(
            &draft(),
            &identity("ada@example.com"),
            &[],
            None,
            Some(rsvp()),
        );
        let parsed = mime::parse(&built.raw);

        assert_eq!(parsed.body.text.as_deref(), Some("Looking now."));
        assert!(
            parsed
                .parts
                .iter()
                .any(|part| part.attachment.mime_type == "text/calendar"),
            "the calendar is reachable as a part"
        );
    }

    #[test]
    fn with_no_calendar_nothing_calendar_is_written() {
        let built = build(&draft(), &identity("ada@example.com"), &[], None, None);

        assert!(
            !String::from_utf8_lossy(&built.raw).contains("text/calendar"),
            "no calendar part appears unasked"
        );
        assert_eq!(tree(&built.raw), "text/plain");
    }

    fn parent(rfc_message_id: &str, references: &[&str]) -> Message {
        let mut message = Message::new(
            AccountId::new(1),
            crate::ids::MailboxId::new(1),
            chrono::Utc::now(),
        );
        message.rfc_message_id = Some(RfcMessageId::new(rfc_message_id));
        message.references = references.iter().map(|id| RfcMessageId::new(*id)).collect();
        message
    }

    #[test]
    fn replying_sets_in_reply_to_and_appends_the_parent_to_its_own_chain() {
        let ada = identity("ada@example.com");
        let root = parent("<root@example.com>", &[]);

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&root), None).raw);
        assert_eq!(
            parsed.in_reply_to,
            Some(RfcMessageId::new("<root@example.com>"))
        );
        assert_eq!(
            parsed.references,
            vec![RfcMessageId::new("<root@example.com>")]
        );
    }

    #[test]
    fn replying_deep_in_a_thread_carries_the_whole_chain_forward() {
        let ada = identity("ada@example.com");
        let middle = parent("<middle@example.com>", &["<root@example.com>"]);

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&middle), None).raw);
        assert_eq!(
            parsed.in_reply_to,
            Some(RfcMessageId::new("<middle@example.com>"))
        );
        assert_eq!(
            parsed.references,
            vec![
                RfcMessageId::new("<root@example.com>"),
                RfcMessageId::new("<middle@example.com>"),
            ],
            "the parent's own ancestors, then the parent itself"
        );
    }

    #[test]
    fn a_parent_with_no_message_id_still_contributes_no_dangling_reference() {
        let ada = identity("ada@example.com");
        let mut orphan = parent("<unused@example.com>", &[]);
        orphan.rfc_message_id = None;

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&orphan), None).raw);
        assert!(
            parsed.in_reply_to.is_none(),
            "nothing to reply to without a Message-ID"
        );
        assert!(
            parsed.references.is_empty(),
            "and no chain either, since the parent contributed nothing to it"
        );
    }

    #[test]
    fn with_no_parent_no_threading_headers_are_written_at_all() {
        let ada = identity("ada@example.com");

        let parsed = mime::parse(&build(&draft(), &ada, &[], None, None).raw);
        assert!(parsed.in_reply_to.is_none());
        assert!(parsed.references.is_empty());
    }

    /// A control character in an address cannot start a header line: mail-builder 1.0 wrote it raw,
    /// injecting a `Bcc:` field.
    #[test]
    fn a_control_character_in_an_address_cannot_inject_a_header() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.to = vec![EmailAddress::new(
            Some("Eve"),
            "eve@example.org>\r\nBcc: evil@example.org\r\nX: <y",
        )];

        let built = build(&draft, &ada, &[], None, None);
        let text = String::from_utf8_lossy(&built.raw);
        let head = text.split("\r\n\r\n").next().expect("a header block");
        assert!(
            !head.lines().any(|line| line.starts_with("Bcc:")),
            "an address started a header line:\n{head}"
        );
    }
}
