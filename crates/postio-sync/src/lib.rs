//! Postio's sync engine: the queue drainer, resync, IDLE and backoff.
//!
//! # Where this sits
//!
//! Everything below is reached through
//! [`MailBackend`](postio_imap::backend::MailBackend); nothing here names a
//! protocol type. That is ADR 0001's requirement and it is what makes the whole
//! engine testable against
//! [`MockBackend`](postio_imap::backend::MockBackend) with no server and no
//! network — every test in this crate does exactly that.
//!
//! # Local-first, drained later
//!
//! A mutating action writes SQLite and enqueues an operation in one
//! transaction, and the UI repaints without waiting (`CLAUDE.md`). This crate
//! owns what happens next:
//!
//! - [`backfill`] decides which message body to download next: newest first
//!   in the background, and immediately for the one the user just opened.
//! - [`coalesce`] folds a batch down to the operations the server actually
//!   needs, so a minute of offline flagging is not replayed keystroke by
//!   keystroke.
//! - [`drain`] sends them in order, resolves conflicts against what the server
//!   says, and settles each queue row.
//! - [`send`] is what [`drain`] calls for `Operation::Send`: build the
//!   message, hand it to SMTP, file the Sent copy.
//! - [`retry`] decides when a failure is worth another attempt, and when the
//!   user has to be told instead.
//! - [`connect`] keeps the session up underneath all of it: exponential
//!   backoff with jitter, a flapping link that converges rather than thrashes,
//!   and a hard stop on a refused password.
//! - [`initial`] enumerates a mailbox for the first time, newest message
//!   first, so the app feels usable before the sync is done.
//! - [`order`] ranks mailboxes *against each other* so INBOX and the folders a
//!   person reads next are queued before a large Archive or Junk.
//! - [`resync`] keeps an already-synced mailbox current: QRESYNC/CONDSTORE
//!   incremental pulls, falling back to [`initial`] for a full re-enumeration
//!   when the local state cannot answer "what changed" — most importantly
//!   when `UIDVALIDITY` moves.
//! - [`status`] turns [`connect::Link`] transitions and [`initial::Progress`]
//!   batches into the throttled status-line state the UI needs, without this
//!   crate having to know what an `Event` is.
//! - [`watch`] decides when to look: `IDLE` on the one mailbox worth a
//!   connection of its own, interval polling everywhere else, and a periodic
//!   reconciliation that keeps a silently deaf `IDLE` from hiding new mail.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backfill;
pub mod coalesce;
pub mod connect;
pub mod discover;
pub mod drafts;
pub mod drain;
pub mod initial;
pub mod order;
pub mod resync;
pub mod retry;
pub mod send;
pub mod status;
pub mod watch;

pub use backfill::{Backfill, BackfillPolicy, BackfillProgress, BodyRequest, Claim, Priority};
pub use coalesce::{Plan, Step, coalesce};
pub use connect::{Blocker, Link, NetworkState, ReconnectPolicy, Supervisor};
pub use drain::{DrainReport, Drainer, FailedOperation, SyncError};
pub use initial::{
    DEFAULT_BATCH_SIZE, Progress, Report, sync_mailbox, sync_mailbox_with_batch_size,
};
pub use order::sync_priority;
pub use resync::{Outcome, resync_mailbox};
pub use retry::RetryPolicy;
pub use send::SmtpContext;
pub use status::{StatusTracker, SyncProgress, SyncStatus};
pub use watch::{Attention, Wake, Watch, WatchPolicy, Watcher};
