//! The composer's seams, answered through the store's owner
//! (specs/007-postio-focus T022, T078; ADR 0041).
//!
//! Each window used to answer these seams for itself: a save is `Req::SaveDraft`, a send
//! `Req::QueueSend`, and the message that leaves is built by the host from
//! the draft the composer hands over -- the same draft, from the same
//! composer, whichever window holds it (FR-050, T081). Nothing here reaches
//! `postio-session` or `postio-host` directly; every seam is answered
//! through [`postio_client::Client`], which is what makes this crate able to
//! hold it at all (ADR 0043).
//!
//! Focus's own wiring (`postio_gtk::compose::seams`) was already shaped
//! this way before this module existed -- reaching only through the client,
//! answering the WebKit-facing seams from a cache it fills ahead of time
//! rather than a blocking read -- and is what this module is built from.
//! The classic composer used a different shape for the same
//! seams: `postio_session::blocking::now` for the seams that had to answer
//! on the spot. Unifying on Focus's shape is what let two of that debt's
//! three sites disappear rather than move (`check-blocking-now-sites.py`);
//! see [`install_inline_images`] and [`install_resume`].
//!
//! What is *not* here: a seam only one app's chrome needs. Focus's Labels
//! row and its "remind if no reply" footer read the same [`Composer`] this
//! module wires, through their own additional handlers on
//! [`Composer::connect_reply_source`] and [`Composer::connect_opened`] --
//! which this module does not register, precisely so Focus's dialog frame
//! can. A composer's seam is a single slot, not a signal several listeners
//! share, so a seam this module owns cannot also be owned by an app's own
//! chrome; [`install_reply_source`] is deliberately the plain version both
//! apps could use unchanged, and Focus keeps its own richer one.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_client::protocol::RecipientDirectory;
use postio_model::ids::{AccountId, BlobId, MailboxId, MessageId};
use postio_model::{Account, DraftId, DraftState};

use crate::composer::{Closing, Composer};

/// How many recipient suggestions to offer at once -- a popover, not a list
/// the user scrolls.
pub const SUGGESTION_LIMIT: usize = 8;

/// How the composer tells the rest of the window something happened, for a
/// window whose news does not otherwise reach it. `None` for a window that
/// already hears the host's events (#1608): the classic app's composition
/// root is one, Focus's is another, so this stays a caller's choice rather
/// than something this module decides.
pub type Announce = Rc<dyn Fn(&postio_core::Event)>;

/// Which message `e`, `E` and `f` answer: the row the cursor is on, or the
/// reading pane is showing.
pub type Current = Rc<dyn Fn() -> Option<MessageId>>;

/// Open the draft behind a Drafts row for editing.
pub type Resume = Rc<dyn Fn(MessageId)>;

/// What [`install_resume`] found when it took a queued or failed draft back
/// for editing, for a caller that wants to say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeNote {
    /// The draft was queued; opening it here cancelled that send (#433).
    Cancelled,
    /// The draft's last send failed, for this reason.
    Failed(String),
}

/// A caller's reaction to a [`ResumeNote`] -- a toast, a status line, a
/// footer's note. Left `None` to say nothing.
pub type OnResumeNote = Rc<dyn Fn(ResumeNote)>;

/// The account's identities and named signatures, read once and put in front
/// of the user (#12); the answer is the account row, for a caller that has
/// its own use for it (the classic window marks its own messages in the
/// conversation pane from the same identities, #1241).
///
/// Nothing called this before #12 was fixed, so the picker had been built,
/// tested and shown with an empty model since it was written: every draft
/// signed with whatever `apply_identity` found on an account of none, which
/// is nothing.
pub async fn install_identities(
    composer: &Composer,
    client: &Client,
    account: AccountId,
) -> Option<Account> {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let accounts = match client.accounts().await {
        Ok(accounts) => accounts,
        Err(error) => {
            tracing::warn!(%error, "could not read the composer's account identities");
            return None;
        }
    };
    let Some(found) = accounts.into_iter().find(|found| found.id == account) else {
        tracing::warn!("the composer's account is not in the database");
        return None;
    };
    composer.set_size_limit(found.max_message_size);
    composer.set_identities(found.identities.clone());
    composer.set_signatures(found.signatures.clone());
    Some(found)
}

/// Puts a resolved default in front of a brand-new draft, before the
/// identity's own (#12's last item, #394): a mailbox's own signature
/// overrides the account's default, which overrides the identity's.
///
/// `selected` answers which mailbox is in view, read fresh on every compose
/// rather than once at install: the classic app reads its sidebar's
/// selection, which changes on every click; Focus has no folder selected and
/// answers `None`, which leaves the account's default signature to decide.
pub fn install_signature_default(
    composer: &Composer,
    client: &Client,
    account: AccountId,
    selected: impl Fn() -> Option<MailboxId> + 'static,
) {
    let client = client.clone();
    let weak = composer.downgrade();
    composer.connect_signature_default(move |answer| {
        let client = client.clone();
        let weak = weak.clone();
        let selected = selected();
        glib::spawn_future_local(async move {
            let resolving = client.default_signature(account, selected);
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // resolves the rest.
            let resolved = resolving.await.ok().flatten();
            // A default the composer has not been told about: one made in
            // Settings since it read the account's signatures, or the first
            // compose, whose read has not landed yet. Read them again first,
            // or the draft signs with the identity's own instead (T234).
            if let (Some(id), Some(composer)) = (resolved, weak.upgrade())
                && !composer.has_signature(id)
            {
                // POSTIO-GLIB-SAFE: one client call, a oneshot receive.
                install_identities(&composer, &client, account).await;
            }
            answer(resolved);
        });
    });
}

/// A caller's reaction to an id a save just landed -- Focus's footer
/// stamps "Saved at HH:MM" from it; the classic app has no such row and
/// passes `None`.
pub type OnSaved = Rc<dyn Fn(DraftId)>;

/// Autosave through the host's `DraftWriter`, and clearing the row once
/// there is nothing left to keep -- sent, discarded, or closed empty.
///
/// Crash recovery is not here: whether a brand-new composer should reopen
/// whatever `account` was writing when the last session died is a
/// composition-root decision (the classic app's does, at mount; Focus's
/// dialog is built lazily, well after startup, and opening a dialog nobody
/// asked for would be a surprise). See [`recover_draft`].
pub fn install_autosave(
    composer: &Composer,
    client: &Client,
    on_saved: Option<OnSaved>,
) -> Rc<Cell<Option<DraftId>>> {
    // The id of whatever `connect_save`'s handler last persisted. Not read
    // from the composer's own draft afterward because `connect_closed` does
    // not carry the draft -- only what became of it -- so this is the one
    // piece of bookkeeping this module has to keep for itself.
    let last_id: Rc<Cell<Option<DraftId>>> = Rc::default();
    let weak = composer.downgrade();
    composer.connect_save({
        let client = client.clone();
        let weak = weak.clone();
        let last_id = Rc::clone(&last_id);
        let on_saved = on_saved.clone();
        move |draft| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let generation = composer.generation();
            // Handed over here, in the order the composer saved.
            let saved = client.save_draft(generation, draft.clone());
            let last_id = Rc::clone(&last_id);
            let weak = weak.clone();
            let on_saved = on_saved.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // write runs on the host's runtime in its `DraftWriter`.
                let Ok(id) = saved.await else {
                    return;
                };
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                if composer.generation() == generation {
                    last_id.set(Some(id));
                    if let Some(on_saved) = on_saved {
                        on_saved(id);
                    }
                }
                composer.adopt_id(generation, id);
            });
        }
    });
    composer.connect_closed({
        let client = client.clone();
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
            // Handed over now, in order; nothing waits for it to land.
            drop(client.discard_draft(composer.previous_generation(), last_id.take()));
        }
    });
    last_id
}

/// Only after a crash (#491): ask `begin_session`, which knows how the last
/// session ended, and open the draft worth reopening by Esc's own rule -- or
/// nothing.
pub async fn recover_draft(
    composer: &Composer,
    client: &Client,
    account: AccountId,
    last_id: &Rc<Cell<Option<DraftId>>>,
) {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host reads
    // on its own runtime.
    match client.recover_draft(account).await {
        Ok(Some(draft)) => {
            last_id.set(Some(draft.id));
            composer.open(draft);
        }
        Ok(None) => {}
        Err(error) => tracing::error!(%error, "could not read drafts to recover: {error}"),
    }
}

/// Sending, now or later: the draft becomes a queue row, and stops being the
/// composer's. Nothing here waits for SMTP, and nothing here opens a
/// connection: the write is one local transaction, and `postio-sync::send`
/// drains the row it leaves whenever there is a network (ADR 0021).
///
/// `last_id` is taken on send too, and deliberately: `Composer::send` closes
/// with `Closing::Drop`, and [`install_autosave`]'s close handler discards
/// whatever `last_id` is holding -- which is precisely the draft just
/// queued, if this did not clear it first.
///
/// `announce` is asked after an immediate send lands, with `account`, so a
/// window whose own news does not otherwise reach it (`None`, see
/// [`Announce`]) can say the list moved a row.
pub fn install_send(
    composer: &Composer,
    client: &Client,
    last_id: Rc<Cell<Option<DraftId>>>,
    account: AccountId,
    announce: Option<Announce>,
) {
    let weak = composer.downgrade();
    composer.connect_send({
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let weak = weak.clone();
        let announce = announce.clone();
        move |draft| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            last_id.set(None);
            // Through the host's writer, after every save this composition
            // handed it: a send racing a first save in flight would insert a
            // second row.
            let queued = client.queue_send(composer.generation(), draft.clone(), None);
            let Some(announce) = announce.clone() else {
                drop(queued);
                return;
            };
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // write ran on the host's runtime.
                let Ok(Some(drafts)) = queued.await else {
                    return;
                };
                announce(&postio_core::Event::MessageListChanged {
                    account,
                    mailbox: drafts,
                });
            });
        }
    });
    composer.connect_send_later({
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        move |draft, send_at| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            last_id.set(None);
            // Handed over now, in order; nothing here waits for it to land.
            drop(client.queue_send(composer.generation(), draft.clone(), Some(send_at)));
        }
    });
}

/// A draft left in Drafts -- by either app -- opens in the composer for
/// editing (US3 scenario 3, US11 scenario 3): a draft whose send is still
/// queued is taken back first, so an edit cannot race the drainer (#433),
/// and a failed send says why, through `on_note`.
pub fn install_resume(
    composer: &Composer,
    client: &Client,
    last_id: Rc<Cell<Option<DraftId>>>,
    on_note: Option<OnResumeNote>,
) -> Resume {
    let weak = composer.downgrade();
    let client = client.clone();
    Rc::new(move |message| {
        let weak = weak.clone();
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let on_note = on_note.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: client calls are oneshot receives.
            let draft = match client.draft_behind(message).await {
                Ok(Some(draft)) => draft,
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!(%error, "could not read the draft behind a row");
                    return;
                }
            };
            let (draft, note) = if draft.state == DraftState::Queued {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let Ok(Some(reopened)) = client.cancel_send(draft.id).await else {
                    return;
                };
                (reopened, Some(ResumeNote::Cancelled))
            } else if draft.state == DraftState::Failed {
                // FR-066's third clause, and #1487: a failed send has to
                // name what went wrong.
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let why = client.send_failure(draft.id).await.ok().flatten();
                (draft, why.map(ResumeNote::Failed))
            } else {
                (draft, None)
            };
            let Some(composer) = weak.upgrade() else {
                return;
            };
            // So that closing it empty clears the right row: `connect_closed`
            // carries what became of the draft and not which one it was.
            last_id.set(Some(draft.id));
            composer.resume(draft);
            if let (Some(note), Some(on_note)) = (note, on_note) {
                on_note(note);
            }
        });
    })
}

/// Recipient completion, answered from memory.
///
/// The composer asks for candidates on every keystroke in `To`, `Cc` and
/// `Bcc`, synchronously, on the GTK thread, so the directory -- the
/// account's groups and contacts, each with the letters the user wrote to it
/// -- is read from the store's owner when this is installed and again each
/// time the composer opens, so a contact first seen in mail that arrived
/// meanwhile is offered in the next composition. Every keystroke is then
/// `postio_ui::recipients::suggest` over what was read -- the one rule every
/// app ranks by (spec 007 T076) -- with no call, no query, and nothing on
/// the thread that draws.
pub fn install_recipients(composer: &Composer, client: &Client, account: AccountId) {
    let directory: Rc<RefCell<Rc<RecipientDirectory>>> = Rc::default();
    let reload = {
        let directory = Rc::clone(&directory);
        let client = client.clone();
        move || {
            let client = client.clone();
            let directory = Rc::clone(&directory);
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime.
                match client.recipient_directory(account).await {
                    Ok(read) => *directory.borrow_mut() = Rc::new(read),
                    Err(error) => tracing::warn!(%error, "could not read the contacts"),
                }
            });
        }
    };
    reload();
    composer.connect_opened(reload);
    composer.connect_recipient_suggestions(move |prefix| {
        let directory = Rc::clone(&directory.borrow());
        postio_ui::recipients::suggest(
            &directory.groups,
            &directory.contacts,
            prefix,
            SUGGESTION_LIMIT,
        )
    });
}

/// Stores a chosen or dropped file as an attachment: the type sniffed off
/// the main loop, the bytes stored by the host.
pub fn install_attach(composer: &Composer, client: &Client) {
    let client = client.clone();
    composer.connect_attach(move |path, then| {
        let client = client.clone();
        glib::spawn_future_local(async move {
            let sniffed = path.clone();
            // POSTIO-GLIB-SAFE: the sniff is a blocking gio call, run on a
            // thread; the store is a client call, a oneshot receive.
            let mime_type = gio::spawn_blocking(move || mime_type_of(&sniffed))
                // POSTIO-GLIB-SAFE: gio's own thread pool answers the sniff.
                .await
                .unwrap_or_else(|_| "application/octet-stream".to_owned());
            let attachment = client
                .attach_as(path, Some(mime_type))
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host stores the bytes on its own blocking pool.
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not store the attachment");
                    None
                });
            then(attachment);
        });
    });
}

/// `path`'s type, as a file manager would sniff it.
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

/// Inline images' bytes, by blob, held for the editing surface: its `cid:`
/// resolver is a synchronous callback, and this answers it from memory
/// rather than wait on the store on the thread that draws
/// (`check-blocking-now-sites.py`) -- unlike the reader's own `cid:` images
/// (`present::reading::cid_source`), which resolve against a message that is
/// already on this machine and so can afford the one blocking read WebKit's
/// callback allows; a composition's inline images may have just been
/// pasted, and a resumed draft's have to be read before they can be shown at
/// all, so a cache filled ahead of render is the only shape that works for
/// both.
type InlineBytes = Rc<RefCell<HashMap<BlobId, Vec<u8>>>>;

/// A pasted image, stored as an inline part, its bytes kept for the surface
/// that shows it; a draft that opens with inline images has theirs read.
pub fn install_inline_images(composer: &Composer, client: &Client) {
    let held: InlineBytes = Rc::default();
    composer.connect_inline_image({
        let client = client.clone();
        let held = Rc::clone(&held);
        move |bytes, mime_type, then| {
            let client = client.clone();
            let held = Rc::clone(&held);
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let stored = client.inline_image(bytes.clone(), mime_type).await;
                let stored = stored.unwrap_or_else(|error| {
                    tracing::warn!(%error, "could not store the pasted image");
                    None
                });
                if let Some(blob) = stored.as_ref().and_then(|part| part.blob_id.clone()) {
                    held.borrow_mut().insert(blob, bytes);
                }
                then(stored);
            });
        }
    });
    let weak = composer.downgrade();
    composer.connect_opened({
        let client = client.clone();
        let held = Rc::clone(&held);
        move || {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let wanted: Vec<_> = composer
                .draft()
                .attachments
                .iter()
                .filter(|part| part.content_id.is_some())
                .filter_map(|part| part.blob_id.clone())
                .filter(|blob| !held.borrow().contains_key(blob))
                .collect();
            for blob in wanted {
                let client = client.clone();
                let held = Rc::clone(&held);
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    if let Ok(Some(bytes)) = client.attachment_bytes(blob.clone()).await {
                        held.borrow_mut().insert(blob, bytes);
                    }
                });
            }
        }
    });
    composer.connect_attachment_bytes(move |attachment| {
        held.borrow().get(attachment.blob_id.as_ref()?).cloned()
    });
}
