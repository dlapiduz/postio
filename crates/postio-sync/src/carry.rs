//! Fetches a forward's carried attachments from the original before the forward is built (#1686).
//! Runs in the drain, so queueing the send never awaits the network; an unreachable server defers
//! the step.

use postio_account::backend::MailBackend;
use postio_account::cancel::CancelToken;
use postio_model::{Attachment, BodyState, DraftId, DraftState};
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{DraftRepository, MessageRepository};

use crate::backfill::BodyRequest;
use crate::drain::{Result, SyncError};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Carried {
    Ready,
    Later(String),
    Impossible(String),
}

/// A missing attachment with no `part_id` is a file the composer is still writing, and is left to
/// the caller's own check.
pub(crate) async fn fetch_missing(
    connection: &Connection,
    backend: &dyn MailBackend,
    blobs: &BlobStore,
    draft_id: DraftId,
) -> Result<Carried> {
    let Some(draft) = DraftRepository::new(connection).get(draft_id).await? else {
        return Ok(Carried::Ready);
    };
    if matches!(
        draft.state,
        DraftState::Sent | DraftState::Sending | DraftState::Unconfirmed
    ) {
        // Past the commit point (ADR 0021): nothing will be built from these bytes again.
        return Ok(Carried::Ready);
    }
    let missing: Vec<&Attachment> = draft
        .attachments
        .iter()
        .filter(|attachment| attachment.blob_id.is_none() && attachment.part_id.is_some())
        .collect();
    if missing.is_empty() {
        return Ok(Carried::Ready);
    }

    let messages = MessageRepository::new(connection);
    let source = match draft.forwarded_from {
        Some(id) => messages.get(id).await?,
        None => None,
    };
    let Some(source) = source else {
        return Ok(Carried::Impossible(format!(
            "the attachment {} could not be forwarded: the message it came from \
             is no longer on this machine",
            describe(missing[0])
        )));
    };

    let cancel = CancelToken::new();
    let mut request: Option<BodyRequest> = None;
    for carried in missing {
        let part_id = carried.part_id.as_deref().unwrap_or_default();
        let Some(original) = source
            .attachments
            .iter()
            .find(|attachment| attachment.part_id.as_deref() == Some(part_id))
        else {
            return Ok(Carried::Impossible(format!(
                "the attachment {} could not be forwarded: the message it came \
                 from no longer has it",
                describe(carried)
            )));
        };

        let blob = match &original.blob_id {
            Some(blob) => blob.clone(),
            None => {
                if request.is_none() {
                    request = messages
                        .backfill_candidate(source.id)
                        .await?
                        .map(BodyRequest::from);
                }
                let Some(request) = &request else {
                    return Ok(Carried::Later(format!(
                        "the attachment {} is waiting for the message it came \
                         from to reach the server",
                        describe(carried)
                    )));
                };
                let fetched = crate::backfill::fetch_section(
                    blobs, backend, request, original, part_id, &cancel,
                )
                .await;
                let blob = match fetched {
                    Ok(Some((blob, _))) => blob,
                    Ok(None) => {
                        return Ok(Carried::Impossible(format!(
                            "the attachment {} could not be fetched from the \
                             message it was forwarded from",
                            describe(carried)
                        )));
                    }
                    Err(SyncError::Backend(error)) => {
                        let reason = format!(
                            "could not fetch the attachment {} to forward: {error}",
                            describe(carried)
                        );
                        return Ok(if error.is_transient() {
                            Carried::Later(reason)
                        } else {
                            Carried::Impossible(reason)
                        });
                    }
                    Err(error) => return Err(error),
                };
                messages
                    .set_attachment_blob(source.id, part_id, &blob)
                    .await?;
                blob
            }
        };
        DraftRepository::new(connection)
            .set_attachment_blob(draft.id, carried.id, &blob)
            .await?;
    }

    // Only from `partial`: a message whose text was never fetched is not made whole by its PDF.
    if let Some(settled) = messages.get(source.id).await?
        && settled.sync.body_state == BodyState::Partial
        && crate::backfill::state_for(&settled.attachments) == BodyState::Full
    {
        messages.set_body_state(source.id, BodyState::Full).await?;
    }
    Ok(Carried::Ready)
}

/// Never the bare word "attachment": a person told that cannot go looking for it (#1686).
pub(crate) fn describe(attachment: &Attachment) -> String {
    match attachment.filename.as_deref().map(str::trim) {
        Some(name) if !name.is_empty() => format!("{name:?}"),
        _ => match attachment.part_id.as_deref() {
            Some(part) => format!("(an unnamed {} part, section {part})", attachment.mime_type),
            None => format!("(an unnamed {} file)", attachment.mime_type),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::MessageId;

    #[test]
    fn a_refusal_names_the_file_by_its_filename() {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 12);
        attachment.filename = Some("statement.pdf".to_owned());
        assert_eq!(describe(&attachment), "\"statement.pdf\"");
    }

    #[test]
    fn an_unnamed_part_is_named_by_its_type_rather_than_as_attachment() {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "image/png", 12);
        attachment.filename = Some("  ".to_owned());
        attachment.part_id = Some("2.1".to_owned());
        let named = describe(&attachment);
        assert_eq!(named, "(an unnamed image/png part, section 2.1)");
        assert_ne!(named, "\"attachment\"");
    }
}
