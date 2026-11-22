//! Builds a reply, reply-all or forward `Draft`. A wrong `In-Reply-To` is noticed only by the
//! person on the other end, whose
//! client starts a new conversation. Signing is `Draft::start_as`'s, against the text already
//! quoted here, so quoting first never doubles up.

use crate::account::Account;
use crate::address::{self, EmailAddress};
use crate::attachment::Attachment;
use crate::draft::{Draft, DraftKind};
use crate::ids::AttachmentId;
use crate::message::{Message, MessageBody};
use crate::subject;

/// The quote arrives rather than being computed (ADR 0003 Q3): rich quoting parses untrusted markup
/// in `postio-body`,
/// which depends on this crate. A caller with nothing better uses `plain_quote`.
pub fn reply(source: &Message, account: &Account, quote: MessageBody) -> Draft {
    build_reply(source, account, false, quote)
}

/// Never includes any of `account`'s own identities, nor the same address twice across `To` and
/// `Cc`.
pub fn reply_all(source: &Message, account: &Account, quote: MessageBody) -> Draft {
    build_reply(source, account, true, quote)
}

pub fn forward(source: &Message, account: &Account, body: MessageBody) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.kind = DraftKind::Forward;
    draft.subject = subject::forward_subject(source.subject.as_deref().unwrap_or_default());
    draft.body = body;
    draft.attachments = carried_attachments(source);
    if source.is_persisted() {
        draft.forwarded_from = Some(source.id);
    }

    if let Some(identity) = account.identity_for(&recipients_of(source)) {
        draft.start_as(identity);
    }
    draft
}

fn build_reply(source: &Message, account: &Account, all: bool, quote: MessageBody) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.kind = if all {
        DraftKind::ReplyAll
    } else {
        DraftKind::Reply
    };
    if source.is_persisted() {
        draft.in_reply_to = Some(source.id);
    }
    draft.thread_id = source.thread_id;
    draft.subject = subject::reply_subject(source.subject.as_deref().unwrap_or_default());

    let (to, cc) = reply_recipients(source, account, all);
    draft.to = to;
    draft.cc = cc;
    draft.body = quote;

    if let Some(identity) = account.identity_for(&recipients_of(source)) {
        draft.start_as(identity);
    }
    draft
}

fn recipients_of(source: &Message) -> Vec<EmailAddress> {
    source.all_recipients().cloned().collect()
}

/// `To` is `source.reply_to` when set (the point of the header), else `source.from`, or `source.to`
/// on a message `account` sent itself.
/// `Cc` (reply-all only) is every other original recipient.
fn reply_recipients(
    source: &Message,
    account: &Account,
    all: bool,
) -> (Vec<EmailAddress>, Vec<EmailAddress>) {
    let answered: &[EmailAddress] = if !source.reply_to.is_empty() {
        &source.reply_to
    } else {
        &source.from
    };
    let primary: &[EmailAddress] = if answered.iter().all(|address| account.owns_address(address)) {
        &source.to
    } else {
        answered
    };

    let mut claimed: Vec<EmailAddress> = Vec::new();
    let to = exclude_self_and_seen(primary, account, &mut claimed);

    let cc = if all {
        let others: Vec<EmailAddress> = source.to.iter().chain(&source.cc).cloned().collect();
        exclude_self_and_seen(&others, account, &mut claimed)
    } else {
        Vec::new()
    };

    (to, cc)
}

/// `claimed` is extended with everything kept, so a second call (reply-all's `Cc` after its `To`)
/// dedupes against the first too.
fn exclude_self_and_seen(
    candidates: &[EmailAddress],
    account: &Account,
    claimed: &mut Vec<EmailAddress>,
) -> Vec<EmailAddress> {
    let mut kept = Vec::new();
    for candidate in candidates {
        if account.owns_address(candidate) {
            continue;
        }
        if claimed
            .iter()
            .any(|already| already.same_address(candidate))
        {
            continue;
        }
        claimed.push(candidate.clone());
        kept.push(candidate.clone());
    }
    kept
}

/// Leads with a blank line so the caret lands above the quote. Every line is prefixed `> `, so an
/// original signature's `-- ` becomes
/// `> -- ` and `signature::split` never mistakes it for this draft's. Plain text only: markup needs
/// `postio-body`, which depends on this crate.
fn quote_body(source: &Message) -> String {
    let attribution = attribution(source);
    match source.body.text.as_deref() {
        Some(text) if !text.is_empty() => format!("\n\n{attribution}\n{}", quote_lines(text)),
        _ => format!("\n\n{attribution}\n"),
    }
}

/// Public because the rich quote is built above this crate and both forms must say the same thing.
pub fn attribution(source: &Message) -> String {
    format!(
        "On {}, {} wrote:",
        source.best_date().format("%Y-%m-%d"),
        source
            .primary_from()
            .map(EmailAddress::display)
            .unwrap_or("someone")
    )
}

pub fn forward_header(source: &Message) -> Vec<String> {
    let from = source
        .primary_from()
        .map(EmailAddress::to_string)
        .unwrap_or_default();
    vec![
        "---------- Forwarded message ----------".to_owned(),
        format!("From: {from}"),
        format!("Date: {}", source.best_date().format("%Y-%m-%d %H:%M")),
        format!("Subject: {}", source.subject.as_deref().unwrap_or_default()),
        format!("To: {}", address::format_list(&source.to)),
    ]
}

pub fn plain_quote(source: &Message) -> MessageBody {
    MessageBody {
        text: Some(quote_body(source)),
        html: None,
    }
}

pub fn plain_forward(source: &Message) -> MessageBody {
    MessageBody {
        text: Some(forward_body(source)),
        html: None,
    }
}

fn quote_lines(text: &str) -> String {
    text.lines()
        .map(|line| {
            if line.is_empty() {
                ">".to_owned()
            } else {
                format!("> {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The original text verbatim, not quote-prefixed: a forward presents the whole message.
fn forward_body(source: &Message) -> String {
    let header = forward_header(source).join("\n");
    let body = source.body.text.as_deref().unwrap_or_default();
    format!("\n\n{header}\n\n{body}")
}

/// `id` and `message_id` are reset to unassigned: `write_attachments` decides insert-or-update by
/// whether `id` is assigned, so a
/// copied id would corrupt the row it was borrowed from. `blob_id` is kept when present; otherwise
/// `part_id` and `Draft::forwarded_from` say where to fetch from (#1686).
pub fn carried_attachments(source: &Message) -> Vec<Attachment> {
    source
        .attachments
        .iter()
        .cloned()
        .map(|mut attachment| {
            attachment.id = AttachmentId::UNASSIGNED;
            attachment.message_id = crate::ids::MessageId::UNASSIGNED;
            attachment
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Identity;
    use crate::ids::{AccountId, IdentityId, MailboxId, MessageId};
    use crate::message::MessageBody;
    use chrono::{TimeZone, Utc};

    fn account(address: &str) -> Account {
        let mut account = Account::new("Test", EmailAddress::new(None::<String>, address));
        account.id = AccountId::new(1);
        let mut identity = Identity::new(account.id, EmailAddress::new(None::<String>, address));
        identity.id = IdentityId::new(1);
        identity.is_default = true;
        account.identities = vec![identity];
        account
    }

    fn received_at() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap()
    }

    fn a_message() -> Message {
        let mut message = Message::new(AccountId::new(1), MailboxId::new(1), received_at());
        message.id = MessageId::new(42);
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
        message.subject = Some("Quarterly report".to_owned());
        message.body = MessageBody {
            text: Some("Please review the attached numbers.\nThanks.".to_owned()),
            html: None,
        };
        message
    }

    #[test]
    fn a_reply_goes_to_the_sender_with_a_re_subject_and_a_quote() {
        let source = a_message();
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(draft.kind, DraftKind::Reply);
        assert_eq!(draft.in_reply_to, Some(MessageId::new(42)));
        assert_eq!(draft.subject, "Re: Quarterly report");
        assert_eq!(
            draft.to,
            vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")]
        );
        assert!(draft.cc.is_empty(), "a plain reply does not add Cc");

        let body = draft.body.text.expect("a quoted body");
        assert!(body.contains("On 2026-03-01, Ada Lovelace wrote:"));
        assert!(body.contains("> Please review the attached numbers."));
        assert!(body.contains("> Thanks."));
        assert!(
            body.trim_start().starts_with("On 2026-03-01"),
            "nothing above the attribution but the caret's blank line: {body:?}"
        );
    }

    #[test]
    fn a_reply_prefers_reply_to_over_from() {
        let mut source = a_message();
        source.reply_to = vec![EmailAddress::new(None::<String>, "list@example.org")];
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(
            draft.to,
            vec![EmailAddress::new(None::<String>, "list@example.org")]
        );
    }

    #[test]
    fn replying_to_an_unpersisted_message_sets_no_in_reply_to() {
        let mut source = a_message();
        source.id = MessageId::UNASSIGNED;
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        assert!(draft.in_reply_to.is_none());
    }

    #[test]
    fn a_reply_carries_the_thread_id_so_the_composer_can_show_it_inline() {
        use crate::ids::ThreadId;

        let mut source = a_message();
        source.thread_id = Some(ThreadId::new(7));
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(draft.thread_id, Some(ThreadId::new(7)));
    }

    #[test]
    fn replying_twice_never_stacks_re_re() {
        let mut source = a_message();
        source.subject = Some("Re: Quarterly report".to_owned());
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(draft.subject, "Re: Quarterly report");
    }

    #[test]
    fn the_from_identity_is_whichever_address_the_mail_arrived_at() {
        let mut work = account("grace@example.com");
        let mut personal = Identity::new(
            work.id,
            EmailAddress::new(None::<String>, "grace+personal@example.com"),
        );
        personal.id = IdentityId::new(2);
        work.identities.push(personal.clone());

        let mut source = a_message();
        source.to = vec![personal.address.clone()];

        let draft = reply(&source, &work, plain_quote(&source));
        assert_eq!(draft.identity_id, Some(IdentityId::new(2)));
    }

    #[test]
    fn a_reply_with_no_fetched_body_still_gets_an_attribution() {
        let mut source = a_message();
        source.body = MessageBody::default();
        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));

        let body = draft.body.text.expect("still a body");
        assert!(body.contains("On 2026-03-01, Ada Lovelace wrote:"));
        assert!(
            !body.contains('>'),
            "nothing to quote, so nothing is quoted"
        );
    }

    #[test]
    fn a_reply_to_your_own_message_goes_to_who_you_wrote_to() {
        let mut source = a_message();
        source.from = vec![EmailAddress::new(None::<String>, "grace@example.com")];
        source.to = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        source.cc = vec![EmailAddress::new(None::<String>, "turing@example.org")];

        let draft = reply(&source, &account("grace@example.com"), plain_quote(&source));
        assert_eq!(
            draft.to,
            vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")],
            "answering your own message leaves nobody to send to"
        );
        assert!(draft.cc.is_empty());

        let draft = reply_all(&source, &account("grace@example.com"), plain_quote(&source));
        assert_eq!(draft.to.len(), 1);
        assert_eq!(
            draft.cc,
            vec![EmailAddress::new(None::<String>, "turing@example.org")]
        );
    }

    #[test]
    fn reply_all_ccs_the_other_recipients_but_never_our_own_identities() {
        let mut source = a_message();
        source.to = vec![
            EmailAddress::new(None::<String>, "grace@example.com"),
            EmailAddress::new(None::<String>, "hopper@example.net"),
        ];
        source.cc = vec![EmailAddress::new(None::<String>, "turing@example.org")];

        let draft = reply_all(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(
            draft.to,
            vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")]
        );
        assert_eq!(
            draft.cc,
            vec![
                EmailAddress::new(None::<String>, "hopper@example.net"),
                EmailAddress::new(None::<String>, "turing@example.org"),
            ],
            "every other original recipient, minus ourselves"
        );
        assert!(
            draft.all_recipients().all(|address| !address
                .same_address(&EmailAddress::new(None::<String>, "grace@example.com"))),
            "reply-all never includes the sender's own address"
        );
    }

    #[test]
    fn reply_all_drops_an_alias_of_ours_as_readily_as_the_primary_address() {
        let mut account = account("grace@example.com");
        let mut alias = Identity::new(
            account.id,
            EmailAddress::new(Some("Grace Hopper"), "g.hopper@example.net"),
        );
        alias.id = IdentityId::new(2);
        account.identities.push(alias);

        let mut source = a_message();
        source.to = vec![
            EmailAddress::new(None::<String>, "turing@example.org"),
            EmailAddress::new(None::<String>, "G.Hopper@Example.NET"),
        ];

        let draft = reply_all(&source, &account, plain_quote(&source));

        assert_eq!(
            draft.cc,
            vec![EmailAddress::new(None::<String>, "turing@example.org")],
            "an address this account sends as is ours however it is cased, \
             and replying to all must not put us on our own reply"
        );
        assert!(
            draft.all_recipients().all(|address| !address
                .same_address(&EmailAddress::new(None::<String>, "g.hopper@example.net"))),
            "the alias survived into the recipients somewhere other than Cc"
        );
    }

    #[test]
    fn reply_all_never_lists_the_same_address_in_to_and_cc() {
        let mut source = a_message();
        source.cc = vec![EmailAddress::new(Some("Ada"), "ADA@EXAMPLE.COM")];

        let draft = reply_all(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(draft.to.len(), 1);
        assert!(
            draft.cc.is_empty(),
            "the same mailbox, case-insensitively, is not a second recipient"
        );
    }

    #[test]
    fn a_forward_has_no_recipients_yet_and_a_fwd_subject() {
        let source = a_message();
        let draft = forward(
            &source,
            &account("grace@example.com"),
            plain_forward(&source),
        );

        assert_eq!(draft.kind, DraftKind::Forward);
        assert!(draft.to.is_empty() && draft.cc.is_empty());
        assert_eq!(draft.subject, "Fwd: Quarterly report");
        assert!(
            draft.in_reply_to.is_none(),
            "a forward starts a new conversation"
        );
        assert!(draft.thread_id.is_none());
    }

    #[test]
    fn a_forward_carries_the_original_headers_and_body_unquoted() {
        let source = a_message();
        let body = forward(
            &source,
            &account("grace@example.com"),
            plain_forward(&source),
        )
        .body
        .text
        .expect("a forward body");

        assert!(body.contains("From: Ada Lovelace <ada@example.com>"));
        assert!(body.contains("Subject: Quarterly report"));
        assert!(body.contains("To: grace@example.com"));
        assert!(
            body.contains("Please review the attached numbers.")
                && !body.contains("> Please review"),
            "forwarded content is not quote-prefixed: {body:?}"
        );
    }

    #[test]
    fn a_forward_carries_attachments_with_fresh_local_ids() {
        let mut source = a_message();
        let mut attachment = Attachment::new(MessageId::new(42), "application/pdf", 1024);
        attachment.id = AttachmentId::new(9);
        attachment.filename = Some("numbers.pdf".to_owned());
        attachment.blob_id = Some(crate::ids::BlobId::new("b".repeat(64)));
        source.attachments = vec![attachment];

        let draft = forward(
            &source,
            &account("grace@example.com"),
            plain_forward(&source),
        );

        assert_eq!(draft.attachments.len(), 1);
        let carried = &draft.attachments[0];
        assert_eq!(carried.filename.as_deref(), Some("numbers.pdf"));
        assert!(
            carried.blob_id.is_some(),
            "the bytes do not need re-uploading"
        );
        assert!(
            !carried.id.is_assigned(),
            "a copy of another message's attachment row is not that row"
        );
        assert!(!carried.message_id.is_assigned());
    }

    #[test]
    fn a_forward_remembers_the_message_its_attachments_come_from() {
        let mut source = a_message();
        let mut attachment = Attachment::new(MessageId::new(42), "application/pdf", 1024);
        attachment.part_id = Some("2".to_owned());
        source.attachments = vec![attachment];

        let draft = forward(
            &source,
            &account("grace@example.com"),
            plain_forward(&source),
        );

        assert_eq!(draft.forwarded_from, Some(MessageId::new(42)));
        assert_eq!(
            draft.attachments[0].part_id.as_deref(),
            Some("2"),
            "the section is what names the part inside the original"
        );
        assert!(
            reply(&source, &account("grace@example.com"), plain_quote(&source))
                .forwarded_from
                .is_none(),
            "a reply carries nothing from its parent"
        );
    }

    #[test]
    fn a_forward_carries_an_inline_image_as_an_inline_part() {
        // An inline image is referenced by `Content-ID`: a carry that filtered on
        // `Disposition::Attachment` or reassigned content ids would break every picture.
        let mut source = a_message();
        let mut logo = Attachment::new(MessageId::new(42), "image/png", 2048);
        logo.id = AttachmentId::new(9);
        logo.filename = Some("logo.png".to_owned());
        logo.content_id = Some("logo@example.invalid".to_owned());
        logo.disposition = crate::attachment::Disposition::Inline;
        logo.blob_id = Some(crate::ids::BlobId::new("c".repeat(64)));
        source.attachments = vec![logo];

        let draft = forward(
            &source,
            &account("grace@example.com"),
            plain_forward(&source),
        );

        let carried = draft
            .attachments
            .first()
            .expect("the inline image was dropped from the forward");
        assert_eq!(
            carried.content_id.as_deref(),
            Some("logo@example.invalid"),
            "the Content-ID was rewritten, so the body's `cid:` reference now \
             points at nothing"
        );
        assert_eq!(
            carried.disposition,
            crate::attachment::Disposition::Inline,
            "an inline part carried as an ordinary attachment shows up as a \
             file to download where a picture used to be"
        );
        assert!(
            carried.blob_id.is_some(),
            "the bytes do not need re-uploading"
        );
        assert!(
            !carried.id.is_assigned(),
            "a copy of another message's attachment row is not that row"
        );
    }
}
