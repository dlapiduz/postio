//! "Please find attached" with nothing attached (spec 002, FR-057).
//!
//! The one composer check whose whole value is catching a mistake the person
//! has already made and cannot see. It costs a confirmation when it is wrong
//! and saves a second email when it is right, which is why the bar for
//! firing is "the user said so in their own words" rather than anything
//! cleverer.
//!
//! Two rules do most of the work, and both are about *whose* words these are:
//! a quoted original is somebody else's, and a signature is boilerplate. A
//! check that read either would fire on every reply to a message that had an
//! attachment, which is the fastest way to teach someone to dismiss a
//! dialog without reading it.

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
    // The check is about the *gap*, not about the words. Someone who wrote
    // "attached" and attached something has made no mistake, and a dialog
    // would be the app arguing with them.
    let draft = with_a_part(draft_saying("Please find attached the report."));
    assert!(!mentions_an_attachment(&draft));
}

#[test]
fn a_quoted_original_is_somebody_elses_words() {
    // The important one. Replying to "here is the report attached" must not
    // ask whether *you* forgot an attachment -- and that case is not rare,
    // it is most of the replies anybody sends to a message with a file on
    // it. Firing there teaches people to dismiss the dialog unread, which
    // costs the check its whole value.
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
    // Somebody whose signature says "Attachments are scanned" would
    // otherwise be asked on every message they ever send.
    let draft = draft_saying("Morning.\n\n-- \nAda Lovelace\nAll attachments are virus-scanned.");
    assert!(
        !mentions_an_attachment(&draft),
        "a signature triggered the check"
    );
}

#[test]
fn a_word_that_merely_contains_one_is_not_a_mention() {
    // `unattached`, `attaches`, and the failure mode of a naive `contains`.
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
    // A pasted screenshot is visibly there, and asking "did you forget an
    // attachment?" about a message with a picture in it is the app not
    // looking at what the person can plainly see.
    let mut draft = with_a_part(draft_saying("Attached is the screenshot."));
    draft.attachments[0].content_id = Some("shot@example.invalid".to_owned());
    draft.attachments[0].disposition = postio_model::attachment::Disposition::Inline;
    assert!(!mentions_an_attachment(&draft));
}
