//! The composer's seams, answered through Focus's client (ADR 0041). Most are the shared ones
//! (`shared`); Focus's own send notes
//! to the frame's subtitle. A composer seam is one slot, not a signal, so `reply_source` and
//! `label_names` cannot be the plainer shared version.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use postio_client::Client;
use postio_model::ids::{AccountId, MessageId};
use postio_widgets::composer::{Composer, ReplyAnswer};
use postio_widgets::present::compose as shared;

use super::frame::Frame;

pub type Current = Rc<dyn Fn() -> Option<MessageId>>;

pub type Resume = shared::Resume;

pub type ResumeDraft = shared::ResumeDraft;

const SEND_CANCELLED: &str = "Send cancelled \u{2014} you're editing this draft again";

pub type Say = Rc<dyn Fn(&str, Rc<dyn Fn()>)>;

const QUEUED: &str = "Message queued to send";
const SCHEDULED: &str = "Send scheduled";

pub fn wire(
    composer: &Composer,
    frame: &Rc<Frame>,
    client: &Client,
    account: AccountId,
    current: Current,
    say: Say,
) -> (Resume, ResumeDraft) {
    composer.set_account(account);
    let identities_composer = composer.downgrade();
    let identities_client = client.clone();
    glib::spawn_future_local(async move {
        if let Some(composer) = identities_composer.upgrade() {
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

/// Undo takes the send off the queue and puts the draft back in the composer (#1752).
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

fn resume(
    composer: &Composer,
    client: &Client,
    last_id: Rc<std::cell::Cell<Option<postio_model::DraftId>>>,
    frame: &Rc<Frame>,
) -> (Resume, ResumeDraft) {
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
    (
        shared::install_resume(
            composer,
            client,
            Rc::clone(&last_id),
            Some(Rc::clone(&on_note)),
        ),
        shared::install_resume_draft(composer, client, last_id, Some(on_note)),
    )
}

/// A reply starts with its conversation's labels, marked as the thread's; a forward starts a
/// conversation of its own, and none.
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
            // A source whose body has not arrived quotes nothing: ask for it for next time, and say
            // so rather than leave an attribution with nothing under it.
            let unquoted = found
                .as_ref()
                .and_then(|(source, _)| postio_ui::compose::unquoted_note(source));
            if unquoted.is_some() {
                client.fetch_body(message);
            }
            answer(found);
            let (Some(composer), Some(frame)) = (weak.upgrade(), frame.upgrade()) else {
                return;
            };
            // Only a reply the answer just opened: a refusal (a draft still open) leaves the
            // composition where it was.
            let opened = Some(composer.generation()) != generation;
            if !opened {
                return;
            }
            if composer.draft().kind == postio_model::DraftKind::Forward {
                // The person asked to forward, so the carried parts are fetched now: local by the
                // time they press send (#1690).
                fetch_carried(&composer, &client, message);
            } else {
                let ids = labels.iter().map(|label| label.id).collect();
                frame.know(labels);
                composer.start_with_labels(ids);
                frame.from_the_thread(&composer);
            }
            // Last: opening a composition clears the note.
            if let Some(note) = unquoted {
                frame.note(note);
            }
        });
    });
}

/// Drawn downloading before the ask, so a part that lands faster than the answer is not drawn
/// downloading again after it landed.
/// An answer that says nothing is on its way takes the drawing back; the send's carry stage fetches
/// what is still missing (#1690).
fn fetch_carried(composer: &Composer, client: &Client, message: MessageId) {
    let carried = postio_ui::compose::carried_sections(&composer.draft().attachments);
    if carried.is_empty() {
        return;
    }
    composer.set_downloading(carried.clone());
    let generation = composer.generation();
    let weak = composer.downgrade();
    let client = client.clone();
    glib::spawn_future_local(async move {
        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
        // asks the engine to queue the parts and answers at once.
        let asked = client.fetch_carried(message, carried).await;
        let Some(composer) = weak.upgrade() else {
            return;
        };
        if composer.generation() != generation {
            return;
        }
        if !matches!(asked, Ok(postio_model::fetch::PayloadAsk::Queued)) {
            composer.set_downloading(Vec::new());
        }
    });
}

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
