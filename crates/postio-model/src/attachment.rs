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

impl Disposition {
    /// A stable lowercase identifier, for storage.
    ///
    /// [`Disposition::Other`] flattens to `other`; the verbatim value it
    /// carries is [`Disposition::raw`], stored beside it so the pair round
    /// trips.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Attachment => "attachment",
            Self::Other(_) => "other",
        }
    }

    /// The verbatim disposition, for a value [`Disposition::as_str`] flattened.
    pub fn raw(&self) -> Option<&str> {
        match self {
            Self::Other(raw) => Some(raw),
            _ => None,
        }
    }

    /// Rebuilds a disposition from [`Disposition::as_str`] and
    /// [`Disposition::raw`].
    ///
    /// `None` for an identifier this build does not know, or for `other` with
    /// no verbatim value to restore.
    pub fn from_parts(name: &str, raw: Option<&str>) -> Option<Self> {
        match name {
            "inline" => Some(Self::Inline),
            "attachment" => Some(Self::Attachment),
            "other" => raw.map(|raw| Self::Other(raw.to_owned())),
            _ => None,
        }
    }
}

/// A `Content-ID` header value as the store holds it — see
/// [`Attachment::set_content_id`], which is how ingest paths should reach it.
pub fn bare_content_id(raw: &str) -> Option<String> {
    let bare = raw
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!bare.is_empty()).then(|| bare.to_owned())
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
    /// The part's MIME header block, as `BODYSTRUCTURE` described it — content
    /// type, charset, transfer encoding.
    ///
    /// `BODY[2.1]` hands back a part's *encoded* bytes and none of its
    /// headers, so nothing in the fetch response says whether they are base64.
    /// Prepending this turns the fetched section back into a self-contained
    /// entity a parser can decode, without a second round trip for
    /// `BODY[2.1.MIME]`. The same trick, and the same reason, as
    /// [`Message::text_part_headers`](crate::Message::text_part_headers).
    ///
    /// `None` for a part nobody fetched by section — one carried on an
    /// outgoing draft, or a row synced before the column existed.
    pub part_headers: Option<String>,
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
            part_headers: None,
            blob_id: None,
        }
    }

    /// Records a `Content-ID` as the store holds it: bare, with the angle
    /// brackets RFC 2045 wraps the header value in taken off.
    ///
    /// Every ingest path goes through here, because the alternative is what
    /// #751 was. `postio_model::mime` trimmed the brackets and the IMAP
    /// header sync did not -- `BODYSTRUCTURE`'s id field is the header value
    /// verbatim, `<logo@example.com>` -- so the same message stored a
    /// different id depending on how it arrived. The sanitizer builds
    /// `postio-cid:logo@example.com` from the HTML either way and
    /// `postio_session::reading::resolve_cid` compares with `==`, so an
    /// IMAP-synced inline image could never resolve however many of its bytes
    /// were local.
    ///
    /// An id that is empty once trimmed is no id: it can match no `cid:` URL,
    /// and `Some("")` would only give the comparison something to be wrong
    /// about.
    pub fn set_content_id(&mut self, raw: Option<&str>) {
        self.content_id = raw.and_then(bare_content_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions_round_trip_through_their_stored_parts() {
        for disposition in [
            Disposition::Inline,
            Disposition::Attachment,
            Disposition::Other("form-data".to_owned()),
        ] {
            assert_eq!(
                Disposition::from_parts(disposition.as_str(), disposition.raw()),
                Some(disposition.clone())
            );
        }
        assert_eq!(
            Disposition::from_parts("other", None),
            None,
            "`other` without the verbatim value cannot be rebuilt"
        );
        assert_eq!(Disposition::from_parts("nonsense", None), None);
    }
}
