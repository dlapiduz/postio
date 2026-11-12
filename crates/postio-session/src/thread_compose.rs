//! Replying to an older message and forwarding a whole thread (spec 013 US7).
//!
//! Local only: every function here reads the store and the blob store and
//! fetches nothing (FR-028, constitution I and VI). A form that needs a
//! message that is not on this machine is refused with the messages named,
//! and the person decides what to do about it.
//!
//! # The typed text is told from the rest by position
//!
//! A reply's body is the person's words, then the attribution and the quote
//! (`postio_body::requote` finds the seam by the same detector the reader
//! folds by). A forward's is the person's words, then a line opening
//! `postio_body::forward::MARKER` and the forwarded content
//! (`typed_above` / `with_typed`). Nothing is stored beside the draft to say
//! where one ends: the body says it, in both halves, so a draft saved and
//! reopened is switched the same way.

use postio_body::forward::{self, Member};
use postio_model::ids::{BlobId, MessageId};
use postio_model::{
    Attachment, Draft, ForwardForm, Forwarded, Message, MessageBody, ThreadId, ThreadMember, reply,
    subject,
};
use postio_storage::repository::{AccountRepository, MessageRepository, ThreadRepository};
use postio_storage::{BlobStore, Checkout, Store};

use crate::reading::{Body, load_body_or_reason};

/// A message with its body, when the body is here.
struct Loaded {
    message: Message,
    local: bool,
}

async fn load(connection: &Checkout, id: MessageId) -> Result<Loaded, String> {
    let mut message = MessageRepository::new(connection)
        .get(id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("That message is gone")?;
    // `offline` only changes the reason a body is missing, never whether it is
    // read, so it does not matter here.
    let local = match load_body_or_reason(connection, id, false).await {
        Body::Ready { body, .. } => {
            message.body = body;
            true
        }
        Body::Absent(_) => false,
    };
    Ok(Loaded { message, local })
}

async fn members_of(connection: &Checkout, thread: ThreadId) -> Result<Vec<ThreadMember>, String> {
    let members = ThreadRepository::new(connection)
        .members(thread)
        .await
        .map_err(|error| error.to_string())?;
    if members.is_empty() {
        return Err("That thread is gone".to_owned());
    }
    Ok(members)
}

/// A new forward of `thread` in `form`: of its latest message for `This`.
pub async fn forward_thread(
    database: &Store,
    blobs: &BlobStore,
    thread: ThreadId,
    form: ForwardForm,
) -> Result<Forwarded, String> {
    let connection = database.read().await.map_err(|error| error.to_string())?;
    let members = members_of(&connection, thread).await?;
    let latest = members
        .last()
        .expect("members_of refuses an empty thread")
        .id;
    let anchor = load(&connection, latest).await?;
    let account = AccountRepository::new(&connection)
        .get(anchor.message.account_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("That account is gone")?;
    let draft = reply::forward(
        &anchor.message,
        &account,
        reply::plain_forward(&anchor.message),
    );
    rebuild(&connection, blobs, draft, anchor, thread, members, form).await
}

/// `draft`, a forward, carrying `form`, the words typed above the forwarded
/// content kept.
pub async fn set_forward_form(
    database: &Store,
    blobs: &BlobStore,
    draft: Draft,
    form: ForwardForm,
) -> Result<Forwarded, String> {
    let anchor_id = draft.forwarded_from.ok_or("That draft is not a forward")?;
    let connection = database.read().await.map_err(|error| error.to_string())?;
    let anchor = load(&connection, anchor_id).await?;
    let thread = anchor
        .message
        .thread_id
        .ok_or("That message is not in a thread")?;
    let members = members_of(&connection, thread).await?;
    rebuild(&connection, blobs, draft, anchor, thread, members, form).await
}

async fn rebuild(
    connection: &Checkout,
    blobs: &BlobStore,
    mut draft: Draft,
    anchor: Loaded,
    thread: ThreadId,
    members: Vec<ThreadMember>,
    form: ForwardForm,
) -> Result<Forwarded, String> {
    let mut loaded = Vec::with_capacity(members.len());
    for member in &members {
        loaded.push(if member.id == anchor.message.id {
            Loaded {
                message: anchor.message.clone(),
                local: anchor.local,
            }
        } else {
            load(connection, member.id).await?
        });
    }
    let typed = forward::typed_above(&draft.body);

    let (subject, body, carried): (String, MessageBody, Vec<Attachment>) = match form {
        ForwardForm::This => {
            let message = &anchor.message;
            let content = forwarded_message(message, draft.rich);
            (
                subject::forward_subject(message.subject.as_deref().unwrap_or_default()),
                forward::with_typed(&typed, &content),
                reply::carried_attachments(message),
            )
        }
        ForwardForm::Thread => {
            let missing: Vec<MessageId> = loaded
                .iter()
                .filter(|member| {
                    !member.local
                        || member
                            .message
                            .attachments
                            .iter()
                            .any(|attachment| attachment.blob_id.is_none())
                })
                .map(|member| member.message.id)
                .collect();
            if !missing.is_empty() {
                tracing::debug!(
                    thread = thread.get(),
                    missing = missing.len(),
                    "thread forward refused: not local"
                );
                return Ok(Forwarded::NotLocal(missing));
            }
            let subject_line = thread_subject_line(&loaded);
            let carried: Vec<Member> = loaded
                .iter()
                .map(|member| Member {
                    header: forward::header(
                        member
                            .message
                            .primary_from()
                            .map_or("someone", |from| from.display()),
                        &member
                            .message
                            .best_date()
                            .format("%Y-%m-%d %H:%M")
                            .to_string(),
                    ),
                    body: member.message.body.clone(),
                    attachments: member.message.attachments.clone(),
                })
                .collect();
            let out = forward::forward_thread(&carried, &subject_line);
            let content = MessageBody {
                text: out.body.text,
                html: out.body.html.filter(|_| draft.rich),
            };
            (
                out.subject,
                forward::with_typed(&typed, &content),
                out.attachments,
            )
        }
        ForwardForm::Attachments => {
            let mut raws = Vec::with_capacity(loaded.len());
            let mut missing = Vec::new();
            for member in &loaded {
                match member
                    .message
                    .raw_blob_id
                    .as_ref()
                    .filter(|blob| blobs.contains(blob))
                    .map(|blob| blobs.get(blob))
                {
                    Some(Ok(bytes)) => raws.push(bytes),
                    _ => missing.push(member.message.id),
                }
            }
            if !missing.is_empty() {
                tracing::debug!(
                    thread = thread.get(),
                    missing = missing.len(),
                    "thread forward refused: no raw source"
                );
                return Ok(Forwarded::NotLocal(missing));
            }
            let mut emls = forward::forward_as_attachments(&raws);
            for (eml, raw) in emls.iter_mut().zip(&raws) {
                eml.blob_id = Some(blobs.put(raw).map_err(|error| error.to_string())?);
            }
            let subject_line = thread_subject_line(&loaded);
            let body = MessageBody {
                text: Some(typed.text.clone().unwrap_or_default()),
                html: typed.html.clone().filter(|_| draft.rich),
            };
            (
                forward::thread_subject(&subject_line, loaded.len()),
                body,
                emls,
            )
        }
    };

    // What any form carries is replaced; what the person attached themselves
    // stays. A form's own files are the thread's attachments and its raw
    // sources, by content hash, or a part carried without its bytes.
    let formed: std::collections::HashSet<BlobId> = loaded
        .iter()
        .flat_map(|member| {
            member
                .message
                .attachments
                .iter()
                .filter_map(|attachment| attachment.blob_id.clone())
                .chain(member.message.raw_blob_id.clone())
        })
        .collect();
    draft
        .attachments
        .retain(|attachment| match &attachment.blob_id {
            Some(blob) => !formed.contains(blob),
            None => attachment.part_id.is_none(),
        });
    let own = std::mem::take(&mut draft.attachments);
    draft.attachments = carried;
    draft.attachments.extend(own);
    draft.subject = subject;
    draft.body = body;
    tracing::debug!(
        thread = thread.get(),
        members = loaded.len(),
        ?form,
        "forward form built"
    );
    Ok(Forwarded::Draft(Box::new(draft)))
}

/// What the thread is called in a forward's subject: its oldest message's
/// subject as sent. The stored thread subject is the normalised one the
/// threader compares by (lower case, no `Re:`), which is not what a person
/// would recognise.
fn thread_subject_line(loaded: &[Loaded]) -> String {
    loaded
        .iter()
        .find_map(|member| member.message.subject.clone())
        .unwrap_or_default()
}

/// One message as a forward carries it: the conventional header block, then
/// the message, opening with `forward::MARKER` and with no caret line above
/// (`with_typed` leaves one).
fn forwarded_message(message: &Message, rich: bool) -> MessageBody {
    let plain = reply::plain_forward(message);
    let text = plain
        .text
        .map(|text| text.trim_start_matches('\n').to_owned());
    let html = rich.then(|| {
        let quote = postio_body::quote_of(
            message.body.html.as_deref(),
            message.body.text.as_deref().unwrap_or_default(),
            "fwd",
        );
        let mut document = postio_body::forwarded(&quote, &reply::forward_header(message));
        // The caret's blank paragraph is `with_typed`'s to leave.
        document.blocks.remove(0);
        document.to_html()
    });
    MessageBody { text, html }
}

/// `draft`, a reply, answering its thread's latest received message: the
/// words above the quote stay, the quote and `In-Reply-To` move.
pub async fn reply_to_latest(database: &Store, mut draft: Draft) -> Result<Draft, String> {
    let connection = database.read().await.map_err(|error| error.to_string())?;
    let thread = match draft.thread_id {
        Some(thread) => thread,
        None => {
            let parent = draft.in_reply_to.ok_or("That draft is not a reply")?;
            load(&connection, parent)
                .await?
                .message
                .thread_id
                .ok_or("That message is not in a thread")?
        }
    };
    let members = members_of(&connection, thread).await?;
    let latest = members
        .iter()
        .rev()
        .find(|member| !member.own)
        .ok_or("Nothing in that thread was received")?;
    if draft.in_reply_to == Some(latest.id) {
        return Ok(draft);
    }
    let newest = load(&connection, latest.id).await?;
    if !newest.local {
        return Err("The newest message has not been downloaded yet".to_owned());
    }
    let message = &newest.message;
    let quote = postio_body::quote_of(
        message.body.html.as_deref(),
        message.body.text.as_deref().unwrap_or_default(),
        "q",
    );
    draft.body = postio_body::requote(&draft.body, &reply::attribution(message), &quote);
    draft.in_reply_to = Some(message.id);
    draft.thread_id = Some(thread);
    tracing::debug!(
        thread = thread.get(),
        "reply moved to the latest received message"
    );
    Ok(draft)
}
