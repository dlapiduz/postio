//! Attachments and inline parts.

use serde::{Deserialize, Serialize};

use crate::ids::{AttachmentId, BlobId, MessageId};

/// How a part is meant to be presented.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Disposition {
    /// Rendered in place, e.g. an embedded image referenced by `Content-ID`.
    Inline,
    /// A file the user downloads or opens.
    #[default]
    Attachment,
    /// Anything else the message declared.
    Other(String),
}

/// One attachment or inline part of a message.
///
/// Metadata is stored eagerly so search and the list can show
/// `has:attachment` / `filename:` without any network round trip; the bytes are
/// fetched lazily and land in the content-addressed blob store, at which point
/// `blob_id` becomes `Some`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// Local id.
    pub id: AttachmentId,
    /// Owning message, or [`MessageId::UNASSIGNED`] while it belongs to a
    /// [`Draft`](crate::Draft) that has not been turned into a message yet.
    pub message_id: MessageId,
    /// Filename as declared by the sender, if any.
    pub filename: Option<String>,
    /// MIME type, e.g. `application/pdf`.
    pub mime_type: String,
    /// Size in bytes as declared by the server.
    pub size: u64,
    /// `Content-ID`, for inline parts referenced from the HTML body.
    pub content_id: Option<String>,
    /// How the part should be presented.
    pub disposition: Disposition,
    /// MIME part path within the message, e.g. `2.1`, for a lazy fetch.
    pub part_id: Option<String>,
    /// Blob store key, present once the bytes have been downloaded.
    pub blob_id: Option<BlobId>,
}

impl Attachment {
    /// Builds unpersisted attachment metadata with no bytes downloaded yet.
    pub fn new(message_id: MessageId, mime_type: impl Into<String>, size: u64) -> Self {
        Self {
            id: AttachmentId::UNASSIGNED,
            message_id,
            filename: None,
            mime_type: mime_type.into(),
            size,
            content_id: None,
            disposition: Disposition::Attachment,
            part_id: None,
            blob_id: None,
        }
    }

    /// Whether the bytes are in the local blob store.
    pub fn is_downloaded(&self) -> bool {
        self.blob_id.is_some()
    }

    /// Whether the part is rendered inside the body rather than listed.
    pub fn is_inline(&self) -> bool {
        self.disposition == Disposition::Inline
    }

    /// The filename to show, falling back to a generic name.
    pub fn display_name(&self) -> &str {
        match self.filename.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => "attachment",
        }
    }

    /// The lowercased extension of the filename, if it has one.
    pub fn extension(&self) -> Option<String> {
        self.filename
            .as_deref()?
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_lowercase())
    }
}
