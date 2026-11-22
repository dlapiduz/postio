//! The half of the runtime that owns a database and a connection, kept out of `postio-core` so no
//! feature
//! union can put SQLite in the view layer's graph (`scripts/checks/check-crate-boundaries.py`).

pub mod engine;
pub mod network;
pub mod remote_images;
pub mod store;

pub use engine::{
    Clock, DrainSummary, Engine, EngineError, EngineParts, FilingSlot, Link, NetworkSource,
    NetworkState, POLL_INTERVAL, SyncSummary, SystemClock, retain, retire, stop,
};
// Re-exported so the composition root that builds `EngineParts` can name them without depending on
// `postio-sync`.
pub use postio_sync::{AttachmentPolicy, BackfillPolicy};
pub use store::{
    ListScope, LocalStore, MailStore, MessagePage, MessageSummary, PageRequest, Read, StoreError,
};
