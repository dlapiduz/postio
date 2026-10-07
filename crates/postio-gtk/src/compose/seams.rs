//! The composer's seams, answered through Focus's client (ADR 0041).
//!
//! Most of these are the shared seams, answered by the one implementation,
//! `postio_widgets::present::compose` (`shared` below; specs/007-postio-focus
//! T022). What is Focus's own is its
//! dialog's chrome: [`resume`]'s and [`autosave`]'s notes go to the frame's
//! subtitle rather than a toast or a status line, and [`reply_source`]
//! additionally reads the thread's labels and draws them (R15), which
//! [`label_names`] fills in the names of when a resumed draft opens knowing
//! only their ids. A composer's seam is one slot, not a signal several
//! listeners share, so neither `reply_source` nor `label_names` can be the
//! shared, plainer version -- see `present::compose`'s own module doc.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_model::ids::{AccountId, MessageId};
use postio_widgets::composer::{Composer, ReplyAnswer};
use postio_widgets::present::compose as shared;

use super::frame::Frame;

/// Which message `e`, `E` and `f` answer: the row the cursor is on.
pub type Current = Rc<dyn Fn() -> Option<MessageId>>;

/// Open the draft behind a Drafts row for editing.
pub type Resume = shared::Resume;

/// What the frame's subtitle says once opening a queued draft has cancelled
/// its send (#433).
const SEND_CANCELLED: &str = "Send cancelled \u{2014} you're editing this draft again";

/// What a send says to the person once it is queued: a sentence and what
/// Undo does, in the window's toast.
pub type Say = Rc<dyn Fn(&str, Rc<dyn Fn()>)>;

/// What the toast says of a send that waits in the Outbox: now, or at a
/// time the person chose.
const QUEUED: &str = "Message queued to send";
const SCHEDULED: &str = "Send scheduled";

/// Answer every seam of `composer` through `client`, for `account`; the
/// answer is how a Drafts row opens its draft here. `say` is how a queued
/// send tells the person, with an Undo that takes it back.
pub fn wire(
    composer: &Composer,
    frame: &Rc<Frame>,
    client: &Client,
    account: AccountId,
    current: Current,
    say: Say,
) -> Resume {
    composer.set_account(account);
    let identities_composer = composer.downgrade();
    let identities_client = client.clone();
    glib::spawn_future_local(async move {
        if let Some(composer) = identities_composer.upgrade() {
            // Focus has no conversation pane and marks nothing from the
            // account row's own use of it, so the account this answers with goes
            // nowhere else.
            // POSTIO-GLIB-SAFE: `install_identities` awaits one client call,
            // a oneshot receive; the host answers on its own runtime.
            shared::install_identities(&composer, &identities_client, account).await;
        }
    });
    // Focus has no folder selected, so the account's default decides.
    shared::install_signature_default(composer, client, account, || None);
    let last_id = autosave(composer, frame, client);
    let on_queued = queued(composer, frame, client, Rc::clone(&last_id), say);
    shared::install_send(
        composer,
        client,
        Rc::clone(&last_id),
        account,
        None,
        Some(on_queued),
    );
    shared::install_recipients(composer, client, account);
    reply_source(composer, frame, client, current);
    label_names(composer, frame, client, account);
    shared::install_attach(composer, client);
    shared::install_inline_images(composer, client);
    resume(composer, client, last_id, frame)
}

/// A send, once queued, is said in a toast that offers Undo for as long as
/// the toast stays: Undo takes the send off the queue and puts the draft
/// back in the composer, as opening it from the Outbox does (#1752).
fn queued(
    composer: &Composer,
    frame: &Rc<Frame>,
    client: &Client,
    last_id: Rc<std::cell::Cell<Option<postio_model::DraftId>>>,
    say: Say,
) -> shared::OnQueued {
    let composer = composer.downgrade();
    let frame = Rc::downgrade(frame);
    let client = client.clone();
    Rc::new(move |queued, at| {
        let undo: Rc<dyn Fn()> = {
            let composer = composer.clone();
            let frame = frame.clone();
            let client = client.clone();
            let last_id = Rc::clone(&last_id);
            Rc::new(move || {
                let composer = composer.clone();
                let frame = frame.clone();
                let client = client.clone();
                let last_id = Rc::clone(&last_id);
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                    let Ok(Some(draft)) = client.cancel_send(queued.draft).await else {
                        return;
                    };
                    let (Some(composer), Some(frame)) = (composer.upgrade(), frame.upgrade())
                    else {
                        return;
                    };
                    last_id.set(Some(draft.id));
                    composer.resume(draft);
                    frame.note(SEND_CANCELLED);
                });
            })
        };
        say(if at.is_some() { SCHEDULED } else { QUEUED }, undo);
    })
}

/// Autosave, with the frame's "Draft saved locally HH:MM" subtitle stamped from
/// [`shared::install_autosave`]'s own hook.
fn autosave(
    composer: &Composer,
    frame: &Rc<Frame>,
    client: &Client,
) -> Rc<std::cell::Cell<Option<postio_model::DraftId>>> {
    let frame = Rc::downgrade(frame);
    let on_saved: shared::OnSaved = Rc::new(move |_id| {
        if let Some(frame) = frame.upgrade() {
            frame.saved(postio_ui::clock::now().to_utc());
        }
    });
    shared::install_autosave(composer, client, Some(on_saved))
}

/// A draft left in Drafts -- by Focus or by another client -- opens in the
/// composer for editing (US3 scenario 3, US11 scenario 3), through
/// [`shared::install_resume`]; the note it hands back becomes the frame's
/// subtitle rather than a toast or status line.
fn resume(
    composer: &Composer,
    client: &Client,
    last_id: Rc<std::cell::Cell<Option<postio_model::DraftId>>>,
    frame: &Rc<Frame>,
) -> Resume {
    let frame = Rc::downgrade(frame);
    let on_note: shared::OnResumeNote = Rc::new(move |note| {
        let Some(frame) = frame.upgrade() else {
            return;
        };
        match note {
            shared::ResumeNote::Cancelled => frame.note(SEND_CANCELLED),
            shared::ResumeNote::Failed(reason) => {
                frame.note(&format!("Not sent \u{2014} {reason}"));
            }
        }
    });
    shared::install_resume(composer, client, last_id, Some(on_note))
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
