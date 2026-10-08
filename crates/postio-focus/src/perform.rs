//! The one place a [`Request`] becomes a client call.
//!
//! Async, and tied to no executor: the GTK app awaits it on glib's main loop
//! and the FFI on tokio. Both get the same mapping, which is the point — the
//! aim of a command, the order of a scope's reads, and the shape of each
//! reply are decided here once (research R1).

use postio_client::Client;
use postio_model::listing::{ListPage, MailStore};
use postio_model::mailbox::MailboxRole;

use crate::{Opened, PageAnswer, Reply, Request};

/// Answer `request` now, when it needs no await: a post, said before
/// anything that follows it is asked. `Err` hands the request back for
/// [`perform()`].
///
/// Order is the point. Mail that left a folder is said to the store before
/// the list re-reads (`Request::NoteRemoved`), so the store's own caches
/// have let go of it first; spawned on an executor, the saying could run
/// after the re-read it was meant to precede.
pub fn perform_now(client: &Client, request: Request) -> Result<Reply, Request> {
    match request {
        Request::NoteRemoved { mailbox, messages } => {
            client.note_removed(mailbox, messages);
            Ok(Reply::Noted)
        }
        other => Err(other),
    }
}

/// Ask the engine; return what the controller is to be told.
pub async fn perform(client: &Client, request: Request) -> Reply {
    match request {
        Request::FocusCounts => Reply::FocusCounts(
            client
                .focus_counts()
                .await
                .map_err(|error| error.to_string()),
        ),
        Request::OpenScope { scope, splices } => {
            // Which folders are inboxes: what lets Focus's inbox ignore mail
            // moving anywhere else rather than re-read on every arrival.
            let mut folders = Vec::new();
            if let Ok(accounts) = client.accounts().await {
                for account in accounts.iter().filter(|account| account.enabled) {
                    if let Ok(mailboxes) = client.mailboxes(account.id).await {
                        folders.extend(
                            mailboxes
                                .iter()
                                .map(|mailbox| (mailbox.id, mailbox.role == MailboxRole::Inbox)),
                        );
                    }
                }
            }
            let total = client
                .list_count(scope)
                .await
                .map_err(|error| error.to_string());
            let surfaced = match splices {
                true => Some(client.surfaced().await.map_err(|error| error.to_string())),
                false => None,
            };
            Reply::Opened(Opened {
                scope,
                folders,
                total,
                surfaced,
            })
        }
        Request::Page {
            page,
            stamp,
            start,
            count,
            wanted,
        } => {
            let answer = match client.list_page(wanted).await {
                Ok(ListPage::Threads(answer)) => {
                    // The page's label pills, for every row at once: one round
                    // trip, one statement at the store (spec 007 T043).
                    let threads = postio_ui::focus_list::label_threads(&answer.rows);
                    let labels = client.thread_labels(threads).await.unwrap_or_else(|error| {
                        tracing::warn!(page, %error, "Focus could not read a page's labels");
                        Vec::new()
                    });
                    Ok(PageAnswer::Threads {
                        total: answer.total,
                        rows: answer.rows,
                        labels,
                    })
                }
                Ok(ListPage::Messages(answer)) => Ok(PageAnswer::Messages {
                    total: answer.total,
                    rows: answer.rows,
                }),
                Err(error) => Err(error.to_string()),
            };
            Reply::Page {
                page,
                stamp,
                start,
                count,
                answer,
            }
        }
        Request::Surfaced => {
            Reply::Surfaced(client.surfaced().await.map_err(|error| error.to_string()))
        }
        Request::NoteRemoved { mailbox, messages } => {
            client.note_removed(mailbox, messages);
            Reply::Noted
        }
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
