//! The half of Postio's runtime that owns a database and a connection.
//!
//! # Why this is not in `postio-core`
//!
//! `postio-core` is the UI-agnostic contract: commands in, events out, the
//! command registry, the keymap, the app state. `postio-gtk` depends on it,
//! and so would any other frontend — that is what makes a second one
//! possible.
//!
//! Which means `postio-core` must have no path to `rusqlite` or `io-imap`,
//! and *no path* is stronger than it sounds. Putting them behind an
//! off-by-default feature is not enough: Cargo resolves features as a union
//! across everything being built, so the moment one crate in the workspace
//! turns that feature on, every crate depending on `postio-core` has SQLite
//! in its graph — including the view layer, which
//! `scripts/check-crate-boundaries.py` refuses outright. That is not a
//! checker being fussy. In a workspace build the view layer really would
//! link the SQL.
//!
//! So the database half lives in a crate of its own. `postio-core` has no
//! optional dependencies at all, and no feature of it can put storage in
//! anybody's graph.
//!
//! # What is here
//!
//! * [`store`] — reading the local store off the calling thread, behind a
//!   trait so a frontend can be written against it and a test can answer from
//!   a table.
//! * [`engine`] — the loop that drains the operation queue, backfills message
//!   bodies, and keeps the connection up.
//!
//! Both are joined to a frontend by `postio-app`, which is the only crate
//! that knows both halves exist.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod engine;
pub mod store;

pub use engine::{DrainSummary, Engine, EngineError, EngineParts, Link, NetworkState};
pub use store::{
    ListScope, MailStore, MessagePage, MessageSummary, PageRequest, Read, SqliteStore, StoreError,
};
