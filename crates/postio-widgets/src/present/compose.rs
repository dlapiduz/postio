//! The composer's seams answered through [`postio_client::Client`], never `postio-session` or
//! `postio-host` directly (ADR 0043). Focus's Labels row and reminder footer use their own handlers
//! on `connect_reply_source` and `connect_opened`, which this module does not register: a seam is a
//! single slot, so the app's chrome keeps its own reply source.

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

pub const SUGGESTION_LIMIT: usize = 8;

/// `None` for a window that already hears the host's events (#1608).
pub type Announce = Rc<dyn Fn(&postio_core::Event)>;

pub type OnQueued =
    Rc<dyn Fn(postio_client::protocol::Queued, Option<chrono::DateTime<chrono::Utc>>)>;

pub type Current = Rc<dyn Fn() -> Option<MessageId>>;

pub type Resume = Rc<dyn Fn(MessageId)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeNote {
    Cancelled,
    Failed(String),
}

pub type OnResumeNote = Rc<dyn Fn(ResumeNote)>;

/// The answer is the account row, for a caller with its own use for it (#1241).
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

/// A mailbox's signature overrides the account's default, which overrides the identity's.
/// `selected` is read fresh on every compose; Focus has no folder selected and answers `None`.
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
            // A default the composer has not been told about (made in Settings since it read the
            // account's signatures, or the first compose before its read landed): read them again
            // first, or the draft signs with the identity's own.
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

pub type OnSaved = Rc<dyn Fn(DraftId)>;

/// Crash recovery is not here: reopening a draft is a composition-root decision, since Focus's
/// dialog is built lazily and opening one nobody asked for would surprise. See [`recover_draft`].
pub fn install_autosave(
    composer: &Composer,
    client: &Client,
    on_saved: Option<OnSaved>,
) -> Rc<Cell<Option<DraftId>>> {
    // Not read from the composer's draft afterward: `connect_closed` carries what became of the
    // draft, not which one.
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
            if outcome != Closing::Drop {
                return;
            }
            let Some(composer) = weak.upgrade() else {
                return;
            };
            drop(client.discard_draft(composer.previous_generation(), last_id.take()));
        }
    });
    last_id
}

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

/// Nothing here waits for SMTP or opens a connection: the write is one local transaction (ADR
/// 0021). `last_id` is taken on send too, or [`install_autosave`]'s close handler (`Composer::send`
/// closes with `Closing::Drop`) would discard the draft just queued.
pub fn install_send(
    composer: &Composer,
    client: &Client,
    last_id: Rc<Cell<Option<DraftId>>>,
    account: AccountId,
    announce: Option<Announce>,
    on_queued: Option<OnQueued>,
) {
    let weak = composer.downgrade();
    composer.connect_send({
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let weak = weak.clone();
        let announce = announce.clone();
        let on_queued = on_queued.clone();
        move |draft| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            last_id.set(None);
            // Through the host's writer, after every save this composition handed it: a send racing
            // a first save in flight would insert a second row.
            let queued = client.queue_send(composer.generation(), draft.clone(), None);
            if announce.is_none() && on_queued.is_none() {
                drop(queued);
                return;
            }
            let announce = announce.clone();
            let on_queued = on_queued.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // write ran on the host's runtime.
                let Ok(queued) = queued.await else {
                    return;
                };
                if let (Some(announce), Some(drafts)) = (announce, queued.drafts) {
                    announce(&postio_core::Event::MessageListChanged {
                        account,
                        mailbox: drafts,
                    });
                }
                if let Some(on_queued) = on_queued {
                    on_queued(queued, None);
                }
            });
        }
    });
    composer.connect_send_later({
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let on_queued = on_queued.clone();
        move |draft, send_at| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            last_id.set(None);
            let queued = client.queue_send(composer.generation(), draft.clone(), Some(send_at));
            let on_queued = on_queued.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                if let (Ok(queued), Some(on_queued)) = (queued.await, on_queued) {
                    on_queued(queued, Some(send_at));
                }
            });
        }
    });
}

/// A queued send is taken back first, so an edit cannot race the drainer (#433); a failed send says
/// why through `on_note`.
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
            // POSTIO-GLIB-SAFE: it awaits only client calls, oneshot receives.
            resume_found(draft, weak, client, last_id, on_note).await;
        });
    })
}

pub type ResumeDraft = Rc<dyn Fn(DraftId)>;

pub fn install_resume_draft(
    composer: &Composer,
    client: &Client,
    last_id: Rc<Cell<Option<DraftId>>>,
    on_note: Option<OnResumeNote>,
) -> ResumeDraft {
    let weak = composer.downgrade();
    let client = client.clone();
    Rc::new(move |id| {
        let weak = weak.clone();
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let on_note = on_note.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: client calls are oneshot receives.
            let draft = match client.draft_by_id(id).await {
                Ok(Some(draft)) => draft,
                Ok(None) => return,
                Err(error) => {
                    tracing::warn!(%error, "could not read a draft by id");
                    return;
                }
            };
            // POSTIO-GLIB-SAFE: it awaits only client calls, oneshot receives.
            resume_found(draft, weak, client, last_id, on_note).await;
        });
    })
}

async fn resume_found(
    draft: postio_model::Draft,
    weak: glib::WeakRef<Composer>,
    client: Client,
    last_id: Rc<Cell<Option<DraftId>>>,
    on_note: Option<OnResumeNote>,
) {
    let (draft, note) = if draft.state == DraftState::Queued {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
        let Ok(Some(reopened)) = client.cancel_send(draft.id).await else {
            return;
        };
        (reopened, Some(ResumeNote::Cancelled))
    } else if draft.state == DraftState::Failed {
        // #1487: a failed send has to name what went wrong.
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
        let why = client.send_failure(draft.id).await.ok().flatten();
        (draft, why.map(ResumeNote::Failed))
    } else {
        (draft, None)
    };
    let Some(composer) = weak.upgrade() else {
        return;
    };
    // So closing it empty clears the right row: `connect_closed` carries what became of the draft,
    // not which one.
    last_id.set(Some(draft.id));
    composer.resume(draft);
    if let (Some(note), Some(on_note)) = (note, on_note) {
        on_note(note);
    }
}

/// The composer asks for candidates on every keystroke, synchronously on the GTK thread, so the
/// directory is read when this is installed and each time the composer opens, then ranked by
/// `postio_ui::recipients::suggest` with no call or query.
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

/// The editing surface's `cid:` resolver is a synchronous callback, so this answers from memory
/// rather than wait on the store on the drawing thread (`check-blocking-now-sites.py`). A
/// composition's inline images may have just been pasted or need reading first, so a cache filled
/// ahead of render is the only shape that works.
type InlineBytes = Rc<RefCell<HashMap<BlobId, Vec<u8>>>>;

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
