//! Putting a file on a draft (#1269): bytes into the blob store first, then the row, so a draft
//! never names a missing blob.
//! The MIME type is handed in because sniffing is a platform service; the caller falls back to
//! `application/octet-stream`.

use std::path::Path;

use postio_model::attachment::Attachment;
use postio_model::ids::MessageId;
use postio_storage::BlobStore;

/// The largest file this will attach: a guard against dragging a disk image into the store,
/// refusing before the bytes are copied.
pub const LARGEST_ATTACHMENT: u64 = 100 * 1024 * 1024;

/// Copy `path` into the blob store and describe it as an attachment, with `MessageId::UNASSIGNED`
/// until the draft is sent.
pub fn attach_file(blobs: &BlobStore, path: &Path, mime_type: &str) -> Result<Attachment, String> {
    let size = std::fs::metadata(path)
        // The path stays out of the message: error strings get pasted into bug reports.
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

const NOT_AN_IMAGE: &str = "That file is not an image. Use Attach file to send it alongside.";

/// Store `bytes` as a picture in the body rather than a file beside it (#1571).
///
/// The `Content-ID` is the blob's digest at the reserved `postio.invalid`, so one rule serves every
/// frontend and the same picture twice is one part. A non-image `mime_type` is refused.
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
