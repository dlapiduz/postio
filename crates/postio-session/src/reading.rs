//! Reading a message's body, and resolving the parts it references.
//!
//! Moved here from `postio-app` (#608). Both halves are the *reading* side of
//! the reader path, and neither is glue: they are judgement earned from bugs,
//! and a second copy on the macOS side would reproduce the bugs rather than
//! the behaviour -- which is what ADR 0019 Q6 exists to prevent.
//!
//! [`load_body_or_reason`] tells apart situations that look identical in the
//! columns: a local draft, another client's draft, not-fetched against
//! offline, and genuinely-empty against a body that will not decode. Issue
//! #70 Cause A was all of them rendering as one blank column.
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
use postio_runtime::Engine;
use postio_storage::repository::{DraftRepository, MessageRepository};
use postio_storage::{BlobStore, Store};
use postio_ui::reader::document::Absent;
use postio_ui::reader::parts::{self, BlobSource, Node};

/// A message's text and HTML, if they have been downloaded.
///
/// Absent rather than an error for a message still `Partial` — headers synced,
/// body not yet — which is the ordinary state of a mailbox mid-backfill, not
/// a fault. Replying to one just quotes nothing, the same way any degraded
/// state here should: fewer words in the draft, never a broken one.
pub async fn load_body(connection: &postio_storage::Checkout, id: MessageId) -> MessageBody {
    let Ok(Some(stored)) = MessageRepository::new(connection).body(id).await else {
        return MessageBody::default();
    };
    MessageBody {
        text: stored.text,
        html: stored.html,
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
    Ready {
        /// The words.
        body: postio_model::MessageBody,
        /// Whether those words are a guess rather than what was sent.
        ///
        /// Carried here rather than dropped at this boundary, which is where
        /// it used to go: `StoredBody` knows, `MessageBody` has nowhere to
        /// put it, and the reading pane is the only place that can say so.
        /// That is the same loss this enum already exists to prevent one
        /// level up — four kinds of "no body" rendered as one blank column
        /// (#70) — applied to "a body, but not the sender's" (#901).
        encoding_problems: bool,
    },
    /// There are none, for this reason.
    Absent(Absent),
}

/// As [`load_body`], but distinguishing the ways a body can be missing.
///
/// The message's own [`BodyState`] is what says whether a body was ever
/// fetched, and it has to be: `body` answers a row holding no parts for
/// a message nobody has downloaded *and* for one that was downloaded and had
/// no body in it. Those two look identical in the columns and are opposite
/// things to a reader — one is worth waiting for and one is finished.
///
/// So:
///
/// * **not fetched** (`NotFetched`, `HeadersOnly`) — the backfill has not
///   been here. The ordinary state of a mailbox that has just been added,
///   and not a fault.
/// * **fetched, holding no parts** — the message really has neither a text
///   nor an HTML part.
/// * **fetched, holding parts that will not decode** — the row and this build
///   disagree about what is in the column. Rare, and a genuine fault.
///
/// `is_offline` is what tells [`Absent::Offline`] from [`Absent::Partial`]
/// for a body that has not been fetched: both are "nothing here yet", but
/// only one of them is worth promising a backfill for. The caller reads it
/// off the engine's `ConnectionState` (`reading.rs`), because this module
/// has no seam of its own onto the sync engine.
///
/// [`BodyState`]: postio_model::message::BodyState
/// [`Absent::Offline`]: Absent::Offline
pub async fn load_body_or_reason(
    connection: &postio_storage::Checkout,
    id: MessageId,
    is_offline: bool,
) -> Body {
    use Absent;

    // A draft's body is not in `messages` and never will be: the composer's
    // buffer is `drafts.body_text`, inline and uncompressed, deliberately,
    // because autosave writes it on a keystroke. Reading the message row
    // would say "still downloading" about words the user is looking at in
    // another pane. #166.
    if let Ok(Some(draft)) = DraftRepository::new(connection).by_message(id).await {
        // A draft is the user's own text in Postio's own buffer: nothing
        // decoded it from anything, so there is nothing to caveat.
        return Body::Ready {
            body: draft.body,
            encoding_problems: false,
        };
    }

    let repository = MessageRepository::new(connection);

    // Has anything been downloaded for this message at all?
    match repository.get(id).await {
        // `\Draft` is set, but the `by_message` lookup above found no local
        // buffer: this row belongs to another client's draft. Its body may
        // well be stored already, but showing it as an ordinary, readable
        // message would be exactly the dead end #175
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

    let stored = match repository.body(id).await {
        Ok(Some(stored)) => stored,
        // The row went between the two reads above and here.
        Ok(None) => return Body::Absent(Absent::Missing),
        Err(error) => {
            // Either the row will not read, or a stored part will not
            // decompress. Both are faults, and both leave the pane empty --
            // what the user needs is to be told it is empty because something
            // is wrong, not because they should wait.
            tracing::warn!(message = id.get(), %error, "cannot read a message's body");
            return Body::Absent(Absent::Missing);
        }
    };

    if stored.text.is_none() && stored.html.is_none() {
        return Body::Absent(Absent::Empty);
    }

    Body::Ready {
        encoding_problems: stored.encoding_problems,
        body: postio_model::MessageBody {
            text: stored.text,
            html: stored.html,
        },
    }
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
/// # Why this blocks
///
/// [`BlobSource::resolve`] is synchronous, because WebKit calls it
/// synchronously while laying out a document: the `cid:` URI has to resolve
/// to bytes before the image can be placed, and there is nothing to hand a
/// future to. It was a blocking read before too -- `rusqlite` on this thread
/// -- so a runtime of its own here is the same work through the async API,
/// not new work on the frame path.
///
/// Through [`crate::blocking::now`], which is the part that is easy to get
/// half right: a runtime built here and blocked on *panics* when this is
/// called from a thread that is already a runtime worker, which the
/// application never is and every test is.
pub fn cid_source(
    showing: impl Fn() -> Option<MessageId> + 'static,
    database: Store,
    blobs: BlobStore,
) -> Rc<dyn BlobSource> {
    Rc::new(move |content_id: &str| {
        let message = showing()?;
        crate::blocking::now(resolve_cid(&database, &blobs, message, content_id))
    })
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
pub async fn resolve_cid(
    database: &Store,
    blobs: &BlobStore,
    message: MessageId,
    content_id: &str,
) -> Option<(Vec<u8>, String)> {
    let connection = database.connect().await.ok()?;
    let part = MessageRepository::new(&connection)
        .get(message)
        .await
        .ok()??
        .attachments
        .into_iter()
        .find(|part| part.content_id.as_deref() == Some(content_id))?;
    let bytes = blobs.get(&part.blob_id?).ok()?;
    Some((bytes, part.mime_type))
}

// ---------------------------------------------------------------------------
// A message's parts, and getting one part's bytes out of the store
// ---------------------------------------------------------------------------

/// How long a save waits for a part it had to ask for.
///
/// Long enough for a slow server on a bad link, short enough that a save that
/// is never going to work says so while the user is still looking at it.
const PART_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// Where one part's bytes are, when they are on this machine at all.
enum PartSource {
    /// The part's own blob — what ADR 0017's payload axis writes into
    /// `attachments.blob_id` when somebody opens an attachment.
    Payload(postio_model::ids::BlobId),
    /// The whole raw message, from which the part is cut.
    ///
    /// Two rows still land here: one fetched before the payload axis existed,
    /// and one whose `BODYSTRUCTURE` was never recorded, so no section could
    /// be named and every byte was the only answer.
    Raw(postio_model::ids::BlobId),
}

/// A message's MIME tree, ready to draw.
///
/// The root and the nodes travel together because neither means anything
/// alone: the nodes hang off a root the message itself declares, and
/// [`postio_ui::reader::parts::summary`] reads the root node for the header
/// line. Two calls would let a frontend draw one message's parts under
/// another message's type.
pub struct MessageParts {
    /// The message's own content type — `multipart/mixed`.
    pub root: String,
    /// Every node, flattened in walk order; the first is the message itself.
    pub nodes: Vec<Node>,
}

/// What a message is made of, read from the store and from nothing else.
///
/// **This fetches nothing and cannot.** It reads the attachment rows a sync
/// already wrote from `BODYSTRUCTURE`, which the server returns without
/// transferring a byte of any part — so the answer is complete and correct
/// for a message whose attachments are all still on the server, and
/// `Node::downloaded` is what says which of them are here. That is the shape
/// "nothing downloads until the user asks" takes at this layer: the call that
/// lists the parts has no route to the bytes at all, and the call that gets
/// them ([`part_bytes_at`]) is a separate, deliberate one.
pub async fn message_parts(database: &Store, message: MessageId) -> Result<MessageParts, String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    let stored = MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".to_string())?;
    let body = load_body(&connection, message).await;

    let root = parts::root_type(stored.content_type.as_deref(), &body, &stored.attachments);
    let nodes = parts::tree(&root, &stored.attachments);
    Ok(MessageParts { root, nodes })
}

/// One part's bytes, fetched first if they are not on this machine yet.
///
/// # Why this is the seam
///
/// A frontend runs its own file dialog and hands back the file the user
/// chose, so the only part worth sharing is what happens next — and that half
/// is nothing to do with any toolkit. Keeping it here, taking store handles
/// and returning bytes, makes "saves a part that was never downloaded,
/// fetching it first" an ordinary async test over a mock server instead of
/// something that needs a display and a file chooser. It is also the reason
/// there is one of it: the judgement below is worth more than the ten lines
/// it takes, and a second copy behind the macOS boundary would have
/// reproduced the bugs rather than the behaviour (ADR 0019).
///
/// # Where a received part's bytes are
///
/// In `Attachment::blob_id`, once somebody has opened it. That column was
/// filled only on the way *out* for the whole life of this project — a
/// composer attaching a file — and the receive path stored the whole raw
/// message instead, so a part had to be cut back out of it with `mime::parse`
/// on every open. ADR 0017 ended that: the text axis stores no raw source at
/// all, and the payload axis fetches `BODY.PEEK[<part_id>]` on demand.
///
/// So the fetch to wait for is the *part's*, and asking twice costs nothing:
/// the second open reads the blob and never reaches the network.
///
/// # This is allowed to reach the network, and almost nothing else is
///
/// The reading pane never does — a body that is not here simply does not
/// draw. This does, and only because the user named these bytes: they pressed
/// save, or dragged this part, or chose "Open with…" on it. That is the
/// difference `ARCHITECTURE.md` §11 turns on, and it is why the *listing*
/// call above cannot do this and this one is never called speculatively.
///
/// Returns `Err` rather than an empty file when the bytes cannot be had. A
/// zero-byte attachment on disk looks like a saved file and is not one.
pub async fn part_bytes_at(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    part_id: &str,
) -> Result<Vec<u8>, String> {
    let source = match locate_part(database, message, part_id).await? {
        Some(source) => source,
        // Never downloaded. Only because the user asked for these bytes by
        // name; see the section above.
        None => {
            let engine =
                engine.ok_or("This account is not syncing, so that part cannot be fetched")?;
            // `request_payloads` puts the section at the front of the backfill
            // and returns as soon as it is queued -- `true` means "there was
            // something to fetch", not "here it is". The bytes land when the
            // engine's own loop claims the job, so the wait is ours.
            if engine
                .request_payloads(message, vec![part_id.to_owned()])
                .await
                .map_err(|error| error.message().to_string())?
            {
                wait_for_part(database, message, part_id).await?
            } else {
                // "Nothing to fetch" has two readings, and the queue cannot
                // tell them apart: there is truly nothing (the message is
                // gone, or AttachmentPolicy::Never), or the background lane
                // fetched this very message between the look above and the
                // queue's answer -- ADR 0016 backfills every mailbox, so
                // both lanes chase the same messages, and the open that
                // races the backfill is an ordinary open, not a corner
                // (#109, four observed failures; a5735a3 is the same race
                // in the runtime's own test). One re-read settles it: a
                // committed write that made the answer `false` is visible
                // to this read, so no wait is needed -- absent here means
                // absent, and the sentence below is then the truth.
                locate_part(database, message, part_id)
                    .await?
                    .ok_or("There is nothing to fetch for that part")?
            }
        }
    };

    match source {
        PartSource::Payload(blob) => blobs.get(&blob).map_err(|error| error.to_string()),
        PartSource::Raw(blob) => {
            let bytes = blobs.get(&blob).map_err(|error| error.to_string())?;
            postio_model::mime::parse(&bytes)
                .parts
                .into_iter()
                .find(|part| part.attachment.part_id.as_deref() == Some(part_id))
                .map(|part| part.content)
                .ok_or_else(|| "That part is not in the message the server sent".into())
        }
    }
}

/// [`part_bytes_at`], for a caller holding an attachment row rather than a
/// MIME path.
///
/// The id is turned into a path here, before anything is fetched, and
/// deliberately. A whole-message fetch REPLACES the message's attachment rows
/// — the parser re-reads the structure and `MessageRepository::update` writes
/// the new set — so the `AttachmentId` a panel is holding does not survive
/// it. The MIME path does: `2` is `2` in every parse of the same bytes.
///
/// Which is also why the FFI boundary names a part by its path and never by
/// its row id: an id that stops meaning anything mid-operation is not
/// something to hand another process.
pub async fn part_bytes(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    attachment: postio_model::ids::AttachmentId,
) -> Result<Vec<u8>, String> {
    let part_id = part_path(database, message, attachment)
        .await?
        .ok_or("That part has no place in the message to read it from")?;
    part_bytes_at(database, blobs, engine, message, &part_id).await
}

/// Put one part's bytes at exactly `path`.
///
/// For the save where the *user* named the file — a save dialog has already
/// run and this is what happens next. Replaces rather than appends: the
/// dialog asked about overwriting, and a save that appended to an existing
/// file would corrupt it silently.
pub async fn save_part(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    part_id: &str,
    path: &std::path::Path,
) -> Result<(), String> {
    let bytes = part_bytes_at(database, blobs, engine, message, part_id).await?;
    std::fs::write(path, &bytes).map_err(|error| format!("Could not save that part: {error}"))?;
    // Ids, counts and outcomes. Not the filename: the sender chose it, and it
    // is as much the user's mail as the body is.
    tracing::debug!(message = %message, part = part_id, bytes = bytes.len(), "part saved");
    Ok(())
}

/// Write one part into `directory`, under the name Postio chose for it.
///
/// The difference from [`save_part`] is who names the file, and it is the
/// whole point. This is what "Open with…" and a drag-out are built on — paths
/// where the file is handed to *another application* — and on those paths the
/// caller supplies only a directory. A frontend cannot pass a filename
/// through, so the sender cannot choose one: the name is always
/// [`postio_ui::reader::parts::save_name`]'s, which has already had its
/// separators and control characters taken out.
///
/// Returns where it landed, which is what the caller hands to the launcher.
pub async fn export_part(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    part_id: &str,
    directory: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let node = node_for(database, message, part_id).await?;
    let bytes = part_bytes_at(database, blobs, engine, message, part_id).await?;

    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join(parts::save_name(&node));
    std::fs::write(&path, &bytes).map_err(|error| error.to_string())?;
    tracing::debug!(message = %message, part = part_id, bytes = bytes.len(), "part exported");
    Ok(path)
}

/// How a "save every part" went.
pub struct SavedParts {
    /// How many landed on disk.
    pub saved: usize,
    /// How many could not be had. Never abandons the rest of the batch: a
    /// message where one attachment is still on an unreachable server should
    /// still give the user the other four.
    pub failed: usize,
}

/// Write every part that holds bytes into `directory`.
///
/// A directory rather than a filename, because there is more than one file,
/// and [`postio_ui::reader::parts::save_names`] gives each part the name it
/// goes in under — including the suffix that stops two parts both calling
/// themselves `invoice.pdf` from becoming one file.
///
/// Containers are skipped rather than failed: a `multipart/alternative` has
/// no bytes of its own, and counting it as a failure would report a complete
/// save as a partial one on every ordinary message.
pub async fn save_all_parts(
    database: &Store,
    blobs: &BlobStore,
    engine: Option<Engine>,
    message: MessageId,
    directory: &std::path::Path,
) -> Result<SavedParts, String> {
    let leaves: Vec<Node> = message_parts(database, message)
        .await?
        .nodes
        .into_iter()
        .filter(Node::is_leaf)
        .collect();
    let names = parts::save_names(&leaves);
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    let mut outcome = SavedParts {
        saved: 0,
        failed: 0,
    };
    for (node, name) in leaves.iter().zip(names) {
        let written =
            match part_bytes_at(database, blobs, engine.clone(), message, &node.part_id).await {
                Ok(bytes) => std::fs::write(directory.join(name), &bytes).is_ok(),
                Err(_) => false,
            };
        if written {
            outcome.saved += 1;
        } else {
            outcome.failed += 1;
        }
    }
    tracing::debug!(
        message = %message,
        saved = outcome.saved,
        failed = outcome.failed,
        "every part saved"
    );
    Ok(outcome)
}

/// The node `part_id` names, for the rules that are about one part.
async fn node_for(database: &Store, message: MessageId, part_id: &str) -> Result<Node, String> {
    message_parts(database, message)
        .await?
        .nodes
        .into_iter()
        .find(|node| node.part_id == part_id)
        .ok_or_else(|| "That part is not in this message".to_string())
}

/// Wait for a queued part to land, or give up saying so.
///
/// Polling rather than listening: the engine announces arrivals on the event
/// stream, but that stream has exactly one reader — the frontend — and a
/// second consumer here would be a second place deciding what an event means.
/// A save the user is waiting on can afford to look.
///
/// It watches for either shape the bytes can arrive in: the part's own blob,
/// which is what a payload fetch writes, and the raw message, which is what
/// the whole-message fallback writes for a row whose section could not be
/// named.
///
/// The deadline is what turns a server that never answers into a sentence
/// rather than a spinner that never stops.
async fn wait_for_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
) -> Result<PartSource, String> {
    let deadline = std::time::Instant::now() + PART_WAIT;
    loop {
        // A read that fails here is usually the writer we are waiting for
        // holding the table, so contention is a reason to look again rather
        // than to give up. Only the deadline ends this.
        match locate_part(database, message, part_id).await {
            Ok(Some(source)) => return Ok(source),
            Ok(None) => {}
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return Err("That part did not arrive in time — it is still \
                        downloading, so try again in a moment"
                .into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// The MIME path of one attachment row, while the row id still means
/// something.
async fn part_path(
    database: &Store,
    message: MessageId,
    attachment: postio_model::ids::AttachmentId,
) -> Result<Option<String>, String> {
    Ok(read_message(database, message)
        .await?
        .attachments
        .iter()
        .find(|part| part.id == attachment)
        .and_then(|part| part.part_id.clone()))
}

/// Whether `part_id`'s bytes are on this machine, and in which shape.
///
/// The part's own blob first: it is the exact bytes, and reading it costs a
/// file open where the raw message costs a parse of the whole thing.
async fn locate_part(
    database: &Store,
    message: MessageId,
    part_id: &str,
) -> Result<Option<PartSource>, String> {
    let row = read_message(database, message).await?;
    if let Some(blob) = row
        .attachments
        .iter()
        .find(|part| part.part_id.as_deref() == Some(part_id))
        .and_then(|part| part.blob_id.clone())
    {
        return Ok(Some(PartSource::Payload(blob)));
    }
    Ok(row.raw_blob_id.map(PartSource::Raw))
}

/// One message row, or the sentence for its absence.
async fn read_message(
    database: &Store,
    message: MessageId,
) -> Result<postio_model::Message, String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| error.to_string())?;
    MessageRepository::new(&connection)
        .get(message)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "That message is no longer here".into())
}
