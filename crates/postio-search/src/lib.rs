//! Postio's search: the query-operator parser and the pure types the search surfaces are built
//! from. [`parse`] turns whatever is in the search entry, half typed included, into a
//! [`ParsedQuery`].
//! Pure: no clock, database or configuration (the reference date is a parameter), so it is safe on
//! every keystroke. Total: no error type; `from:` or `is:unr` parse into a [`query::Partial`] that
//! constrains nothing, and unknown operators degrade to free text.
//! The FTS5 index and executor live in `postio-index`, so this crate stays free of `rusqlite`
//! (`check-crate-boundaries.py`).

pub mod date;
pub mod edit;
pub mod facets;
pub mod highlight;
pub mod matcher;
pub mod natural;
mod parser;
pub mod passage;
pub mod query;
pub mod relax;
pub mod results;
mod size;
pub mod suggest;

pub use facets::{Facets, Refinement, Scope};
pub use highlight::Highlighted;
pub use parser::parse;
pub use query::ParsedQuery;
pub use results::{Instead, ResultOrder, SearchHit, SearchResults, TOTAL_HITS_CAP};
