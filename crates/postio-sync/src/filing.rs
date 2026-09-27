//! Filing what an incremental pass brings in, for a host in Focus mode
//! (spec 007, `contracts/engine.md`, "The filing pass").
//!
//! Focus sorts new mail as it lands -- filtering, holding for a digest,
//! cancelling a reminder -- and it does so in the transaction that filed
//! it, so a message is never on screen in one place and then moved to
//! another. [`FilingPass`] is that seam: [`crate::resync_mailbox_filing`]
//! hands one every message an incremental pass filed, after the upsert and
//! the threading and before any event is emitted.
//!
//! # Only what arrived
//!
//! An incremental pass is the one whose new messages are new mail, the
//! ones [`crate::Outcome::Incremental`] names as `arrived`. A first sync,
//! a rebuild after a renumbering and the re-enumeration of a mailbox that
//! came up short all file the backlog -- years of inbox, not arrivals -- and
//! none of them reaches a filing pass (FR-118). That is why the seam is the
//! incremental pass's write unit and not `commit_batch`: every pass that
//! goes through `commit_batch` is one of those.
//!
//! # Only while Focus runs
//!
//! Nothing here installs one. A host hands a filing pass to its engines only
//! when Focus switches Focus mode on (`postio_host::Host::enable_focus`), so
//! while the classic app or the terminal holds the store, no mail is filed
//! by Focus's rules (FR-134).

use postio_model::{MailboxRole, Message, ThreadId};
use postio_storage::Connection;

use crate::drain::SyncError;

/// One message an incremental pass filed, as a filing pass is told it.
///
/// Only what is known at filing: the message as it was stored -- envelope,
/// `References`, `List-Id`, flags, structure, and its id -- the thread it
/// joined, and the role of the folder it was filed into. No body: filing
/// happens before one is fetched.
#[derive(Debug, Clone, Copy)]
pub struct FiledMessage<'a> {
    /// The message, with the id the store gave it.
    pub message: &'a Message,
    /// The conversation threading put it in.
    pub thread: ThreadId,
    /// The role of the folder it was filed into.
    pub role: MailboxRole,
}

/// What a filing pass did to the messages it was handed.
///
/// Nothing yet: the pass Focus mode runs today files nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FilingEffects {}

/// Files what an incremental pass brought in.
///
/// Called inside the pass's own write transaction, once per write unit
/// that filed something new, with exactly the new messages. `transaction`
/// is that transaction: whatever the pass writes commits or rolls back with
/// the mail it is about.
#[async_trait::async_trait]
pub trait FilingPass: Send + Sync + std::fmt::Debug {
    /// Files `filed`, and says what it did.
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError>;
}

/// The filing pass that files nothing: Focus mode's, until Focus's rules
/// are written.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFiling;

#[async_trait::async_trait]
impl FilingPass for NoFiling {
    async fn file(
        &self,
        _transaction: &Connection,
        _filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError> {
        Ok(FilingEffects::default())
    }
}
