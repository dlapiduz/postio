//! A backend rather than an importer, so a maildir gets threading, indexing, the windowed list and
//! idempotent incremental passes identical to the server path (#1278). `uidlist` invents the
//! numbers the seam needs and writes them down.

pub mod backend;
pub mod store;
pub mod uidlist;

pub use backend::MaildirBackend;
pub use store::{Entry, Folder, LocalStore, Snapshot};
pub use uidlist::UidList;
