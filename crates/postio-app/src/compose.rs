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
use postio_gtk::composer::{Closing, Composer};
use postio_gtk::window::Window;
use postio_model::ids::{AccountId, MessageId};
use postio_model::{Attachment, Draft, DraftId, DraftState, EmailAddress, MessageBody};
use postio_storage::repository::{
    AccountRepository, ContactRepository, DraftRepository, MessageRepository,
};
use postio_storage::{BlobStore, Database};

/// How many recipient suggestions to offer at once — a popover, not a list
/// the user scrolls.
const SUGGESTION_LIMIT: u32 = 8;

/// Wires `window`'s composer to `database` for `account`: autosave with
/// crash recovery, recipient completion from contacts, replying to whatever
/// the list last opened, and attaching files into `blobs`. `runtime` is only
/// for [`install_attach`] — everything else here is synchronous.
pub fn install(
    window: &Window,
    account: AccountId,
    database: Database,
    blobs: BlobStore,
    runtime: tokio::runtime::Handle,
) {
    let composer = window.composer();
    composer.set_account(account);

    install_autosave(&composer, database.clone(), account);
    install_recipient_suggestions(&composer, database.clone(), account);
    install_reply_source(window, &composer, database, blobs.clone());
    install_attach(&composer, blobs, runtime);
}

/// Autosave to [`DraftRepository`], crash recovery, and clearing the row once
/// there is nothing left to keep — sent, discarded, or closed empty.
fn install_autosave(composer: &Composer, database: Database, account: AccountId) {
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

    recover(composer, &database, account, &last_id);
}

/// Autosave: the local row, and the queue row that carries it to the account's
/// Drafts mailbox, in one write.
///
/// The enqueue is what makes a draft survive more than this machine — see
/// `DraftRepository::save_and_sync`. It costs nothing extra here: the queue
/// row is written inside the same transaction, and the engine sends it when
/// there is a connection. A run of autosaves folds into one upload.
fn save_draft(database: &Database, draft: &mut Draft) -> postio_storage::Result<()> {
    let connection = database.connection()?;
    DraftRepository::new(&connection).save_and_sync(draft, Utc::now())?;
    Ok(())
}

/// Discard: the local row goes now, and the server copy is queued for removal.
fn delete_draft(database: &Database, id: DraftId) -> postio_storage::Result<()> {
    let connection = database.connection()?;
    DraftRepository::new(&connection).discard(id, Utc::now())?;
    Ok(())
}

/// Opens whatever draft `account` was still editing when Postio last
/// stopped — the crash-recovery half of the bead, and the whole reason it
/// matters: a draft is not durable until it comes back on its own.
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

/// Recipient completion, ranked by [`ContactRepository::search`].
fn install_recipient_suggestions(composer: &Composer, database: Database, account: AccountId) {
    composer.connect_recipient_suggestions(move |prefix| {
        let connection = match database.connection() {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(%error, "could not search contacts");
                return Vec::new();
            }
        };
        match ContactRepository::new(&connection).search(Some(account), prefix, SUGGESTION_LIMIT) {
            Ok(contacts) => contacts.iter().map(resolved_address).collect(),
            Err(error) => {
                tracing::warn!(%error, "could not search contacts");
                Vec::new()
            }
        }
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

/// `e`/`E`/`f` reply to whatever the list last activated — read-only against
/// [`postio_gtk::window::Window::list`]'s existing signal, so nothing here
/// needs to touch the list itself.
fn install_reply_source(
    window: &Window,
    composer: &Composer,
    database: Database,
    blobs: BlobStore,
) {
    let current: Rc<Cell<Option<MessageId>>> = Rc::new(Cell::new(None));

    window.list().connect_activated({
        let current = Rc::clone(&current);
        move |row| current.set(Some(row.id))
    });

    composer.connect_reply_source(move || {
        let id = current.get()?;
        let connection = database
            .connection()
            .map_err(|error| tracing::warn!(%error, "could not open a reply source"))
            .ok()?;
        let mut message = MessageRepository::new(&connection).get(id).ok().flatten()?;
        message.body = load_body(&connection, &blobs, id);
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

/// A message's text and HTML, if they have been downloaded.
///
/// Absent rather than an error for a message still `Partial` — headers synced,
/// body not yet — which is the ordinary state of a mailbox mid-backfill, not
/// a fault. Replying to one just quotes nothing, the same way any degraded
/// state here should: fewer words in the draft, never a broken one.
pub(crate) fn load_body(
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
pub(crate) enum Body {
    /// Bytes are on this machine, and these are them.
    Ready(postio_model::MessageBody),
    /// There are none, for this reason.
    Absent(postio_gtk::reader::Absent),
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
/// [`Absent::Offline`] is deliberately not produced here: telling it from
/// `Partial` needs the engine's `ConnectionState`, which does not reach this
/// crate yet. The pane says "downloading" in both cases meanwhile, which is
/// true but less useful than it should be — see the follow-up issue on #70.
///
/// [`BodyState`]: postio_model::message::BodyState
/// [`Absent::Offline`]: postio_gtk::reader::Absent::Offline
pub(crate) fn load_body_or_reason(
    connection: &postio_storage::PooledConnection,
    blobs: &BlobStore,
    id: MessageId,
) -> Body {
    use postio_gtk::reader::Absent;

    let repository = MessageRepository::new(connection);

    // Has anything been downloaded for this message at all?
    match repository.get(id) {
        Ok(Some(message)) if !message.sync.body_state.has_body() => {
            return Body::Absent(Absent::Partial);
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

fn read_blob_text(blobs: &BlobStore, id: &postio_model::ids::BlobId) -> Option<String> {
    let bytes = blobs
        .get(id)
        .map_err(|error| tracing::warn!(%error, "could not read a message body blob"))
        .ok()?;
    String::from_utf8(bytes)
        .map_err(|error| tracing::warn!(%error, "a body blob was not valid UTF-8"))
        .ok()
}

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
    //! `scripts/check-no-gtk-init-in-unit-tests.py`.

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
        let account = seed_account(&Database::open(&db_path).unwrap());
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
            let database = Database::open(&db_path).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(&window, account, database, blobs, runtime.handle().clone());
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
            let database = Database::open(&db_path).unwrap();
            let blobs = BlobStore::open(&blobs_path).unwrap();
            let window = Window::default();
            window.present();
            settle();

            install(&window, account, database, blobs, runtime.handle().clone());
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
