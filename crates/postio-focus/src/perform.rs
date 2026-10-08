//! The one place a [`Request`] becomes a client call.
//!
//! Async, and tied to no executor: the GTK app awaits it on glib's main loop
//! and the FFI on tokio. Both get the same mapping, which is the point — the
//! aim of a command, the order of a scope's reads, and the shape of each
//! reply are decided here once (research R1).

use postio_client::Client;

use crate::{Reply, Request};

/// Ask the engine; return what the controller is to be told.
pub async fn perform(client: &Client, request: Request) -> Reply {
    match request {
        Request::FocusCounts => Reply::FocusCounts(
            client
                .focus_counts()
                .await
                .map_err(|error| error.to_string()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};

    use postio_client::Transport;
    use postio_client::api::Call;
    use postio_client::protocol::{FocusCounts, Req, Resp};
    use postio_core::EventEnvelope;

    use super::*;

    /// Answers from a script and records what it was asked.
    struct Scripted {
        asked: Mutex<Vec<Req>>,
        answers: Mutex<Vec<Resp>>,
        events: async_channel::Receiver<EventEnvelope>,
    }

    impl Transport for Scripted {
        fn call(&self, request: Req) -> Call<'static> {
            self.asked.lock().unwrap().push(request);
            let answer = self.answers.lock().unwrap().remove(0);
            Box::pin(async move { Ok(answer) })
        }
        fn post(&self, request: Req) {
            self.asked.lock().unwrap().push(request);
        }
        fn events(&self) -> async_channel::Receiver<EventEnvelope> {
            self.events.clone()
        }
    }

    fn client(answers: Vec<Resp>) -> (Client, Arc<Scripted>) {
        let (_tx, events) = async_channel::unbounded();
        let transport = Arc::new(Scripted {
            asked: Mutex::new(Vec::new()),
            answers: Mutex::new(answers),
            events,
        });
        (Client::new(transport.clone()), transport)
    }

    /// The scripted transport answers at once, so one poll finishes it.
    fn now<F: Future>(future: F) -> F::Output {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("a scripted answer is ready on the first poll"),
        }
    }

    #[test]
    fn counts_are_one_focus_counts_request() {
        let counts = FocusCounts {
            conversations: 312,
            unread: 41,
            has_action: 7,
            filtered_today: 186,
        };
        let (client, transport) = client(vec![Resp::FocusCounts(counts)]);
        assert_eq!(
            now(perform(&client, Request::FocusCounts)),
            Reply::FocusCounts(Ok(counts)),
        );
        assert_eq!(*transport.asked.lock().unwrap(), vec![Req::FocusCounts]);
    }
}
