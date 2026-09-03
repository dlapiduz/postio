//! The RFC 5322 audit, made executable (#462).
//!
//! `docs/rfc-compliance.md` states the verdicts; this is the half a machine
//! can check. Every case here is one line of that document, so a verdict that
//! stops being true fails a test rather than quietly becoming a wrong
//! sentence in a file nobody re-reads.
//!
//! **What is not here.** The one gap the audit found — a decoded header value
//! carrying CR/LF reaching the generator, which turns the rest of that value
//! into headers — has no test yet on purpose: a test asserting the bug is
//! worthless and a test asserting the fix cannot pass before the fix. The
//! *input* half is here (a header value really can arrive with CR/LF in it),
//! the corpus carries `encoded-word-crlf-in-header.eml`, and the outgoing
//! half lands with the fix. See the document.

use postio_model::address::EmailAddress;
use postio_model::{AccountId, Draft, Identity, MailboxId, Message, RfcMessageId, mime, outgoing};

fn identity() -> Identity {
    Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
    )
}

fn draft(subject: &str, to: Vec<EmailAddress>) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.subject = subject.to_owned();
    draft.to = to;
    draft.body.text = Some("body\r\n".to_owned());
    draft
}

fn wire_lines(raw: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(raw)
        .split("\r\n")
        .map(str::to_owned)
        .collect()
}

// ---------------------------------------------------------------------------
// §2.1.1 — line length
// ---------------------------------------------------------------------------

#[test]
fn no_generated_line_exceeds_the_998_octet_limit_or_the_78_recommendation() {
    // 998 is a MUST and 78 a SHOULD, and the case that reaches either is a
    // long recipient list rather than anything a person types: forty
    // addresses is one team.
    let recipients: Vec<EmailAddress> = (0..40)
        .map(|n| {
            EmailAddress::new(
                Some(format!("Recipient Number {n:02}")),
                format!("recipient.number.{n:02}@rather-long-domain.example.com"),
            )
        })
        .collect();
    let built = outgoing::build(&draft("many", recipients), &identity(), &[], None);

    let longest = wire_lines(&built.raw)
        .into_iter()
        .max_by_key(String::len)
        .expect("a message has lines");
    assert!(
        longest.len() <= 78,
        "a generated line is {} octets, past the 78 recommendation: {longest:?}",
        longest.len()
    );

    assert_eq!(
        mime::parse(&built.raw).to.len(),
        40,
        "folding a long recipient list must not lose a recipient"
    );
}

#[test]
fn a_long_reference_chain_folds_and_survives_the_round_trip() {
    // The header that grows without anybody deciding to grow it: every reply
    // on a long list thread adds one. Losing an entry here splits somebody's
    // conversation, which is why this is checked by count rather than by eye.
    let mut parent = Message::new(AccountId::new(1), MailboxId::new(1), chrono::Utc::now());
    parent.rfc_message_id = Some(RfcMessageId::new("parent.0123456789@example.com"));
    parent.references = (0..25)
        .map(|n| {
            RfcMessageId::new(format!(
                "ancestor.{n:03}.0123456789abcdef@lists.example.com"
            ))
        })
        .collect();

    let reply = draft(
        "Re: chain",
        vec![EmailAddress::new(None::<String>, "a@example.com")],
    );
    let built = outgoing::build(&reply, &identity(), &[], Some(&parent));

    assert!(
        wire_lines(&built.raw).iter().all(|line| line.len() <= 78),
        "a long References chain was written as one over-long line"
    );

    let back = mime::parse(&built.raw);
    assert_eq!(
        back.references.len(),
        26,
        "the chain lost an ancestor on the way out or back: {:?}",
        back.references
    );
    assert_eq!(
        back.in_reply_to.as_ref().map(RfcMessageId::as_str),
        Some("<parent.0123456789@example.com>"),
        "In-Reply-To must be the parent's own id (§3.6.4)"
    );
    assert_eq!(
        back.references.last().map(RfcMessageId::as_str),
        Some("<parent.0123456789@example.com>"),
        "§3.6.4: the parent's id is appended to the parent's References"
    );
}

// ---------------------------------------------------------------------------
// §2.2.3 — folding and unfolding
// ---------------------------------------------------------------------------

#[test]
fn a_folded_header_unfolds_to_one_line_with_the_fold_as_a_single_space() {
    let raw = b"Subject: a subject the sender\r\n \
folded\r\n\tacross three lines\r\nFrom: ada@example.com\r\n\r\nbody\r\n";
    let parsed = mime::parse(raw);
    assert_eq!(
        parsed.subject.as_deref(),
        Some("a subject the sender folded across three lines")
    );
    assert_eq!(
        parsed.headers.get("Subject"),
        Some("a subject the sender folded across three lines"),
        "the raw header block the reader shows under `view source` unfolds too"
    );
}

#[test]
fn adjacent_encoded_words_join_without_the_whitespace_between_them() {
    // RFC 2047 §6.2, and the case a fold creates whether the sender meant it
    // or not: the whitespace separating two encoded words is not part of
    // either, so a decoder that keeps it inserts a space nobody sent.
    for raw in [
        &b"Subject: =?utf-8?Q?Hello?=\r\n =?utf-8?Q?World?=\r\nFrom: a@example.com\r\n\r\nx\r\n"[..],
        &b"Subject: =?utf-8?Q?Hello?= =?utf-8?Q?World?=\r\nFrom: a@example.com\r\n\r\nx\r\n"[..],
    ] {
        assert_eq!(mime::parse(raw).subject.as_deref(), Some("HelloWorld"));
    }
    // And ordinary text between them keeps its spaces, which is the other
    // half of the same rule.
    assert_eq!(
        mime::parse(
            b"Subject: =?utf-8?Q?Hello?= plain =?utf-8?Q?World?=\r\nFrom: a@example.com\r\n\r\nx\r\n"
        )
        .subject
        .as_deref(),
        Some("Hello plain World")
    );
}

// ---------------------------------------------------------------------------
// §3.4 — address syntax, and RFC 6854
// ---------------------------------------------------------------------------

#[test]
fn a_group_is_flattened_to_its_members_including_in_from() {
    // RFC 6854 permits a group in `From:`. Postio keeps people, not groups —
    // it shows and replies to addresses — and the group name survives in the
    // raw header block for anyone who wants it.
    let raw = b"From: Board: ada@example.com, grace@example.net;\r\n\
To: ada@example.com\r\nSubject: g\r\n\r\nx\r\n";
    let parsed = mime::parse(raw);
    assert_eq!(
        parsed.from,
        vec![
            EmailAddress::new(None::<String>, "ada@example.com"),
            EmailAddress::new(None::<String>, "grace@example.net"),
        ]
    );
    assert!(
        parsed
            .headers
            .get("From")
            .is_some_and(|value| value.contains("Board")),
        "the group name must still be readable in the raw header block"
    );
}

#[test]
fn an_empty_group_yields_no_recipients_and_keeps_its_header() {
    // `undisclosed-recipients:;` is a group with no members, and it is what a
    // bcc-only message carries. There is nobody to show, and the honest
    // answer is an empty list rather than an invented address.
    let raw = b"From: ada@example.com\r\nTo: undisclosed-recipients:;\r\n\
Subject: g\r\n\r\nx\r\n";
    let parsed = mime::parse(raw);
    assert!(parsed.to.is_empty());
    assert_eq!(parsed.headers.get("To"), Some("undisclosed-recipients:;"));
}

#[test]
fn a_display_name_needing_quotes_survives_the_round_trip() {
    // A comma in a display name is the case that breaks a naive splitter in
    // both directions: writing it unquoted makes two recipients out of one,
    // and reading it as a separator makes two out of one again.
    let built = outgoing::build(
        &draft(
            "quoting",
            vec![EmailAddress::new(Some("Hopper, Grace"), "gh@example.org")],
        ),
        &identity(),
        &[],
        None,
    );
    assert_eq!(
        mime::parse(&built.raw).to,
        vec![EmailAddress::new(Some("Hopper, Grace"), "gh@example.org")]
    );
}

#[test]
fn a_non_ascii_display_name_and_subject_are_encoded_and_decode_back() {
    // §3.4 is US-ASCII only; anything else has to be an RFC 2047 encoded
    // word, and a client that wrote raw UTF-8 into a header would produce
    // mail some servers reject and others mangle.
    let mut d = draft(
        "Grüße und Küsse — ünïcodé",
        vec![EmailAddress::new(Some("Ünïcodé Nâme"), "u@example.com")],
    );
    d.cc = vec![EmailAddress::new(None::<String>, "plain@example.com")];
    let built = outgoing::build(&d, &identity(), &[], None);

    assert!(
        built.raw.is_ascii(),
        "a generated header block must be US-ASCII on the wire"
    );
    let back = mime::parse(&built.raw);
    assert_eq!(back.subject.as_deref(), Some("Grüße und Küsse — ünïcodé"));
    assert_eq!(
        back.to,
        vec![EmailAddress::new(Some("Ünïcodé Nâme"), "u@example.com")]
    );
}

// ---------------------------------------------------------------------------
// §3.6.4 — Message-ID
// ---------------------------------------------------------------------------

#[test]
fn a_generated_message_id_is_unique_and_under_the_sending_domain() {
    let ids: Vec<String> = (0..64)
        .map(|_| {
            outgoing::reserve_message_id(Some("example.com"))
                .as_str()
                .to_owned()
        })
        .collect();
    let distinct: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(distinct.len(), ids.len(), "two sends shared a Message-ID");

    for id in &ids {
        assert!(id.starts_with('<') && id.ends_with('>'));
        let inner = &id[1..id.len() - 1];
        let (left, right) = inner.split_once('@').expect("id-left @ id-right");
        assert_eq!(right, "example.com", "the id must name the sending domain");
        assert!(
            left.chars()
                .all(|c| c.is_ascii_alphanumeric() || "!#$%&'*+-/=?^_`{|}~.".contains(c)),
            "id-left must be a dot-atom: {left:?}"
        );
    }
}

#[test]
fn a_message_id_that_could_not_identify_a_message_is_dropped_rather_than_kept() {
    // The corpus carries all three: an empty `<>`, an unterminated angle-addr
    // and a bare token. None of them names a message, and keeping one gives
    // the threading pass an edge that can only ever match other garbage.
    let raw = b"From: a@example.com\r\nTo: b@example.com\r\n\
Message-ID: <>\r\nReferences: <> not-an-id <real.id@example.com>\r\n\
Subject: s\r\n\r\nx\r\n";
    let parsed = mime::parse(raw);
    assert_eq!(parsed.rfc_message_id, None);
    assert_eq!(
        parsed.references,
        vec![RfcMessageId::new("real.id@example.com")]
    );
}

#[test]
fn message_id_equality_ignores_case_and_brackets() {
    // Headers get rewritten in transit, and a chain that stops matching at a
    // case change is a thread that splits in two.
    assert_eq!(
        RfcMessageId::new("A.B@Example.COM"),
        RfcMessageId::new("<a.b@example.com>")
    );
}

// ---------------------------------------------------------------------------
// §2.1 — what arrives, arrives
// ---------------------------------------------------------------------------

#[test]
fn a_decoded_header_value_can_carry_cr_and_lf() {
    // Not a bug in the parser: an encoded word encodes octets, `=0D=0A` is
    // two of them, and reporting what arrived is what this parser promises.
    // It is here because it is the *input* to the one gap the audit found,
    // and because a future parser that silently stripped them would make the
    // outgoing side look fixed when it was not. See
    // `docs/rfc-compliance.md` and `encoded-word-crlf-in-header.eml`.
    let parsed =
        mime::parse(postio_model::test_corpus::load("encoded-word-crlf-in-header").bytes());
    assert!(
        parsed
            .subject
            .as_deref()
            .is_some_and(|subject| subject.contains(['\r', '\n'])),
        "the fixture no longer delivers a header value with line breaks in it, \
         so the case the outgoing side has to defend against is untested"
    );
}

#[test]
fn generated_bytes_use_crlf_throughout() {
    let built = outgoing::build(
        &draft(
            "crlf",
            vec![EmailAddress::new(None::<String>, "a@example.com")],
        ),
        &identity(),
        &[],
        None,
    );
    let text = String::from_utf8_lossy(&built.raw);
    assert!(
        !text.replace("\r\n", "").contains(['\r', '\n']),
        "a bare CR or LF reached the wire: {text:?}"
    );
}
