//! Fetching a forward's carried attachments before the forward is built.
//!
//! A forward carries the original's attachment rows (`reply::forward`), and
//! under ADR 0017 an attachment's bytes stay on the server until somebody
//! asks for them. So the ordinary forward — of a message whose PDF nobody
//! opened — is a draft holding a part with no bytes on this machine, and
//! building its MIME from the blob store alone is impossible. Sending used to
//! refuse it outright, as though the file were still being written, and the
//! composer had already closed by then (#1686).
//!
//! The send is queued at once whatever is local — the UI never awaits the
//! network — and the drain, which is already talking to the server, fetches
//! the missing sections from the original here, before [`crate::send`] or
//! [`crate::drafts`] resolve the draft into bytes. The section goes into the
//! blob store once and onto both rows: the draft's, so the message can be
//! built, and the original's, so opening it later is not a second download.
//!
//! A server that cannot be reached is a wait, not a failure: the step is
//! deferred like any other transient error. What *is* a failure — the
//! original is gone from this machine, or the part cannot be fetched by
//! section — names the file, because "attachment" is not something a person
//! can go looking for.

use postio_account::backend::MailBackend;
use postio_account::cancel::CancelToken;
use postio_model::{Attachment, BodyState, DraftId, DraftState};
use postio_storage::BlobStore;
use postio_storage::Connection;
use postio_storage::repository::{DraftRepository, MessageRepository};

use crate::backfill::BodyRequest;
use crate::drain::{Result, SyncError};

/// Whether a draft's carried attachments are all local now.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Carried {
    /// Nothing is missing that this can fetch — resolve the draft as usual.
    Ready,
    /// Not yet: the server could not be asked, or the original has no
    /// server identity yet. Deferred with the ordinary backoff.
    Later(String),
    /// Never: the bytes have nowhere left to come from.
    Impossible(String),
}

/// Fetches every attachment `draft_id` carries from the message it was
/// forwarded from whose bytes are not in the blob store yet.
///
/// A missing attachment with no `part_id` is not a carried one — it is a
/// file the composer is still writing — and is left for the caller's own
/// check, which already knows how to wait for it.
pub(crate) async fn fetch_missing(
    connection: &Connection,
    backend: &dyn MailBackend,
    blobs: &BlobStore,
    draft_id: DraftId,
) -> Result<Carried> {
    let Some(draft) = DraftRepository::new(connection).get(draft_id).await? else {
        // Gone: the resolve that follows says so in its own words.
        return Ok(Carried::Ready);
    };
    if matches!(
        draft.state,
        DraftState::Sent | DraftState::Sending | DraftState::Unconfirmed
    ) {
        // Past the commit point (ADR 0021). Nothing will be built from these
        // bytes again, so there is nothing to fetch them for.
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

        // Downloaded since the forward was made — opened in the reader, or an
        // eager pass got there. No round trip for bytes already on the disk.
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
                    // No UID yet — a message moved a moment ago and not yet
                    // resynced. It will have one after the next sync.
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

    // The original's body state, the way a payload fetch leaves it: `full`
    // once every part is local, which is what makes its chip say "open". Only
    // from `partial` — the words are already here — because a message whose
    // text was never fetched is not made whole by having its PDF.
    if let Some(settled) = messages.get(source.id).await?
        && settled.sync.body_state == BodyState::Partial
        && crate::backfill::state_for(&settled.attachments) == BodyState::Full
    {
        messages.set_body_state(source.id, BodyState::Full).await?;
    }
    Ok(Carried::Ready)
}

/// How a refusal names an attachment: its filename, quoted, or — when the
/// sender gave it none — its type and where it sits in the message.
///
/// Never the bare word "attachment". A person told that "attachment" could
/// not be sent has nothing to look for, which is what #1686's refusal said.
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
