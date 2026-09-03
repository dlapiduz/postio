//! The cross-account move saga's server halves (#188, ADR 0005 Q9).
//!
//! Two per-account queues carry one move: the target account's drainer runs
//! [`copy`] (phases 1–2: append and confirm), the source account's runs
//! [`remove`] (phase 3: `\Deleted` + expunge). The saga row — see
//! `postio_storage::repository::CrossAccountMoveRepository` — is what the
//! queues share instead of a transaction, and its phase walk is what makes
//! the only reachable failure a duplicate, never a loss:
//!
//! - [`copy`] is **idempotent by Message-ID**: it confirms before it
//!   appends, with the same lookup confirmation itself uses, so a replay
//!   after a crash finds the earlier copy instead of making a second.
//! - An append it cannot confirm — no UIDPLUS, and the search found
//!   nothing — parks the saga in `unconfirmed` and **fails loudly**. It
//!   never guesses and never deletes.
//! - [`remove`] refuses to run until the saga says `confirmed`. A remove
//!   drained before its copy simply retries later; there is no interleaving
//!   of the two drainers that deletes an unproven message.
//! - A target that no longer exists — folder deleted, account removed
//!   mid-saga — aborts the saga with the source intact (Q13).

use postio_account::backend::{AppendMessage, MailBackend};
use postio_model::ids::CrossAccountMoveId;
use postio_storage::BlobStore;
use postio_storage::repository::{
    CrossAccountMove, CrossAccountMoveRepository, MailboxRepository, MessageRepository, MovePhase,
};
use rusqlite::Connection;

use crate::drain::Outcome;

/// Phases 1–2, on the **target** account's drainer.
pub(crate) async fn copy(
    backend: &dyn MailBackend,
    blobs: Option<&BlobStore>,
    connection: &Connection,
    saga_id: CrossAccountMoveId,
) -> Outcome {
    let sagas = CrossAccountMoveRepository::new(connection);
    let saga = match sagas.get(saga_id) {
        Ok(Some(saga)) => saga,
        Ok(None) => {
            return Outcome::Obsolete {
                reason: "the move this copy belonged to no longer exists".to_owned(),
            };
        }
        Err(error) => return failed(format!("could not read the move: {error}")),
    };

    match saga.phase {
        // A replayed operation after the work is done: nothing to do.
        MovePhase::Confirmed | MovePhase::Done => return Outcome::Applied,
        MovePhase::Aborted => {
            return Outcome::Obsolete {
                reason: "the move was abandoned; the source copy is intact".to_owned(),
            };
        }
        MovePhase::Copying | MovePhase::Unconfirmed => {}
    }

    // The target must still exist to receive anything. If it is gone — the
    // folder deleted, the account removed — the saga aborts and the source
    // copy stays exactly where it is (Q13).
    let Some(target_path) = target_path(connection, &saga) else {
        return abort(
            &sagas,
            saga_id,
            "the destination no longer exists; the move was abandoned and \
             the source copy is intact",
        );
    };

    // Confirm before append — both the idempotency rule and phase 2 itself.
    // A crash after a successful APPEND replays this operation, and the
    // search finds that copy rather than making another.
    if let Some(rfc) = saga.rfc_message_id.as_deref() {
        match backend.find_by_message_id(&target_path, rfc).await {
            Ok(Some(uid)) => {
                return match sagas.confirm(saga_id, Some(&uid)) {
                    Ok(()) => Outcome::Applied,
                    Err(error) => failed(format!("could not record the confirmation: {error}")),
                };
            }
            Ok(None) => {}
            Err(error) => {
                return Outcome::Retry {
                    reason: format!("could not search the destination: {error}"),
                    after: None,
                };
            }
        }
    }

    if saga.phase == MovePhase::Unconfirmed {
        // The append already ran and nothing can prove where it landed. The
        // ADR's answer is exact: stop and ask. Loud, settled, and the
        // source copy untouched.
        return failed(
            "the message was uploaded but its arrival could not be confirmed; \
             nothing was deleted — check the destination folder"
                .to_owned(),
        );
    }

    // Phase 1: the append itself.
    let Some(bytes) = raw_bytes(blobs, &saga) else {
        return Outcome::Retry {
            reason: "the message's raw copy is not local yet".to_owned(),
            after: None,
        };
    };
    let mapping = match backend
        .append(
            &target_path,
            &AppendMessage {
                raw: bytes,
                flags: Default::default(),
                internal_date: None,
            },
        )
        .await
    {
        Ok(mapping) => mapping,
        Err(error) => {
            return Outcome::Retry {
                reason: format!("the upload failed: {error}"),
                after: None,
            };
        }
    };

    // Phase 2: the proof. APPENDUID is one; the Message-ID search is the
    // fallback; neither is a guess.
    if let Some(mapping) = mapping {
        return match sagas.confirm(saga_id, Some(&mapping.destination_remote_id())) {
            Ok(()) => Outcome::Applied,
            Err(error) => failed(format!("could not record the confirmation: {error}")),
        };
    }
    if let Some(rfc) = saga.rfc_message_id.as_deref()
        && let Ok(Some(uid)) = backend.find_by_message_id(&target_path, rfc).await
    {
        return match sagas.confirm(saga_id, Some(&uid)) {
            Ok(()) => Outcome::Applied,
            Err(error) => failed(format!("could not record the confirmation: {error}")),
        };
    }
    if let Err(error) = sagas.transition(saga_id, MovePhase::Unconfirmed) {
        return failed(format!("could not record the unconfirmed append: {error}"));
    }
    failed(
        "the message was uploaded but its arrival could not be confirmed; \
         nothing was deleted — check the destination folder"
            .to_owned(),
    )
}

/// Phase 3, on the **source** account's drainer.
pub(crate) async fn remove(
    backend: &dyn MailBackend,
    connection: &Connection,
    saga_id: CrossAccountMoveId,
) -> Outcome {
    let sagas = CrossAccountMoveRepository::new(connection);
    let saga = match sagas.get(saga_id) {
        Ok(Some(saga)) => saga,
        Ok(None) => {
            return Outcome::Obsolete {
                reason: "the move this removal belonged to no longer exists".to_owned(),
            };
        }
        Err(error) => return failed(format!("could not read the move: {error}")),
    };

    match saga.phase {
        MovePhase::Done => return Outcome::Applied,
        MovePhase::Aborted => {
            return Outcome::Obsolete {
                reason: "the move was abandoned; nothing to remove".to_owned(),
            };
        }
        // **The rule that cannot be traded away.** Until the copy is
        // proven, this operation waits — however many times the drainer
        // comes back, however the two queues interleave.
        MovePhase::Copying | MovePhase::Unconfirmed => {
            return Outcome::Retry {
                reason: "the destination has not confirmed its copy yet".to_owned(),
                after: None,
            };
        }
        MovePhase::Confirmed => {}
    }

    let path = saga
        .source_mailbox
        .and_then(|mailbox| MailboxRepository::new(connection).get(mailbox).ok())
        .flatten()
        .map(|mailbox| mailbox.path);
    let remote_id = saga
        .source_message
        .and_then(|message| MessageRepository::new(connection).get(message).ok())
        .flatten()
        .and_then(|message| message.server.remote_id);
    let (Some(path), Some(remote_id)) = (path, remote_id) else {
        // The source copy is already gone — another client removed it, or a
        // resync did. The move is complete either way.
        return match sagas.transition(saga_id, MovePhase::Done) {
            Ok(()) => Outcome::Applied,
            Err(error) => failed(format!("could not settle the move: {error}")),
        };
    };

    let ids = [remote_id];
    if let Err(error) = backend
        .store_flags(
            &path,
            &ids,
            &postio_account::backend::FlagChange::Add(deleted_flag()),
        )
        .await
    {
        return Outcome::Retry {
            reason: format!("could not mark the source copy deleted: {error}"),
            after: None,
        };
    }
    if let Err(error) = backend.expunge(&path, Some(&ids)).await {
        return Outcome::Retry {
            reason: format!("could not expunge the source copy: {error}"),
            after: None,
        };
    }
    match sagas.transition(saga_id, MovePhase::Done) {
        Ok(()) => Outcome::Applied,
        Err(error) => failed(format!("could not settle the move: {error}")),
    }
}

fn target_path(connection: &Connection, saga: &CrossAccountMove) -> Option<String> {
    let mailbox = saga.target_mailbox?;
    saga.target_account?;
    MailboxRepository::new(connection)
        .get(mailbox)
        .ok()
        .flatten()
        .map(|mailbox| mailbox.path)
}

fn raw_bytes(blobs: Option<&BlobStore>, saga: &CrossAccountMove) -> Option<Vec<u8>> {
    let blob = saga.raw_blob_id.as_deref()?;
    blobs?.get(&postio_model::BlobId::new(blob.to_owned())).ok()
}

fn abort(
    sagas: &CrossAccountMoveRepository<'_>,
    saga: CrossAccountMoveId,
    reason: &str,
) -> Outcome {
    if let Err(error) = sagas.transition(saga, MovePhase::Aborted) {
        return failed(format!("could not abandon the move: {error}"));
    }
    failed(reason.to_owned())
}

fn failed(reason: String) -> Outcome {
    Outcome::Failed { reason }
}

/// The one flag phase 3 sets.
fn deleted_flag() -> postio_model::FlagSet {
    let mut flags = postio_model::FlagSet::new();
    flags.insert(postio_model::Flag::Deleted);
    flags
}
