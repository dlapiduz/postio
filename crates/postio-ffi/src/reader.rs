//! The reader's document, and the inline parts it may reference.

/// Whether the reader may fetch images the message points at.
///
/// Crosses because it is the frontend's per-sender decision to make, and
/// **`Blocked` is the default everywhere**: `PRODUCT.md`'s "nothing leaves this
/// machine that the user did not ask for" starts here, at the tracking pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteImagesFfi {
    /// Images the message points at are not fetched.
    Blocked,
    /// The sender is allowed, so remote images load.
    Allowed,
}

impl From<RemoteImagesFfi> for postio_body::RemoteImages {
    fn from(remote: RemoteImagesFfi) -> Self {
        match remote {
            RemoteImagesFfi::Blocked => postio_body::RemoteImages::Blocked,
            RemoteImagesFfi::Allowed => postio_body::RemoteImages::Allowed,
        }
    }
}

/// One part of a message, referenced from its body by `Content-ID`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InlinePart {
    /// The bytes, already on this machine.
    pub bytes: Vec<u8>,
    /// What the part says it is, for the response's `Content-Type`.
    pub mime_type: String,
}
