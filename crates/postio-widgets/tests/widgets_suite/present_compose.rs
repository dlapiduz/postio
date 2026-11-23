use std::sync::{Arc, Mutex};

use gtk::prelude::*;
use postio_client::api::Call;
use postio_client::protocol::{Req, Resp};
use postio_client::{Client, Transport};
use postio_core::{CommandId, EventEnvelope};
use postio_model::ids::{AccountId, DraftId};
use postio_widgets::composer::Composer;

struct Scripted {
    asked: Mutex<Vec<Req>>,
}

impl Transport for Scripted {
    fn call(&self, request: Req) -> Call<'static> {
        let answer = match &request {
            Req::SaveDraft { .. } => Resp::DraftSaved(DraftId::new(1)),
            Req::QueueSend { .. } => Resp::Queued(postio_client::protocol::Queued {
                drafts: None,
                draft: postio_model::DraftId::new(1),
            }),
            _ => Resp::Done,
        };
        self.asked.lock().expect("never poisoned").push(request);
        Box::pin(async move { Ok(answer) })
    }

    fn post(&self, request: Req) {
        self.asked.lock().expect("never poisoned").push(request);
    }

    fn events(&self) -> async_channel::Receiver<EventEnvelope> {
        let (_tell, events) = async_channel::unbounded();
        events
    }
}

pub fn dispatching_send_queues_the_draft_through_the_client() {
    if adw::init().is_err() {
        return;
    }
    let account = AccountId::new(9);
    let host = Arc::new(Scripted {
        asked: Mutex::new(Vec::new()),
    });
    let client = Client::new(host.clone());

    let composer = Composer::new();
    composer.set_account(account);
    let last_id = postio_widgets::present::compose::install_autosave(&composer, &client, None);
    postio_widgets::present::compose::install_send(
        &composer, &client, last_id, account, None, None,
    );

    let window = gtk::Window::new();
    window.set_default_size(900, 700);
    window.set_child(Some(&composer));
    window.present();
    composer.set_visible(true);

    composer.dispatch(CommandId::Compose);
    assert!(
        composer.is_open(),
        "dispatching Compose did not open a draft"
    );
    // Not `test_set_body`: it types through a WebKit surface this bare window never maps.
    composer.test_set_to("quinn@example.net");
    composer.test_set_subject("Tide gate interlock");

    composer.dispatch(CommandId::Send);
    assert!(
        crate::support::until(|| host
            .asked
            .lock()
            .expect("never poisoned")
            .iter()
            .any(|request| matches!(request, Req::QueueSend { .. }))),
        "dispatching Send never reached the client"
    );

    let asked = host.asked.lock().expect("never poisoned");
    let queued = asked
        .iter()
        .find_map(|request| match request {
            Req::QueueSend { draft, at, .. } => Some((draft.clone(), *at)),
            _ => None,
        })
        .expect("a queued send");
    assert_eq!(queued.0.to.len(), 1, "the recipient that was typed");
    assert_eq!(queued.0.to[0].address, "quinn@example.net");
    assert_eq!(queued.0.subject, "Tide gate interlock");
    assert_eq!(queued.1, None, "an immediate send, not a scheduled one");
    assert!(
        !composer.is_open(),
        "sending closes the composer; the message is the queue's problem now"
    );
}
