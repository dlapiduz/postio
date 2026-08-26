//! Building a reply, reply-all or forward [`Draft`] from an existing
//! [`Message`].
//!
//! # Why this matters more than it looks like it should
//!
//! Nobody inside Postio notices a wrong `In-Reply-To` or a dropped `Re:` —
//! only the person on the other end, whose client silently starts a new
//! conversation instead of threading the reply. `outgoing::build` writes the
//! RFC 5322 headers once it has a parent [`Message`] to read them from; this
//! module is what decides *which* message that is and what the reply looks
//! like before it gets there: `draft.in_reply_to` names it by local id, and
//! `draft.thread_id` is copied across so the composer can show the reply
//! inline in the conversation it belongs to.
//!
//! # What is not here
//!
//! Nothing here sends anything or resolves an identity's signature bytes —
//! [`Draft::use_identity`] already does the signature, and does it correctly
//! against text this module has already quoted (see its doc for why quoting
//! first and signing after never doubles up). Attaching this to the reading
//! pane, and deciding which `Message` a reply's `in_reply_to` local id points
//! at when it is time to send, are the composer's and `postio-sync`'s jobs.

use crate::account::Account;
use crate::address::{self, EmailAddress};
use crate::attachment::Attachment;
use crate::draft::{Draft, DraftKind};
use crate::ids::AttachmentId;
use crate::message::{Message, MessageBody};
use crate::subject;

/// Builds a reply to `source`'s sender only, starting from `quote`.
///
/// The quote arrives rather than being computed — ADR 0003 Q3's inversion.
/// Quoting rich content means parsing untrusted markup, the parser lives in
/// `postio-body`, and that crate depends on this one; a caller with both
/// builds the quote (`postio_body::quoted_reply`) and hands it down. A
/// caller with nothing better uses [`plain_quote`].
pub fn reply(source: &Message, account: &Account, quote: MessageBody) -> Draft {
    build_reply(source, account, false, quote)
}

/// Builds a reply to `source`'s sender and every other original recipient.
///
/// Never includes any of `account`'s own identities, and never lists the same
/// address twice across `To` and `Cc`. See [`reply`] for where `quote` comes
/// from.
pub fn reply_all(source: &Message, account: &Account, quote: MessageBody) -> Draft {
    build_reply(source, account, true, quote)
}

/// Builds a forward of `source`: a blank set of recipients for the user to
/// fill in, `body` — the original content below a forwarding header block,
/// built by the caller as [`reply`] describes or by [`plain_forward`] — and
/// its attachments carried over.
pub fn forward(source: &Message, account: &Account, body: MessageBody) -> Draft {
    let mut draft = Draft::new(account.id);
    draft.kind = DraftKind::Forward;
    draft.subject = subject::forward_subject(source.subject.as_deref().unwrap_or_default());
    draft.body = body;
    draft.attachments = carried_attachments(source);

    if let Some(identity) = account.identity_for(&recipients_of(source)) {
        draft.use_identity(identity);
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
        draft.use_identity(identity);
    }
    draft
}

/// The recipients `source` was addressed to, for
/// [`Account::identity_for`] — which of our addresses this reply or forward
/// should come from.
fn recipients_of(source: &Message) -> Vec<EmailAddress> {
    source.all_recipients().cloned().collect()
}

/// `To` and `Cc` for a reply, excluding `account`'s own identities and
/// without listing the same address twice.
///
/// `To` answers whoever should see the reply at all: `source.reply_to` when
/// the sender set one, since that is the whole point of the header, else
/// `source.from`. `Cc` — only for reply-all — is every other original
/// recipient, so the rest of the conversation stays on it.
fn reply_recipients(
    source: &Message,
    account: &Account,
    all: bool,
) -> (Vec<EmailAddress>, Vec<EmailAddress>) {
    let primary: &[EmailAddress] = if !source.reply_to.is_empty() {
        &source.reply_to
    } else {
        &source.from
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

/// `candidates`, minus `account`'s own addresses and anything already in
/// `claimed` — which is extended with everything kept, so a second call
/// (reply-all's `Cc` after its `To`) dedupes against the first as well as
/// itself.
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

/// The quoted body a reply starts from: an attribution line, then `source`'s
/// text with every line prefixed `> `.
///
/// Leads with a blank line so the composer's caret lands above the quote, and
/// leaves the signature separator alone: quoting prefixes every line of
/// `source`'s body with `> `, so an original signature's own `-- ` becomes
/// `> -- ` and [`crate::signature::split`] never mistakes it for this draft's.
///
/// Quoting is plain text, and this crate can only do plain text: turning
/// markup into text needs a parser, `postio-body` is where that lives, and
/// `postio-body` depends on *this* crate — so reaching for it here would be a
/// cycle. A caller that has both converts first and hands the text down;
/// `postio_gtk::composer::quotable` is the one that does. Before it existed,
/// an HTML-only message quoted as an attribution line with nothing under it.
fn quote_body(source: &Message) -> String {
    let attribution = attribution(source);
    match source.body.text.as_deref() {
        Some(text) if !text.is_empty() => format!("\n\n{attribution}\n{}", quote_lines(text)),
        _ => format!("\n\n{attribution}\n"),
    }
}

/// The line that introduces a quote: `On 2026-08-26, Ada Lovelace wrote:`.
///
/// Public because the rich quote is built above this crate (see [`reply`])
/// and both forms must say the same thing.
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

/// The conventional header block a forward opens with, one line per entry.
///
/// Public for the same reason as [`attribution`].
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

/// Today's plain-text quote as a [`MessageBody`] — the fallback for a caller
/// that cannot build a rich one.
pub fn plain_quote(source: &Message) -> MessageBody {
    MessageBody {
        text: Some(quote_body(source)),
        html: None,
    }
}

/// The plain-text forward body, as [`plain_quote`] is to [`reply`].
pub fn plain_forward(source: &Message) -> MessageBody {
    MessageBody {
        text: Some(forward_body(source)),
        html: None,
    }
}

/// Prefixes every line of `text` with `> `, `>` alone for a blank line so no
/// trailing whitespace is invented.
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

/// The body a forward starts from: a conventional header block naming who it
/// was from, when, and to, followed by the original text verbatim — not
/// quote-prefixed, since a forward presents the whole message rather than
/// answering a fragment of it.
fn forward_body(source: &Message) -> String {
    let header = forward_header(source).join("\n");
    let body = source.body.text.as_deref().unwrap_or_default();
    format!("\n\n{header}\n\n{body}")
}

/// `source`'s attachments, ready to belong to a draft rather than a sent
/// message: [`Attachment::id`] and [`Attachment::message_id`] reset to
/// unassigned, since a copy of somebody else's attachment row is not that
/// row — [`postio_storage`](../../postio_storage/index.html)'s
/// `write_attachments` decides insert-or-update by whether `id` is already
/// assigned, and forwarding one message's attachment id into another
/// message's row would corrupt the one it was borrowed from. `blob_id` is
/// kept: the bytes are already in the blob store and sending does not need to
/// re-upload them.
fn carried_attachments(source: &Message) -> Vec<Attachment> {
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

    // -----------------------------------------------------------------------
    // Reply
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // Reply-all
    // -----------------------------------------------------------------------

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
    fn reply_all_never_lists_the_same_address_in_to_and_cc() {
        let mut source = a_message();
        // The sender's own address turns up again in Cc, as a "reply to all"
        // header block from a real client sometimes has.
        source.cc = vec![EmailAddress::new(Some("Ada"), "ADA@EXAMPLE.COM")];

        let draft = reply_all(&source, &account("grace@example.com"), plain_quote(&source));

        assert_eq!(draft.to.len(), 1);
        assert!(
            draft.cc.is_empty(),
            "the same mailbox, case-insensitively, is not a second recipient"
        );
    }

    // -----------------------------------------------------------------------
    // Forward
    // -----------------------------------------------------------------------

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
}
