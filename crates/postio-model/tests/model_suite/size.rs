//! One size total, and a refusal that says what to do about it (spec 002,
//! FR-055 and FR-056).
//!
//! There was no client-side size check at all. A draft over the provider's
//! limit was queued, sent, and rejected by the server — after the composer
//! had closed, which is the failure FR-056 exists to prevent. By then the
//! message is a `Failed` draft and the person has to work out for themselves
//! which of six attachments to remove.
//!
//! The limit comes from the account's configuration and never from the SMTP
//! `SIZE` capability (research §3): `postio-smtp` is explicit that Postio
//! "announces nothing and relies on nothing", and a refusal conditional on a
//! capability list is how that erodes.

use postio_model::attachment::Disposition;
use postio_model::ids::{AccountId, AttachmentId, MessageId};
use postio_model::size::{self, TooLarge};
use postio_model::{Attachment, Draft};

fn part(name: &str, size: u64, inline: bool) -> Attachment {
    Attachment {
        id: AttachmentId::UNASSIGNED,
        message_id: MessageId::UNASSIGNED,
        filename: Some(name.to_owned()),
        mime_type: "application/octet-stream".to_owned(),
        size,
        content_id: inline.then(|| format!("{name}@example.invalid")),
        disposition: if inline {
            Disposition::Inline
        } else {
            Disposition::Attachment
        },
        part_id: None,
        part_headers: None,
        blob_id: None,
    }
}

fn draft_with(parts: Vec<Attachment>) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.body.text = Some("Here they are.".to_owned());
    draft.attachments = parts;
    draft
}

#[test]
fn inline_images_and_attached_files_count_against_one_total() {
    // FR-055, and the maintainer's clarification: one number, not two. Two
    // budgets is the arrangement where a message under both is still over
    // what the server will take -- which is the only limit that exists.
    let draft = draft_with(vec![
        part("diagram.png", 4_000_000, true),
        part("report.pdf", 4_000_000, false),
    ]);

    assert_eq!(
        size::of(&draft),
        8_000_000 + size::of_body(&draft),
        "an inline image and an attachment must land in the same total"
    );
}

#[test]
fn the_body_counts_too_because_the_server_weighs_the_whole_message() {
    // A long quoted reply is not free. The limit is on the message, so the
    // check has to be on the message.
    let mut draft = draft_with(Vec::new());
    draft.body.text = Some("x".repeat(10_000));
    assert!(
        size::of(&draft) >= 10_000,
        "the body was not counted: {}",
        size::of(&draft)
    );
}

#[test]
fn a_draft_under_the_limit_is_not_refused() {
    let draft = draft_with(vec![part("small.pdf", 1_000, false)]);
    assert_eq!(size::check(&draft, Some(25_000_000)), None);
}

#[test]
fn an_oversize_draft_names_the_limit_the_overage_and_the_largest_items() {
    // FR-056. "Too large" on its own leaves the person guessing which of six
    // attachments to remove, so the refusal carries the three facts that
    // turn it into an action: what the ceiling is, how far over they are,
    // and what is taking up the room.
    let draft = draft_with(vec![
        part("tiny.txt", 1_000, false),
        part("video.mp4", 30_000_000, false),
        part("diagram.png", 5_000_000, true),
    ]);

    let TooLarge {
        total,
        limit,
        over_by,
        largest,
    } = size::check(&draft, Some(25_000_000)).expect("this draft is over the limit");

    assert_eq!(limit, 25_000_000);
    assert_eq!(total, size::of(&draft));
    assert_eq!(over_by, total - limit, "the overage must be exact");

    assert_eq!(
        largest.first().map(|item| item.name.as_str()),
        Some("video.mp4"),
        "the largest item must be named first -- it is the one removal that \
         solves the problem"
    );
    assert_eq!(
        largest.get(1).map(|item| item.name.as_str()),
        Some("diagram.png"),
        "an inline image is as removable as an attachment and belongs in the \
         same list"
    );
    assert!(
        largest.len() <= 3,
        "a refusal that lists every part is a refusal nobody reads: {largest:?}"
    );
    assert!(
        !largest.iter().any(|item| item.name == "tiny.txt"),
        "a part too small to matter is noise in a refusal: {largest:?}"
    );
}

#[test]
fn no_configured_limit_checks_nothing() {
    // T039, decided: when the account carries no limit, the composer checks
    // nothing rather than inventing a number. A guessed ceiling refuses mail
    // the server would have taken, and the person has no way to tell that it
    // was Postio's opinion rather than their provider's.
    let draft = draft_with(vec![part("enormous.bin", 500_000_000, false)]);
    assert_eq!(
        size::check(&draft, None),
        None,
        "an unconfigured limit must not become a guessed one"
    );
}

#[test]
fn the_refusal_reads_as_a_sentence_a_person_can_act_on() {
    // The wording is part of the requirement: FR-056 says the refusal names
    // the limit, the overage and the largest items, and a struct nobody
    // renders names nothing.
    let draft = draft_with(vec![part("video.mp4", 30_000_000, false)]);
    let refusal = size::check(&draft, Some(25_000_000)).expect("over the limit");
    let said = refusal.to_string();

    assert!(said.contains("video.mp4"), "{said}");
    assert!(
        said.contains("25"),
        "the limit is not in the message: {said}"
    );
    assert!(
        said.to_lowercase().contains("mb") || said.to_lowercase().contains("byte"),
        "the sizes have no unit, so the numbers mean nothing: {said}"
    );
}
