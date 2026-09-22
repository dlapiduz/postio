//! Putting a file on a draft (#1269).

use postio_session::attaching::{LARGEST_ATTACHMENT, attach_file, inline_image};
use postio_storage::test_support;

/// A blob store of its own, and a file to put in it.
fn a_store() -> (postio_storage::BlobStore, tempfile::TempDir) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    (blobs, scratch)
}

#[test]
fn a_file_becomes_bytes_in_the_store_and_a_row_on_the_draft() {
    let (blobs, scratch) = a_store();
    let path = scratch.path().join("gate-plan.txt");
    std::fs::write(&path, b"the gate closes at six").expect("a file to attach");

    let attachment = attach_file(&blobs, &path, "text/plain").expect("it attaches");

    assert_eq!(attachment.filename.as_deref(), Some("gate-plan.txt"));
    assert_eq!(attachment.mime_type, "text/plain");
    assert_eq!(attachment.size, 22);
    assert_eq!(
        attachment.message_id,
        postio_model::ids::MessageId::UNASSIGNED,
        "it belongs to no message until the draft is sent"
    );

    // The bytes are actually there — an attachment naming a blob that is not
    // is a message that cannot be sent and cannot be repaired from inside the
    // application.
    let id = attachment.blob_id.expect("the attachment names a blob");
    let mut held = Vec::new();
    std::io::Read::read_to_end(
        &mut blobs.reader(&id).expect("the blob is readable"),
        &mut held,
    )
    .expect("a read");
    assert_eq!(held, b"the gate closes at six");
}

#[test]
fn a_file_that_is_not_there_is_refused_without_naming_it() {
    // An attachment's name is the user's, and an error string is a thing
    // people paste into bug reports.
    let (blobs, scratch) = a_store();
    let missing = scratch.path().join("not-here.pdf");

    let error = attach_file(&blobs, &missing, "application/pdf").expect_err("nothing to attach");

    assert!(error.contains("could not be read"), "{error}");
    assert!(
        !error.contains("not-here"),
        "the name is not in the error: {error}"
    );
}

#[test]
fn something_far_too_large_is_refused_before_its_bytes_are_copied() {
    // The accident this guards: a disk image dragged into a compose window,
    // quietly costing a gigabyte of the store for a message no server will
    // accept anyway.
    let (blobs, scratch) = a_store();
    let path = scratch.path().join("enormous.img");
    let file = std::fs::File::create(&path).expect("a file");
    file.set_len(LARGEST_ATTACHMENT + 1).expect("a sparse file");
    drop(file);

    let error = attach_file(&blobs, &path, "application/octet-stream").expect_err("too big");

    assert!(error.contains("will not attach"), "{error}");
    // And it says how big, in the units a person thinks in.
    assert!(error.contains("MB") || error.contains("GB"), "{error}");
}

#[test]
fn a_type_nothing_recognises_still_attaches() {
    // "Some bytes" is better than refusing a file over a type nothing knows.
    let (blobs, scratch) = a_store();
    let path = scratch.path().join("thing.whatever");
    std::fs::write(&path, b"...").expect("a file");

    let attachment =
        attach_file(&blobs, &path, "application/octet-stream").expect("it still attaches");
    assert_eq!(attachment.mime_type, "application/octet-stream");
}

// -- an image in the body rather than beside it (#1571) ----------------------

/// A few bytes that say they are a PNG. Nothing here decodes them: the type
/// is the caller's answer, as it is for `attach_file`.
const PIXELS: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

#[test]
fn an_inline_image_is_bytes_in_the_store_named_by_a_content_id() {
    let (blobs, _scratch) = a_store();

    let attachment = inline_image(&blobs, PIXELS, "image/png").expect("it inlines");

    assert_eq!(
        attachment.disposition,
        postio_model::attachment::Disposition::Inline,
        "an attachment disposition is a file beside the message, not a picture in it"
    );
    assert_eq!(attachment.mime_type, "image/png");
    assert_eq!(attachment.size, PIXELS.len() as u64);
    assert_eq!(attachment.filename.as_deref(), Some("inline-image.png"));

    // The id the body will reference, on a reserved domain: it can never be
    // mistaken for, or collide with, anything real.
    let content_id = attachment
        .content_id
        .clone()
        .expect("an inline part has a Content-ID");
    assert!(content_id.ends_with("@postio.invalid"), "{content_id}");

    let id = attachment.blob_id.expect("the attachment names a blob");
    let mut held = Vec::new();
    std::io::Read::read_to_end(&mut blobs.reader(&id).expect("readable"), &mut held)
        .expect("a read");
    assert_eq!(held, PIXELS);
}

#[test]
fn the_same_picture_twice_is_one_content_id() {
    // The blob store deduplicates by content, and the id is the blob's: an
    // image pasted twice is one part referenced twice, not two parts.
    let (blobs, _scratch) = a_store();
    let first = inline_image(&blobs, PIXELS, "image/png").expect("it inlines");
    let second = inline_image(&blobs, PIXELS, "image/png").expect("it inlines");
    assert_eq!(first.content_id, second.content_id);
}

#[test]
fn a_file_that_is_not_an_image_is_refused_and_pointed_at_attach() {
    let (blobs, _scratch) = a_store();

    let error = inline_image(&blobs, b"%PDF-1.7", "application/pdf").expect_err("not a picture");

    assert!(error.contains("not an image"), "{error}");
    assert!(
        error.contains("Attach file"),
        "says what to do instead: {error}"
    );
}
