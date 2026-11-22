//! Search as the frontend sees it: an excerpt as text plus ranges, and a query as chips. One query
//! language: `postio-search`
//! parses, and Swift does not re-implement operator parsing.

/// **Ranges, not marked-up text** (#568): each frontend marks them its own way.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SnippetFfi {
    pub text: String,
    /// Byte ranges within `text` that matched the query.
    pub ranges: Vec<MatchRangeFfi>,
}

/// Bytes, not characters, because that is what the highlighter works in; each frontend converts
/// once, at its own edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct MatchRangeFfi {
    pub start: u32,
    pub end: u32,
}

/// One operator in the query, drawn as a pill: `postio-search`'s reading, so both frontends draw
/// the same chips.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChipFfi {
    /// Position in the query's tokens, for popping this one out.
    pub index: u32,
    /// The exact source text, so what the chip says is what is in the field.
    pub label: String,
    pub negated: bool,
    /// Whether the operator has a value yet; a half-typed `from:` is still drawn.
    pub complete: bool,
    /// What a screen reader should say instead of the raw text (`postio_ui::search::spoken`): "from
    /// Ada", not "from colon Ada".
    pub spoken: String,
}

/// The query's operators, in the order typed; free text is not a chip. A free function: it reads no
/// session state, so a bar can draw chips before any store is open.
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
