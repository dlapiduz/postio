//! Files what an incremental pass brings in, for a host in Focus mode, inside the transaction that
//! filed it so mail is never shown in one place then moved.
//! Only arrivals (FR-118): first syncs and rebuilds never reach it, except a modseq-less backend's
//! re-enumeration, whose inserts are the arrivals (#564).
//! Installed only while Focus mode is on (FR-134).

mod facts;
mod focus;

use postio_model::{MailboxId, MessageId};
use postio_storage::Connection;

use crate::drain::SyncError;

pub use focus::{BuiltIn, Classifier, FocusFiling};

pub use postio_model::filing::FiledMessage;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilingEffects {
    pub filtered: Vec<MessageId>,
    pub held: Vec<MessageId>,
}

/// Called inside the pass's write transaction, once per write unit that filed something new.
/// An error rolls back only what the pass wrote: the arrivals stay in the inbox and the insert
/// commits.
#[async_trait::async_trait]
pub trait FilingPass: Send + Sync + std::fmt::Debug {
    async fn file(
        &self,
        transaction: &Connection,
        filed: &[FiledMessage<'_>],
    ) -> Result<FilingEffects, SyncError>;
}

/// Runs `pass` in a scope of its own inside `transaction`. Errors never lose mail (ADR 0008 Q6,
/// FR-112): a failed pass rolls back its own writes,
/// the messages stay where sync filed them, and the failure is logged by ids and outcome only.
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
