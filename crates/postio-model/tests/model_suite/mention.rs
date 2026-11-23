use postio_model::ids::{AccountId, AttachmentId, MessageId};
use postio_model::mention::mentions_an_attachment;
use postio_model::{Attachment, Draft};

fn draft_saying(text: &str) -> Draft {
    let mut draft = Draft::new(AccountId::new(1));
    draft.body.text = Some(text.to_owned());
    draft
}

fn with_a_part(mut draft: Draft) -> Draft {
    draft.attachments = vec![Attachment {
        id: AttachmentId::UNASSIGNED,
        message_id: MessageId::UNASSIGNED,
        filename: Some("report.pdf".to_owned()),
        mime_type: "application/pdf".to_owned(),
        size: 2_048,
        content_id: None,
        disposition: postio_model::attachment::Disposition::Attachment,
        part_id: None,
        part_headers: None,
        blob_id: None,
    }];
    draft
}

#[test]
fn the_usual_ways_of_saying_it_are_recognised() {
    for said in [
        "Please find attached the tide gate report.",
        "I've attached the schedule.",
        "Attaching the photos now.",
        "The figures are enclosed.",
        "See attachment.",
        "ATTACHED IS THE INVOICE",
    ] {
        assert!(
            mentions_an_attachment(&draft_saying(said)),
            "not recognised: {said:?}"
        );
    }
}

#[test]
fn a_draft_that_actually_carries_a_part_says_nothing() {
    let draft = with_a_part(draft_saying("Please find attached the report."));
    assert!(!mentions_an_attachment(&draft));
}

#[test]
fn a_quoted_original_is_somebody_elses_words() {
    let draft = draft_saying(
        "Thanks, that works.\n\n\
         On 2026-09-10, Ada wrote:\n\
         > Please find attached the tide gate report.\n\
         > Let me know what you think.",
    );
    assert!(
        !mentions_an_attachment(&draft),
        "a quoted original triggered the check"
    );
}

#[test]
fn a_signature_is_boilerplate_and_not_a_claim() {
    let draft = draft_saying("Morning.\n\n-- \nAda Lovelace\nAll attachments are virus-scanned.");
    assert!(
        !mentions_an_attachment(&draft),
        "a signature triggered the check"
    );
}

#[test]
fn a_word_that_merely_contains_one_is_not_a_mention() {
    for said in [
        "The unattached bracket is the problem.",
        "The bolt attaches to the frame.",
        "detached from the mount",
    ] {
        assert!(
            !mentions_an_attachment(&draft_saying(said)),
            "fired on a word that merely contains one: {said:?}"
        );
    }
}

#[test]
fn an_empty_draft_says_nothing() {
    assert!(!mentions_an_attachment(&Draft::new(AccountId::new(1))));
}

#[test]
fn an_inline_image_counts_as_something_being_attached() {
    let mut draft = with_a_part(draft_saying("Attached is the screenshot."));
    draft.attachments[0].content_id = Some("shot@example.invalid".to_owned());
    draft.attachments[0].disposition = postio_model::attachment::Disposition::Inline;
    assert!(!mentions_an_attachment(&draft));
}
