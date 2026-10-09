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

use std::time::Duration;

pub use postio_search::results::Location;

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
