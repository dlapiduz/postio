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
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gio;
use gtk::prelude::*;
use postio_gtk::composer::{Closing, Composer};
use postio_gtk::window::Window;
use postio_host::compose::{self, DraftWriter};
use postio_model::ids::AccountId;
use postio_model::{DraftId, DraftState};
use postio_storage::repository::{ContactGroupRepository, ContactRepository};
use postio_storage::{BlobStore, Store};

use crate::recipients::{Directory, resolved_address};

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

    let writer = DraftWriter::spawn(database.clone(), &runtime);
    let last_id = install_autosave(&composer, database.clone(), account, &writer);
    install_send(&composer, &writer, Rc::clone(&last_id), account, announce);
    install_send_later(&composer, &writer, Rc::clone(&last_id));
    install_resume(window, &composer, database.clone(), last_id);
    install_recipient_suggestions(&composer, database.clone(), account, &runtime);
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
            let attachment = compose::inline_attachment(&blobs, bytes, &mime_type);
            let _ = sender.send_blocking(attachment);
        });
        gtk::glib::spawn_future_local(async move {
            then(receiver.recv().await.ok().flatten());
        });
    });
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
    if let Some(account) = compose::account(database, account).await {
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
                .send(compose::default_signature(&database, account, selected).await)
                .await;
        });
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads ran on the runtime.
            answer(resolved.recv().await.ok().flatten());
        });
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
                    let Some(draft) = compose::draft_behind(&database, row.id).await else {
                        return;
                    };
                    let draft = if draft.state == DraftState::Queued {
                        // #433: the row stays in the Drafts folder for as long as the
                        // send sits in the queue, and opening it here used to reopen
                        // it live for editing while the drainer could pick the same
                        // row up at any moment — an edit landed or did not, purely on
                        // timing. Cancelling the send is what makes editing it again
                        // safe: see `DraftRepository::cancel_send`.
                        let Some(reopened) = compose::cancel_queued_send(&database, draft.id).await
                        else {
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
                        compose::why_the_send_failed(&database, draft.id).await
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
        // the save's answer and is written onto the composer -- only while
        // the same composition is in its fields -- and into `last_id`.
        let weak = composer.downgrade();
        composer.connect_save({
            let writer = writer.clone();
            let weak = weak.clone();
            let last_id = Rc::clone(&last_id);
            move |draft| {
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                let generation = composer.generation();
                let saved = writer.save(generation, draft.clone());
                let last_id = Rc::clone(&last_id);
                let weak = weak.clone();
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a channel receive; the write runs on
                    // the runtime in `DraftWriter`.
                    let Ok(id) = saved.await else {
                        return;
                    };
                    let Some(composer) = weak.upgrade() else {
                        return;
                    };
                    if composer.generation() == generation {
                        last_id.set(Some(id));
                    }
                    composer.adopt_id(generation, id);
                });
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
                // Handed over now, in order; nothing waits for it to land.
                drop(writer.discard(composer.previous_generation(), last_id.take()));
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
        let queued = writer.send(composer.generation(), draft.clone(), None);
        let announce = announce.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the write ran on the runtime.
            let Ok(Some(drafts)) = queued.await else {
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
        // Handed over now, in order; nothing here waits for it to land.
        drop(writer.send(composer.generation(), draft.clone(), Some(send_at)));
    });
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
    let drafts = match compose::drafts_of(database, account).await {
        Ok(drafts) => drafts,
        Err(error) => {
            tracing::error!(%error, "could not read drafts to recover: {error}");
            return;
        }
    };
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

/// Recipient completion, answered from memory (see [`crate::recipients`]).
///
/// The directory is read off the GTK thread when this is installed and again
/// each time the composer opens, so a contact first seen in mail that
/// arrived meanwhile is offered in the next composition. Every keystroke is
/// then [`Directory::suggest`] over what was read: no connection, no query,
/// nothing on the thread that draws.
fn install_recipient_suggestions(
    composer: &Composer,
    database: Store,
    account: AccountId,
    runtime: &tokio::runtime::Handle,
) {
    let directory: Rc<RefCell<Rc<Directory>>> = Rc::default();
    let reload = {
        let directory = Rc::clone(&directory);
        let runtime = runtime.clone();
        move || {
            let answer = crate::search::ask(&database, &runtime, move |connection| async move {
                read_directory(&connection, account)
                    .await
                    .map_err(|error| tracing::warn!(%error, "could not read the contacts"))
                    .ok()
            });
            let directory = Rc::clone(&directory);
            glib::spawn_future_local(async move {
                if let Ok(Some(read)) = answer.recv().await {
                    *directory.borrow_mut() = Rc::new(read);
                }
            });
        }
    };
    reload();
    composer.connect_opened(reload);
    composer.connect_recipient_suggestions(move |prefix| {
        let directory = Rc::clone(&directory.borrow());
        directory.suggest(prefix, SUGGESTION_LIMIT as usize)
    });
}

/// Every group (with its members) and contact `account` can complete, in
/// the order they are offered.
async fn read_directory(
    connection: &postio_storage::Checkout,
    account: AccountId,
) -> postio_storage::Result<Directory> {
    let groups = ContactGroupRepository::new(connection);
    let mut named = Vec::new();
    for group in groups.list(Some(account)).await? {
        let members = groups.members(group.id).await?;
        named.push((group.name, members.iter().map(resolved_address).collect()));
    }
    let contacts = ContactRepository::new(connection)
        .search(Some(account), "", DIRECTORY_LIMIT)
        .await?;
    Ok(Directory::new(named, contacts))
}

/// The most contacts a directory holds: the same bound the search
/// surface's `@` list reads with, for the same reason.
const DIRECTORY_LIMIT: u32 = 50_000;

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
            let _ = sender
                .send(compose::reply_source(&database, id).await)
                .await;
        });
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a channel receive; the reads ran on the runtime.
            answer(found.recv().await.ok().flatten());
        });
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
        runtime.spawn(async move {
            let attachment = compose::attach_file(&blobs, &path, mime_type_of(&path));
            let _ = sender.send_blocking(attachment);
        });
        gtk::glib::spawn_future_local(async move {
            then(receiver.recv().await.ok().flatten());
        });
    });
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
// #70's blank column rather than the fix. `load_body`'s last reader here was
// the search preview, which asks the host for it now (`Req::StoredBody`).
pub(crate) use postio_session::reading::Body;

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
    use chrono::Utc;
    use postio_model::{Draft, EmailAddress};
    use postio_storage::repository::{AccountRepository, DraftRepository, MessageRepository};

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
