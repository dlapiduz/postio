//! Putting a file on a draft (#1269).
//!
//! An attachment is bytes in the blob store plus a row on the draft, and the
//! order matters for the same reason it does everywhere else here: the bytes
//! first, so a draft never names a blob that is not there. The reverse is a
//! message that cannot be sent and cannot be repaired from inside the
//! application.
//!
//! # Why the MIME type is handed in
//!
//! Sniffing a file's type is a **platform service**: freedesktop reads the
//! shared-mime-info database through `gio`, macOS asks
//! `UniformTypeIdentifiers`, and neither is available to the other. So each
//! frontend answers that question its own way and passes the answer here,
//! where everything that is *not* platform-specific happens once.
//!
//! Falling back to `application/octet-stream` is the caller's job too, and
//! deliberately: "some bytes" is better than refusing to attach a file over a
//! type nothing recognises.

use std::path::Path;

use postio_model::attachment::Attachment;
use postio_model::ids::MessageId;
use postio_storage::BlobStore;

/// The largest file this will attach.
///
/// Not a limit anybody asked for — it is a guard against the accident:
/// dragging a disk image into a compose window and having Postio quietly
/// spend a gigabyte of the store on it. Servers refuse well below this
/// anyway, and a refusal here says so before the bytes are copied rather
/// than after.
pub const LARGEST_ATTACHMENT: u64 = 100 * 1024 * 1024;

/// Copy `path` into the blob store and describe it as an attachment.
///
/// The attachment carries [`MessageId::UNASSIGNED`] until the draft it is on
/// becomes a sent message, which is what `Draft::attachments` documents.
pub fn attach_file(blobs: &BlobStore, path: &Path, mime_type: &str) -> Result<Attachment, String> {
    let size = std::fs::metadata(path)
        // The path is deliberately not in the message: an attachment's name
        // is the user's, and an error string is a thing people paste into bug
        // reports.
        .map_err(|error| format!("That file could not be read: {error}"))?
        .len();
    if size > LARGEST_ATTACHMENT {
        return Err(format!(
            "That file is {} and Postio will not attach anything over {}.",
            postio_ui::format::human_size(size),
            postio_ui::format::human_size(LARGEST_ATTACHMENT)
        ));
    }

    let file = std::fs::File::open(path)
        .map_err(|error| format!("That file could not be opened: {error}"))?;
    let blob_id = blobs
        .put_reader(file)
        .map_err(|error| format!("The attachment could not be stored: {error}"))?;

    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type.to_owned(), size);
    attachment.filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    attachment.blob_id = Some(blob_id);
    Ok(attachment)
}

/// What refusing a non-image says, and where it points instead.
const NOT_AN_IMAGE: &str = "That file is not an image. Use Attach file to send it alongside.";

/// Store `bytes` as a picture in the body rather than a file beside it
/// (#1571).
///
/// An inline part is a blob and a row, like any attachment, plus two things
/// that make it a picture: [`Disposition::Inline`], and a `Content-ID` the
/// body's `<img>` references. The id is the blob's digest at
/// `postio.invalid` — unique by construction (the store deduplicates by
/// content, so the same picture twice is one part referenced twice) and on a
/// reserved domain, so it cannot collide with or be mistaken for anything
/// real. One rule, here, rather than one per frontend: a composer that minted
/// ids its own way would be a second answer to what a sent message says its
/// parts are called.
///
/// `mime_type` is the caller's, for [`attach_file`]'s reason. What is decided
/// here is that it has to be an image: anything else belongs beside the
/// message, and saying so beats a broken picture the recipient cannot open.
///
/// [`Disposition::Inline`]: postio_model::attachment::Disposition::Inline
pub fn inline_image(
    blobs: &BlobStore,
    bytes: &[u8],
    mime_type: &str,
) -> Result<Attachment, String> {
    if !mime_type.starts_with("image/") {
        return Err(NOT_AN_IMAGE.to_owned());
    }
    let size = bytes.len() as u64;
    if size > LARGEST_ATTACHMENT {
        return Err(format!(
            "That image is {} and Postio will not attach anything over {}.",
            postio_ui::format::human_size(size),
            postio_ui::format::human_size(LARGEST_ATTACHMENT)
        ));
    }
    let blob_id = blobs
        .put(bytes)
        .map_err(|error| format!("The image could not be stored: {error}"))?;

    let extension = mime_type.strip_prefix("image/").unwrap_or("png");
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type.to_owned(), size);
    attachment.filename = Some(format!("inline-image.{extension}"));
    attachment.disposition = postio_model::attachment::Disposition::Inline;
    attachment.content_id = Some(format!("{}@postio.invalid", blob_id.as_str()));
    attachment.blob_id = Some(blob_id);
    Ok(attachment)
}
