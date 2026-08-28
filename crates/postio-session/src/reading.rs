//! Reading a message's body, and resolving the parts it references.
//!
//! Moved here from `postio-app` (#608). Both halves are the *reading* side of
//! the reader path, and neither is glue: they are judgement earned from bugs,
//! and a second copy on the macOS side would reproduce the bugs rather than
//! the behaviour -- which is what ADR 0019 Q6 exists to prevent.
//!
//! [`load_body_or_reason`] tells apart situations that look identical at the
//! blob layer: a local draft, another client's draft, not-fetched against
//! offline, and genuinely-empty against blobs that will not read. Issue #70
//! Cause A was all of them rendering as one blank column.
//!
//! [`cid_source`] carries a security property in fifteen lines: a
//! `Content-ID` resolves only within the message that declares it, so one
//! sender cannot address another sender's parts.
//!
//! Toolkit-free, like the rest of this crate: the absent states come from
//! `postio_ui::reader`, the frontend-independent half of the reader.

use std::rc::Rc;

use postio_model::MessageBody;
use postio_model::ids::MessageId;
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::{BlobStore, Database};
use postio_ui::reader::document::Absent;
use postio_ui::reader::parts::BlobSource;

/// A message's text and HTML, if they have been downloaded.
///
/// Absent rather than an error for a message still `Partial` — headers synced,
/// body not yet — which is the ordinary state of a mailbox mid-backfill, not
/// a fault. Replying to one just quotes nothing, the same way any degraded
/// state here should: fewer words in the draft, never a broken one.
pub fn load_body(
    connection: &postio_storage::PooledConnection,
    blobs: &BlobStore,
    id: MessageId,
) -> MessageBody {
    let Ok(Some(blob_ids)) = MessageRepository::new(connection).body_blobs(id) else {
        return MessageBody::default();
    };
    MessageBody {
        text: blob_ids.text.and_then(|id| read_blob_text(blobs, &id)),
        html: blob_ids.html.and_then(|id| read_blob_text(blobs, &id)),
    }
}

/// A message's body, or which kind of "no body" this is.
///
/// [`load_body`] answers `MessageBody::default()` for four situations that
/// are not the same situation, and the reading pane used to render all four
/// as a blank column (issue #70, Cause A). Replying does not care — quoting
/// nothing is the right degraded behaviour either way — so `load_body` keeps
/// its shape and this sits beside it for the caller that has to *show*
/// something.
pub enum Body {
    /// Bytes are on this machine, and these are them.
    Ready(postio_model::MessageBody),
    /// There are none, for this reason.
    Absent(Absent),
}

/// As [`load_body`], but distinguishing the ways a body can be missing.
///
/// The message's own [`BodyState`] is what says whether a body was ever
/// fetched, and it has to be: `body_blobs` answers a row naming no blobs for
/// a message nobody has downloaded *and* for one that was downloaded and had
/// no body in it. Those two look identical at the blob layer and are opposite
/// things to a reader — one is worth waiting for and one is finished.
///
/// So:
///
/// * **not fetched** (`NotFetched`, `HeadersOnly`) — the backfill has not
///   been here. The ordinary state of a mailbox that has just been added,
///   and not a fault.
/// * **fetched, naming no blobs** — the message really has neither a text
///   nor an HTML part.
/// * **fetched, naming blobs that will not read** — the database and the
///   blob directory disagree. Rare, and a genuine fault.
///
/// `is_offline` is what tells [`Absent::Offline`] from [`Absent::Partial`]
/// for a body that has not been fetched: both are "nothing here yet", but
/// only one of them is worth promising a backfill for. The caller reads it
/// off the engine's `ConnectionState` (`reading.rs`), because this module
/// has no seam of its own onto the sync engine.
///
/// [`BodyState`]: postio_model::message::BodyState
/// [`Absent::Offline`]: Absent::Offline
pub fn load_body_or_reason(
    connection: &postio_storage::PooledConnection,
    blobs: &BlobStore,
    id: MessageId,
    is_offline: bool,
) -> Body {
    use Absent;

    // A draft's row has no body in the blob store and never will: the
    // composer's buffer is inline TEXT, deliberately, because a
    // content-addressed store would take one immutable blob per keystroke.
    // Reading the row would say "still downloading" about words the user is
    // looking at in another pane. #166.
    if let Ok(Some(draft)) = DraftRepository::new(connection).by_message(id) {
        return Body::Ready(draft.body);
    }

    let repository = MessageRepository::new(connection);

    // Has anything been downloaded for this message at all?
    match repository.get(id) {
        // `\Draft` is set, but the `by_message` lookup above found no local
        // buffer: this row belongs to another client's draft. Its body may
        // well be sitting in the blob store already, but showing it as an
        // ordinary, readable message would be exactly the dead end #175
        // exists to close -- there is nothing here this machine can edit,
        // whatever state the body is in.
        Ok(Some(message)) if message.flags.is_draft() => {
            return Body::Absent(Absent::ForeignDraft);
        }
        Ok(Some(message)) if !message.sync.body_state.has_body() => {
            let reason = if is_offline {
                Absent::Offline
            } else {
                Absent::Partial
            };
            return Body::Absent(reason);
        }
        Ok(Some(_)) => {}
        // The row is gone, or unreadable. Either way there is nothing to
        // wait for, so do not tell the user to wait.
        Ok(None) => return Body::Absent(Absent::Missing),
        Err(error) => {
            tracing::warn!(message = id.get(), %error, "cannot read a message row");
            return Body::Absent(Absent::Missing);
        }
    }

    let blob_ids = match repository.body_blobs(id) {
        Ok(Some(blob_ids)) => blob_ids,
        // Fetched, and nothing recorded to show for it.
        Ok(None) => return Body::Absent(Absent::Empty),
        Err(error) => {
            tracing::warn!(message = id.get(), %error, "cannot read a message's body record");
            return Body::Absent(Absent::Missing);
        }
    };

    if blob_ids.text.is_none() && blob_ids.html.is_none() {
        return Body::Absent(Absent::Empty);
    }

    let body = postio_model::MessageBody {
        text: blob_ids.text.and_then(|id| read_blob_text(blobs, &id)),
        html: blob_ids.html.and_then(|id| read_blob_text(blobs, &id)),
    };
    if body.text.is_none() && body.html.is_none() {
        // Blobs were named and none of them read back. `read_blob_text` has
        // already logged why; what the user needs is to be told the pane is
        // empty because something is wrong, not because they should wait.
        return Body::Absent(Absent::Missing);
    }
    Body::Ready(body)
}

/// A body blob as text, or nothing with the reason logged.
///
/// Module-private, as it was in `postio-app`: it is the shared tail of
/// [`load_body`] and [`load_body_or_reason`], not a way for a caller to reach
/// past them into the blob store.
fn read_blob_text(blobs: &BlobStore, id: &postio_model::ids::BlobId) -> Option<String> {
    let bytes = blobs
        .get(id)
        .map_err(|error| tracing::warn!(%error, "could not read a message body blob"))
        .ok()?;
    String::from_utf8(bytes)
        .map_err(|error| tracing::warn!(%error, "a body blob was not valid UTF-8"))
        .ok()
}

/// Where a rendered message resolves its `cid:` parts from.
///
/// # Scoped to one message on purpose
///
/// A `Content-ID` is only meaningful inside the message that declares it, so
/// resolving one globally would let a sender address another sender's parts.
/// [`BlobSource`] carries no message, so the caller supplies `showing` and
/// this asks it at the moment the scheme handler runs — which is also what
/// makes it correct while the pane is changing.
///
/// # A part that is not here does not draw
///
/// The hardened view has network access off, so a part whose bytes are not
/// already on this machine resolves to nothing. That is the privacy
/// commitment working rather than a failure to handle: a remote fetch here
/// would be the tracking pixel the reader spent so much effort blocking,
/// arriving through the back door.
///
/// Shared with the search preview, which has the same problem with a
/// different notion of "the message on screen" — hence the closure rather
/// than a widget.
pub fn cid_source(
    showing: impl Fn() -> Option<MessageId> + 'static,
    database: Database,
    blobs: BlobStore,
) -> Rc<dyn BlobSource> {
    Rc::new(move |content_id: &str| resolve_cid(&database, &blobs, showing()?, content_id))
}

/// One inline part of `message`, by its `Content-ID`.
///
/// The same resolution [`cid_source`] performs, as a plain call — because a
/// frontend across an FFI cannot hold an `Rc<dyn BlobSource>`, and a second
/// implementation of these six lines would be a second chance to get the
/// scoping wrong.
///
/// # Scoped to `message`, and that is the whole point
///
/// A `Content-ID` is only meaningful inside the message that declares it, so
/// resolving one globally would let a sender address another sender's parts.
/// The message is a parameter rather than something read from ambient state,
/// so a caller cannot forget to supply it.
///
/// # A part that is not here does not draw
///
/// `None` when the bytes are not already on this machine. That is the privacy
/// commitment working rather than a gap to fill in later: fetching here would
/// be the tracking pixel the reader spends so much effort blocking, arriving
/// through the back door.
pub fn resolve_cid(
    database: &Database,
    blobs: &BlobStore,
    message: MessageId,
    content_id: &str,
) -> Option<(Vec<u8>, String)> {
    let connection = database.connection().ok()?;
    let part = MessageRepository::new(&connection)
        .get(message)
        .ok()??
        .attachments
        .into_iter()
        .find(|part| part.content_id.as_deref() == Some(content_id))?;
    let bytes = blobs.get(&part.blob_id?).ok()?;
    Some((bytes, part.mime_type))
}
