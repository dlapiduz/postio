//! `MailBackend`: the seam between Postio and a mail server.
//!
//! Everything above this trait — the sync engine, the runtime, the UI — speaks
//! domain types and knows nothing about IMAP. Everything below it is one
//! protocol's problem. That is not a stylistic preference: `io-imap` is
//! pre-1.0 and published six minor releases in eleven weeks, one of which
//! broke nearly every public signature. ADR 0001 makes the boundary a hard
//! requirement, and this module is where it is drawn.
//!
//! # The rules
//!
//! 1. **No protocol type crosses this line.** Not in an argument, not in a
//!    return value, not in an error. A test in `tests/backend.rs` reads the
//!    sources in this directory and fails if one names `io_imap`.
//! 2. **Capabilities are detected, never assumed.** Everything that depends on
//!    an extension goes through [`Capabilities::require`], and the set comes
//!    from the list read *after* authentication. iCloud advertises neither
//!    CONDSTORE nor QRESYNC nor IDLE nor UIDPLUS before login.
//! 3. **The caller owns batching and cancellation.** [`UidSet::chunks`] splits
//!    a large fetch; a [`CancelToken`] stops one. A backend never decides to
//!    hold ten thousand messages in memory on the caller's behalf.
//! 4. **Bytes stream.** Bodies and attachments go to a [`BodySink`], never
//!    into a returned buffer.
//!
//! # Testing against it
//!
//! [`MockBackend`] is an in-memory implementation with injectable faults and
//! latency. It is not `#[cfg(test)]`: `postio-sync` is developed against it,
//! and the whole sync engine can therefore be built and tested with no server
//! and no network at all.
//!
//! ```
//! # use postio_account::backend::{MailBackend, MailboxFilter, MockBackend, MockMailbox};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // The folders are named here, deliberately: `MockBackend::new` has none,
//! // so a test that needs an inbox has to say where it came from. See its
//! // own docs for the bug that arrangement was hiding.
//! let backend = MockBackend::builder().mailbox(MockMailbox::new("INBOX")).build();
//! let capabilities = backend.connect().await?;
//! let mailboxes = backend.list_mailboxes(&MailboxFilter::all()).await?;
//!
//! assert!(capabilities.supports_incremental_sync());
//! assert_eq!(mailboxes[0].path, "INBOX");
//! # Ok(())
//! # }
//! ```

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
use postio_model::{ModSeq, RemoteId, Uid};

use crate::cancel::CancelToken;

pub use self::capability::{Capabilities, Capability};
pub use self::error::{BackendError, BackendResult};
pub use self::message::{
    AppendMessage, BodyPart, BodyStructure, Envelope, FetchedBody, FetchedMessage, FlagChange,
    FlagUpdate, MailboxEvent, MailboxFilter, MailboxStatus, MailboxSummary, PartNode, SelectMode,
    UidMapping,
};
pub use self::mock::{
    Fault, FetchEvent, MockBackend, MockBackendBuilder, MockMailbox, MockMessage,
};
pub use self::sink::{BodySink, CountingSink, VecSink};
pub use self::uid_set::UidSet;

/// Re-exported so a caller can describe a MIME part without also depending on
/// `postio-model` directly.
pub use postio_model::Disposition;

/// A mail server, as the rest of Postio sees one.
///
/// Implementations are shared across tasks (`Arc<dyn MailBackend>`) and every
/// method takes `&self`: connection state, pooling and serialization are the
/// backend's problem, not the caller's. A caller that wants two fetches at
/// once simply issues two.
///
/// # Errors
///
/// Callers branch on [`BackendError::is_transient`],
/// [`is_authentication_failure`](BackendError::is_authentication_failure) and
/// [`requires_full_resync`](BackendError::requires_full_resync) — never on the
/// variant — so that adding a variant cannot silently change how existing code
/// retries.
///
/// # Adding a protocol
///
/// This trait is the porting surface. A JMAP or Exchange backend implements
/// these methods and nothing above `postio-sync` changes. The concepts that
/// look IMAP-shaped are the ones every mail protocol has under some name:
/// a folder, a server-assigned message id ([`RemoteId`] — opaque here; the
/// IMAP adapter packs its generation-and-uid pair into one and keeps the
/// renumbering dance behind this seam, per ADR 0018 Q2), and a change
/// counter ([`ModSeq`]). A protocol without one reports `None` and the sync
/// engine falls back to comparing listings.
#[async_trait]
pub trait MailBackend: Send + Sync + fmt::Debug {
    /// Short name of this backend, for diagnostics.
    fn describe(&self) -> &'static str;

    /// Opens a session and authenticates.
    ///
    /// Returns the capability set observed **after** authentication. An empty
    /// set is [`BackendError::EmptyCapabilities`], never a silent downgrade:
    /// see ADR 0001, Q3.
    async fn connect(&self) -> BackendResult<Capabilities>;

    /// Closes the session. Closing an already-closed backend succeeds.
    async fn disconnect(&self) -> BackendResult<()>;

    /// The capability set from the current session.
    async fn capabilities(&self) -> BackendResult<Capabilities>;

    /// Lists mailboxes, resolving each one's role at the edge.
    async fn list_mailboxes(&self, filter: &MailboxFilter) -> BackendResult<Vec<MailboxSummary>>;

    /// Opens a mailbox and reports its state.
    async fn select(&self, path: &str, mode: SelectMode) -> BackendResult<MailboxStatus>;

    /// Reports a mailbox's state without opening it.
    ///
    /// This is the cheap per-mailbox change check for every folder that is not
    /// the one being watched.
    async fn status(&self, path: &str) -> BackendResult<MailboxStatus>;

    /// Fetches metadata — envelope, structure, flags, size — for `uids`.
    ///
    /// No body bytes are downloaded. With `changed_since`, only messages whose
    /// modification sequence is *greater than* it are returned, which is what
    /// makes an incremental flag pull cheap.
    ///
    /// The caller is expected to have split large sets with
    /// [`UidSet::chunks`]: this returns everything it fetched at once, and a
    /// ten-thousand-message set fetched in one call is a ten-thousand-message
    /// allocation.
    async fn fetch_headers(
        &self,
        mailbox: &str,
        uids: &UidSet,
        changed_since: Option<ModSeq>,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<FetchedMessage>>;

    /// Streams one part of one message into `sink`.
    ///
    /// The sink sees the bytes as they arrive and is the only place they exist
    /// in full. On failure or cancellation, [`BodySink::finish`] is *not*
    /// called and whatever reached the sink must be discarded.
    async fn fetch_part(
        &self,
        mailbox: &str,
        id: &RemoteId,
        part: &BodyPart,
        sink: &mut dyn BodySink,
        cancel: &CancelToken,
    ) -> BackendResult<FetchedBody>;

    /// Streams a whole message into `sink`.
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

    /// Changes flags on `ids` and reports what they are now.
    async fn store_flags(
        &self,
        mailbox: &str,
        ids: &[RemoteId],
        change: &FlagChange,
    ) -> BackendResult<Vec<FlagUpdate>>;

    /// Moves messages between mailboxes.
    ///
    /// The returned mapping is empty unless the server speaks
    /// [`Capability::UidPlus`]; without it the destination UIDs have to be
    /// found by searching, which is the caller's decision to make.
    async fn move_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>>;

    /// Copies messages between mailboxes, leaving the source intact.
    async fn copy_messages(
        &self,
        from: &str,
        ids: &[RemoteId],
        to: &str,
    ) -> BackendResult<Vec<UidMapping>>;

    /// Expunges messages marked `\Deleted`, and reports which went.
    ///
    /// With `ids`, only those are considered — the `UID EXPUNGE` of RFC 4315,
    /// which is what stops Postio from expunging a message another client
    /// marked in the same mailbox. A server without that extension cannot
    /// honour a targeted expunge at all, and declines rather than widening it
    /// to everything marked `\Deleted`.
    ///
    /// **An IMAP server reports the removals as sequence numbers**, not UIDs,
    /// so a real backend returns an empty list here however much it removed:
    /// turning a sequence number into an identity needs a map this layer does
    /// not keep, and a plausible wrong id is worse than none. Treat a
    /// successful expunge as "resync this mailbox", and read the returned ids
    /// only from implementations that genuinely know them, such as
    /// [`MockBackend`].
    async fn expunge(
        &self,
        mailbox: &str,
        ids: Option<&[RemoteId]>,
    ) -> BackendResult<Vec<RemoteId>>;

    /// Uploads a message into a mailbox.
    ///
    /// Returns where it landed when the server speaks
    /// [`Capability::UidPlus`], and `None` otherwise.
    async fn append(
        &self,
        mailbox: &str,
        message: &AppendMessage,
    ) -> BackendResult<Option<UidMapping>>;

    /// Finds the one message in `mailbox` whose RFC 5322 `Message-ID`
    /// header is `message_id`, if the backend can search at all.
    ///
    /// The cross-account move saga's confirmation fallback (#188, ADR 0005
    /// Q9): on a server without UIDPLUS an append proves nothing, and this
    /// targeted `UID SEARCH HEADER Message-ID` is what stands in. The
    /// default answers `None` — "this backend cannot search" — which the
    /// saga reads as *unconfirmed*: it stops and asks, it never guesses.
    /// More than one match also answers the newest, since any copy proves
    /// arrival.
    async fn find_by_message_id(
        &self,
        mailbox: &str,
        message_id: &str,
    ) -> BackendResult<Option<RemoteId>> {
        let _ = (mailbox, message_id);
        Ok(None)
    }

    /// Every UID that currently exists in `mailbox`, if the backend can say.
    ///
    /// What a first sync needs in order to ask only for messages that are
    /// *there*. Without it the caller has nothing to go on but the UID
    /// ceiling, so it walks `1..=UIDNEXT-1` in chunks and pays a round trip
    /// for every chunk whose UIDs were all expunged — a cost proportional to
    /// `UIDNEXT` rather than to how much mail the folder holds. Measured
    /// against a real account, that was 46% of a first sync's wall clock
    /// spent fetching nothing (#78, #727).
    ///
    /// The default answers `None` — "this backend cannot enumerate" — and the
    /// caller falls back to that walk, which is what every backend did before
    /// this existed. A protocol whose listing is already cheap has no reason
    /// to implement it.
    ///
    /// Ordering is not promised; the caller sorts. Neither is freshness: this
    /// is a snapshot, and a message may be expunged between this call and the
    /// fetch that asks for it. That is ordinary and already handled — a
    /// `FETCH` for a UID that has gone returns nothing rather than failing.
    async fn existing_uids(
        &self,
        mailbox: &str,
        cancel: &CancelToken,
    ) -> BackendResult<Option<Vec<Uid>>> {
        let _ = (mailbox, cancel);
        Ok(None)
    }

    /// Waits for the server to say something about `mailbox`.
    ///
    /// Returns as soon as anything arrives, when `timeout` elapses, or when
    /// `cancel` fires — the last two with an empty vector, because "nothing
    /// happened" is not a failure.
    ///
    /// The events are deliberately raw. They say *that* the mailbox changed,
    /// not what it now contains; the correct response is a resync pull, not
    /// applying them as a diff.
    ///
    /// # What a watcher is not told
    ///
    /// A server reports what *other* connections did. Changes this backend
    /// itself made came back in their own command responses and are not
    /// repeated here, so a caller must never wait on a watch to confirm its
    /// own write — it already has the answer.
    ///
    /// [`MockBackend`] is more talkative than that: having one connection and
    /// no way to tell whose change it was, it queues every change made
    /// through it, including the caller's own. That is the safe direction to
    /// be wrong in — the engine is exercised on a resync path that a real
    /// second client can also trigger — but it means a watcher against a real
    /// server fires *less* often than the mock suggests. Anything that
    /// depends on the difference is depending on the mock, not on the
    /// protocol; `crates/postio-account/tests/backend_parity.rs` marks the seam
    /// where the two part company.
    async fn idle(
        &self,
        mailbox: &str,
        timeout: Duration,
        cancel: &CancelToken,
    ) -> BackendResult<Vec<MailboxEvent>>;
}
