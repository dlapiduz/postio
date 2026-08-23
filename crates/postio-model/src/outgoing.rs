//! Building outgoing RFC 5322 messages from a [`Draft`].
//!
//! This is the other half of [`mime::parse`](crate::mime::parse): that module
//! turns bytes the network handed us into the domain model, this one turns a
//! composed [`Draft`] back into bytes for `postio-smtp` to send. Both stay in
//! `postio-model` for the same reason — [`mail_builder`], like [`mail_parser`],
//! is a pure format library with no I/O — and neither one touches the blob
//! store: [`OutgoingAttachment`] pairs a [`Draft`]'s attachment metadata with
//! bytes the caller already resolved, exactly as [`ParsedPart`](crate::mime::ParsedPart)
//! does for the parsing side.
//!
//! # What this does not do
//!
//! It does not thread. `In-Reply-To` and `References` need the RFC
//! `Message-ID` chain of the message being replied to, which is not part of a
//! [`Draft`] — that is `postio-p8q`'s job, building on top of this.

use chrono::Utc;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MbAddress;
use mail_builder::headers::date::Date as MbDate;
use mail_builder::mime::make_boundary;

use crate::account::Identity;
use crate::address::EmailAddress;
use crate::attachment::{Attachment, Disposition};
use crate::draft::Draft;
use crate::ids::RfcMessageId;

/// A domain to fall back on when an identity's address has none.
///
/// RFC 2606 reserves `.invalid` for exactly this: an address that should never
/// resolve. It is only reached when an `Identity` was built without a valid
/// `@domain`, which the account setup flow does not allow.
const FALLBACK_DOMAIN: &str = "invalid";

/// One of a draft's attachments, resolved to its bytes.
///
/// `postio-model` has no blob store, so the caller reads
/// [`Draft::attachments`] against it and hands the bytes back paired with the
/// metadata they belong to.
#[derive(Debug, Clone, Copy)]
pub struct OutgoingAttachment<'a> {
    /// The attachment's metadata, from [`Draft::attachments`].
    pub attachment: &'a Attachment,
    /// Its decoded bytes.
    pub content: &'a [u8],
}

/// A [`Draft`] built into raw bytes, ready to hand to SMTP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltMessage {
    /// The complete RFC 5322 message: headers and body.
    pub raw: Vec<u8>,
    /// The `Message-ID` this build generated.
    pub message_id: RfcMessageId,
}

/// Builds `draft` into an outgoing message sent as `identity`.
///
/// `attachments` must correspond to `draft.attachments`, resolved to their
/// bytes by the caller; an attachment is embedded inline via `cid:` when it is
/// [`Disposition::Inline`] and carries a `content_id`, and as a regular
/// attachment otherwise.
///
/// A fresh `Message-ID` and the current time are generated on every build —
/// a draft may be edited and resent, and each attempt is its own message.
pub fn build(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
) -> BuiltMessage {
    let message_id = generate_message_id(identity.address.domain().unwrap_or(FALLBACK_DOMAIN));

    let mut builder = MessageBuilder::new()
        .message_id(message_id.without_brackets().to_owned())
        .date(MbDate::new(Utc::now().timestamp()))
        .from(mb_address(&identity.address))
        .reply_to(mb_address(identity.effective_reply_to()));

    if !draft.subject.is_empty() {
        builder = builder.subject(draft.subject.clone());
    }
    if let Some(to) = recipient_list(&draft.to) {
        builder = builder.to(to);
    }
    if let Some(cc) = recipient_list(&draft.cc) {
        builder = builder.cc(cc);
    }
    if let Some(bcc) = recipient_list(&draft.bcc) {
        builder = builder.bcc(bcc);
    }
    if let Some(text) = &draft.body.text {
        builder = builder.text_body(text.clone());
    }
    if let Some(html) = &draft.body.html {
        builder = builder.html_body(html.clone());
    }

    for outgoing in attachments {
        builder = add_attachment(builder, outgoing);
    }

    let raw = builder
        .write_to_vec()
        .expect("writing RFC 5322 bytes to a Vec<u8> cannot fail");
    BuiltMessage { raw, message_id }
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

/// A non-empty address list, or `None` — mail-builder writes a header even for
/// an empty list, and a message has no reason to carry an empty `Cc`.
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

/// A fresh, unique `Message-ID` under `domain`.
///
/// Built from [`make_boundary`], the same pseudo-unique token mail-builder
/// uses for MIME boundaries, rather than pulling in a UUID dependency or the
/// hostname of this machine — which the `Message-ID` `mail-builder` would
/// generate on its own carries, and which CLAUDE.md's "nothing leaves this
/// machine that the user did not ask for" says a header on every outgoing
/// message is not the place for.
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
        let built = build(&draft(), &ada, &[]);

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
    fn each_build_gets_a_fresh_message_id() {
        let ada = identity("ada@example.com");
        let first = build(&draft(), &ada, &[]);
        let second = build(&draft(), &ada, &[]);
        assert_ne!(first.message_id, second.message_id);
    }

    #[test]
    fn an_explicit_reply_to_and_bcc_come_through() {
        let mut ada = identity("ada@example.com");
        ada.reply_to = Some(EmailAddress::new(None::<String>, "replies@example.org"));
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let parsed = mime::parse(&build(&draft, &ada, &[]).raw);
        assert_eq!(parsed.reply_to[0].address, "replies@example.org");
        assert_eq!(parsed.bcc[0].address, "quiet@example.com");
    }

    #[test]
    fn non_ascii_subjects_and_display_names_survive() {
        let ada = identity("jurgen@example.com");
        let mut draft = draft();
        draft.subject = "Gruß aus München".to_owned();
        draft.to = vec![EmailAddress::new(Some("田中 陽子"), "yoko@example.net")];

        let parsed = mime::parse(&build(&draft, &ada, &[]).raw);
        assert_eq!(parsed.subject.as_deref(), Some("Gruß aus München"));
        assert_eq!(parsed.to[0].name.as_deref(), Some("田中 陽子"));
    }

    #[test]
    fn text_and_html_bodies_both_survive_as_multipart_alternative() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.body.html = Some("<p>Looking now.</p>".to_owned());

        let parsed = mime::parse(&build(&draft, &ada, &[]).raw);
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

        let parsed = mime::parse(&build(&draft, &ada, &[]).raw);
        assert!(parsed.subject.is_none());
    }
}
