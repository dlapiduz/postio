//! Search, as the frontend sees it.
//!
//! Two things cross: a result set becomes the list's contents, and each hit's
//! excerpt crosses as text plus *ranges*. Neither the query language nor the
//! highlighting is re-decided here — `postio-search` parses,
//! `postio_session::search` runs, and this only carries the answers. **One
//! query language**: Swift does not re-implement operator parsing, or the two
//! platforms accept different queries.

/// One hit, kept so the frontend can ask for its excerpt while it draws.
///
/// Bounded by `postio_session::search::HIT_LIMIT`, which is why holding the
/// whole set is not the thing `PRODUCT.md` §18 forbids: two hundred excerpts,
/// not a mailbox. A search matching forty thousand messages is still a count
/// and a few resident pages.
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
/// `AttributedString`; if the marking happened here one frontend would be
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
/// Bytes rather than characters because that is what the highlighter works
/// in; each frontend converts once, at its own edge. Converting twice is
/// where an off-by-one in a non-ASCII subject line comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MatchRangeFfi {
    /// First byte of the match.
    pub start: u32,
    /// One past the last byte.
    pub end: u32,
}

/// A `postio-search` snippet, split into plain text and match ranges.
///
/// `postio_search::highlight` marks matches with its own control characters
/// and `from_snippet` takes them back out — including dropping an unbalanced
/// marker, so a message that contains one cannot paint a highlight the query
/// did not earn. Splitting here rather than in each frontend is what keeps one
/// answer to what matched.
pub fn snippet_of(marked: &str) -> SnippetFfi {
    let highlighted = postio_search::highlight::from_snippet(marked);
    SnippetFfi {
        ranges: highlighted
            .matches
            .iter()
            .map(|range| MatchRangeFfi {
                start: range.start as u32,
                end: range.end as u32,
            })
            .collect(),
        text: highlighted.text,
    }
}

/// One operator in the query, drawn as a pill.
///
/// The parse, not a second one. `postio-search` says where each operator sits
/// and what its source text is; both frontends draw the same reading of the
/// same query, which matters more here than anywhere because **the chips are
/// how somebody learns Postio's query language**. Two readings would be two
/// languages on two platforms.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChipFfi {
    /// Position in the query's tokens, for popping this one out.
    pub index: u32,
    /// The exact source text, so what the chip says is what is in the field.
    pub label: String,
    /// Whether it was negated with a leading `-`.
    pub negated: bool,
    /// Whether the operator has a value yet.
    ///
    /// A half-typed `from:` is still worth drawing: it tells the user the
    /// parser understood the keyword, which is the moment the language is
    /// being learned.
    pub complete: bool,
    /// What a screen reader should say instead of the raw text.
    ///
    /// `postio_ui::search::spoken`'s wording — "from Ada", not "from colon
    /// Ada" — carried rather than re-derived, for the same reason the label
    /// is.
    pub spoken: String,
}

/// The query's operators, in the order they were typed.
///
/// Free text is not a chip: it stays plain, because it is the part the user is
/// usually still editing.
///
/// A free function, like `commands()`: it parses a string and reads no session
/// state, so a bar can draw chips before any store is open.
#[uniffi::export]
pub fn query_chips(query: String) -> Vec<ChipFfi> {
    let parsed = postio_search::parse(&query, chrono::Utc::now().date_naive());
    postio_ui::search::chips(&parsed)
        .into_iter()
        .map(|chip| ChipFfi {
            index: chip.index as u32,
            label: chip.label.clone(),
            negated: chip.negated,
            complete: chip.complete,
            spoken: postio_ui::search::spoken(&chip),
        })
        .collect()
}

/// What one search turned out to be, as the field's right-hand end says it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutcomeFfi {
    /// The line canvas 2b draws — "14 hits · 11 ms", plus any caveats.
    pub readout: String,
    /// The same facts in words, for a screen reader: "·" and "ms" are
    /// punctuation and an abbreviation rather than something to read out.
    pub spoken: String,
    /// How many messages matched.
    pub hits: u64,
}
