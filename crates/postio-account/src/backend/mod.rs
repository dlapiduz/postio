//! The seam between Postio and a mail server (ADR 0001). No protocol type crosses it (a test in
//! `tests/backend.rs` fails if a source here names `io_imap`); capabilities are detected after
//! authentication, never assumed.
//! The caller owns batching (`UidSet::chunks`) and cancellation (`CancelToken`); bodies stream to a
//! `BodySink`, never a returned buffer.

mod capability;
mod error;
pub mod identity;
mod message;
mod mock;
mod sink;
mod uid_set;

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use postio_model::{ChangeFeed, FlagSet, ModSeq, RemoteId, Uid};

use crate::cancel::CancelToken;

pub use self::capability::{Capabilities, Capability};
pub use self::error::{BackendError, BackendResult};
pub use self::message::{
    AppendMessage, BodyPart, BodyStructure, Envelope, FetchedBody, FetchedMessage, FlagChange,
    FlagUpdate, MailboxEvent, MailboxFilter, MailboxStatus, MailboxSummary, PartNode, SelectMode,
    UidMapping, resolve_roles, resolve_roles_with_known_names,
};
pub use self::mock::{
    Fault, FetchEvent, Held, MockBackend, MockBackendBuilder, MockMailbox, MockMessage,
};
pub use self::sink::{BodySink, CountingSink, VecSink};
pub use self::uid_set::UidSet;

pub use postio_model::Disposition;

/// Shared across tasks (`Arc<dyn MailBackend>`); every method takes `&self`, so pooling and
/// serialization are the backend's problem.
/// Callers branch on `is_transient`, `is_authentication_failure` and `requires_full_resync`, never
/// on the variant.
/// A protocol without a change counter (`ModSeq`) reports `None` and sync falls back to comparing
/// listings (ADR 0018 Q2).
#[async_trait]
pub trait MailBackend: Send + Sync + fmt::Debug {
    fn describe(&self) -> &'static str;

    /// `None` where the backend has no priorities. A backfill holding every shared connection makes
    /// a click wait; a priority-queued backend hands the next free connection to the click.
    fn for_background(&self) -> Option<std::sync::Arc<dyn MailBackend>> {
        None
    }

    /// Returns the capability set observed after authentication. An empty set is
    /// `EmptyCapabilities`, never a silent downgrade (ADR 0001 Q3).
    async fn connect(&self) -> BackendResult<Capabilities>;

    /// Closes the session. Closing an already-closed backend succeeds.
    async fn disconnect(&self) -> BackendResult<()>;

    async fn capabilities(&self) -> BackendResult<Capabilities>;

    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>>;

    /// Idempotent: an already-existing mailbox is success, since two clients may race.
    /// Called only by discovery, for a reserved role that resolves to no folder. A refusal is
    /// `Rejected` so it is recorded and not retried every pass.
    /// No default implementation: `Ok(())` would report success for nothing, and an error would let
    /// a backend forget to answer.
    async fn create_mailbox(&self, path: &str) -> BackendResult<()>;

    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus>;

    async fn status(&self, path: &str) -> BackendResult<MailboxStatus>;

    /// With `changed_since`, only messages whose modification sequence is greater than it are
    /// returned. Callers split large sets with `UidSet::chunks`: everything is returned at once.
    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>>;

    /// Also reads the three promoted headers (`List-Unsubscribe`, `Precedence`, `Auto-Submitted`)
    /// into `FetchedMessage::promoted`. Incremental passes use it; a first sync does not, since
    /// every enumerated message would pay for the fields (ADR 0025).
    /// The default answers like `fetch_headers` with the facts unknown.
    async fn fetch_headers_for_filing(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>> {
        self.fetch_headers(mailbox, uids, changed_since, cancel)
            .await
    }

    /// On failure or cancellation `BodySink::finish` is not called and whatever reached the sink
    /// must be discarded.
    async fn fetch_part(
        &self,
        mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody>;

    async fn fetch_body(
        &self,
        mailbox: &str,
        id: &RemoteId,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody> {
        self.fetch_part(mailbox, id, &BodyPart::Whole, sink, cancel)
            .await
    }

    /// Fetches one section of many messages in few round trips: grouping by section turns tens of
    /// thousands of waits into hundreds (#1551).
    /// Does not stream: every answer is held in memory, so the caller must bound the set by
    /// `RFC822.SIZE` first.
    /// The default loops over `fetch_part`. One entry per id the server returned, any order; a
    /// message expunged since is absent, not an error.
    async fn fetch_sections(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        part: &BodyPart,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<(RemoteId, Vec<u8>)>> {
        let mut answered = Vec::with_capacity(ids.len());
        for id in ids {
            let mut sink = crate::backend::sink::VecSink::new();
            match self.fetch_part(mailbox, id, part, &mut sink, cancel).await {
                Ok(_) if sink.is_finished() => answered.push((id.clone(), sink.into_inner())),
                // An unfinished sink is a fragment, and a fragment is worse
                // than nothing (the sink's own contract).
                Ok(_) => continue,
                Err(BackendError::NoSuchMessage { .. }) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(answered)
    }

    async fn store_flags(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>>;

    /// The mapping is empty unless the server speaks UIDPLUS.
    async fn move_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>>;

    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>>;

    /// With `ids`, only those are expunged (`UID EXPUNGE`, RFC 4315), so another client's
    /// `\Deleted` marks survive; a server without it declines rather than widen.
    /// An IMAP server reports sequence numbers, so a real backend returns an empty list: treat
    /// success as "resync this mailbox".
    async fn expunge(
        &self,
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>>;

    /// Returns where it landed under UIDPLUS, `None` otherwise.
    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>>;

    /// The cross-account move saga's confirmation fallback (#188, ADR 0005 Q9). The default answers
    /// `None`, "cannot search", which the saga reads as unconfirmed. More than one match answers
    /// the newest.
    async fn find_by_message_id(
        &self,
        mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        let _ = (mailbox, message_id);
        Ok(None)
    }

    /// Without it the caller walks `1..=UIDNEXT-1` in chunks, paying a round trip per
    /// fully-expunged chunk (46% of a first sync's wall clock on a real account, #78, #727).
    /// The default answers `None`, "cannot enumerate". Unordered, and a snapshot: a UID expunged
    /// since returns nothing from `FETCH`.
    async fn existing_uids(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<Uid>>> {
        let _ = (mailbox, cancel);
        Ok(None)
    }

    /// Only a UID and flag list per message crosses the wire, so a sixty-thousand-message folder
    /// costs megabytes, not hundreds.
    /// The whole truth about flags: an unlisted message was not there. No `MODSEQ`.
    /// The default answers `None`, "cannot list flags alone"; other backends have a reliable
    /// `change_feed` and are never swept.
    async fn fetch_flags(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<(Uid, FlagSet)>>> {
        let _ = (mailbox, cancel);
        Ok(None)
    }

    /// `Unreliable` makes the sync engine read every flag now and then rather than only what
    /// `CHANGEDSINCE` reports. The IMAP backend looks this up in the provider preset table.
    fn change_feed(&self) -> ChangeFeed {
        ChangeFeed::Reliable
    }

    /// Returns on any event, on `timeout`, or on `cancel`; the last two with an empty vector, since
    /// "nothing happened" is not a failure.
    /// Events are raw: they say that the mailbox changed, not what it holds. Respond with a resync
    /// pull, not a diff.
    /// A real server reports only other connections' changes, so never wait on a watch to confirm
    /// your own write. The mock queues every change, the caller's own included
    /// (`tests/backend_parity.rs` marks the seam).
    async fn idle(
        &self,
        mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>>;
}
