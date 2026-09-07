//! A maildir on this machine, read through the same seam as a server (#1278).
//!
//! # Why a backend rather than an importer
//!
//! The obvious shape for "open a maildir" is an importer: walk the tree, write
//! every message into Postio's store, done. It is also the shape that has to
//! answer, alone, every question the sync machinery already answers — what is
//! new since last time, what happened to a message that was deleted, what a
//! second run does. `MailBackend` is that machinery's seam, so a maildir that
//! implements it gets threading, indexing, the windowed list, incremental
//! passes and "running it twice changes nothing" for free, and gets them
//! *identical* to the server path rather than similar to it.
//!
//! What a maildir does not have is the numbers that seam is written in terms
//! of. [`uidlist`] invents them and writes them down.

pub mod backend;
pub mod store;
pub mod uidlist;

pub use backend::MaildirBackend;
pub use store::{Entry, Folder, LocalStore, Snapshot};
pub use uidlist::UidList;
