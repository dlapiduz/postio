//! Search, as the frontend sees it.
//!
//! Two things cross: a result set becomes the list's contents, and each hit's
//! excerpt crosses as text plus *ranges*. Neither the query language nor the
//! highlighting is re-decided here — `postio-search` parses, `postio-session`
//! runs, and this only carries the answers.

/// One hit, kept so the frontend can ask for its excerpt while it draws.
///
/// Bounded by `postio_session::search::HIT_LIMIT`, which is why holding the
/// whole set is not the thing `PRODUCT.md` §18 forbids: it is two hundred
/// excerpts, not a mailbox.
#[derive(Debug, Clone)]
pub struct Hit {
    /// The message this hit points at.
    pub message: i64,
    /// Its excerpt, already located.
    pub snippet: SnippetFfi,
}

/// A hit's excerpt, and where in it the query matched.
///
/// **Ranges, not marked-up text** — the same decision #568 made for the
/// palette. GTK escapes them into Pango markup and Swift builds an
/// `AttributedString`; if the marking happened here, one frontend would be
/// escaping the other's markup, and if it happened in each frontend there
/// would be two answers to what matched.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnippetFfi {
    /// The excerpt, as plain text.
    pub text: String,
    /// Byte ranges within `text` that matched the query.
    pub ranges: Vec<MatchRangeFfi>,
}

/// One matched span, in bytes into [`SnippetFfi::text`].
///
/// Bytes rather than characters because that is what both toolkits index by
/// once they have the string, and converting twice is where an off-by-one in
/// a non-ASCII subject line would come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MatchRangeFfi {
    /// First byte of the match.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

/// A `postio-search` snippet, split into plain text and match ranges.
///
/// `postio_search::highlight` marks matches with its own delimiters; splitting
/// them out here rather than in each frontend is what keeps one answer to what
/// matched. See [`SnippetFfi`].
pub fn snippet_of(marked: &str) -> SnippetFfi {
    let highlighted = postio_search::highlight::from_snippet(marked);
    SnippetFfi {
        text: highlighted.text.clone(),
        ranges: highlighted
            .matches
            .iter()
            .map(|range| MatchRangeFfi {
                start: range.start as u32,
                end: range.end as u32,
            })
            .collect(),
    }
}
