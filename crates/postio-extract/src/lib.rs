//! The text inside an attachment, located: bytes in, units of text out. A pure
//! leaf with no store, runtime, network or toolkit.
//! Every format is read under [`Limits`]; hitting one keeps what was read
//! ([`Outcome::Truncated`]). The PDF reader panics on some files, so it runs under `catch_unwind`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

pub use postio_search::results::Location;

pub mod helper;
mod limits;
mod ooxml;
mod pdf;
mod text;
pub mod wire;

use limits::{Budget, Stop};

/// Bump whenever what [`extract`] produces for the same bytes changes: the
/// index treats an older version as missing and extracts again.
pub const EXTRACTOR_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_input: u64,
    pub max_text: usize,
    pub max_units: usize,
    pub max_time: Duration,
    pub max_entry: u64,
    /// Inflated size over compressed size.
    pub max_ratio: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_input: 25 * 1024 * 1024,
            max_text: 1024 * 1024,
            max_units: 5_000,
            max_time: Duration::from_secs(2),
            max_entry: 50 * 1024 * 1024,
            max_ratio: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Text,
    Units,
    Time,
    Entry,
    Ratio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    Encrypted,
    Unsupported,
    Empty,
    TooLarge,
    /// The helper is missing or belongs to another build. Recorded by the
    /// indexer, never produced by [`extract`].
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Complete,
    Truncated(Limit),
    Skipped(Skip),
    /// Malformed, truncated or the reader panicked; units read before it are kept.
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Truncated(_) => "truncated",
            Self::Skipped(_) => "skipped",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub location: Location,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// The units, in the file's order. Empty units are left out.
    pub units: Vec<Unit>,
    pub outcome: Outcome,
}

/// Read the text out of one attachment.
///
/// `mime_type` decides the format; a generic type falls back to `name`'s
/// extension, and a generic PDF to its signature. Never panics and never runs
/// past `limits`. CPU-bound: call it off any thread that draws (`spawn_blocking`).
pub fn extract(bytes: &[u8], mime_type: &str, name: Option<&str>, limits: &Limits) -> Extracted {
    let started = Instant::now();
    if bytes.len() as u64 > limits.max_input {
        return Extracted {
            units: Vec::new(),
            outcome: Outcome::Skipped(Skip::TooLarge),
        };
    }
    let (essence, charset) = parse_mime(mime_type);
    let Some(format) = Format::of(&essence, name, bytes) else {
        return Extracted {
            units: Vec::new(),
            outcome: Outcome::Skipped(Skip::Unsupported),
        };
    };

    let mut budget = Budget::new(limits, started);
    let read = match format {
        Format::Pdf => pdf::extract(bytes, &mut budget),
        // A panic is a failure here, never an unwind into the indexer.
        Format::Ooxml(kind) => catch_unwind(AssertUnwindSafe(|| {
            ooxml::extract(std::io::Cursor::new(bytes), kind, &mut budget)
        }))
        .unwrap_or(Err(Stop::Failed)),
        Format::Text => catch_unwind(AssertUnwindSafe(|| {
            text::extract(bytes, charset.as_deref(), &mut budget)
        }))
        .unwrap_or(Err(Stop::Failed)),
    };
    let outcome = match read {
        Ok(()) if budget.units.is_empty() => Outcome::Skipped(Skip::Empty),
        Ok(()) => Outcome::Complete,
        Err(Stop::Limit(limit)) => Outcome::Truncated(limit),
        Err(Stop::Skip(skip)) => Outcome::Skipped(skip),
        Err(Stop::Failed) => Outcome::Failed,
    };
    Extracted {
        units: budget.units,
        outcome,
    }
}

/// The one format whose reader is a third-party parser that recurses on the
/// file, so the indexer never reads it in its own process.
pub fn is_pdf(mime_type: &str, name: Option<&str>, bytes: &[u8]) -> bool {
    let (essence, _) = parse_mime(mime_type);
    Format::of(&essence, name, bytes) == Some(Format::Pdf)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pdf,
    Ooxml(ooxml::Kind),
    Text,
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

const GENERIC: &[&str] = &[
    "",
    "application/octet-stream",
    "application/zip",
    "application/x-zip-compressed",
    "application/x-zip",
    "application/download",
    "application/unknown",
    "binary/octet-stream",
];

/// `text/*` that is markup: read as lines, every tag would match.
const MARKUP: &[&str] = &["text/html", "text/rtf", "text/enriched", "text/richtext"];

impl Format {
    fn of(essence: &str, name: Option<&str>, bytes: &[u8]) -> Option<Self> {
        match essence {
            "application/pdf" | "application/x-pdf" => return Some(Self::Pdf),
            DOCX => return Some(Self::Ooxml(ooxml::Kind::Docx)),
            XLSX => return Some(Self::Ooxml(ooxml::Kind::Xlsx)),
            PPTX => return Some(Self::Ooxml(ooxml::Kind::Pptx)),
            text if text.starts_with("text/") && !MARKUP.contains(&text) => {
                return Some(Self::Text);
            }
            generic if GENERIC.contains(&generic) => {}
            _ => return None,
        }
        let extension = name
            .and_then(|name| name.rsplit_once('.'))
            .map(|(_, extension)| extension.to_ascii_lowercase());
        match extension.as_deref() {
            Some("pdf") => Some(Self::Pdf),
            Some("docx") => Some(Self::Ooxml(ooxml::Kind::Docx)),
            Some("xlsx") => Some(Self::Ooxml(ooxml::Kind::Xlsx)),
            Some("pptx") => Some(Self::Ooxml(ooxml::Kind::Pptx)),
            Some("txt" | "text" | "csv" | "tsv" | "md" | "markdown" | "log") => Some(Self::Text),
            _ if bytes.starts_with(b"%PDF-") => Some(Self::Pdf),
            _ => None,
        }
    }
}

fn parse_mime(mime: &str) -> (String, Option<String>) {
    let mut parts = mime.split(';');
    let essence = parts.next().unwrap_or("").trim().to_ascii_lowercase();
    let charset = parts.find_map(|parameter| {
        let (key, value) = parameter.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| value.trim().trim_matches('"').to_owned())
    });
    (essence, charset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_types_essence_and_charset_are_read_whatever_their_case() {
        assert_eq!(
            parse_mime("Text/Plain; Charset=\"ISO-8859-1\""),
            ("text/plain".to_owned(), Some("ISO-8859-1".to_owned()))
        );
        assert_eq!(
            parse_mime("application/pdf"),
            ("application/pdf".to_owned(), None)
        );
    }

    #[test]
    fn markup_is_not_text() {
        assert_eq!(Format::of("text/html", Some("page.html"), b""), None);
        assert_eq!(Format::of("text/csv", None, b""), Some(Format::Text));
    }

    #[test]
    fn a_generic_type_falls_back_to_the_name_then_the_signature() {
        assert_eq!(
            Format::of("application/octet-stream", Some("Budget.XLSX"), b""),
            Some(Format::Ooxml(ooxml::Kind::Xlsx))
        );
        assert_eq!(
            Format::of("application/octet-stream", Some("scan"), b"%PDF-1.7"),
            Some(Format::Pdf)
        );
        assert_eq!(Format::of("image/png", Some("a.pdf"), b"%PDF-"), None);
    }

    #[test]
    fn a_pdf_is_known_by_its_type_its_name_or_its_signature() {
        assert!(is_pdf("Application/PDF; name=x", None, b""));
        assert!(is_pdf("application/octet-stream", Some("scan.PDF"), b""));
        assert!(is_pdf("application/octet-stream", None, b"%PDF-1.7"));
        assert!(!is_pdf("text/plain", Some("notes.pdf"), b"%PDF-1.7"));
        assert!(!is_pdf(
            "application/octet-stream",
            Some("budget.xlsx"),
            b"PK"
        ));
    }
}
