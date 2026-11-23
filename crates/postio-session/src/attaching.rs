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

// ---------------------------------------------------------------------------
// Remote images: Download and attach (spec 014 D7, `contracts/remote-image.md`)
// ---------------------------------------------------------------------------

/// The largest picture Download and attach takes. Larger is not meant to be
/// inline, and the attachment path already warns about large files.
pub const REMOTE_IMAGE_LIMIT: usize = 10 * 1024 * 1024;

/// How long one download may take, connection to last byte.
pub const REMOTE_IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Why a fetcher came back without a picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// It could not be reached: DNS, the connection, TLS, the time limit,
    /// a status of 400 or more. The reason is a few words, never the address.
    Unreachable(String),
    /// Over [`REMOTE_IMAGE_LIMIT`], declared or streamed.
    TooLarge,
    /// Not a PNG, JPEG, GIF or WebP by its own bytes.
    NotAnImage,
}

/// What makes the one request a press makes.
///
/// A trait so the privacy rule is testable: a counting fake stands in, and
/// no case reaches the network. The production one is [`HttpFetcher`].
pub trait Fetcher: Send + Sync {
    /// One GET of `href`, within the limits, or why not.
    fn get(&self, href: &postio_body::WebHref) -> Result<Vec<u8>, FetchError>;
}

/// The production [`Fetcher`]: Pimalaya's `io-http`, through the reader's
/// remote-image fetch -- one plain GET with no cookie, credential or
/// `Referer` and a `User-Agent` that says nothing about who, at most three
/// redirects, http and https only.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpFetcher;

impl Fetcher for HttpFetcher {
    fn get(&self, href: &postio_body::WebHref) -> Result<Vec<u8>, FetchError> {
        use postio_runtime::remote_images::{Failure, Fetched, fetch_one};
        let url = url::Url::parse(href.as_str())
            .map_err(|_| FetchError::Unreachable("the address does not parse".to_owned()))?;
        match fetch_one(&url, REMOTE_IMAGE_TIMEOUT, REMOTE_IMAGE_LIMIT) {
            Fetched::Image(bytes) => Ok(bytes.to_vec()),
            Fetched::Failed(Failure::TooLarge) => Err(FetchError::TooLarge),
            Fetched::Failed(Failure::NotAnImage) => Err(FetchError::NotAnImage),
            Fetched::Failed(failure) => Err(FetchError::Unreachable(match failure {
                Failure::Status(status) => format!("it answered {status}"),
                Failure::TimedOut => "it took too long".to_owned(),
                Failure::TooManyRedirects => "it redirected too often".to_owned(),
                Failure::NotHttp => "it redirected away from the web".to_owned(),
                _ => "the connection failed".to_owned(),
            })),
        }
    }
}

/// A [`Fetcher`] that answers every request with the same picture and
/// never dials: for a demo, whose storyboards must not reach the network.
#[derive(Clone, Debug)]
pub struct LocalImage(pub Vec<u8>);

impl Fetcher for LocalImage {
    fn get(&self, _href: &postio_body::WebHref) -> Result<Vec<u8>, FetchError> {
        Ok(self.0.clone())
    }
}

/// Why Download and attach left the image as a link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchRefusal {
    /// It could not be reached, and why, in a few words.
    Unreachable {
        /// The few words.
        reason: String,
    },
    /// Over [`REMOTE_IMAGE_LIMIT`].
    TooLarge,
    /// Not a picture: a page, an SVG, anything but PNG, JPEG, GIF or WebP.
    NotAnImage,
    /// The session knows it is offline, so no request was made.
    Offline,
}

/// Download `href` once and attach it as an inline image: the one network
/// gesture the composer has, made only by the `attach_remote_image` press
/// (spec 014 FR-031). Nothing is retried or queued, and offline nothing is
/// requested at all.
///
/// POSTIO-CONSENT: the only caller is that command's handler, which
/// `scripts/checks/check-remote-image-fetch-site.py` holds. The log line
/// carries the outcome and the byte count, never the address.
pub fn fetch_remote_image(
    fetcher: &dyn Fetcher,
    blobs: &BlobStore,
    draft: postio_model::DraftId,
    href: &postio_body::WebHref,
    online: bool,
) -> Result<Attachment, FetchRefusal> {
    let draft = draft.get();
    if !online {
        tracing::info!(
            draft,
            outcome = "offline",
            bytes = 0,
            "remote image not fetched"
        );
        return Err(FetchRefusal::Offline);
    }
    let outcome = match fetcher.get(href) {
        Ok(bytes) if bytes.len() > REMOTE_IMAGE_LIMIT => Err(FetchRefusal::TooLarge),
        Ok(bytes) => match image_mime(&bytes) {
            Some(mime_type) => inline_image(blobs, &bytes, mime_type)
                .map_err(|reason| FetchRefusal::Unreachable { reason }),
            None => Err(FetchRefusal::NotAnImage),
        },
        Err(FetchError::TooLarge) => Err(FetchRefusal::TooLarge),
        Err(FetchError::NotAnImage) => Err(FetchRefusal::NotAnImage),
        Err(FetchError::Unreachable(reason)) => Err(FetchRefusal::Unreachable { reason }),
    };
    match &outcome {
        Ok(attachment) => {
            tracing::info!(
                outcome = "attached",
                bytes = attachment.size,
                "remote image fetched"
            );
        }
        Err(refusal) => {
            tracing::info!(
                draft,
                outcome = refusal_word(refusal),
                bytes = 0,
                "remote image refused"
            );
        }
    }
    outcome
}

fn refusal_word(refusal: &FetchRefusal) -> &'static str {
    match refusal {
        FetchRefusal::Unreachable { .. } => "unreachable",
        FetchRefusal::TooLarge => "too_large",
        FetchRefusal::NotAnImage => "not_an_image",
        FetchRefusal::Offline => "offline",
    }
}

/// A picture's type by its own bytes, or `None` for anything else.
fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if !postio_runtime::remote_images::image_format(bytes) {
        return None;
    }
    Some(if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else {
        "image/webp"
    })
}
