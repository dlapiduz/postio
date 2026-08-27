//! Keeping the account's Drafts mailbox in step with the composer.
//!
//! # Why a draft is not just an append
//!
//! A draft is edited. The server has no way to say "this message, but with
//! another paragraph" — `APPEND` only adds — so keeping one copy of a draft on
//! the server rather than one copy per autosave means *append the new text,
//! then remove the old copy*, in that order and never the other way round: a
//! removal that ran first and an append that then failed would lose the
//! user's words, which is the one outcome a composer may never produce.
//!
//! # Why the bytes are built here
//!
//! `Operation::SaveDraft` carries no message. Building a draft's RFC 5322
//! form means reading every attachment out of the blob store and assembling
//! MIME, and a draft is saved as the user types — doing that per keystroke
//! would be work thrown away, and storing the result would put one immutable
//! blob per keystroke in a content-addressed store. So the queue row says only
//! *which* draft is stale, and the bytes are built once, here, from whatever
//! the draft says by the time this drains. It is also what makes two queued
//! saves fold into one: they do not describe different work.
//!
//! # Why a discard carries its identity
//!
//! The opposite problem. Discarding a draft deletes the local row at once —
//! local-first, like every other mutation — so by the time
//! `Operation::DiscardDraft` drains there is nothing left to read the server
//! copy's name from. It therefore carries its own [`RemoteId`]; for IMAP
//! that id packs the generation it was observed under, so a renumbered
//! mailbox makes it *stale* — refused by the adapter — rather than silently
//! naming a different message, which would be deleting the user's mail.
//!
//! # What the server is not asked for
//!
//! Nothing here searches. If the server cannot say where an appended message
//! landed (no [`Capability::UidPlus`]), the local row records that it does not
//! know, the mailbox is flagged for a resync, and the ordinary sync pass
//! reconciles — rather than this guessing which message in Drafts is the one
//! it just wrote.

use std::collections::BTreeSet;

use postio_imap::backend::{AppendMessage, Capabilities, Capability, FlagChange, MailBackend};
use postio_model::ids::DraftId;
use postio_model::{Flag, FlagSet, MailboxId, OutgoingAttachment, RemoteId, outgoing};
use postio_storage::BlobStore;
use postio_storage::repository::{AccountRepository, DraftRepository, MailboxRepository};
use rusqlite::Connection;

use crate::drain::{Outcome, Result};

/// Everything one draft step needs, resolved from local storage before
/// anything is opened — the same split [`crate::send`] makes, and for the same
/// reason: a missing draft or an account with no identity is a local fact and
/// should cost no connection to discover.
#[derive(Debug, Clone)]
pub(crate) enum DraftJob {
    /// Put this text in Drafts, replacing the copy already there.
    Save {
        draft: DraftId,
        mailbox: MailboxId,
        path: String,
        raw: Vec<u8>,
        /// The copy this one replaces, when the draft has one.
        previous: Option<RemoteId>,
    },
    /// Take the draft's copy out of Drafts.
    Discard {
        mailbox: MailboxId,
        path: String,
        copy: RemoteId,
    },
}

/// What resolving a draft operation against local storage found.
pub(crate) enum ResolvedDraft {
    /// Ready to hand to [`run`].
    Ready(Box<DraftJob>),
    /// Nothing to do any more — the draft, or the folder, is gone.
    Obsolete(String),
    /// Not yet, but plausibly soon: an attachment whose bytes have not
    /// finished being written locally.
    Later(String),
    /// Cannot be done and retrying will not change that.
    Impossible(String),
}

/// Resolves an `Operation::SaveDraft`: loads the draft, its identity and its
/// attachments' bytes, and builds the message to upload. Nothing here is
/// async — every input is a database row or a blob store read.
pub(crate) fn resolve_save(
    connection: &Connection,
    blobs: Option<&BlobStore>,
    draft_id: DraftId,
    mailbox: MailboxId,
) -> Result<ResolvedDraft> {
    let Some(draft) = DraftRepository::new(connection).get(draft_id)? else {
        return Ok(ResolvedDraft::Obsolete(
            "the draft is no longer in the local store".to_owned(),
        ));
    };
    let Some(folder) = MailboxRepository::new(connection).get(mailbox)? else {
        return Ok(ResolvedDraft::Obsolete(
            "the Drafts mailbox is no longer in the local store".to_owned(),
        ));
    };

    let Some(account) = AccountRepository::new(connection).get(draft.account_id)? else {
        return Ok(ResolvedDraft::Impossible(
            "the account is no longer in the local store".to_owned(),
        ));
    };
    let identity = draft
        .identity_id
        .and_then(|id| account.identities.iter().find(|identity| identity.id == id))
        .or_else(|| account.default_identity());
    let Some(identity) = identity else {
        return Ok(ResolvedDraft::Impossible(
            "the account has no identity to compose as".to_owned(),
        ));
    };

    let mut buffers = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let Some(blob_id) = &attachment.blob_id else {
            // The composer hands a file to the blob store off the main thread,
            // so a draft saved the instant something was dropped on it can
            // reach here before the bytes have landed. That is a wait, not a
            // failure: uploading the draft without the attachment would put a
            // copy on the server that is quietly missing part of itself.
            return Ok(ResolvedDraft::Later(
                "an attachment has not finished being written locally".to_owned(),
            ));
        };
        let Some(blobs) = blobs else {
            return Ok(ResolvedDraft::Impossible(
                "this drainer has no blob store to read attachments from".to_owned(),
            ));
        };
        buffers.push((attachment.clone(), blobs.get(blob_id)?));
    }
    let attachments: Vec<OutgoingAttachment<'_>> = buffers
        .iter()
        .map(|(attachment, content)| OutgoingAttachment {
            attachment,
            content,
        })
        .collect();

    // `build_draft` rather than `build`: this copy goes to the user's own
    // Drafts folder and reaches nobody, so it keeps the `Bcc` the outgoing
    // bytes must never carry. A draft picked up on another client with its
    // bcc'd recipients silently gone would send to fewer people than the user
    // asked for.
    //
    // No `in_reply_to` parent is resolved: the copy exists so the *user* can
    // find their unfinished words elsewhere, and threading headers are
    // generated fresh by the send that finally goes out.
    let built = outgoing::build_draft(&draft, identity, &attachments, None);

    Ok(ResolvedDraft::Ready(Box::new(DraftJob::Save {
        draft: draft_id,
        mailbox,
        path: folder.path,
        raw: built.raw,
        previous: server_copy(&draft),
    })))
}

/// Resolves an `Operation::DiscardDraft`. There is no draft row left to
/// read — that is the point of the operation carrying its own identity — so
/// this only has to find the folder. Whether the identity is still of the
/// mailbox's live generation is the adapter's own check now (#543): a stale
/// id comes back as the same resync answer a renumber discovered at SELECT
/// gives, and [`run`] reads it as obsolete rather than retrying.
pub(crate) fn resolve_discard(
    connection: &Connection,
    mailbox: MailboxId,
    copy: RemoteId,
) -> Result<ResolvedDraft> {
    let Some(folder) = MailboxRepository::new(connection).get(mailbox)? else {
        return Ok(ResolvedDraft::Obsolete(
            "the Drafts mailbox is no longer in the local store".to_owned(),
        ));
    };

    Ok(ResolvedDraft::Ready(Box::new(DraftJob::Discard {
        mailbox,
        path: folder.path,
        copy,
    })))
}

/// Performs a resolved draft step.
pub(crate) async fn run(
    connection: &Connection,
    backend: &dyn MailBackend,
    capabilities: &Capabilities,
    resync: &mut BTreeSet<i64>,
    job: &DraftJob,
) -> Outcome {
    match job {
        DraftJob::Save {
            draft,
            mailbox,
            path,
            raw,
            previous,
        } => {
            save(
                connection,
                backend,
                capabilities,
                resync,
                *draft,
                *mailbox,
                path,
                raw,
                previous.clone(),
            )
            .await
        }
        DraftJob::Discard {
            mailbox,
            path,
            copy,
        } => match remove(backend, capabilities, path, copy).await {
            Ok(Removal::Gone) => Outcome::Obsolete {
                reason: "the draft was no longer in the Drafts mailbox on the server".to_owned(),
            },
            Ok(Removal::Marked) => {
                // Marked `\Deleted` but not expunged: without UID EXPUNGE a
                // targeted expunge cannot be honoured, and widening it to
                // everything marked in the folder could remove a message
                // another client marked. The folder is flagged instead.
                resync.insert(mailbox.get());
                Outcome::Applied
            }
            Ok(Removal::Removed) => Outcome::Applied,
            Err(error) if error.requires_full_resync() => {
                // The mailbox was renumbered under the queued discard: the
                // id names nothing now, and retrying can never change that.
                // The folder reconciles by resync instead.
                resync.insert(mailbox.get());
                Outcome::Obsolete {
                    reason: "the Drafts mailbox has been renumbered since the draft was                              uploaded"
                        .to_owned(),
                }
            }
            Err(error) => Outcome::from_error(error),
        },
    }
}

/// The flags a draft arrives with.
///
/// `\Draft` is what tells every other client this is unfinished. `\Seen`
/// because the user wrote it: an unread badge on your own half-typed message
/// is noise, and it is the one thing every mail client agrees on here.
fn draft_flags() -> FlagSet {
    let mut flags = FlagSet::new();
    flags.insert(Flag::Draft);
    flags.insert(Flag::Seen);
    flags
}

#[allow(clippy::too_many_arguments)]
async fn save(
    connection: &Connection,
    backend: &dyn MailBackend,
    capabilities: &Capabilities,
    resync: &mut BTreeSet<i64>,
    draft: DraftId,
    mailbox: MailboxId,
    path: &str,
    raw: &[u8],
    previous: Option<RemoteId>,
) -> Outcome {
    let append = AppendMessage::new(raw.to_vec()).with_flags(draft_flags());
    let landed = match backend.append(path, &append).await {
        Ok(landed) => landed,
        Err(error) => {
            if error.requires_full_resync() {
                resync.insert(mailbox.get());
            }
            return Outcome::from_error(error);
        }
    };

    // Recorded before the old copy is touched. If everything after this fails,
    // the worst outcome is two copies in Drafts — recoverable, and visible to
    // the user as their own text twice. Losing the id of the copy that *is*
    // there is not recoverable without searching the folder.
    let location = match &landed {
        Some(mapping) => Some(postio_storage::repository::ServerCopyLocation {
            remote_id: mapping.destination_remote_id(),
            uid: mapping.destination,
            uid_validity: mapping.uid_validity,
        }),
        // The server does not report where an append landed. Say so rather
        // than guessing: the next save cannot remove this copy, so the folder
        // is flagged and an ordinary sync pass reconciles it.
        None => {
            resync.insert(mailbox.get());
            None
        }
    };
    if let Err(error) = DraftRepository::new(connection).set_server_copy(draft, location.as_ref()) {
        // The draft was discarded while its own upload was in flight. The copy
        // just made is orphaned, so the folder is flagged rather than left
        // silently wrong.
        resync.insert(mailbox.get());
        return Outcome::Obsolete {
            reason: format!("the draft went away while it was being uploaded: {error}"),
        };
    }

    let Some(previous) = previous else {
        return Outcome::Applied;
    };
    // A renumbered mailbox makes the old identity stale. The adapter is the
    // one that knows (#543): the removal below comes back as the resync
    // answer, and the folder reconciles rather than this guessing.
    match remove(backend, capabilities, path, &previous).await {
        // Already gone, or removed: either way there is one copy in Drafts.
        Ok(Removal::Removed | Removal::Gone) => Outcome::Applied,
        Ok(Removal::Marked) => {
            resync.insert(mailbox.get());
            Outcome::Applied
        }
        // The new copy is up. Failing the step now would retry the whole
        // save and upload the text a second time, so the leftover copy is
        // reported as a folder to reconcile instead.
        Err(_) => {
            resync.insert(mailbox.get());
            Outcome::Applied
        }
    }
}

/// What became of a copy this asked the server to remove.
pub(crate) enum Removal {
    /// Marked `\Deleted` and expunged.
    Removed,
    /// Marked `\Deleted`, but the server cannot expunge just this message.
    Marked,
    /// It was not there to remove.
    Gone,
}

/// Takes one message out of a mailbox: `\Deleted`, then `UID EXPUNGE`.
pub(crate) async fn remove(
    backend: &dyn MailBackend,
    capabilities: &Capabilities,
    path: &str,
    copy: &RemoteId,
) -> std::result::Result<Removal, postio_imap::backend::BackendError> {
    let ids = std::slice::from_ref(copy);
    let mut flags = FlagSet::new();
    flags.insert(Flag::Deleted);

    let updated = backend
        .store_flags(path, ids, &FlagChange::Add(flags))
        .await?;
    if updated.is_empty() {
        // A store over a non-empty id set that changed nothing means the
        // message is not in the mailbox any more — see `crate::drain`.
        return Ok(Removal::Gone);
    }

    if !capabilities.contains(Capability::UidPlus) {
        return Ok(Removal::Marked);
    }
    backend.expunge(path, Some(ids)).await?;
    Ok(Removal::Removed)
}

/// The server copy a draft row names, when it names one.
pub(crate) fn server_copy(draft: &postio_model::Draft) -> Option<RemoteId> {
    draft.server.remote_id.clone()
}
