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
//! # Threading
//!
//! `In-Reply-To` and `References` need the RFC `Message-ID` chain of the
//! message being replied to, which is not part of a [`Draft`] — a `Draft`
//! only carries the *local* id of that message ([`Draft::in_reply_to`]), so
//! [`build`] takes the parent [`Message`] itself and computes both headers
//! from [`Message::reference_chain`]. [`crate::reply`] is what builds the
//! reply `Draft` in the first place; this is only where its headers get
//! written.

use chrono::Utc;
use mail_builder::MessageBuilder;
use mail_builder::headers::address::Address as MbAddress;
use mail_builder::headers::date::Date as MbDate;
use mail_builder::headers::message_id::MessageId as MbMessageId;
use mail_builder::mime::make_boundary;

use crate::account::Identity;
use crate::address::EmailAddress;
use crate::attachment::{Attachment, Disposition};
use crate::draft::Draft;
use crate::ids::RfcMessageId;
use crate::message::Message;

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
/// `in_reply_to` is the message `draft` answers, when it answers one —
/// the same message [`Draft::in_reply_to`] names by local id, resolved by the
/// caller to the domain object this crate needs. Its `Message-ID` becomes
/// `In-Reply-To`, and its own chain plus that id becomes `References`; see
/// [`Message::reference_chain`] for why that is the whole rule. A parent with
/// no `Message-ID` of its own (the sender omitted one) contributes no
/// `In-Reply-To`, but its chain still becomes `References` if it has one.
///
/// A fresh `Message-ID` and the current time are generated on every build —
/// a draft may be edited and resent, and each attempt is its own message.
pub fn build(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
) -> BuiltMessage {
    assemble(draft, identity, attachments, in_reply_to, Bcc::Omit)
}

/// Builds `draft` into the copy filed in the account's **Drafts** mailbox.
///
/// Identical to [`build`] except for one header: this one carries `Bcc`.
///
/// The rule [`build`] exists to enforce — that a bcc'd address never travels
/// inside the message — is about the bytes handed to `DATA`, which reach every
/// `To` and `Cc` recipient as they stand. A copy in the user's own Drafts
/// folder reaches nobody. Dropping `Bcc` there would instead lose the
/// recipients the user typed: a draft picked up on their phone would look
/// finished and send to fewer people than they asked for, silently.
///
/// The two entry points are separate rather than a boolean because the wrong
/// answer to that boolean discloses a bcc'd address to every other recipient,
/// and that is not a mistake worth making reachable by passing `true`.
pub fn build_draft(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
) -> BuiltMessage {
    assemble(draft, identity, attachments, in_reply_to, Bcc::Include)
}

/// Whether the assembled message carries a `Bcc` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bcc {
    /// For bytes that go to a recipient. See [`build`].
    Omit,
    /// For the copy in the user's own Drafts folder. See [`build_draft`].
    Include,
}

fn assemble(
    draft: &Draft,
    identity: &Identity,
    attachments: &[OutgoingAttachment<'_>],
    in_reply_to: Option<&Message>,
    bcc: Bcc,
) -> BuiltMessage {
    let message_id = generate_message_id(identity.address.domain().unwrap_or(FALLBACK_DOMAIN));

    let mut builder = MessageBuilder::new()
        .message_id(message_id.without_brackets().to_owned())
        .date(MbDate::new(Utc::now().timestamp()))
        .from(mb_address(&identity.address))
        .reply_to(mb_address(identity.effective_reply_to()));

    if let Some(parent) = in_reply_to {
        builder = add_threading_headers(builder, parent);
    }

    if !draft.subject.is_empty() {
        builder = builder.subject(draft.subject.clone());
    }
    if let Some(to) = recipient_list(&draft.to) {
        builder = builder.to(to);
    }
    if let Some(cc) = recipient_list(&draft.cc) {
        builder = builder.cc(cc);
    }
    // Bcc is written only for the Drafts copy. Whatever bytes DATA carries go
    // to every envelope recipient as-is, so a Bcc header on an outgoing
    // message would hand every To/Cc recipient the bcc'd list — the opposite
    // of what Bcc means. A bcc'd address is still a `RCPT TO` the caller adds
    // to the envelope; it is just never inside a message anybody else
    // receives. See `postio-sync`'s send path, and `build_draft`.
    if bcc == Bcc::Include
        && let Some(addresses) = recipient_list(&draft.bcc)
    {
        builder = builder.bcc(addresses);
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

/// Sets `In-Reply-To` and `References` from `parent`.
///
/// `References` is exactly what a message threading *into* `parent` would
/// have claimed as its own ancestors: `parent`'s [`Message::reference_chain`]
/// — its own `References` plus its `In-Reply-To`, deduplicated the same way —
/// with `parent`'s `Message-ID` appended at the end. That is the RFC 5322
/// rule stated the other way around, and it is why threading a reply here
/// costs nothing beyond what parsing already computed for `parent`.
fn add_threading_headers<'x>(builder: MessageBuilder<'x>, parent: &Message) -> MessageBuilder<'x> {
    let mut references: Vec<String> = parent
        .reference_chain()
        .map(|id| id.without_brackets().to_owned())
        .collect();

    let mut builder = builder;
    if let Some(parent_id) = &parent.rfc_message_id {
        builder = builder.in_reply_to(parent_id.without_brackets().to_owned());
        references.push(parent_id.without_brackets().to_owned());
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
        let built = build(&draft(), &ada, &[], None);

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
        let first = build(&draft(), &ada, &[], None);
        let second = build(&draft(), &ada, &[], None);
        assert_ne!(first.message_id, second.message_id);
    }

    #[test]
    fn an_explicit_reply_to_comes_through() {
        let mut ada = identity("ada@example.com");
        ada.reply_to = Some(EmailAddress::new(None::<String>, "replies@example.org"));

        let parsed = mime::parse(&build(&draft(), &ada, &[], None).raw);
        assert_eq!(parsed.reply_to[0].address, "replies@example.org");
    }

    #[test]
    fn bcc_recipients_never_appear_in_the_sent_bytes() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let built = build(&draft, &ada, &[], None);
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
    fn the_drafts_copy_keeps_the_bcc_the_sent_bytes_drop() {
        // The Drafts folder is the user's own mailbox and reaches nobody. A
        // draft picked up on another client with its Bcc silently gone would
        // look finished and send to fewer people than the user asked for.
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.com")];

        let filed = mime::parse(&build_draft(&draft, &ada, &[], None).raw);
        let sent = mime::parse(&build(&draft, &ada, &[], None).raw);

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

        let parsed = mime::parse(&build(&draft, &ada, &[], None).raw);
        assert_eq!(parsed.subject.as_deref(), Some("Gruß aus München"));
        assert_eq!(parsed.to[0].name.as_deref(), Some("田中 陽子"));
    }

    #[test]
    fn text_and_html_bodies_both_survive_as_multipart_alternative() {
        let ada = identity("ada@example.com");
        let mut draft = draft();
        draft.body.html = Some("<p>Looking now.</p>".to_owned());

        let parsed = mime::parse(&build(&draft, &ada, &[], None).raw);
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

        let parsed = mime::parse(&build(&draft, &ada, &[], None).raw);
        assert!(parsed.subject.is_none());
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

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&root)).raw);
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

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&middle)).raw);
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

        let parsed = mime::parse(&build(&draft(), &ada, &[], Some(&orphan)).raw);
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

        let parsed = mime::parse(&build(&draft(), &ada, &[], None).raw);
        assert!(parsed.in_reply_to.is_none());
        assert!(parsed.references.is_empty());
    }
}
