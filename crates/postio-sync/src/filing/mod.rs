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
//! none of them reaches a filing pass (FR-118).
//!
//! One re-enumeration is the exception, because for some accounts it is
//! the only pass there is. A backend with no `MODSEQ` -- the JMAP and Gmail
//! adapters, an IMAP server without CONDSTORE -- plans
//! [`postio_model::FullResyncReason::NoModSeq`] on every pass after the
//! first, and re-reads the folder in place (#564). What such a pass
//! *inserts* is what arrived since the last one, so its write units hand
//! exactly those rows to the filing pass; the rows it only refreshes are
//! not arrivals, and are not handed over. Without this, Focus would file
//! nothing for those accounts.
//!
//! # Only while Focus runs
//!
//! Nothing here installs one. A host hands a filing pass to its engines only
//! when Focus switches Focus mode on (`postio_host::Host::enable_focus`), so
//! while the classic app or the terminal holds the store, no mail is filed
//! by Focus's rules (FR-134).

mod facts;
mod focus;

use postio_model::{MailboxId, MessageId};
use postio_storage::Connection;

use crate::drain::SyncError;

pub use focus::{BuiltIn, Classifier, FocusFiling};

/// One message an incremental pass filed, as a filing pass is told it: the
/// model's one type, which the classifier reads as it is handed over
/// (`postio_model::filing`).
pub use postio_model::filing::FiledMessage;

/// What a filing pass did to the messages it was handed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilingEffects {
    /// The arrivals it filed away out of the inbox, into Filtered.
    pub filtered: Vec<MessageId>,
}

/// Files what an incremental pass brought in.
///
/// Called inside the pass's own write transaction, once per write unit
/// that filed something new, with exactly the new messages. `transaction`
/// is that transaction: whatever the pass writes commits with the mail it is
/// about.
///
/// An error does not take the mail with it. The pass runs in a scope of its
/// own inside the transaction, and a failure rolls back only what the pass
/// wrote: the arrivals stay where they were filed, in the inbox, and the
/// insert commits ([`file_arrivals`]).
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

/// Hands `filed` to `pass`, in a scope of its own inside `transaction`, and
/// answers what it did.
///
/// **Errors never lose mail** (ADR 0008 Q6, `contracts/engine.md`). When
/// the pass fails, the scope rolls back what it wrote and nothing else: the
/// messages stay where sync filed them, the transaction goes on to commit
/// the insert, and the failure is logged by ids and outcome, never by
/// content. When in doubt, mail goes to the inbox (FR-112) -- and a pass
/// that failed is doubt. Failing the unit instead would lose the arrival
/// for this pass, and, for a pass that fails every time, for good.
pub(crate) async fn file_arrivals(
    transaction: &Connection,
    pass: &dyn FilingPass,
    mailbox: MailboxId,
    filed: &[FiledMessage<'_>],
) -> FilingEffects {
    let filing = postio_storage::transaction(transaction, move |scope| async move {
        pass.file(&scope, filed).await
    })
    .await;
    filing.unwrap_or_else(|error: SyncError| {
        tracing::warn!(
            mailbox = mailbox.get(),
            arrivals = filed.len(),
            %error,
            "the filing pass failed; what arrived stays where it was filed"
        );
        FilingEffects::default()
    })
}
