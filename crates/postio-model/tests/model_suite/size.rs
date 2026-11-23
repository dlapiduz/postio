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
    let draft = draft_with(vec![part("enormous.bin", 500_000_000, false)]);
    assert_eq!(
        size::check(&draft, None),
        None,
        "an unconfigured limit must not become a guessed one"
    );
}

#[test]
fn the_refusal_reads_as_a_sentence_a_person_can_act_on() {
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
