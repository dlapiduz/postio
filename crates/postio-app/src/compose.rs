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

use std::cell::Cell;
use std::rc::Rc;

use chrono::Utc;
use gtk::gio;
use gtk::prelude::*;
use postio_gtk::composer::{Closing, Composer, RecipientCandidate};
use postio_gtk::window::Window;
use postio_model::ids::{AccountId, MessageId};
use postio_model::signature_default;
use postio_model::{Attachment, Draft, DraftId, DraftState, EmailAddress};
use postio_storage::repository::{
    AccountRepository, CancelSendOutcome, ContactGroupRepository, ContactRepository,
    DraftRepository, MailboxRepository, MessageRepository,
};
use postio_storage::{BlobStore, Database};

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
pub fn install(
    window: &Window,
    account: AccountId,
    database: Database,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
    showing: crate::reading::Showing,
) {
    let composer = window.composer();
    composer.set_account(account);
    install_identities(&composer, &database, account);
    install_signature_default(&composer, window, database.clone(), account);

    let last_id = install_autosave(&composer, database.clone(), account);
    install_send(&composer, database.clone(), Rc::clone(&last_id));
    install_send_later(&composer, database.clone(), Rc::clone(&last_id));
    install_resume(window, &composer, database.clone(), last_id);
    install_recipient_suggestions(&composer, database.clone(), account);
    install_reply_source(&composer, database, showing);
    install_attach(&composer, blobs.clone(), runtime.clone());
    install_inline_image(&composer, blobs.clone(), runtime);
    install_attachment_bytes(&composer, blobs);
}

/// Writes pasted image bytes into `blobs` and mints a `Content-ID` for the
/// inline attachment, off the main thread like [`install_attach`].
///
/// The id is the blob digest at `postio.invalid` — unique by construction
/// (same bytes, same blob, same reference) and on a reserved domain, so it
/// can never collide with, or be mistaken for, anything real.
fn install_inline_image(composer: &Composer, blobs: BlobStore, runtime: tokio::runtime::Handle) {
    composer.connect_inline_image(move |bytes, mime_type, then| {
        let blobs = blobs.clone();
        let (sender, receiver) = async_channel::bounded(1);
        runtime.spawn_blocking(move || {
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
fn install_identities(composer: &Composer, database: &Database, account: AccountId) {
    let Ok(connection) = database.connection() else {
        return;
    };
    match AccountRepository::new(&connection).get(account) {
        Ok(Some(account)) => {
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
    database: Database,
    account: AccountId,
) {
    let sidebar = window.sidebar();
    composer.connect_signature_default(move || {
        let connection = database
            .connection()
            .map_err(|error| tracing::warn!(%error, "could not resolve a default signature"))
            .ok()?;
        let account_default = AccountRepository::new(&connection)
            .get(account)
            .ok()
            .flatten()?
            .default_signature_id;
        let mailbox_signature = sidebar
            .selected()
            .and_then(|id| MailboxRepository::new(&connection).get(id).ok().flatten())
            .and_then(|mailbox| mailbox.signature_id);
        signature_default::resolve(mailbox_signature, account_default)
    });
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
/// ordinary, readable message: [`load_body_or_reason`] recognises `\Draft`
/// with no local buffer and reports [`postio_gtk::reader::Absent::ForeignDraft`]
/// instead, whatever the body's own download state is. See
/// `docs/engineering-notes.md`.
/// What the composer says once opening a queued draft has cancelled its
/// pending send (#433).
const SEND_CANCELLED: &str = "send cancelled — you're editing this draft again";

fn install_resume(
    window: &Window,
    composer: &Composer,
    database: Database,
    last_id: Rc<Cell<Option<DraftId>>>,
) {
    window.list().connect_activated({
        let composer = composer.clone();
        let window = window.clone();
        move |row| {
            if !row.draft {
                return;
            }
            let Some(draft) = draft_behind(&database, row.id) else {
                return;
            };
            let draft = if draft.state == DraftState::Queued {
                // #433: the row stays in the Drafts folder for as long as the
                // send sits in the queue, and opening it here used to reopen
                // it live for editing while the drainer could pick the same
                // row up at any moment — an edit landed or did not, purely on
                // timing. Cancelling the send is what makes editing it again
                // safe: see `DraftRepository::cancel_send`.
                let Some(reopened) = cancel_queued_send(&database, draft.id) else {
                    return;
                };
                window.show_action_completed(SEND_CANCELLED, false);
                reopened
            } else {
                draft
            };
            // So that closing it empty clears the right row: `connect_closed`
            // carries what became of the draft and not which one it was.
            last_id.set(Some(draft.id));
            composer.resume(draft);
        }
    });
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
fn cancel_queued_send(database: &Database, id: DraftId) -> Option<Draft> {
    let connection = database
        .connection()
        .map_err(|error| tracing::warn!(%error, "could not open the store to cancel a send"))
        .ok()?;
    let drafts = DraftRepository::new(&connection);
    match drafts.cancel_send(id, Utc::now()) {
        Ok(CancelSendOutcome::Cancelled) => drafts
            .get(id)
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

/// The draft a message row is listing, if it is listing one.
fn draft_behind(database: &Database, message: MessageId) -> Option<Draft> {
    let connection = database
        .connection()
        .map_err(|error| tracing::warn!(%error, "could not open the store to resume a draft"))
        .ok()?;
    DraftRepository::new(&connection)
        .by_message(message)
        .map_err(|error| tracing::warn!(%error, "could not read the draft behind a row"))
        .ok()?
}

/// Autosave to [`DraftRepository`], crash recovery, and clearing the row once
/// there is nothing left to keep — sent, discarded, or closed empty.
fn install_autosave(
    composer: &Composer,
    database: Database,
    account: AccountId,
) -> Rc<Cell<Option<DraftId>>> {
    // The id of whatever `connect_save`'s handler last persisted. Not read
    // from the composer's own draft afterward because `connect_closed` does
    // not carry the draft — only what became of it — so this is the one
    // piece of bookkeeping this module has to keep for itself.
    let last_id: Rc<Cell<Option<DraftId>>> = Rc::new(Cell::new(None));

    composer.connect_save({
        let database = database.clone();
        let last_id = Rc::clone(&last_id);
        move |draft| match save_draft(&database, draft) {
            Ok(()) => last_id.set(Some(draft.id)),
            Err(error) => tracing::error!(%error, "could not autosave the draft"),
        }
    });

    composer.connect_closed({
        let database = database.clone();
        let last_id = Rc::clone(&last_id);
        move |outcome| {
            // Kept: Esc with something still in it. The row stays exactly as
            // autosaved, ready to recover it right back.
            if outcome != Closing::Drop {
                return;
            }
            let Some(id) = last_id.take() else {
                return;
            };
            if let Err(error) = delete_draft(&database, id) {
                tracing::warn!(%error, "could not clear the finished draft");
            }
        }
    });

    // Only after a crash. `DraftState::Editing` alone is not evidence of
    // one — Esc parks a draft in exactly that state on purpose — and the
    // difference is the whole of #491: a client that opens into a stale
    // compose buffer instead of the inbox reads as broken. `begin_session`
    // is what knows how the last session ended, and this is its one caller,
    // before anything else consults the marker it flips.
    if postio_session::begin_session(&database) {
        recover(composer, &database, account, &last_id);
    }
    last_id
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
fn save_draft(database: &Database, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write()?;
    DraftRepository::new(&connection).save_and_sync(draft, Utc::now())?;
    Ok(())
}

/// Discard: the local row goes now, and the server copy is queued for removal.
fn delete_draft(database: &Database, id: DraftId) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write()?;
    DraftRepository::new(&connection).discard(id, Utc::now())?;
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
fn install_send(composer: &Composer, database: Database, last_id: Rc<Cell<Option<DraftId>>>) {
    composer.connect_send(move |draft| {
        // Cloned because the seam hands out `&Draft`: unlike a save, which
        // writes the assigned id back onto the composer's own draft, nothing
        // survives this — the composer is about to be refilled and closed.
        let mut draft = draft.clone();
        last_id.set(None);
        if let Err(error) = queue_send(&database, &mut draft) {
            // The draft is still in the store, unsent and unqueued. Not a
            // status line: `Composer::send` closes straight after this, so
            // there is nothing on screen left to read it.
            tracing::error!(%error, "could not queue the draft for sending");
        }
    });
}

/// Send: the draft goes to `Queued` and its `Operation::Send` row is written,
/// in one transaction — see `DraftRepository::queue_send`.
fn queue_send(database: &Database, draft: &mut Draft) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write()?;
    DraftRepository::new(&connection).queue_send(draft, Utc::now())?;
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
fn install_send_later(composer: &Composer, database: Database, last_id: Rc<Cell<Option<DraftId>>>) {
    composer.connect_send_later(move |draft, send_at| {
        let mut draft = draft.clone();
        last_id.set(None);
        if let Err(error) = queue_send_at(&database, &mut draft, send_at) {
            tracing::error!(%error, "could not schedule the draft for sending");
        }
    });
}

/// Schedule send: the draft goes to `Queued` and its `Operation::Send` row is
/// written with `send_at` as the time the drainer must not touch it before —
/// see `DraftRepository::queue_send_at`.
fn queue_send_at(
    database: &Database,
    draft: &mut Draft,
    send_at: chrono::DateTime<Utc>,
) -> postio_storage::Result<()> {
    let (connection, _permit) = database.interactive_write()?;
    DraftRepository::new(&connection).queue_send_at(draft, Utc::now(), send_at)?;
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
fn recover(
    composer: &Composer,
    database: &Database,
    account: AccountId,
    last_id: &Rc<Cell<Option<DraftId>>>,
) {
    let Ok(connection) = database.connection() else {
        return;
    };
    let drafts = match DraftRepository::new(&connection).list_for_account(account) {
        Ok(drafts) => drafts,
        Err(error) => {
            tracing::error!(%error, "could not read drafts to recover");
            return;
        }
    };
    drop(connection);

    let Some(draft) = drafts
        .into_iter()
        .find(|draft| draft.state == DraftState::Editing)
    else {
        return;
    };
    last_id.set(Some(draft.id));
    composer.open(draft);
}

/// Recipient completion: contact groups whose name matches `prefix`, then
/// contacts ranked by [`ContactRepository::search`] — groups first, since a
/// group is a deliberate choice the user is more likely typing towards.
fn install_recipient_suggestions(composer: &Composer, database: Database, account: AccountId) {
    composer.connect_recipient_suggestions(move |prefix| {
        let connection = match database.connection() {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "could not search contacts");
                return Vec::new();
            }
        };

        let mut candidates: Vec<RecipientCandidate> = Vec::new();
        let groups = ContactGroupRepository::new(&connection);
        match groups.list(Some(account)) {
            Ok(list) => {
                let prefix_lower = prefix.to_lowercase();
                for group in list {
                    if !group.name.to_lowercase().starts_with(&prefix_lower) {
                        continue;
                    }
                    match groups.members(group.id) {
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

        match ContactRepository::new(&connection).search(Some(account), prefix, SUGGESTION_LIMIT) {
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
fn install_reply_source(composer: &Composer, database: Database, showing: crate::reading::Showing) {
    composer.connect_reply_source(move || {
        // `None` is ordinary: `e` on a window nobody has read from yet is
        // nothing to reply to, not an error. It is logged all the same,
        // because the *other* way to reach here is a miswiring, and #325
        // spent its whole life indistinguishable from working software.
        let Some(id) = showing.get() else {
            tracing::debug!("reply asked for with no message in the reading pane");
            return None;
        };
        let connection = database
            .connection()
            .map_err(|error| tracing::warn!(%error, "could not open a reply source"))
            .ok()?;
        let mut message = MessageRepository::new(&connection).get(id).ok().flatten()?;
        message.body = load_body(&connection, id);
        let account = AccountRepository::new(&connection)
            .get(message.account_id)
            .ok()
            .flatten()?;
        Some((message, account))
    });
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
        runtime.spawn_blocking(move || {
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
pub(crate) use postio_session::reading::{Body, load_body, load_body_or_reason};

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
    //! otherwise. Add a second one here only with that in mind.
    //!
    //! POSTIO-GTK-INIT: the paragraph above is the argument. A binary crate
    //! has nothing for `tests/` to link against, so this one cannot move out
    //! the way `postio-gtk`'s toast tests did. See issue #41 and
    //! `scripts/checks/check-no-gtk-init-in-unit-tests.py`.

    use gtk::gdk;

    use super::*;

    fn settle() {
        while gtk::glib::MainContext::default().iteration(false) {}
    }

    /// A real account row, since `DraftRepository::save`'s first insert
    /// requires one to reference.
    fn seed_account(database: &Database) -> AccountId {
        let connection = database.connection().unwrap();
        let mut account = postio_model::Account::new(
            "Test",
            EmailAddress::new(None::<String>, "ada@example.com"),
        );
        AccountRepository::new(&connection)
            .create(&mut account)
            .unwrap();
        account.id
    }

    // ── `load_body_or_reason` ────────────────────────────────────────────
    //
    // Pure data: no display, no `adw::init()`. See the module doc above for
    // why a GTK-touching test does not belong beside these.

    #[test]
    fn a_message_with_no_body_yet_names_offline_only_when_the_engine_is() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::HeadersOnly;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .unwrap();
        drop(connection);

        let connection = database.connection().unwrap();

        assert!(
            matches!(
                load_body_or_reason(&connection, id, false),
                Body::Absent(postio_gtk::reader::Absent::Partial)
            ),
            "online and not yet fetched is the ordinary backfill wait"
        );
        assert!(
            matches!(
                load_body_or_reason(&connection, id, true),
                Body::Absent(postio_gtk::reader::Absent::Offline)
            ),
            "offline and not yet fetched has to say so, not promise a backfill \
             that cannot run"
        );
    }

    #[test]
    fn a_message_whose_body_already_arrived_ignores_whether_the_engine_is_offline() {
        // Offline-ness only changes the story for a body that has not landed
        // yet. A message with real bytes on disk must read the same whether
        // or not the engine happens to be connected right now.
        let database = postio_storage::test_support::memory();
        let connection = database.connection().unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .unwrap();
        drop(connection);

        let connection = database.connection().unwrap();

        // No blobs were ever named for it, so this is the "fetched, naming
        // no blobs" case -- `Absent::Empty` -- either way.
        assert!(matches!(
            load_body_or_reason(&connection, id, false),
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
        assert!(matches!(
            load_body_or_reason(&connection, id, true),
            Body::Absent(postio_gtk::reader::Absent::Empty)
        ));
    }

    /// #175: a draft's row with `\Draft` set but no local `Draft` buffer
    /// behind it (`DraftRepository::by_message` is `None`) was written by
    /// another client. Even once its body backfills, opening it must not
    /// look like an ordinary, readable message -- there is nothing here that
    /// can be edited, and pretending otherwise is the dead end #175 exists
    /// to close.
    #[test]
    fn a_foreign_drafts_row_says_so_even_once_its_body_has_arrived() {
        let database = postio_storage::test_support::memory();
        let connection = database.connection().unwrap();
        let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection);
        let mut message = postio_model::Message::new(account.id, inbox, Utc::now());
        message.flags.insert(postio_model::Flag::Draft);
        message.sync.body_state = postio_model::BodyState::Full;
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .unwrap();
        drop(connection);

        let connection = database.connection().unwrap();

        // No local `DraftRepository` row exists for this message, which is
        // exactly what makes it another client's draft rather than one this
        // machine is editing.
        assert!(matches!(
            load_body_or_reason(&connection, id, false),
            Body::Absent(postio_gtk::reader::Absent::ForeignDraft)
        ));
    }

    #[test]
    fn a_parked_draft_after_a_clean_exit_leaves_the_next_start_on_the_inbox() {
        // #491, reported directly: "i reopened the app and it opened in a
        // compose window from a draft. cold start should start with the
        // inbox". `DraftState::Editing` is not evidence of a crash — Esc on
        // a draft with content parks it in exactly that state on purpose —
        // so recovery has to ask how the last session *ended*, not what the
        // drafts table holds. The crash test below is this test's twin: the
        // same two runs, with the clean shutdown removed.
        let state_dir =
            std::env::temp_dir().join(format!("postio-app-clean-exit-{}", std::process::id()));
        std::fs::create_dir_all(&state_dir).unwrap();
        // SAFETY: first statement of a single-threaded test.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &state_dir)
        };

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        postio_gtk::fonts::install().expect("the embedded fonts should install");
        postio_gtk::style::install(&display);
        postio_gtk::app::install_icons(&display);

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account =
            seed_account(&Database::open(&db_path, &postio_storage::test_support::key()).unwrap());
        let runtime = tokio::runtime::Runtime::new().unwrap();

        // ── Run one: type, park the draft with Esc, exit cleanly ─────────
        {
            let database = Database::open(&db_path, &postio_storage::test_support::key()).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.handle().clone(),
                crate::reading::Showing::default(),
            );
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
            postio_session::end_session(&database);
        }

        // ── Run two: the draft is parked, not in the way ─────────────────
        {
            let database = Database::open(&db_path, &postio_storage::test_support::key()).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database.clone(),
                blobs,
                runtime.handle().clone(),
                crate::reading::Showing::default(),
            );
            settle();

            assert!(
                !window.composer().is_open(),
                "a cleanly-exited session's parked draft must not take over                  the next start — the inbox is the first thing a mail client                  shows"
            );
            // Never lost: the row is still in Drafts, exactly as autosaved,
            // reachable through the Drafts folder's own resume path.
            let connection = database.connection().unwrap();
            let parked = DraftRepository::new(&connection)
                .list_for_account(account)
                .expect("drafts read");
            assert_eq!(parked.len(), 1);
            assert_eq!(parked[0].subject, "Finish this on Thursday");
            assert_eq!(parked[0].state, DraftState::Editing);
        }
    }

    #[test]
    fn a_draft_saved_before_the_process_stops_is_open_again_on_the_next_start() {
        let state_dir =
            std::env::temp_dir().join(format!("postio-app-crash-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&state_dir).unwrap();
        // SAFETY: first statement of a single-threaded test.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("XDG_STATE_HOME", &state_dir)
        };

        if adw::init().is_err() || gdk::Display::default().is_none() {
            eprintln!("skipping: no display (run under `xvfb-run` to exercise this)");
            return;
        }
        let display = gdk::Display::default().unwrap();
        postio_gtk::fonts::install().expect("the embedded fonts should install");
        postio_gtk::style::install(&display);
        postio_gtk::app::install_icons(&display);

        let db_path = state_dir.join("postio.db");
        let blobs_path = state_dir.join("blobs");
        let account =
            seed_account(&Database::open(&db_path, &postio_storage::test_support::key()).unwrap());
        // Only `install_attach` ever spawns onto this; nothing in this test
        // attaches a file, so it exists purely to give `install` a handle.
        let runtime = tokio::runtime::Runtime::new().unwrap();

        // ── Run one: open, type, autosave, and stop cold ─────────────────
        //
        // No `composer.close()`, no clean shutdown of anything — a crash
        // does not call those either. Going out of scope at the end of this
        // block is the whole simulation: the transaction `save()` already
        // committed is what has to survive it, not an orderly exit.
        {
            let database = Database::open(&db_path, &postio_storage::test_support::key()).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.handle().clone(),
                crate::reading::Showing::default(),
            );
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
            let database = Database::open(&db_path, &postio_storage::test_support::key()).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(
                &window,
                account,
                database,
                blobs,
                runtime.handle().clone(),
                crate::reading::Showing::default(),
            );
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
