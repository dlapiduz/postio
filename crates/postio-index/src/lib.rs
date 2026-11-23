//! Postio's full-text search index and query executor, in a crate apart from `postio-search`
//! (parser, query model, facets, highlighting).
//! Different packages, not a feature: Cargo unions features across the workspace, and one member
//! turning it on put SQLite in the view layer's graph.

pub mod error;
pub mod events;
pub mod executor;
pub mod index;

pub use error::{Error, Result};
pub use executor::{SearchRequest, search};
