//! The text inside an attachment, located (spec 010 D10, research R5).
//!
//! A pure leaf: bytes in, located units of text out. It opens no store,
//! starts no runtime, reaches no network and links no toolkit (FR-052,
//! enforced by `check-crate-boundaries.py`). The indexer in
//! `postio-session` reads a blob that is already on this machine, calls
//! [`extract`] on a blocking thread, folds each unit and writes it to the
//! index; nothing here knows that.
//!
//! # Hostile input is the normal case
//!
//! An attachment is bytes a stranger chose. Every format is read under
//! [`Limits`] — bytes in, text out, units, wall time, and for zip packages
//! each entry's size and compression ratio — and stopping at a limit keeps
//! what was read and says which limit it was ([`Outcome::Truncated`]).
//! The PDF reader panics on some malformed files, so it runs under
//! `catch_unwind` and a panic is [`Outcome::Failed`], never an unwind into
//! the caller.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::{Duration, Instant};

pub use postio_search::results::Location;

mod limits;
mod ooxml;
mod pdf;
mod text;

use limits::{Budget, Stop};

/// The extractor's version. **Bump it whenever what [`extract`] produces
/// for the same bytes changes** — a new format, a different unit, a fixed
/// walker — and every attachment is extracted again: the index keeps the
/// version each row was made with and treats an older one as missing.
///
/// History:
/// 1 — PDF pages, DOCX paragraphs, XLSX rows, PPTX slides, `text/*` lines.
pub const EXTRACTOR_VERSION: u32 = 1;

/// How far one extraction may go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// The largest input read at all, in bytes. Larger is
    /// [`Skip::TooLarge`], untouched.
    pub max_input: u64,
    /// The most text kept, in bytes, across every unit.
    pub max_text: usize,
    /// The most units kept: pages, rows, slides, paragraphs, lines.
    pub max_units: usize,
    /// The longest one extraction may take.
    pub max_time: Duration,
    /// The largest one zip entry may inflate to, in bytes.
    pub max_entry: u64,
    /// The largest ratio of a zip entry's inflated size to its compressed
    /// size: past it, the entry is a bomb, not a document.
    pub max_ratio: u32,
}

impl Default for Limits {
    /// Research R5's: 25 MB in, 1 MB of text out, 5,000 units, 2 s, zip
    /// entries at most 50 MB inflated and 100 times their compressed size.
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

/// Which limit an extraction stopped at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    /// [`Limits::max_text`].
    Text,
    /// [`Limits::max_units`].
    Units,
    /// [`Limits::max_time`].
    Time,
    /// [`Limits::max_entry`]: a zip entry inflated past it.
    Entry,
    /// [`Limits::max_ratio`]: a zip entry compressed too well to be honest.
    Ratio,
}

/// Why nothing was extracted, when nothing went wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// Encrypted with a password nobody gave.
    Encrypted,
    /// Not a format this crate reads.
    Unsupported,
    /// Read, and there was no text in it.
    Empty,
    /// Larger than [`Limits::max_input`]; not opened.
    TooLarge,
}

/// How an extraction ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Everything there was.
    Complete,
    /// Stopped at a limit; the units read before it are kept.
    Truncated(Limit),
    /// Nothing read, for a reason that is not a fault.
    Skipped(Skip),
    /// The file could not be read: malformed, truncated, or the reader
    /// panicked on it. Units read before the fault are kept.
    Failed,
}

impl Outcome {
    /// The outcome as the index records it: `complete`, `truncated`,
    /// `skipped` or `failed`. Also what a log line may say about a file.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Truncated(_) => "truncated",
            Self::Skipped(_) => "skipped",
            Self::Failed => "failed",
        }
    }
}

/// One located piece of an attachment's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// Where in the file: a page, a sheet row, a slide, a paragraph, a line.
    pub location: Location,
    /// The text there, as written (not folded).
    pub text: String,
}

/// What one extraction produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// The units, in the file's order. Empty units are left out.
    pub units: Vec<Unit>,
    /// How it ended.
    pub outcome: Outcome,
}

/// Read the text out of one attachment.
///
/// `mime_type` decides the format, parameters and all (`charset=` for
/// text); a generic type (`application/octet-stream`, a bare zip) falls
/// back to `name`'s extension, and a generic PDF to its signature.
/// Never panics and never runs past `limits`: every way it can end is an
/// [`Outcome`], and the units read before a limit or a fault are kept.
///
/// Synchronous and CPU-bound: call it off any thread that draws or
/// answers requests (`spawn_blocking`). A PDF is read on a thread of its
/// own so its deadline holds even inside one slow page.
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
        // Guarded inside: its own thread, `catch_unwind` per page.
        Format::Pdf => pdf::extract(bytes, &mut budget),
        // The zip and XML readers return errors rather than panic, but
        // they read a stranger's bytes too: a panic is a failure here,
        // never an unwind into the indexer.
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

/// The formats this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pdf,
    Ooxml(ooxml::Kind),
    Text,
}

const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const XLSX: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
const PPTX: &str = "application/vnd.openxmlformats-officedocument.presentationml.presentation";

/// Types that say only "some bytes": the name decides instead.
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

/// `text/*` that is markup rather than words: read as lines, every tag
/// would be a match.
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

/// A MIME type's essence, lowercased, and its `charset` parameter.
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
}
