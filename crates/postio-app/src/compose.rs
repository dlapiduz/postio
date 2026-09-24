//! Wires storage into the composer's seams.
//!
//! `postio-gtk::composer` builds a widget that edits a [`Draft`] and calls
//! back through a handful of seams — `connect_save`, `connect_recipient_
//! suggestions`, `connect_reply_source` — without knowing anything persists
//! it. This is the other half: the composition root reads and writes
//! `postio-storage` directly, because it is the one crate allowed to.
//!
//! # Why these reads and writes are synchronous
//!
//! `postio_runtime::store::MailStore` (see `feed.rs`) crosses onto a tokio
//! worker for every read, because a message-list page can be a genuine scan
//! across a mailbox and the GTK main thread must never wait on that. Nothing
//! here is that: an autosave is one row, a recipient search is one indexed
//! query, and a reply-source lookup is one message by id. `main.rs`'s own
//! `first_account` already reads the database directly and synchronously for
//! the same reason — a small, bounded, local read is not what the `MailStore`
//! crossing exists to protect against. Attachments are the exception: a
//! file's bytes can be large enough to actually cost wall-clock time, so
//! [`install_attach`] hands the blob-store write to `runtime` and answers
//! through the callback [`Composer::connect_attach`] gives it whenever the
//! write finishes, rather than reading the file inline the way everything
//! else here does.
//!
//! # Carrying the draft's id forward
//!
//! [`Composer::connect_save`] hands its handler `&mut Draft` for exactly one
//! reason: `DraftRepository::save` is idempotent on `Draft::id`, inserting
//! once and updating forever after, and the composer has to learn whatever id
//! the first save assigned or every later autosave would insert a second row.
//! `Composer::save` writes that id back onto its own draft; this module keeps
//! its own record of the same id only for the one thing the composer cannot
//! tell it after the fact — which row to delete when the draft is dropped.

use gtk::glib;
use std::cell::Cell;
use std::rc::Rc;

use chrono::Utc;
use gtk::gio;
use gtk::prelude::*;
use postio_gtk::composer::{Closing, Composer, RecipientCandidate};
use postio_gtk::window::Window;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::signature_default;
use postio_model::{Attachment, Draft, DraftId, DraftState, EmailAddress, OperationTarget};
use postio_storage::repository::{
    AccountRepository, CancelSendOutcome, ContactGroupRepository, ContactRepository,
    DraftRepository, MailboxRepository, MessageRepository, OperationQueueRepository,
};
use postio_storage::{BlobStore, Store};

/// How many recipient suggestions to offer at once — a popover, not a list
/// the user scrolls.
const SUGGESTION_LIMIT: u32 = 8;

/// Wires `window`'s composer to `database` for `account`: autosave with
/// crash recovery, recipient completion from contacts, replying to whatever
/// the reading pane is showing, and attaching files into `blobs`. `runtime`
/// is only for [`install_attach`] — everything else here is synchronous.
///
/// `showing` is the reading pane's own record of which message is on screen
/// ([`crate::reading::Showing`]), which is what `e`, `E` and `f` have to act
/// on. It is passed in rather than derived here for the reason #325 records.
/// How the composer tells the rest of the window something happened.
///
/// A callback rather than the `Feeds` themselves: what this module needs is
/// "say so", and handing it the panes would let it reach into them. It is also
/// what lets a composer test run without building a message list to ignore.
pub type Announce = Rc<dyn Fn(&postio_core::Event)>;

pub async fn install(
    window: &Window,
    account: AccountId,
    database: Store,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
    showing: crate::reading::Showing,
    announce: Announce,
) {
    let composer = window.composer();
    composer.set_account(account);
    install_mailto(window, &composer, account);
    install_identities(window, &composer, &database, account).await;
    install_signature_default(&composer, window, database.clone(), account, &runtime);

    let writer = DraftWriter::spawn(database.clone(), account, &runtime);
    let last_id = install_autosave(&composer, database.clone(), account, &writer);
    install_send(&composer, &writer, Rc::clone(&last_id), account, announce);
    install_send_later(&composer, &writer, Rc::clone(&last_id));
    install_resume(window, &composer, database.clone(), last_id);
    install_recipient_suggestions(&composer, database.clone(), account).await;
    install_reply_source(&composer, database, showing, &runtime);
    install_attach(&composer, blobs.clone(), runtime.clone());
    install_inline_image(&composer, blobs.clone(), runtime).await;
    install_attachment_bytes(&composer, blobs);
}

/// A `mailto:` link opens the composer on a draft for `account`.
///
/// The link arrives at the window (`postio_gtk::app` hands every URI the
/// desktop passes to `Window::deliver_mailto`), and this is the half that
/// knows which account a new message is from. Connecting here is also what
/// releases a link that arrived *before* the store was fed — a cold launch
/// from a browser — which the window holds until somebody can act on it.
///
/// `Composer::open`, not `resume`: one composition at a time is the
/// composer's rule, so a link arriving mid-composition puts the keyboard
/// back in the draft already open rather than replacing what was typed.
/// That is said in the log, at info, because from the browser's side the
/// click did nothing.
fn install_mailto(window: &Window, composer: &Composer, account: AccountId) {
    let composer = composer.clone();
    window.connect_mailto(move |mailto| {
        if composer.is_open() {
            tracing::info!("a mailto link arrived while a composition was open; kept the open one");
        }
        composer.open(mailto.into_draft(account));
    });
}

/// Writes pasted image bytes into `blobs` and mints a `Content-ID` for the
/// inline attachment, off the main thread like [`install_attach`].
///
/// The id is the blob digest at `postio.invalid` — unique by construction
/// (same bytes, same blob, same reference) and on a reserved domain, so it
/// can never collide with, or be mistaken for, anything real.
async fn install_inline_image(
    composer: &Composer,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
) {
    composer.connect_inline_image(move |bytes, mime_type, then| {
        let blobs = blobs.clone();
        let (sender, receiver) = async_channel::bounded(1);
        runtime.spawn(async move {
            let attachment = inline_attachment(&blobs, bytes, &mime_type);
            let _ = sender.send_blocking(attachment);
        });
        gtk::glib::spawn_future_local(async move {
            then(receiver.recv().await.ok().flatten());
        });
    });
}

/// Blocking half of [`install_inline_image`].
fn inline_attachment(blobs: &BlobStore, bytes: Vec<u8>, mime_type: &str) -> Option<Attachment> {
    let size = bytes.len() as u64;
    let blob_id = blobs
        .put(&bytes)
        .map_err(|error| tracing::warn!(%error, "could not store the pasted image"))
        .ok()?;

    let extension = mime_type.strip_prefix("image/").unwrap_or("png");
    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type, size);
    attachment.filename = Some(format!("inline-image.{extension}"));
    attachment.disposition = postio_model::attachment::Disposition::Inline;
    attachment.content_id = Some(format!("{}@postio.invalid", blob_id.as_str()));
    attachment.blob_id = Some(blob_id);
    Some(attachment)
}

/// Resolves an attachment's bytes for the composer's inline-image display.
///
/// Synchronous, as the scheme handler requires; a blob read is a local file
/// open, the same cost the reader already pays per inline image.
fn install_attachment_bytes(composer: &Composer, blobs: BlobStore) {
    composer.connect_attachment_bytes(move |attachment| {
        let blob_id = attachment.blob_id.as_ref()?;
        let mut file = blobs
            .reader(blob_id)
            .map_err(|error| tracing::warn!(%error, "could not read an inline image blob"))
            .ok()?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut bytes)
            .map_err(|error| tracing::warn!(%error, "could not read an inline image blob"))
            .ok()?;
        Some(bytes)
    });
}

/// Puts the account's sending identities and named signatures in front of the
/// user (#12).
///
/// Read once at startup rather than watched: both change only when the
/// account is edited, which goes through the settings panel and is rare
/// enough that a restart is a fair price — where getting it wrong means the
/// composer offering an address the account no longer has.
///
/// Nothing called `set_identities` before this, so the picker had been built,
/// tested and shown with an empty model since it was written: every draft
/// signed with whatever `apply_identity` found on an account of none, which
/// is nothing.
async fn install_identities(
    window: &Window,
    composer: &Composer,
    database: &Store,
    account: AccountId,
) {
    let Ok(connection) = database.connect().await else {
        return;
    };
    match AccountRepository::new(&connection).get(account).await {
        Ok(Some(account)) => {
            // The conversation pane needs the same fact for a different
            // reason: it marks the user's own messages with an outline
            // rather than a fill (#1241). One read of the account row
            // answers both.
            let addresses: Vec<_> = account
                .identities
                .iter()
                .map(|identity| identity.address.clone())
                .collect();
            window.conversation().set_own_addresses(&addresses);
            composer.set_size_limit(account.max_message_size);
            composer.set_identities(account.identities);
            composer.set_signatures(account.signatures);
        }
        Ok(None) => tracing::warn!("the composer's account is not in the database"),
        Err(error) => tracing::warn!(%error, "could not read the account's identities"),
    }
}

/// Puts a resolved default in front of a brand-new draft, before the
/// identity's own (#12's last item, #394): a mailbox's own signature
/// overrides the account's default, which overrides the identity's.
///
/// Read fresh on every compose rather than once at startup like
/// [`install_identities`] — the sidebar selection this depends on changes on
/// every click, where the account's identities and named signatures change
/// only through the settings panel.
fn install_signature_default(
    composer: &Composer,
    window: &Window,
    database: Store,
    account: AccountId,
    runtime: &tokio::runtime::Handle,
) {
    let sidebar = window.sidebar();
    let runtime = runtime.clone();
    composer.connect_signature_default(move |answer| {
        // Which mailbox is selected is the sidebar's, read here; the two
        // store reads run on the runtime and the answer lands back on the
        // main context (#1608).
        let selected = sidebar.selected();
        let database = database.clone();
        let (sender, resolved) = async_channel::bounded(1);
        runtime.spawn(async move {
            let _ = sender
                .send(default_signature(&database, account, selected).await)
                .await;
        });
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads ran on the runtime.
            answer(resolved.recv().await.ok().flatten());
        });
    });
}

/// The signature a new draft for `account` starts with: the selected
/// mailbox's override, else the account's default, else none.
async fn default_signature(
    database: &Store,
    account: AccountId,
    selected: Option<MailboxId>,
) -> Option<postio_model::SignatureId> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not resolve a default signature"))
        .ok()?;
    let account_default = AccountRepository::new(&connection)
        .get(account)
        .await
        .ok()
        .flatten()?
        .default_signature_id;
    // Spelled out rather than chained: `and_then` takes a closure, and a
    // closure cannot await.
    let mailbox_signature = match selected {
        Some(id) => MailboxRepository::new(&connection)
            .get(id)
            .await
            .ok()
            .flatten()
            .and_then(|mailbox| mailbox.signature_id),
        None => None,
    };
    signature_default::resolve(mailbox_signature, account_default)
}

/// Activating a draft's row in the Drafts folder opens it in the composer.
///
/// # Why activation and not the cursor
///
/// The reading pane follows the cursor — `j` over a row previews it, and
/// nothing waits for Return (#70). Taking the pane away from the reader every
/// time the cursor crossed a draft would make scrolling through the Drafts
/// folder open and close the composer under the user. So the cursor previews
/// and Return opens, which is what Return means on every other row too.
///
/// # Why the reader is the wrong answer here
///
/// A draft's row is a snapshot of a buffer the composer owns, and the reader
/// cannot edit it. Before #166 a draft's row could only ever be that snapshot
/// — a dead end with a signpost. The row now leads back to the draft, so it
/// leads to the thing that can actually be done with it.
///
/// A row with no local draft behind it is another client's draft. It still
/// opens in the reader — there is no buffer to resume, and adopting somebody
/// else's draft into one is a decision with its own questions (what becomes
/// of their server copy? whose autosave wins?) that #175 chose to leave
/// unopened for v1 rather than resolve as a side effect of this path. What
/// changed under #175 is that the reader no longer pretends it is an
/// ordinary, readable message: [`load_body_or_reason`](postio_session::reading::load_body_or_reason) recognises `\Draft`
/// with no local buffer and reports [`postio_gtk::reader::Absent::ForeignDraft`]
/// instead, whatever the body's own download state is. See
/// `docs/engineering-notes.md`.
/// What the composer says once opening a queued draft has cancelled its
/// pending send (#433).
const SEND_CANCELLED: &str = "send cancelled — you're editing this draft again";

fn install_resume(
    window: &Window,
    composer: &Composer,
    database: Store,
    last_id: Rc<Cell<Option<DraftId>>>,
) {
    postio_session::blocking::now(async {
        // Weak: the window owns the list that owns this handler (#1072).
        let weak = glib::object::ObjectExt::downgrade(window);
        window.list().connect_activated({
            let composer = composer.clone();
            move |row| {
                postio_session::blocking::now(async {
                    if row.send_state.is_none() {
                        return;
                    }
                    let Some(window) = weak.upgrade() else {
                        return;
                    };
                    let Some(draft) = draft_behind(&database, row.id).await else {
                        return;
                    };
                    let draft = if draft.state == DraftState::Queued {
                        // #433: the row stays in the Drafts folder for as long as the
                        // send sits in the queue, and opening it here used to reopen
                        // it live for editing while the drainer could pick the same
                        // row up at any moment — an edit landed or did not, purely on
                        // timing. Cancelling the send is what makes editing it again
                        // safe: see `DraftRepository::cancel_send`.
                        let Some(reopened) = cancel_queued_send(&database, draft.id).await else {
                            return;
                        };
                        window.show_action_completed(SEND_CANCELLED, false);
                        reopened
                    } else {
                        draft
                    };
                    // FR-066's third clause, and #1487: a failed send has to name
                    // what went wrong. The reason was computed, written to the queue
                    // row and carried all the way up the engine's report -- whose own
                    // doc says "the reason the user should see" -- and then read by
                    // nobody. Said here because this is where the person has come
                    // back to do something about it.
                    // Spelled out rather than chained: `then` takes a
                    // closure, and a closure cannot await.
                    let failure = if draft.state == DraftState::Failed {
                        why_the_send_failed(&database, draft.id).await
                    } else {
                        None
                    };

                    // So that closing it empty clears the right row: `connect_closed`
                    // carries what became of the draft and not which one it was.
                    last_id.set(Some(draft.id));
                    composer.resume(draft);
                    if let Some(reason) = failure {
                        composer.set_status(&format!("Not sent — {reason}"));
                    }
                })
            }
        });
    })
}

/// Cancels a queued draft's pending send and returns it as it now stands, so
/// the caller can resume the composer on live state rather than the stale
/// `Queued` snapshot it read before cancelling.
///
/// `None` when there is nothing safe to resume: the send already drained,
/// started draining, or the draft is gone — [`DraftRepository::cancel_send`]'s
/// non-[`CancelSendOutcome::Cancelled`] outcomes. Opening the composer on a
/// draft mid-send would risk a second, different message going out behind
/// the one already on the wire, so this declines rather than guessing.
async fn cancel_queued_send(database: &Store, id: DraftId) -> Option<Draft> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to cancel a send"))
        .ok()?;
    let drafts = DraftRepository::new(&connection);
    match drafts.cancel_send(id, Utc::now()).await {
        Ok(CancelSendOutcome::Cancelled) => drafts
            .get(id)
            .await
            .map_err(|error| tracing::warn!(%error, "could not reread a draft after cancelling its send"))
            .ok()
            .flatten(),
        Ok(CancelSendOutcome::NotQueued | CancelSendOutcome::AlreadyInFlight) => None,
        Err(error) => {
            tracing::warn!(%error, "could not cancel a queued draft's send");
            None
        }
    }
}

/// Why this draft's last send attempt gave up, if it did.
///
/// Read from the queue row rather than from the draft, because that is where
/// the drainer writes it and a second copy is one that can come to disagree
/// with the first (#1487).
async fn why_the_send_failed(database: &Store, id: DraftId) -> Option<String> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to read a send failure"))
        .ok()?;
    OperationQueueRepository::new(&connection)
        .last_failure_for(OperationTarget::Draft(id))
        .await
        .map_err(|error| tracing::warn!(%error, "could not read why a send failed"))
        .ok()
        .flatten()
}

/// The draft a message row is listing, if it is listing one.
async fn draft_behind(database: &Store, message: MessageId) -> Option<Draft> {
    let connection = database
        .connect()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open the store to resume a draft"))
        .ok()?;
    DraftRepository::new(&connection)
        .by_message(message)
        .await
        .map_err(|error| tracing::warn!(%error, "could not read the draft behind a row"))
        .ok()?
}

/// What the composer asks of the store, in the order it asked (#1608).
enum DraftOp {
    /// Autosave this composition's draft.
    Save { generation: u64, draft: Draft },
    /// Queue it to send -- now, or at `at` -- and answer with the Drafts
    /// folder, whose list just changed, when the queue write landed.
    Send {
        generation: u64,
        draft: Draft,
        at: Option<chrono::DateTime<Utc>>,
        reply: async_channel::Sender<Option<MailboxId>>,
    },
    /// This composition was closed empty: its autosaved row goes.
    Discard {
        generation: u64,
        known: Option<DraftId>,
    },
}

/// Writes a composer's drafts on the runtime, one request at a time, in the
/// order the composer made them (#1608).
///
/// Every save, send and discard used to run under `blocking::now` on the GTK
/// thread, each waiting for the interactive write permit -- which lets the
/// background unit already holding the writer finish first -- so an autosave
/// tick could stall the thread that draws for as long as a sync's unit took.
/// Here the composer hands the request over and returns.
///
/// What the synchronous version guaranteed by construction has to be kept by
/// hand, and it is kept by order and by remembering one id. A draft's first
/// save assigns its id; the writer remembers it for that composition, so a
/// second save made before the first landed updates the same row rather than
/// inserting another, and a send queues the row the saves made. A discard is
/// behind every save its composition made, so it deletes the row they left.
#[derive(Clone)]
struct DraftWriter {
    ops: async_channel::Sender<DraftOp>,
    /// Each save that landed: which composition, and the id it has.
    saved: async_channel::Receiver<(u64, DraftId)>,
}

impl DraftWriter {
    fn spawn(database: Store, account: AccountId, runtime: &tokio::runtime::Handle) -> Self {
        let (ops, requests) = async_channel::unbounded::<DraftOp>();
        let (report, saved) = async_channel::unbounded();
        runtime.spawn(async move {
            // The composition whose draft was saved last, and its id.
            let mut current: Option<(u64, DraftId)> = None;
            let id_for = |current: &Option<(u64, DraftId)>, generation: u64, draft: &mut Draft| {
                if !draft.id.is_assigned()
                    && let Some((saved_for, id)) = *current
                    && saved_for == generation
                {
                    draft.id = id;
                }
            };
            while let Ok(op) = requests.recv().await {
                match op {
                    DraftOp::Save {
                        generation,
                        mut draft,
                    } => {
                        id_for(&current, generation, &mut draft);
                        match save_draft(&database, &mut draft).await {
                            Ok(()) => {
                                current = Some((generation, draft.id));
                                let _ = report.send((generation, draft.id)).await;
                            }
                            Err(error) => {
                                tracing::error!(%error, "could not autosave the draft: {error}")
                            }
                        }
                    }
                    DraftOp::Send {
                        generation,
                        mut draft,
                        at,
                        reply,
                    } => {
                        id_for(&current, generation, &mut draft);
                        // Taken on failure too, and deliberately: a queue write
                        // that fails leaves the autosaved row where it is,
                        // `Editing`, recoverable, and the close that follows a
                        // send must not delete the user's words on the way out.
                        // Losing the send is recoverable, losing the message is
                        // not.
                        if current.is_some_and(|(saved_for, _)| saved_for == generation) {
                            current = None;
                        }
                        let queued = match at {
                            Some(at) => queue_send_at(&database, &mut draft, at).await,
                            None => queue_send(&database, &mut draft).await,
                        };
                        let moved = match queued {
                            Ok(()) if at.is_none() => drafts_mailbox(&database, account).await,
                            Ok(()) => None,
                            Err(error) => {
                                tracing::error!(%error, "could not queue the draft for sending: {error}");
                                None
                            }
                        };
                        let _ = reply.send(moved).await;
                    }
                    DraftOp::Discard { generation, known } => {
                        let id = match current {
                            Some((saved_for, id)) if saved_for == generation => {
                                current = None;
                                Some(id)
                            }
                            _ => known,
                        };
                        if let Some(id) = id
                            && let Err(error) = delete_draft(&database, id).await
                        {
                            tracing::warn!(%error, "could not clear the finished draft");
                        }
                    }
                }
            }
        });
        DraftWriter { ops, saved }
    }

    fn send(&self, op: DraftOp) {
        // Unbounded: the composer never waits on the writer, which is the
        // point. A closed channel means the runtime is gone with the app.
        let _ = self.ops.try_send(op);
    }
}

/// Autosave to [`DraftRepository`], crash recovery, and clearing the row once
/// there is nothing left to keep — sent, discarded, or closed empty.
fn install_autosave(
    composer: &Composer,
    database: Store,
    account: AccountId,
    writer: &DraftWriter,
) -> Rc<Cell<Option<DraftId>>> {
    postio_session::blocking::now(async {
        // The id of whatever `connect_save`'s handler last persisted. Not read
        // from the composer's own draft afterward because `connect_closed` does
        // not carry the draft — only what became of it — so this is the one
        // piece of bookkeeping this module has to keep for itself.
        let last_id: Rc<Cell<Option<DraftId>>> = Rc::new(Cell::new(None));

        // Off the GTK thread (#1608): a save is handed to the writer and the
        // handler returns. The id a first save assigns comes back over
        // `saved` and is written onto the composer -- only while the same
        // composition is in its fields -- and into `last_id`.
        let weak = composer.downgrade();
        composer.connect_save({
            let writer = writer.clone();
            let weak = weak.clone();
            move |draft| {
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                writer.send(DraftOp::Save {
                    generation: composer.generation(),
                    draft: draft.clone(),
                });
            }
        });
        glib::spawn_future_local({
            let saved = writer.saved.clone();
            let last_id = Rc::clone(&last_id);
            let weak = weak.clone();
            async move {
                // POSTIO-GLIB-SAFE: a channel receive; the writes run on the
                // runtime in `DraftWriter`.
                while let Ok((generation, id)) = saved.recv().await {
                    let Some(composer) = weak.upgrade() else {
                        return;
                    };
                    if composer.generation() == generation {
                        last_id.set(Some(id));
                    }
                    composer.adopt_id(generation, id);
                }
            }
        });

        composer.connect_closed({
            let writer = writer.clone();
            let last_id = Rc::clone(&last_id);
            move |outcome| {
                // Kept: Esc with something still in it. The row stays exactly as
                // autosaved, ready to recover it right back.
                if outcome != Closing::Drop {
                    return;
                }
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                // The composition just closed, not the empty one the close
                // refilled the fields with -- and after every save it handed
                // out, because the writer takes them in order.
                writer.send(DraftOp::Discard {
                    generation: composer.previous_generation(),
                    known: last_id.take(),
                });
            }
        });

        // Only after a crash. `DraftState::Editing` alone is not evidence of
        // one — Esc parks a draft in exactly that state on purpose — and the
        // difference is the whole of #491: a client that opens into a stale
        // compose buffer instead of the inbox reads as broken. `begin_session`
        // is what knows how the last session ended, and this is its one caller,
        // before anything else consults the marker it flips.
        if postio_session::begin_session(&database).await {
            recover(composer, &database, account, &last_id).await;
        }
        last_id
    })
}

/// Autosave: the local row, and the queue row that carries it to the account's
/// Drafts mailbox, in one write.
///
/// The enqueue is what makes a draft survive more than this machine — see
/// `DraftRepository::save_and_sync`. It costs nothing extra here: the queue
/// row is written inside the same transaction, and the engine sends it when
/// there is a connection. A run of autosaves folds into one upload.
///
/// `interactive_write` rather than a bare connection: a draft autosave is a
/// write the person typing is waiting on, so it goes ahead of a backfill's
/// bulk writes rather than queueing behind them (#425).
async fn save_draft(database: &Store, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .save_and_sync(draft, Utc::now())
        .await?;
    Ok(())
}

/// Discard: the local row goes now, and the server copy is queued for removal.
async fn delete_draft(database: &Store, id: DraftId) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .discard(id, Utc::now())
        .await?;
    Ok(())
}

/// Sending: the draft becomes a queue row, and stops being the composer's.
///
/// This is the seam #423 was about. `Composer::connect_send` had no caller
/// anywhere in the workspace from the composer's first commit, so
/// `Composer::send` found its handler list empty on every press of
/// `ctrl+Return` and said so in wording that read like a misconfigured
/// account. No message had ever been sendable through the UI.
///
/// Nothing here waits for SMTP, and nothing here opens a connection: the
/// write is one local transaction, and `postio-sync::send` drains the row it
/// leaves whenever there is a network. That is the same local-first rule the
/// autosave beside it follows, and it is what lets the composer close the
/// instant the key is pressed.
///
/// # Why this clears `last_id`
///
/// `Composer::send` closes with [`Closing::Drop`], and the close handler
/// [`install_autosave`] registered discards whatever `last_id` is holding —
/// which is precisely the draft just queued. Left alone, the local row would
/// be deleted a moment after the enqueue, and `postio-sync::send` resolves a
/// `Send` whose draft is gone as obsolete: the message would vanish rather
/// than be sent. Taking the id here is what tells the close path that this
/// draft has already been dealt with.
///
/// It is taken on failure too, and deliberately. A queue write that fails
/// leaves the autosaved row where it is, `Editing`, listed in the Drafts
/// folder and recoverable; letting the close path run instead would delete
/// the user's words on the way out. Losing the send is recoverable, losing
/// the message is not.
fn install_send(
    composer: &Composer,
    writer: &DraftWriter,
    last_id: Rc<Cell<Option<DraftId>>>,
    account: AccountId,
    announce: Announce,
) {
    let weak = composer.downgrade();
    let writer = writer.clone();
    composer.connect_send(move |draft| {
        let Some(composer) = weak.upgrade() else {
            return;
        };
        last_id.set(None);
        // Through the writer, after every save this composition handed it:
        // a send racing a first save in flight would insert a second row.
        // The writer queues the send and says which folder moved; the
        // announcement is made here, where the list lives (#1608).
        let (reply, answer) = async_channel::bounded(1);
        writer.send(DraftOp::Send {
            generation: composer.generation(),
            draft: draft.clone(),
            at: None,
            reply,
        });
        let announce = announce.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the write ran on the runtime.
            let Ok(Some(drafts)) = answer.recv().await else {
                return;
            };
            // Say so, or the write is invisible until something else redraws.
            //
            // This is the last step of the local-first order -- write, enqueue,
            // emit, repaint. `MessageListChanged` rather than a state-change
            // event of its own: what happened *is* a list membership change, in
            // both directions at once -- the row leaves Drafts and joins the
            // Outbox -- and both scopes already answer `Reload` to it.
            announce(&postio_core::Event::MessageListChanged {
                account,
                mailbox: drafts,
            });
        });
    });
}

/// The account's Drafts folder, which is where a draft's row lives whatever
/// its send state — the Outbox is a predicate over that folder, not a second
/// one (spec 003).
///
/// `None` before the first sync has found one, in which case the draft has no
/// row to have moved and there is nothing to announce.
async fn drafts_mailbox(database: &Store, account: AccountId) -> Option<MailboxId> {
    let connection = database.connect().await.ok()?;
    postio_storage::repository::MailboxRepository::new(&connection)
        .by_role(account, postio_model::MailboxRole::Drafts)
        .await
        .ok()
        .flatten()
        .map(|mailbox| mailbox.id)
}

/// Send: the draft goes to `Queued` and its `Operation::Send` row is written,
/// in one transaction — see `DraftRepository::queue_send`.
async fn queue_send(database: &Store, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .queue_send(draft, Utc::now())
        .await?;
    Ok(())
}

/// [`install_send`]'s counterpart for [`Composer::connect_send_later`] — the
/// picker behind [`CommandId::ScheduleSend`](postio_core::CommandId::ScheduleSend).
///
/// Everything [`install_send`]'s own doc comment says about `last_id` and
/// about failing without a status line applies here unchanged: the composer
/// closes the instant a time is chosen, the same way it does for an
/// immediate send, so there is nothing on screen left to read a status line
/// from by the time a queue error could be reported.
fn install_send_later(
    composer: &Composer,
    writer: &DraftWriter,
    last_id: Rc<Cell<Option<DraftId>>>,
) {
    let weak = composer.downgrade();
    let writer = writer.clone();
    composer.connect_send_later(move |draft, send_at| {
        let Some(composer) = weak.upgrade() else {
            return;
        };
        last_id.set(None);
        let (reply, _answer) = async_channel::bounded(1);
        writer.send(DraftOp::Send {
            generation: composer.generation(),
            draft: draft.clone(),
            at: Some(send_at),
            reply,
        });
    });
}

/// Schedule send: the draft goes to `Queued` and its `Operation::Send` row is
/// written with `send_at` as the time the drainer must not touch it before —
/// see `DraftRepository::queue_send_at`.
async fn queue_send_at(
    database: &Store,
    draft: &mut Draft,
    send_at: chrono::DateTime<Utc>,
) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write().await?;
    DraftRepository::new(&connection)
        .queue_send_at(draft, Utc::now(), send_at)
        .await?;
    Ok(())
}

/// Opens whatever draft `account` was still editing when Postio last
/// stopped — the crash-recovery half of the bead, and the whole reason it
/// matters: a draft is not durable until it comes back on its own.
///
/// Called only when [`postio_session::begin_session`] says the last session
/// died uncleanly (#491). After a *clean* exit a mid-edit draft is parked,
/// not lost: autosaved, a row in Drafts, resumable from there — and the
/// next start belongs to the inbox.
///
/// The most recently edited one, since the composer holds exactly one draft
/// at a time (`postio-cj7`'s "one composition" invariant); a v1 with several
/// concurrent drafts would recover all of them into a real Drafts mailbox
/// instead, which does not exist yet.
async fn recover(
    composer: &Composer,
    database: &Store,
    account: AccountId,
    last_id: &Rc<Cell<Option<DraftId>>>,
) {
    let Ok(connection) = database.connect().await else {
        return;
    };
    let drafts = match DraftRepository::new(&connection)
        .list_for_account(account)
        .await
    {
        Ok(drafts) => drafts,
        Err(error) => {
            tracing::error!(%error, "could not read drafts to recover: {error}");
            return;
        }
    };
    drop(connection);

    // Worth recovering, by the same rule Esc uses. An untouched buffer is
    // not work -- and recovering one is self-perpetuating, because the
    // composer it reopens autosaves another `Editing` row for the empty
    // draft it is now holding, so every unclean stop after the first opens
    // the client into a stale compose buffer. That is the state #491's own
    // doc calls reading broken, arrived at by #491's own fix.
    //
    // `closing` rather than a second definition of empty: it is what decides
    // whether Esc parks a draft or drops it, and the two questions are the
    // same question. Whitespace and the signature do not count, per its rule.
    let Some(draft) = drafts.into_iter().find(|draft| {
        draft.state == DraftState::Editing
            && postio_gtk::composer::closing(draft) == postio_gtk::composer::Closing::Keep
    }) else {
        return;
    };
    last_id.set(Some(draft.id));
    composer.open(draft);
}

/// Recipient completion: contact groups whose name matches `prefix`, then
/// contacts ranked by [`ContactRepository::search`] — groups first, since a
/// group is a deliberate choice the user is more likely typing towards.
async fn install_recipient_suggestions(composer: &Composer, database: Store, account: AccountId) {
    composer.connect_recipient_suggestions(move |prefix| {
        postio_session::blocking::now(async {
            let connection = match database.connect().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!(%error, "could not search contacts");
                    return Vec::new();
                }
            };

            let mut candidates: Vec<RecipientCandidate> = Vec::new();
            let groups = ContactGroupRepository::new(&connection);
            match groups.list(Some(account)).await {
                Ok(list) => {
                    let prefix_lower = prefix.to_lowercase();
                    for group in list {
                        if !group.name.to_lowercase().starts_with(&prefix_lower) {
                            continue;
                        }
                        match groups.members(group.id).await {
                            // A group with no members yet expands to nothing, so
                            // offering it would be a suggestion that does nothing
                            // when accepted.
                            Ok(members) if !members.is_empty() => {
                                candidates.push(RecipientCandidate::Group {
                                    name: group.name,
                                    members: members.iter().map(resolved_address).collect(),
                                });
                            }
                            Ok(_) => {}
                            Err(error) => tracing::warn!(%error, "could not read group members"),
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "could not search contact groups"),
            }

            match ContactRepository::new(&connection)
                .search(Some(account), prefix, SUGGESTION_LIMIT)
                .await
            {
                Ok(contacts) => candidates.extend(
                    contacts
                        .iter()
                        .map(resolved_address)
                        .map(RecipientCandidate::Contact),
                ),
                Err(error) => tracing::warn!(%error, "could not search contacts"),
            }

            candidates.truncate(SUGGESTION_LIMIT as usize);
            candidates
        })
    });
}

/// The address a contact offers: the name the user set, or the last one seen
/// on the address, over the addr-spec `record` accumulated sightings under.
fn resolved_address(contact: &postio_model::Contact) -> EmailAddress {
    let name = contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone());
    EmailAddress::new(name, contact.address.address.clone())
}

/// `e`/`E`/`f` reply to whatever the reading pane is showing.
///
/// # Why not the list's own activation
///
/// It used to keep a `Cell` of its own, fed by `List::connect_activated` —
/// Enter, or a double click. Nobody reads mail that way here: the pane
/// follows the *cursor* (#70, Cause B), so a session spent moving with `j`
/// left that cell `None` from beginning to end and reply, reply-all and
/// forward were all inert, silently (#325). Two copies of "the current
/// message", updated by different signals, can only ever be one signal away
/// from disagreeing; reading `showing` is the version of this that has no
/// second copy to drift.
fn install_reply_source(
    composer: &Composer,
    database: Store,
    showing: crate::reading::Showing,
    runtime: &tokio::runtime::Handle,
) {
    let runtime = runtime.clone();
    composer.connect_reply_source(move |answer| {
        let Some(id) = showing.get() else {
            tracing::debug!("reply asked for with no message in the reading pane");
            answer(None);
            return;
        };
        // The message, its body and its account are read on the runtime and
        // the reply opens when they land (#1608): they were read on the GTK
        // thread, two connections and a body decode in front of the composer.
        let database = database.clone();
        let (sender, found) = async_channel::bounded(1);
        runtime.spawn(async move {
            let _ = sender.send(reply_source(&database, id).await).await;
        });
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads ran on the runtime.
            answer(found.recv().await.ok().flatten());
        });
    });
}

/// The message `id` with its body, and the account it belongs to.
async fn reply_source(
    database: &Store,
    id: MessageId,
) -> Option<(postio_model::Message, postio_model::Account)> {
    let connection = database
        .read()
        .await
        .map_err(|error| tracing::warn!(%error, "could not open a reply source"))
        .ok()?;
    let mut message = MessageRepository::new(&connection)
        .get(id)
        .await
        .ok()
        .flatten()?;
    message.body = load_body(&connection, id).await;
    let account = AccountRepository::new(&connection)
        .get(message.account_id)
        .await
        .ok()
        .flatten()?;
    Some((message, account))
}

/// Writes a chosen or dropped file into `blobs` without blocking the
/// composer on it. The read, the MIME sniff and the write are all blocking
/// calls, so they run on `runtime`'s blocking pool rather than an async
/// task — a worker thread costs nothing borrowed from anywhere else, where a
/// blocking call inside a tokio task would stall whatever else that task's
/// worker was meant to poll.
fn install_attach(composer: &Composer, blobs: BlobStore, runtime: tokio::runtime::Handle) {
    composer.connect_attach(move |path, then| {
        let blobs = blobs.clone();
        let (sender, receiver) = async_channel::bounded(1);
        runtime.spawn(async move {
            let attachment = attach_file(&blobs, &path);
            let _ = sender.send_blocking(attachment);
        });
        gtk::glib::spawn_future_local(async move {
            then(receiver.recv().await.ok().flatten());
        });
    });
}

/// Reads `path`'s size and MIME type and writes its bytes into `blobs`.
///
/// Blocking throughout, deliberately — see [`install_attach`], the only
/// place this is ever called from.
fn attach_file(blobs: &BlobStore, path: &std::path::Path) -> Option<Attachment> {
    let size = std::fs::metadata(path).ok()?.len();
    let mime_type = mime_type_of(path);
    let file = std::fs::File::open(path)
        // The path is deliberately not logged: an attachment's name is the
        // user's, and a log line is a thing people paste into bug reports.
        .map_err(|error| tracing::warn!(%error, "could not read the file to attach"))
        .ok()?;
    let blob_id = blobs
        .put_reader(file)
        .map_err(|error| tracing::warn!(%error, "could not store the attachment"))
        .ok()?;

    let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime_type, size);
    attachment.filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    attachment.blob_id = Some(blob_id);
    Some(attachment)
}

/// A best guess at `path`'s MIME type, from the same shared-mime-info
/// database a file manager reads — sniffed from content and extension
/// together, not just the extension. Falls back to the generic "some bytes"
/// type rather than failing the attachment over a type nothing recognises.
fn mime_type_of(path: &std::path::Path) -> String {
    gio::File::for_path(path)
        .query_info(
            "standard::content-type",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()
        .and_then(|info| info.content_type())
        .map(|content_type| content_type.to_string())
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

// `load_body`, `Body`, `load_body_or_reason` and `read_blob_text` moved to
// `postio_session::reading` (#608): the macOS frontend needs the same six-way
// answer about why a body is missing, and a second copy of it would reproduce
// #70's blank column rather than the fix.
pub(crate) use postio_session::reading::{Body, load_body};

#[cfg(test)]
mod tests {
    //! One test, and it is the point of the whole module: a draft
    //! autosaved before the process stops does not need the process to stop
    //! *cleanly* to come back.
    //!
    //! `postio-app` is a binary crate with no library target, so an
    //! integration test under `tests/` cannot link against `compose::install`
    //! at all — this has to be an inline `#[cfg(test)]` unit test in the same
    //! module, which is also why it is the *only* GTK-touching test in this
    //! crate: `adw::init()` and a display are process-wide state, and
    //! `cargo test` runs every unit test in one process unless told
    //! otherwise.
    //!
    //! **A second one is not a judgement call, it is a red run.** One was
    //! added, passed locally for weeks, and panicked the moment a runner
    //! gave both tests a display at once. A new GTK-touching scenario
    //! becomes another call from the one `#[test]` below, never another
    //! `#[test]`; `check-no-gtk-init-in-unit-tests.py` now enforces the
    //! single init this file is allowed.
    //!
    //! POSTIO-GTK-INIT: the paragraph above is the argument. A binary crate
    //! has nothing for `tests/` to link against, so this one cannot move out
    //! the way `postio-gtk`'s toast tests did. See issue #41 and
    //! `scripts/checks/check-no-gtk-init-in-unit-tests.py`.
    use postio_session::reading::load_body_or_reason;

    use gtk::gdk;

    use super::*;

    fn settle() {
        while gtk::glib::MainContext::default().iteration(false) {}
    }

    /// A real account row, since `DraftRepository::save`'s first insert
    /// requires one to reference.
    async fn seed_account(database: &Store) -> AccountId {
        let connection = database.connect().await.unwrap();
        let mut account = postio_model::Account::new(
            "Test",
            EmailAddress::new(None::<String>, "ada@example.com"),
        );
        AccountRepository::new(&connection)
            .create(&mut account)
            .await
            .unwrap();
        account.id
    }

    // ── `load_body_or_reason` ────────────────────────────────────────────
    //
    // Pure data: no display, no `adw::init()`. See the module doc above for
    // why a GTK-touching test does not belong beside these.

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_with_no_body_yet_names_offline_only_when_the_engine_is() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::HeadersOnly;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        assert!(
            matches!(
                load_body_or_reason(&connection, id, false).await,
                Body::Absent(postio_gtk::reader::Absent::Partial)
            ),
            "online and not yet fetched is the ordinary backfill wait"
        );
        assert!(
            matches!(
                load_body_or_reason(&connection, id, true).await,
                Body::Absent(postio_gtk::reader::Absent::Offline)
            ),
            "offline and not yet fetched has to say so, not promise a backfill \
             that cannot run"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_whose_body_already_arrived_ignores_whether_the_engine_is_offline() {
        // Offline-ness only changes the story for a body that has not landed
        // yet. A message with real bytes on disk must read the same whether
        // or not the engine happens to be connected right now.
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        // No blobs were ever named for it, so this is the "fetched, naming
        // no blobs" case -- `Absent::Empty` -- either way.
        assert!(matches!(
            load_body_or_reason(&connection, id, false).await,
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
        assert!(matches!(
            load_body_or_reason(&connection, id, true).await,
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
    }

    /// #175: a draft's row with `\Draft` set but no local `Draft` buffer
    /// behind it (`DraftRepository::by_message` is `None`) was written by
    /// another client. Even once its body backfills, opening it must not
    /// look like an ordinary, readable message -- there is nothing here that
    /// can be edited, and pretending otherwise is the dead end #175 exists
    /// to close.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_foreign_drafts_row_says_so_even_once_its_body_has_arrived() {
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.flags.insert(postio_model::Flag::Draft);
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .unwrap();
        drop(connection);

        let connection = database.connect().await.unwrap();

        // No local `DraftRepository` row exists for this message, which is
        // exactly what makes it another client's draft rather than one this
        // machine is editing.
        assert!(matches!(
            load_body_or_reason(&connection, id, false).await,
            Body::Absent(postio_gtk::reader::Absent::ForeignDraft)
        ));
    }

    /// Both halves of #491's rule, in one `#[test]` because they cannot be
    /// two.
    ///
    /// They were two, briefly, and it cost a red CI run: `adw::init()` is
    /// process-wide and belongs to the thread that called it, `cargo test`
    /// gives every `#[test]` a thread of its own, and the second one to
    /// reach `init` panics with "Attempted to initialize GTK from two
    /// different threads". Locally the pair passed — whichever ran second
    /// found the display already gone and took the skip branch — so the
    /// race only ever showed on a runner. Two tests here cannot be made
    /// safe by ordering or a mutex: GTK objects belong to the initializing
    /// thread, so the second test has no thread it is allowed to use them
    /// from. One test, two scenarios, run in sequence, is the shape that
    /// works.
    ///
    /// The scenarios are twins — the same two runs of the app, differing
    /// only in whether the first one exited cleanly, which is precisely the
    /// question recovery has to answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn recovery_reopens_a_crashed_draft_and_leaves_a_parked_one_alone() {
        if !gui_ready() {
            eprintln!("skipping: no display (see scripts/test-headless.sh --status)");
            return;
        }

        a_crashed_session_reopens_its_draft().await;
        a_clean_exit_leaves_the_next_start_on_the_inbox().await;
    }

    /// `adw::init()` and the style/icon setup the scenarios share.
    ///
    /// Returns whether there is a display to draw on; see the caller for why
    /// exactly one test may call this.
    fn gui_ready() -> bool {
        if adw::init().is_err() || gdk::Display::default().is_none() {
            return false;
        }
        let display = gdk::Display::default().unwrap();
        postio_gtk::fonts::install().expect("the embedded fonts should install");
        postio_gtk::style::install(&display);
        postio_gtk::app::install_icons(&display);
        true
    }

    async fn a_clean_exit_leaves_the_next_start_on_the_inbox() {
        // #491, reported directly: "i reopened the app and it opened in a
        // compose window from a draft. cold start should start with the
        // inbox". `DraftState::Editing` is not evidence of a crash — Esc on
        // a draft with content parks it in exactly that state on purpose —
        // so recovery has to ask how the last session *ended*, not what the
        // drafts table holds.
        let state_dir_guard = tempfile::tempdir().expect("a state directory");
        let state_dir = state_dir_guard.path();
        // SAFETY: the scenarios run in sequence on one thread, and each
        // needs its own state directory — the clean-exit marker is the
        // variable under test, so they cannot share one. Nothing else in
        // this binary reads `XDG_STATE_HOME`.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", state_dir)
        };

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account = seed_account(
            &Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap(),
        )
        .await;
        // The ambient one: this test is a `#[tokio::test]`, and building a
        // second runtime inside one panics on drop.
        let runtime = tokio::runtime::Handle::current();

        // ── Run one: type, park the draft with Esc, exit cleanly ─────────
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            let composer = window.composer();
            composer.open(Draft::new(account));
            settle();
            composer.test_set_subject("Finish this on Thursday");
            settle();
            composer.save();
            // Esc: a deliberate close that keeps the row Editing, "ready to
            // recover it right back".
            composer.close();
            settle();
            // The orderly exit path `run()` takes after `application.run()`
            // returns.
            postio_session::end_session(&database).await;
        }

        // ── Run two: the draft is parked, not in the way ─────────────────
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            settle();

            assert!(
                !window.composer().is_open(),
                "a cleanly-exited session's parked draft must not take over                  the next start — the inbox is the first thing a mail client                  shows"
            );
            // Never lost: the row is still in Drafts, exactly as autosaved,
            // reachable through the Drafts folder's own resume path.
            let connection = database.connect().await.unwrap();
            let parked = DraftRepository::new(&connection)
                .list_for_account(account)
                .await
                .expect("drafts read");
            assert_eq!(parked.len(), 1);
            assert_eq!(parked[0].subject, "Finish this on Thursday");
            assert_eq!(parked[0].state, DraftState::Editing);
        }
    }

    async fn a_crashed_session_reopens_its_draft() {
        let state_dir_guard = tempfile::tempdir().expect("a state directory");
        let state_dir = state_dir_guard.path();
        // SAFETY: as above — sequential, and its own directory.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", state_dir)
        };

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account = seed_account(
            &Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap(),
        )
        .await;
        // Only `install_attach` ever spawns onto this; nothing in this test
        // attaches a file, so it exists purely to give `install` a handle.
        // The ambient one: this test is a `#[tokio::test]`, and building a
        // second runtime inside one panics on drop.
        let runtime = tokio::runtime::Handle::current();

        // ── Run one: open, type, autosave, and stop cold ─────────────────
        //
        // No `composer.close()`, no clean shutdown of anything — a crash
        // does not call those either. Going out of scope at the end of this
        // block is the whole simulation: the transaction `save()` already
        // committed is what has to survive it, not an orderly exit.
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            let composer = window.composer();
            composer.open(Draft::new(account));
            settle();
            composer.test_set_subject("Q3 numbers, one more time");
            settle();
            // The debounce is real product behaviour and is already proven
            // in `gtk_composer_autosave.rs`; calling `save()` directly here
            // keeps this test about recovery, not about timing.
            composer.save();
        }

        // ── Run two: a fresh window, a fresh database handle, same file ──
        {
            let database = Store::open(&db_path, &postio_storage::test_support::key())
                .await
                .unwrap();
            let blobs =
                BlobStore::open(&blobs_path, &postio_storage::test_support::blob_keys()).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.clone(),
                crate::reading::Showing::default(),
                // These tests are about the composer's own behaviour, not
                // about what the list does afterwards; nobody is listening.
                Rc::new(|_: &postio_core::Event| {}),
            )
            .await;
            settle();

            let composer = window.composer();
            assert!(
                composer.is_open(),
                "a recovered draft should be sitting in the reading pane, not waiting to be asked for"
            );
            assert_eq!(composer.draft().subject, "Q3 numbers, one more time");
        }
    }
}
