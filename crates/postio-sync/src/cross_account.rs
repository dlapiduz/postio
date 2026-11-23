//! Server halves of the cross-account move saga (#188, ADR 0005 Q9): `copy` on the target's
//! drainer, `remove` on the source's.
//! `copy` is idempotent by Message-ID and fails loudly when it cannot confirm; `remove` waits for
//! `confirmed`. The only failure is a duplicate, never a loss.

use postio_account::backend::{AppendMessage, MailBackend};
use postio_model::ids::CrossAccountMoveId;
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{
    CrossAccountMove, CrossAccountMoveRepository, MailboxRepository, MessageRepository, MovePhase,
};

use crate::drain::Outcome;

pub(crate) async fn copy(
    backend: &dyn MailBackend,
    blobs: Option<&BlobStore>,
    connection: &Connection,
    saga_id: CrossAccountMoveId,
) -> Outcome {
    let sagas = CrossAccountMoveRepository::new(connection);
    let saga = match sagas.get(saga_id).await {
        Ok(Some(saga)) => saga,
        Ok(None) => {
            return Outcome::Obsolete {
                reason: "the move this copy belonged to no longer exists".to_owned(),
            };
        }
        Err(error) => return failed(format!("could not read the move: {error}")),
    };

    match saga.phase {
        MovePhase::Confirmed | MovePhase::Done => return Outcome::Applied,
        MovePhase::Aborted => {
            return Outcome::Obsolete {
                reason: "the move was abandoned; the source copy is intact".to_owned(),
            };
        }
        MovePhase::Copying | MovePhase::Unconfirmed => {}
    }

    // A missing target aborts the saga with the source intact (Q13).
    let Some(target_path) = target_path(connection, &saga).await else {
        return abort(
            &sagas,
            saga_id,
            "the destination no longer exists; the move was abandoned and \
             the source copy is intact",
        )
        .await;
    };

    // Confirm before append: a replay after a crash finds the earlier copy instead of making
    // another.
    if let Some(rfc) = saga.rfc_message_id.as_deref() {
        match backend.find_by_message_id(&target_path, rfc).await {
            Ok(Some(uid)) => {
                return match sagas.confirm(saga_id, Some(&uid)).await {
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
        // The append ran and nothing can prove where it landed: stop and ask, source untouched.
        return failed(
            "the message was uploaded but its arrival could not be confirmed; \
             nothing was deleted — check the destination folder"
                .to_owned(),
        );
    }

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

    if let Some(mapping) = mapping {
        return match sagas
            .confirm(saga_id, Some(&mapping.destination_remote_id()))
            .await
        {
            Ok(()) => Outcome::Applied,
            Err(error) => failed(format!("could not record the confirmation: {error}")),
        };
    }
    if let Some(rfc) = saga.rfc_message_id.as_deref()
        && let Ok(Some(uid)) = backend.find_by_message_id(&target_path, rfc).await
    {
        return match sagas.confirm(saga_id, Some(&uid)).await {
            Ok(()) => Outcome::Applied,
            Err(error) => failed(format!("could not record the confirmation: {error}")),
        };
    }
    if let Err(error) = sagas.transition(saga_id, MovePhase::Unconfirmed).await {
        return failed(format!("could not record the unconfirmed append: {error}"));
    }
    failed(
        "the message was uploaded but its arrival could not be confirmed; \
         nothing was deleted — check the destination folder"
            .to_owned(),
    )
}

pub(crate) async fn remove(
    backend: &dyn MailBackend,
    connection: &Connection,
    saga_id: CrossAccountMoveId,
    snapshot: &[postio_model::RemoteId],
) -> Outcome {
    let sagas = CrossAccountMoveRepository::new(connection);
    let saga = match sagas.get(saga_id).await {
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
        // Until the copy is proven this waits, however the two queues interleave.
        MovePhase::Copying | MovePhase::Unconfirmed => {
            return Outcome::Retry {
                reason: "the destination has not confirmed its copy yet".to_owned(),
                after: None,
            };
        }
        MovePhase::Confirmed => {}
    }

    // Spelled out rather than chained: a closure cannot await.
    let path = match saga.source_mailbox {
        Some(mailbox) => MailboxRepository::new(connection)
            .get(mailbox)
            .await
            .ok()
            .flatten()
            .map(|mailbox| mailbox.path),
        None => None,
    };
    // The queue row's snapshot first, the live row second (ADR 0026, #531): an inverse saga's
    // source is a provisional copy with no identity until phase 2 confirms (#940).
    let remote_id = match snapshot.first().cloned() {
        Some(id) => Some(id),
        None => match saga.source_message {
            Some(message) => MessageRepository::new(connection)
                .get(message)
                .await
                .ok()
                .flatten()
                .and_then(|message| message.server.remote_id),
            None => None,
        },
    };
    let (Some(path), Some(remote_id)) = (path, remote_id) else {
        // No server copy is named: settled, because retrying cannot produce a coordinate, but
        // logged since only one cause is healthy. Ids only.
        tracing::info!(
            saga = saga_id.get(),
            source_message = ?saga.source_message.map(postio_model::MessageId::get),
            had_snapshot = !snapshot.is_empty(),
            "settling a cross-account removal that names no server copy"
        );
        return match sagas.transition(saga_id, MovePhase::Done).await {
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
    match sagas.transition(saga_id, MovePhase::Done).await {
        Ok(()) => Outcome::Applied,
        Err(error) => failed(format!("could not settle the move: {error}")),
    }
}

async fn target_path(connection: &Connection, saga: &CrossAccountMove) -> Option<String> {
    let mailbox = saga.target_mailbox?;
    saga.target_account?;
    MailboxRepository::new(connection)
        .get(mailbox)
        .await
        .ok()
        .flatten()
        .map(|mailbox| mailbox.path)
}

fn raw_bytes(blobs: Option<&BlobStore>, saga: &CrossAccountMove) -> Option<Vec<u8>> {
    let blob = saga.raw_blob_id.as_deref()?;
    blobs?.get(&postio_model::BlobId::new(blob.to_owned())).ok()
}

async fn abort(
    sagas: &CrossAccountMoveRepository<'_>,
    saga: CrossAccountMoveId,
    reason: &str,
) -> Outcome {
    if let Err(error) = sagas.transition(saga, MovePhase::Aborted).await {
        return failed(format!("could not abandon the move: {error}"));
    }
    failed(reason.to_owned())
}

fn failed(reason: String) -> Outcome {
    Outcome::Failed { reason }
}

fn deleted_flag() -> postio_model::FlagSet {
    let mut flags = postio_model::FlagSet::new();
    flags.insert(postio_model::Flag::Deleted);
    flags
}
