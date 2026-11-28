use serde::{Deserialize, Serialize};

use crate::ids::{AttachmentId, BlobId, MessageId};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Disposition {
    Inline,
    #[default]
    Attachment,
    Other(String),
}

impl Disposition {
    /// `Other` flattens to `other`; the verbatim value is `raw`, stored beside it so the pair
    /// round-trips.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Attachment => "attachment",
            Self::Other(_) => "other",
        }
    }

    pub fn raw(&self) -> Option<&str> {
        match self {
            Self::Other(raw) => Some(raw),
            _ => None,
        }
    }

    /// `None` for an unknown identifier, or `other` with no verbatim value.
    pub fn from_parts(name: &str, raw: Option<&str>) -> Option<Self> {
        match name {
            "inline" => Some(Self::Inline),
            "attachment" => Some(Self::Attachment),
            "other" => raw.map(|raw| Self::Other(raw.to_owned())),
            _ => None,
        }
    }
}

pub fn bare_content_id(raw: &str) -> Option<String> {
    let bare = raw
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .trim();
    (!bare.is_empty()).then(|| bare.to_owned())
}

/// Metadata is stored eagerly; bytes are fetched lazily into the blob store, when `blob_id` becomes
/// `Some`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    /// `MessageId::UNASSIGNED` while it belongs to a draft not yet turned into a message.
    pub message_id: MessageId,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: u64,
    pub content_id: Option<String>,
    pub disposition: Disposition,
    pub part_id: Option<String>,
    /// The part's MIME header block from `BODYSTRUCTURE`. `BODY[2.1]` returns encoded bytes with no
    /// headers, so prepending this makes the section decodable without a second round trip. `None`
    /// if not fetched by section.
    pub part_headers: Option<String>,
    pub blob_id: Option<BlobId>,
}

impl Attachment {
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

    /// Stores a bare `Content-ID` (brackets off) so every ingest path stores the same id:
    /// `resolve_cid` compares with `==`, and a bracketed id never resolved (#751). An id empty once
    /// trimmed is no id.
    pub fn set_content_id(&mut self, raw: Option<&str>) {
        self.content_id = raw.and_then(bare_content_id);
    }

    pub fn is_downloaded(&self) -> bool {
        self.blob_id.is_some()
    }

    pub fn is_inline(&self) -> bool {
        self.disposition == Disposition::Inline
    }

    pub fn display_name(&self) -> &str {
        match self.filename.as_deref() {
            Some(name) if !name.trim().is_empty() => name,
            _ => "attachment",
        }
    }

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
