//! Repositories: the only place that maps between [`postio_model`] and SQL. Each is a thin borrow
//! of a connection.
//! `create` takes `&mut` and writes the id back; `get` returns `Option`; `delete` returns whether
//! there was anything, since "already gone" is the expected outcome of a retried operation.
//! A write spanning tables runs through [`crate::sql::in_scope`], a `SAVEPOINT` when nested so it
//! composes inside the caller's transaction (sync commits messages and sync state together);
//! outermost it is `BEGIN IMMEDIATE`, which is not an optimisation.

mod settings;

mod accounts;
mod calendar;
mod contact_groups;
mod contact_settings;
mod contact_suggestions;
mod contacts;
mod correspondents;
mod cross_account;
mod digests;
mod drafts;
mod egress;
mod expiry;
mod filter_decisions;
mod focus_classified;
mod labels;
mod mailbox_roles;
mod mailboxes;
mod markers;
mod messages;
mod operations;
mod reminders;
mod sweeps;
mod sync_state;
mod threading;
mod threads;
mod unsubscribe;

pub use accounts::{AccountRepository, IdentityRepository, SignatureRepository};
pub use calendar::{
    CalendarRepository, ChangeKind, ChangeState, DueReminder, EventChange, HeldEvent,
    PlannedReminder, ReminderState, RemoteCalendar, StoredEvent, StoredOccurrence,
};
pub use contact_groups::ContactGroupRepository;
pub use contact_settings::{ContactSettingsRepository, DerivedSettings};
pub use contact_suggestions::ContactSuggestionRepository;
#[cfg(feature = "test-support")]
pub(crate) use contacts::list_statements as contact_list_statements;
pub(crate) use contacts::release_own_address;
#[cfg(feature = "test-support")]
pub(crate) use contacts::suggestion_statements as contact_suggestion_statements;
pub use contacts::{
    BookChanges, BookPerson, CatchUp, ContactCursor, ContactRepository, ExportRow, GroupExport,
    SavedForm, SyncLinkRow,
};
pub use correspondents::{Correspondent, CorrespondentRepository};
pub use cross_account::{
    CrossAccountMove, CrossAccountMoveRepository, MovePhase, NewCrossAccountMove,
};
pub use digests::{DigestRepository, OpenDelivery};
pub use drafts::{CancelSendOutcome, DraftRepository, ServerCopyLocation};
pub use egress::EgressLogRepository;
pub use expiry::{Expired, ExpiryRepository};
pub use filter_decisions::{FilterDecision, FilterDecisionRepository, FilterLayer, FilterReason};
pub use focus_classified::{FocusClassifiedRepository, FocusStage};
pub use labels::LabelRepository;
pub use mailbox_roles::MailboxRoleRepository;
pub use mailboxes::{DraftCounts, MailboxRepository};
pub use markers::{InviteIdentity, InviteState, Marker, MarkerRepository, MarkerSource};
pub use operations::{OperationQueueRepository, QueuedOperation};
pub use reminders::{Reminder, ReminderRepository};
pub use settings::SettingsRepository;
pub use sweeps::{Found, ListMail, SweepRepository};
pub use sync_state::SyncStateRepository;
pub use threading::{Threaded, ThreadingRepository};
pub use threads::{
    DEFAULT_THREAD_PAGE_SIZE, FocusListQuery, Marked, ThreadCursor, ThreadGroup, ThreadListQuery,
    ThreadListRow, ThreadOrder, ThreadRepository, UnifiedThreadListQuery,
};
pub use unsubscribe::UnsubscribeRepository;

pub use messages::{
    BackfillCandidate, ColumnFlag, DEFAULT_PAGE_SIZE, FlagSource, ListCursor, ListQuery, ListScope,
    MessageListRow, MessageRepository, MessageSet, StorageFootprint, StoredBody, StoredServerState,
    UpsertReport,
};

use chrono::{DateTime, Utc};

use crate::error::{Error, Result};

/// A message hidden pending a remote delete or an unfinished snooze is not in the list, so counting
/// it would show a number the user cannot reconcile. `snoozed_until` is compared against SQLite's
/// clock to match the trigger this mirrors (migration 0021).
pub(crate) const VISIBLE: &str = "deleted_locally = 0 AND (snoozed_until IS NULL OR snoozed_until <= (strftime('%s','now') * 1000))";

/// The one place mail Focus holds back leaves its inbox (spec 007, research R13): a message a
/// digest rule holds until its delivery is archived, and a conversation whose reminder fired and
/// stands (its reminder row stands for it). Every Focus read appends this, so rows, total and seek
/// marks agree. Each test is a lookup by key, a cost per candidate row, never a walk.
pub(crate) fn focus_excludes(alias: &str) -> String {
    let (message, thread) = if alias.is_empty() {
        ("messages.id".to_owned(), "messages.thread_id".to_owned())
    } else {
        (format!("{alias}id"), format!("{alias}thread_id"))
    };
    format!(
        " AND NOT EXISTS (
             SELECT 1 FROM digest_holds held
              WHERE held.message_id = {message}
                AND (held.delivery_id IS NULL
                     OR NOT EXISTS (SELECT 1 FROM digest_deliveries delivered
                                     WHERE delivered.id = held.delivery_id
                                       AND delivered.archived_at IS NOT NULL)))
           AND NOT EXISTS (
             SELECT 1 FROM reminders surfaced
              WHERE surfaced.thread_id = {thread}
                AND surfaced.fired_at IS NOT NULL
                AND surfaced.cancelled_at IS NULL
                AND surfaced.settled_at IS NULL)"
    )
}

pub(crate) fn to_millis(at: DateTime<Utc>) -> i64 {
    at.timestamp_millis()
}

/// A value the database cannot represent is clamped, not dropped: refusing to show the message
/// would be worse than an odd date.
pub fn from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or(if millis < 0 {
        DateTime::<Utc>::MIN_UTC
    } else {
        DateTime::<Utc>::MAX_UTC
    })
}

/// Only fires for a database written by a newer Postio or edited by hand: guessing would be worse.
pub(crate) fn unknown_enum(column: &'static str, value: impl Into<String>) -> Error {
    Error::UnknownEnum {
        column,
        value: value.into(),
    }
}

pub(crate) fn require_persisted(id: i64, entity: &'static str) -> Result<i64, Error> {
    if id > 0 {
        Ok(id)
    } else {
        Err(Error::NotPersisted { entity })
    }
}
