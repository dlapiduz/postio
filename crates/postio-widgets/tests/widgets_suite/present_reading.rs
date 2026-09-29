//! The reader's remote images, fetched through the shared glue (spec 006
//! FR-025; specs/007-postio-focus T022): the reader asks by the document's
//! own spelling, the app's fetcher answers by parsed URL on its runtime, and
//! what arrives is handed back on the main loop under the spelling it was
//! asked by. A failed image is absent, not an empty picture.
//!
//! [`cid_source`](postio_widgets::present::reading::cid_source) is the other
//! half of the reader's images (T022's remainder): a `cid:` reference,
//! resolved through `postio-client` rather than a direct store read, scoped
//! to whichever message the pane is showing.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use postio_client::api::Call;
use postio_client::protocol::{Req, Resp};
use postio_client::{Client, Transport};
use postio_core::EventEnvelope;
use postio_model::MessageId;

use crate::support::until;

/// A host that answers `InlinePart` for exactly one message, and records
/// every request it was asked.
struct Scripted {
    known: MessageId,
    bytes: (Vec<u8>, String),
    asked: Mutex<Vec<Req>>,
}

impl Transport for Scripted {
    fn call(&self, request: Req) -> Call<'static> {
        let answer = match &request {
            Req::InlinePart { message, .. } if *message == self.known => {
                Resp::InlinePart(Some(self.bytes.clone()))
            }
            Req::InlinePart { .. } => Resp::InlinePart(None),
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

/// What came back to the main loop, once it has.
type Arrived = Rc<RefCell<Option<Vec<(String, Vec<u8>)>>>>;

pub fn fetched_images_come_back_under_the_documents_spelling() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .expect("a runtime");
    let fetch = postio_widgets::present::reading::remote_fetch(
        runtime.handle().clone(),
        |urls: Vec<url::Url>| async move {
            urls.into_iter()
                .map(|url| {
                    let bytes = url.path().ends_with("map.png").then(|| b"PNG".to_vec());
                    (url, bytes)
                })
                .collect()
        },
    );
    let arrived: Arrived = Rc::default();
    fetch(
        vec![
            "https://Images.invalid/map.png".to_owned(),
            "https://images.invalid/gone.png".to_owned(),
            "not a url".to_owned(),
        ],
        Box::new({
            let arrived = arrived.clone();
            move |images| {
                arrived.replace(Some(images));
            }
        }),
    );
    assert!(
        until(|| arrived.borrow().is_some()),
        "nothing came back to the main loop"
    );
    assert_eq!(
        arrived.borrow().clone().expect("an answer"),
        [("https://Images.invalid/map.png".to_owned(), b"PNG".to_vec())],
        "the image did not come back under the spelling the document asked by, \
         or a failed one came back at all"
    );
}

/// A `cid:` reference resolves against whichever message the pane is
/// showing, through the client -- never a direct store read.
pub fn a_cid_reference_resolves_through_the_client_for_the_message_on_screen() {
    let shown = MessageId::new(1);
    let host = Arc::new(Scripted {
        known: shown,
        bytes: (b"logo bytes".to_vec(), "image/png".to_owned()),
        asked: Mutex::new(Vec::new()),
    });
    let client = Client::new(host.clone());
    let source = postio_widgets::present::reading::cid_source(move || Some(shown), client);

    let resolved = source.resolve("logo@example.invalid");

    assert_eq!(
        resolved,
        Some((b"logo bytes".to_vec(), "image/png".to_owned())),
        "an inline part on the message the pane is showing did not resolve"
    );
    let asked = host.asked.lock().expect("never poisoned");
    assert_eq!(
        asked.len(),
        1,
        "expected exactly one request to the client, got {asked:?}"
    );
}

/// Nothing is showing: the source answers nothing, and never asks the host
/// at all -- there is no message to scope the request to.
pub fn nothing_showing_resolves_to_nothing_without_asking_the_host() {
    let host = Arc::new(Scripted {
        known: MessageId::new(1),
        bytes: (b"logo bytes".to_vec(), "image/png".to_owned()),
        asked: Mutex::new(Vec::new()),
    });
    let client = Client::new(host.clone());
    let source = postio_widgets::present::reading::cid_source(|| None, client);

    assert_eq!(source.resolve("logo@example.invalid"), None);
    assert!(
        host.asked.lock().expect("never poisoned").is_empty(),
        "asked the host with nothing on screen to scope the request to"
    );
}
