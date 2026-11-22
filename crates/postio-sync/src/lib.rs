//! Postio's sync engine: queue drainer, resync, IDLE, backoff, backfill. Everything goes through
//! `MailBackend`; nothing here names a protocol type (ADR 0001),
//! so every test runs against `MockBackend`. A mutating action writes SQLite and enqueues in one
//! transaction; this crate owns what happens next.

pub mod backfill;
pub mod blob_sink;
mod carry;
pub mod coalesce;
pub mod connect;
mod contacts;
mod correspondents;
mod cross_account;
pub mod discover;
pub mod drafts;
pub mod drain;
pub mod filing;
mod flag_sweep;
pub mod initial;
pub mod order;
pub mod resync;
pub mod retry;
pub mod send;
pub mod status;
pub mod threaded;
pub mod watch;

pub use backfill::{
    AttachmentPolicy, Backfill, BackfillPolicy, BackfillProgress, BodyRequest, Claim, Priority,
    Want,
};
pub use coalesce::{Plan, Step, coalesce};
pub use connect::{Blocker, Link, NetworkState, ReconnectPolicy, Supervisor};
pub use drain::{DrainReport, Drainer, FailedOperation, SyncError};
pub use filing::{BuiltIn, Classifier, FiledMessage, FilingEffects, FilingPass, FocusFiling};
pub use initial::{
    DEFAULT_BATCH_SIZE, Progress, Report, commit_batch, sync_mailbox, sync_mailbox_with_batch_size,
};
pub use order::sync_priority;
pub use resync::{Outcome, resync_mailbox, resync_mailbox_filing, resync_mailbox_with_policy};
pub use retry::RetryPolicy;
pub use send::SmtpContext;
pub use status::{StatusTracker, SyncProgress, SyncStatus};
pub use watch::{Attention, Wake, Watch, WatchPolicy, Watcher};
