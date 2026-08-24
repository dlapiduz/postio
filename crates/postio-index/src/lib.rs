//! Postio's FTS5 search index and query executor.
//!
//! # Why this is a separate crate from `postio-search`
//!
//! `postio-search` used to gate this half behind an optional `index` Cargo
//! feature, on the theory that whoever wires the executor into the running
//! application would enable it explicitly. Nobody could: Cargo resolves
//! features as a **union across the whole workspace resolve**, and
//! `postio-gtk` depends on `postio-search` for the pure query-operator parser.
//! The moment *any* workspace member turned `index` on, `rusqlite` landed in
//! the view layer's dependency graph and
//! `scripts/check-crate-boundaries.py` failed — correctly, since a workspace
//! build really would link SQLite into `postio-gtk`. No manifest ever enabled
//! it, so [`search`] and [`index::ensure_schema`] had never run inside Postio
//! (`postio-svx`).
//!
//! This is the identical trap CLAUDE.md already documents for
//! `postio-core`/`postio-runtime`, and it has the same answer: the pure half
//! and the SQL half have to be different *packages*, not different features
//! of one package. So the executor, the FTS5 schema and this crate's error
//! type live here, in a crate only the runtime side depends on, while the
//! parser, the query model, facets and highlighting stay in `postio-search`
//! where `postio-gtk` can keep reaching them without ever seeing `rusqlite`.
//!
//! # What is here
//!
//! * [`index`] — the FTS5 schema, its sync triggers, and the rebuild path.
//! * [`executor`] — [`search`]: combining a [`postio_search::ParsedQuery`]
//!   with the FTS5 index, ranking the results and cutting snippets.
//! * [`error`] — this crate's `thiserror` error type.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod executor;
pub mod index;

pub use error::{Error, Result};
pub use executor::{SearchRequest, search};
