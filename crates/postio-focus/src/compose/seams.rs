//! The composer's seams, answered through Focus's client (ADR 0041).
//!
//! The classic app answers the same seams in `postio-app`'s `compose.rs`,
//! which Focus may not depend on. What each does is the host's: a save is
//! `Req::SaveDraft`, a send `Req::QueueSend`, and the message that leaves is
//! built by the host from the draft the composer hands over -- the same
//! draft, from the same composer, whichever app holds it (FR-050, T081).
//!
//! As there, a seam that has to answer on the spot (recipient completion,
//! an inline image's bytes) answers from memory or from one bounded local
//! read; every other one asks the client on the main context and answers
//! when the host does.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};
use postio_client::Client;
use postio_client::protocol::RecipientDirectory;
use postio_model::DraftId;
use postio_model::ids::{AccountId, MessageId};
use postio_widgets::composer::{Closing, Composer, ReplyAnswer};

use super::frame::Frame;

/// How many recipient suggestions to offer at once, as the classic app does.
const SUGGESTION_LIMIT: usize = 8;

/// Which message `e`, `E` and `f` answer: the row the cursor is on.
pub type Current = Rc<dyn Fn() -> Option<MessageId>>;

/// Answer every seam of `composer` through `client`, for `account`.
pub fn wire(
    composer: &Composer,
    frame: &Rc<Frame>,
    client: &Client,
    account: AccountId,
    current: Current,
) {
    composer.set_account(account);
    identities(composer, client, account);
    signature_default(composer, client, account);
    let last_id = autosave(composer, frame, client);
    send(composer, client, Rc::clone(&last_id));
    recipients(composer, client, account);
    reply_source(composer, frame, client, current);
    label_names(composer, frame, client, account);
    attach(composer, client);
    inline_images(composer, client);
}

/// The account's identities and signatures, read once.
fn identities(composer: &Composer, client: &Client, account: AccountId) {
    let client = client.clone();
    let composer = composer.downgrade();
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // answers on its own runtime.
        let Ok(accounts) = client.accounts().await else {
            return;
        };
        let (Some(account), Some(composer)) = (
            accounts.into_iter().find(|found| found.id == account),
            composer.upgrade(),
        ) else {
            return;
        };
        composer.set_size_limit(account.max_message_size);
        composer.set_identities(account.identities);
        composer.set_signatures(account.signatures);
    });
}

/// What a new draft signs with: Focus has no folder selected, so the
/// account's default decides.
fn signature_default(composer: &Composer, client: &Client, account: AccountId) {
    let client = client.clone();
    composer.connect_signature_default(move |answer| {
        let client = client.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            answer(client.default_signature(account, None).await.ok().flatten());
        });
    });
}

/// Autosave through the host's draft writer, and clearing the row once there
/// is nothing left to keep. `Esc` keeps what was written (FR-051): the close
/// flushes a pending save, and a kept draft stays in Drafts.
fn autosave(composer: &Composer, frame: &Rc<Frame>, client: &Client) -> Rc<Cell<Option<DraftId>>> {
    let last_id: Rc<Cell<Option<DraftId>>> = Rc::default();
    let weak = composer.downgrade();
    composer.connect_save({
        let client = client.clone();
        let weak = weak.clone();
        let last_id = Rc::clone(&last_id);
        let frame = Rc::downgrade(frame);
        move |draft| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            let generation = composer.generation();
            // Handed over now, in the order the composer saved.
            let saved = client.save_draft(generation, draft.clone());
            let last_id = Rc::clone(&last_id);
            let weak = weak.clone();
            let frame = frame.clone();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // write runs in the host's draft writer.
                let Ok(id) = saved.await else {
                    return;
                };
                let Some(composer) = weak.upgrade() else {
                    return;
                };
                if composer.generation() == generation {
                    last_id.set(Some(id));
                    if let Some(frame) = frame.upgrade() {
                        frame.saved(chrono::Utc::now());
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

/// Sending, now or later: the draft becomes a queue row in the Outbox, and
/// nothing waits for the network (FR-055, ADR 0021).
fn send(composer: &Composer, client: &Client, last_id: Rc<Cell<Option<DraftId>>>) {
    let weak = composer.downgrade();
    composer.connect_send({
        let client = client.clone();
        let last_id = Rc::clone(&last_id);
        let weak = weak.clone();
        move |draft| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            // Taken, so the close that follows a send does not discard the
            // draft just queued (postio-app's `install_send` says why).
            last_id.set(None);
            drop(client.queue_send(composer.generation(), draft.clone(), None));
        }
    });
    composer.connect_send_later({
        let client = client.clone();
        move |draft, at| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            last_id.set(None);
            drop(client.queue_send(composer.generation(), draft.clone(), Some(at)));
        }
    });
}

/// Recipient completion from the directory, read when the composer opens
/// and ranked by the one rule both apps use (T076).
fn recipients(composer: &Composer, client: &Client, account: AccountId) {
    let directory: Rc<RefCell<Rc<RecipientDirectory>>> = Rc::default();
    let reload = {
        let directory = Rc::clone(&directory);
        let client = client.clone();
        move || {
            let client = client.clone();
            let directory = Rc::clone(&directory);
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
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

/// `e`, `E` and `f` answer the row the cursor is on. A reply starts with its
/// conversation's labels, marked as the thread's (FR-053, screen 06); a
/// forward starts a conversation of its own, and none.
fn reply_source(composer: &Composer, frame: &Rc<Frame>, client: &Client, current: Current) {
    let client = client.clone();
    let weak = composer.downgrade();
    let frame = Rc::downgrade(frame);
    composer.connect_reply_source(move |answer: ReplyAnswer| {
        let Some(message) = current() else {
            answer(None);
            return;
        };
        let client = client.clone();
        let weak = weak.clone();
        let frame = frame.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: client calls are oneshot receives; the host
            // reads the message, its body and its thread's labels.
            let found = client.reply_source(message).await.ok().flatten();
            let thread = found.as_ref().and_then(|(source, _)| source.thread_id);
            let labels = match thread {
                Some(thread) => client
                    .thread_labels(vec![thread])
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    .await
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(_, label)| label)
                    .collect(),
                None => Vec::new(),
            };
            let generation = weak.upgrade().map(|composer| composer.generation());
            answer(found);
            let (Some(composer), Some(frame)) = (weak.upgrade(), frame.upgrade()) else {
                return;
            };
            // Only a reply the answer just opened: a refusal (a draft still
            // open) leaves the composition where it was.
            let opened = Some(composer.generation()) != generation;
            if !opened || composer.draft().kind == postio_model::DraftKind::Forward {
                return;
            }
            let ids = labels.iter().map(|label| label.id).collect();
            frame.know(labels);
            composer.start_with_labels(ids);
            frame.from_the_thread(&composer);
        });
    });
}

/// The names of the labels a draft opened with, when the Labels row does
/// not know them yet: a draft resumed from Drafts carries only their ids.
fn label_names(composer: &Composer, frame: &Rc<Frame>, client: &Client, account: AccountId) {
    let client = client.clone();
    let weak = composer.downgrade();
    let frame = Rc::downgrade(frame);
    composer.connect_opened(move || {
        let (Some(composer), Some(known)) = (weak.upgrade(), frame.upgrade()) else {
            return;
        };
        if known.knows_all(&composer) {
            return;
        }
        let client = client.clone();
        let weak = weak.clone();
        let frame = frame.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            let Ok(labels) = client.labels(account).await else {
                return;
            };
            let (Some(composer), Some(frame)) = (weak.upgrade(), frame.upgrade()) else {
                return;
            };
            frame.know(labels);
            frame.draw_labels(&composer);
        });
    });
}

/// A chosen or dropped file, stored as an attachment: the type sniffed off
/// the main loop, the bytes stored by the host.
fn attach(composer: &Composer, client: &Client) {
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
/// resolver is a synchronous callback, and Focus answers it from memory
/// rather than wait on the store on the thread that draws
/// (`check-blocking-now-sites.py`).
type InlineBytes = Rc<RefCell<std::collections::HashMap<postio_model::ids::BlobId, Vec<u8>>>>;

/// A pasted image, stored as an inline part, its bytes kept for the surface
/// that shows it; a draft that opens with inline images has theirs read.
fn inline_images(composer: &Composer, client: &Client) {
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
