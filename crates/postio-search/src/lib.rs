//! Postio's search: the query-operator parser, and the pure types the search
//! surfaces are built from.
//!
//! Search is one of the three things Postio has to do better than every other
//! mail client (`docs/PRODUCT.md` §25), and the query bar is its front door. This
//! crate provides the parser half of that: [`parse`] turns whatever is in the
//! search entry — including a query that is half typed — into a
//! [`ParsedQuery`].
//!
//! ```
//! use chrono::NaiveDate;
//! use postio_search::{parse, query::Filter};
//!
//! let today = NaiveDate::from_ymd_opt(2026, 8, 22).unwrap();
//! let query = parse("from:lena after:aug1 has:attach kubernetes", today);
//!
//! let filters: Vec<_> = query.filters().map(|c| c.filter.clone()).collect();
//! assert_eq!(filters[0], Filter::From("lena".into()));
//! assert_eq!(filters[2], Filter::HasAttachment);
//! assert_eq!(query.fts_match().as_deref(), Some(r#""kubernetes""#));
//! ```
//!
//! # Design
//!
//! * **Pure.** [`parse`] reads no clock, no database and no configuration; the
//!   reference date for relative operators is a parameter. That is what makes
//!   it exhaustively unit-testable and safe to call on every keystroke.
//! * **Total.** There is no error type. `from:`, `is:unr` and `after:2026-`
//!   are ordinary intermediate states of someone typing, so they parse into a
//!   [`query::Partial`] that constrains nothing. Unknown operators degrade to
//!   free text.
//! * **Structured, not SQL.** The output separates the operator filters from
//!   the free text and offers the free text as an FTS5 `MATCH` expression via
//!   [`ParsedQuery::fts_match`]. Building the statement, ranking and snippeting
//!   belong to the query executor, not here.
//! * **Chip-ready.** Every token carries its byte [`query::Span`] and its raw
//!   source text, so the search bar can render one chip per token, find the
//!   chip under the caret with [`ParsedQuery::token_at`] and pop it with
//!   [`ParsedQuery::remove_token`].
//!
//! # Operators
//!
//! `from:` `to:` `subject:` `has:attach` `is:unread` `is:flagged` `before:`
//! `after:` `in:` `filename:` `larger:` `smaller:` `list:`, each optionally
//! negated with a leading `-`, each composable with the others and with free
//! text. Dates accept ISO (`2026-01-01`), loose (`aug1`) and relative
//! (`yesterday`, `last week`, `3m`) forms; sizes accept `K`/`M`/`G`.
//!
//! # Where the FTS5 index and executor went
//!
//! The index (the FTS5 schema and triggers) and the query executor used to
//! sit behind an `index` cargo feature here, off by default, on the theory
//! that whoever wired search into the running application would enable it.
//! Nobody could: Cargo resolves features as a union across the whole
//! workspace resolve, and `postio-gtk` depends on this crate for the parser
//! alone, so the moment anything turned `index` on, `rusqlite` landed in the
//! view layer's graph and `scripts/check-crate-boundaries.py` failed —
//! correctly (`postio-svx`). They now live in `postio-index`, a crate only
//! the runtime side depends on; this crate stays pure and feature-free, and
//! `postio-gtk` needs no change to keep reaching [`parse`], [`ParsedQuery`],
//! [`facets`] and [`highlight`].

mod date;
pub mod facets;
pub mod highlight;
mod parser;
pub mod query;
pub mod results;
mod size;

pub use facets::{Facets, Refinement, Scope};
pub use highlight::Highlighted;
pub use parser::parse;
pub use query::ParsedQuery;
pub use results::{SearchHit, SearchResults, TOTAL_HITS_CAP};
