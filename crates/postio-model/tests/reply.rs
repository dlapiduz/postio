//! Reply/reply-all/forward threading, against the corpus's awkward cases.
//!
//! postio-p8q's whole point: a wrong `In-Reply-To` or a dropped `References`
//! entry breaks the conversation in *somebody else's* mail client, silently,
//! which is exactly what the `list-thread-*` fixtures and `broken-references`
//! exist to catch before that happens for real. Each test here builds a reply
//! to a fixture with [`reply::reply`], builds the outgoing message with
//! [`outgoing::build`] as if it were about to be sent, and re-parses it —
//! proving the headers as they would actually appear on the wire, not just
//! the intermediate `Draft`.

#![cfg(feature = "test-corpus")]

use postio_model::account::{Account, Identity};
use postio_model::address::EmailAddress;
use postio_model::ids::{AccountId, IdentityId};
use postio_model::{RfcMessageId, mime, outgoing, reply, test_corpus};

fn account() -> Account {
    let mut account = Account::new(
        "Test",
        EmailAddress::new(None::<String>, "reader@example.com"),
    );
    account.id = AccountId::new(1);
    let mut identity = Identity::new(
        account.id,
        EmailAddress::new(None::<String>, "reader@example.com"),
    );
    identity.id = IdentityId::new(1);
    identity.is_default = true;
    account.identities = vec![identity.clone()];
    account
}

fn built_reply(
    fixture: &str,
) -> (
    mime::ParsedMessage,
    postio_model::Message,
    postio_model::Draft,
) {
    let account = account();
    let identity = account.identities[0].clone();
    let source = test_corpus::load(fixture).parse();
    let draft = reply::reply(&source, &account);
    let built = outgoing::build(&draft, &identity, &[], Some(&source));
    (mime::parse(&built.raw), source, draft)
}

fn id(raw: &str) -> RfcMessageId {
    RfcMessageId::new(raw)
}

#[test]
fn replying_to_a_thread_root_threads_with_just_the_root() {
    let (sent, source, _draft) = built_reply("list-thread-01-root");

    assert_eq!(sent.in_reply_to, source.rfc_message_id);
    assert_eq!(
        sent.references,
        vec![id("<harbour-dev.20260302T081200.a1@lists.example.org>")]
    );
}

#[test]
fn replying_to_a_deep_reply_carries_the_whole_chain_forward() {
    let (sent, source, _draft) = built_reply("list-thread-04-reply-deep");

    assert_eq!(sent.in_reply_to, source.rfc_message_id);
    assert_eq!(
        sent.references,
        vec![
            id("<harbour-dev.20260302T081200.a1@lists.example.org>"),
            id("<harbour-dev.20260302T093345.b2@lists.example.org>"),
            id("<harbour-dev.20260302T114930.d4@lists.example.org>"),
        ],
        "the parent's own References, then its In-Reply-To, then the parent itself"
    );
}

#[test]
fn replying_to_a_message_with_in_reply_to_but_no_references_still_threads() {
    let (sent, source, _draft) = built_reply("list-thread-05-reply-no-references");

    assert_eq!(sent.in_reply_to, source.rfc_message_id);
    assert_eq!(
        sent.references,
        vec![
            id("<harbour-dev.20260302T114930.d4@lists.example.org>"),
            id("<harbour-dev.20260302T131500.e5@lists.example.org>"),
        ],
        "the parent's own In-Reply-To carries forward even with no References header at all"
    );
}

#[test]
fn replying_to_a_message_with_neither_in_reply_to_nor_references_still_threads() {
    let (sent, source, _draft) = built_reply("list-thread-06-reply-subject-only");

    assert_eq!(sent.in_reply_to, source.rfc_message_id);
    assert_eq!(
        sent.references,
        vec![id("<harbour-dev.20260303T072211.f6@lists.example.org>")],
        "with nothing to inherit, References is just the parent itself"
    );
}

#[test]
fn replying_to_a_message_with_broken_references_threads_on_whatever_survived_parsing() {
    let (sent, source, _draft) = built_reply("broken-references");

    assert_eq!(
        sent.in_reply_to, source.rfc_message_id,
        "our own In-Reply-To is the parent we are actually answering, \
         not the dangling id the parent's own In-Reply-To named"
    );
    assert_eq!(
        sent.references,
        vec![
            id("<harbour-dev.20260302T081200.a1@lists.example.org>"),
            id("<harbour-dev.20260302T093345.b2@lists.example.org>"),
            id("<harbour-dev.20260302T093345.b2@lists.example.org>"),
            id("<dangling-parent@nowhere.example.invalid>"),
            id("<this-message-id-was-never-seen@nowhere.example.invalid>"),
            id("<20260304T084500.brokenrefs@example.net>"),
        ],
        "exactly what the parser salvaged from the parent, in order, plus the parent \
         -- even the unterminated angle-addr recovers, since the parser tolerates a \
         missing '>' the same way a strict reader of the wire format would not"
    );
    for broken in ["not-an-angle-addr-at-all", "<>"] {
        assert!(
            !sent
                .references
                .iter()
                .any(|reference| reference.as_str().contains(broken)),
            "a reference that never parsed on the way in must not reappear on the way out"
        );
    }
}

#[test]
fn replying_to_an_already_prefixed_subject_does_not_stack_re() {
    let (sent, _source, _draft) = built_reply("list-thread-02-reply");

    let subject = sent.subject.expect("a subject");
    assert_eq!(
        subject.matches("Re:").count(),
        1,
        "replying to a subject that is already a reply must not add a second Re:: {subject:?}"
    );
}

#[test]
fn forwarding_carries_no_threading_headers_at_all() {
    let account = account();
    let identity = account.identities[0].clone();
    let source = test_corpus::load("list-thread-04-reply-deep").parse();

    let draft = reply::forward(&source, &account);
    assert!(draft.in_reply_to.is_none());

    // A forward starts a new conversation, so nothing is passed as the parent.
    let built = outgoing::build(&draft, &identity, &[], None);
    let sent = mime::parse(&built.raw);
    assert!(sent.in_reply_to.is_none());
    assert!(sent.references.is_empty());
}
