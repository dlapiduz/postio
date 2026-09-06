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

/// What the reader is holding back, and about whom.
///
/// Drawn as one row that never wraps (canvas screen 26): an icon, the count,
/// `Show`, and a `⋯` that opens the grants. It is **per message**, never per
/// pane — a conversation of eight messages can hold back pictures from three
/// different senders, and one notice above them all could not say whose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderNoticeFfi {
    /// The sentence: `6 remote images blocked`.
    pub summary: String,
    /// The address the grant would be about.
    pub sender: String,
    /// Its domain, for the second grant — the one that covers a service
    /// which never sends twice from the same address.
    pub domain: String,
    /// Whether this sender is already allowed, in which case the reader has
    /// stopped asking.
    pub allowed: bool,
}

/// An address with its middle elided: `notices_at_…@relay.example.net`.
///
/// The middle is what goes because both ends identify the sender — the local
/// part says which service and the domain says whose it is — and a grant is a
/// decision about exactly that. A tail-truncated address hides the half a
/// person judges by. `postio_ui::format::middle_truncate`'s wording, so the
/// two frontends elide the same address the same way.
#[uniffi::export]
pub fn middle_truncate(text: String, width: u32) -> String {
    postio_ui::format::middle_truncate(&text, width as usize)
}
