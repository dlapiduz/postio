//! Repositories: the only place that maps between [`postio_model`] and SQL.
//!
//! # Shape
//!
//! Each repository is a thin borrow of a connection — `AccountRepository::new(&connection)` —
//! so it costs nothing to make one per call and none of them own state. A
//! caller that has checked a connection out of the pool builds whichever
//! repositories it needs and drops them with the connection.
//!
//! # Conventions
//!
//! * **`create` assigns the id.** It takes `&mut` and writes the row id back
//!   into the value, so the caller holds a persisted entity afterwards rather
//!   than having to thread an id around by hand.
//! * **`get` returns `Option`.** A missing row is not an error; a broken row is.
//! * **`delete` returns `bool`** — whether there was anything to delete —
//!   because "already gone" is the expected outcome of a retried operation.
//! * **A write that spans tables runs in one transaction.** An account and its
//!   identities, a mailbox and its sync state: never half of one. Repositories
//!   open that transaction through [`crate::sql::in_scope`], which becomes a
//!   `SAVEPOINT` when it is nested — so the same call also composes inside a
//!   transaction the *caller* opened. The sync engine depends on that: the
//!   messages it fetched and the sync state describing them have to commit
//!   together, and they are written by two different repositories. An
//!   outermost scope is a `BEGIN IMMEDIATE` instead, which is not an
//!   optimisation — see [`crate::sql::in_scope`].
//! * **Timestamps are integer milliseconds, UTC**, and enums are stored as the
//!   `as_str` spelling the model documents, which the schema's `CHECK`
//!   constraints then enforce.

mod settings;

mod accounts;
mod contact_groups;
mod contacts;
mod correspondents;
mod cross_account;
mod digests;
mod drafts;
mod egress;
mod filter_decisions;
mod focus_classified;
mod labels;
mod mailbox_roles;
mod mailboxes;
mod markers;
mod messages;
mod operations;
mod reminders;
mod sync_state;
mod threading;
mod threads;
mod unsubscribe;

pub use accounts::{AccountRepository, IdentityRepository, SignatureRepository};
pub use contact_groups::ContactGroupRepository;
pub use contacts::ContactRepository;
pub use correspondents::{Correspondent, CorrespondentRepository};
pub use cross_account::{
    CrossAccountMove, CrossAccountMoveRepository, MovePhase, NewCrossAccountMove,
};
pub use digests::DigestRepository;
pub use drafts::{CancelSendOutcome, DraftRepository, ServerCopyLocation};
pub use egress::EgressLogRepository;
pub use filter_decisions::{FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason};
pub use focus_classified::{FocusClassifiedRepository, FocusStage};
pub use labels::LabelRepository;
pub use mailbox_roles::MailboxRoleRepository;
pub use mailboxes::{DraftCounts, MailboxRepository};
pub use markers::{InviteIdentity, InviteState, Marker, MarkerRepository, MarkerSource};
pub use operations::{OperationQueueRepository, QueuedOperation};
pub use reminders::{Reminder, ReminderRepository};
pub use settings::SettingsRepository;
pub use sync_state::SyncStateRepository;
pub use threading::{Threaded, ThreadingRepository};
pub use threads::{
    DEFAULT_THREAD_PAGE_SIZE, FocusListQuery, ThreadCursor, ThreadGroup, ThreadListQuery,
    ThreadListRow, ThreadOrder, ThreadRepository, UnifiedThreadListQuery,
};
pub use unsubscribe::UnsubscribeRepository;

pub use messages::{
    BackfillCandidate, ColumnFlag, DEFAULT_PAGE_SIZE, FlagSource, ListCursor, ListQuery, ListScope,
    MessageListRow, MessageRepository, MessageSet, StorageFootprint, StoredBody, UpsertReport,
};

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

/// What counts as a message for the sidebar: one that the list would show.
///
/// A message hidden pending a remote delete or a snooze not yet due is not
/// in the list, so counting it would put a number on screen the user cannot
/// reconcile with what they see. `snoozed_until` is compared against
/// SQLite's own clock rather than a bound parameter, matching the trigger
/// this mirrors (migration 0021) — both are the cached-count half of the
/// same two-tier arrangement the live list query (`where_clause`) is the
/// other half of.
pub(crate) const VISIBLE: &str = "deleted_locally = 0 AND (snoozed_until IS NULL OR snoozed_until <= (strftime('%s','now') * 1000))";

/// What Focus's lists leave out beyond what every list does ([`VISIBLE`]),
/// as a conjunct on the message `alias` names (`"rep."`, `"messages."`, or
/// `""` for a statement's only `messages`): the one place mail Focus holds
/// back leaves its inbox (spec 007, research R13).
///
/// A message a digest rule holds, until its delivery is archived (`⇧A`):
/// the digest's own row stands for it in the meantime, and a release
/// deletes the hold, so the message rejoins. Every read of a Focus scope
/// appends this -- the window, its representative's `NOT EXISTS` and its
/// slice, the count, the fold's keys and a flat read of the same view -- so
/// the rows, the total and the seek marks agree about what a row is. Each
/// test is a lookup by key into `digest_holds` and, for a delivered hold,
/// `digest_deliveries`: a cost per candidate row, never a walk.
pub(crate) fn focus_excludes(alias: &str) -> String {
    let message = if alias.is_empty() {
        "messages.id".to_owned()
    } else {
        format!("{alias}id")
    };
    format!(
        " AND NOT EXISTS (
             SELECT 1 FROM digest_holds held
              WHERE held.message_id = {message}
                AND (held.delivery_id IS NULL
                     OR NOT EXISTS (SELECT 1 FROM digest_deliveries delivered
                                     WHERE delivered.id = held.delivery_id
                                       AND delivered.archived_at IS NOT NULL)))"
    )
}

/// A timestamp as the schema stores it: milliseconds since the Unix epoch, UTC.
pub(crate) fn to_millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

/// The inverse of [`to_millis`].
///
/// A value the database cannot represent as a timestamp is clamped rather than
/// dropped: it came out of a row, so something is there, and refusing to show
/// the message would be worse than showing it with an odd date.
pub fn from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or(if millis < 0 {
        DateTime::<Utc>::MIN_UTC
    } else {
        DateTime::<Utc>::MAX_UTC
    })
}

/// Fails a read whose row holds a value this build does not understand.
///
/// The schema's `CHECK` constraints keep the enum columns to a known
/// vocabulary, so this only fires for a database written by a newer Postio or
/// edited by hand — in both cases guessing is worse than saying so.
pub(crate) fn unknown_enum(column: &'static str, value: impl Into<String>) -> Error {
    Error::UnknownEnum {
        column,
        value: value.into(),
    }
}

/// The `id` of a value that must already be persisted.
pub(crate) fn require_persisted(id: i64, entity: &'static str) -> Result<i64, Error> {
    if id > 0 {
        Ok(id)
    } else {
        Err(Error::NotPersisted { entity })
    }
}
