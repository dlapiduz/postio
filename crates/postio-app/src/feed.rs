//! The adapter: `postio-core`'s store, as the frontend's sources.
//!
//! Two traits meet here and neither crate can name the other's. `postio-gtk`
//! asks for rows through [`MessageSource`] and folders through
//! [`MailboxSource`]; `postio-core` answers through [`MailStore`]. Joining
//! them is composition, which is what this crate is for — and it is why the
//! join lives here rather than in either of them.
//!
//! # Crossing the two loops
//!
//! The frontend's futures are awaited by `glib::spawn_future_local` on the
//! GTK main context. The store's are tokio futures: they end in
//! `spawn_blocking`, which needs a tokio runtime to be polled at all. Neither
//! loop can drive the other.
//!
//! So the request is `spawn`ed onto the tokio runtime and the answer comes
//! back over an `async_channel`, which both loops can wait on. The future the
//! frontend awaits is a channel receive and nothing more, so the GTK main
//! loop is never inside a query — which is the rule the whole seam exists to
//! keep.

use std::rc::Rc;
use std::sync::Arc;

use postio_gtk::feed::{
    MailboxFuture, MailboxSource, MessageSource, Page, PageFuture, PageRequest, ResultSource,
    RowsFuture,
};
use postio_gtk::list::Row;
use postio_model::ListScope;
use postio_model::ids::{AccountId, MessageId};
use postio_runtime::store::{ListPage, MailStore, PageRequest as StoreRequest};

/// The frontend's two sources, over one store.
pub struct Sources {
    store: Arc<dyn MailStore>,
    /// The runtime the reads are polled on. Cloning a `Handle` is cheap and
    /// gives another way into the same runtime.
    runtime: tokio::runtime::Handle,
}

impl Sources {
    /// Read `store`, on `runtime`.
    pub fn new(store: Arc<dyn MailStore>, runtime: tokio::runtime::Handle) -> Rc<Self> {
        Rc::new(Sources { store, runtime })
    }

    /// Run `read` on the runtime and hand the answer back over a channel.
    ///
    /// One slot: there is exactly one answer, and a sender that cannot block
    /// is a sender that cannot hold up the runtime.
    fn ask<T, F>(&self, read: F) -> async_channel::Receiver<Result<T, String>>
    where
        T: Send + 'static,
        F: FnOnce(Arc<dyn MailStore>) -> SendRead<T>,
    {
        let (sender, receiver) = async_channel::bounded(1);
        let future = read(self.store.clone());
        self.runtime.spawn(async move {
            let answer = future.await.map_err(|error| error.to_string());
            let _ = sender.send(answer).await;
        });
        receiver
    }
}

/// The shape `ask` takes: a `Send` future the runtime can poll.
type SendRead<T> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<T, postio_runtime::store::StoreError>> + Send>,
>;

impl MessageSource for Sources {
    fn fetch(&self, request: PageRequest) -> PageFuture {
        let wanted = StoreRequest {
            scope: request.scope,
            offset: request.offset,
            limit: request.limit,
        };
        // Which window answers is the store's decision, not this one's:
        // folders thread, query views list messages, and Drafts is a folder
        // that does not (ADR 0015, and `SqliteStore::lists_conversations`).
        // Asking here would put that line in two places, and the list model,
        // the row widget and the verbs all work from what comes back rather
        // than from a mode any of them keeps.
        let answer = self.ask(move |store| Box::pin(async move { store.list_page(wanted).await }));
        Box::pin(async move {
            match answer.recv().await {
                Ok(Ok(page)) => {
                    let page = match page {
                        ListPage::Threads(page) => Page {
                            total: page.total,
                            rows: page.rows.into_iter().map(thread_row).collect(),
                        },
                        ListPage::Messages(page) => Page {
                            total: page.total,
                            rows: page.rows.into_iter().map(row).collect(),
                        },
                    };
                    // The list asking, and what it got. A list that draws
                    // nothing is either a model that never asked or a store
                    // that answered empty, and only this line tells them
                    // apart — which is the whole of `postio-qhz.7`.
                    tracing::debug!(
                        scope = %scope_name(request.scope),
                        page = request.page,
                        offset = request.offset,
                        rows = page.rows.len(),
                        total = page.total,
                        "list page read"
                    );
                    Ok(page)
                }
                Ok(Err(reason)) => Err(reason),
                // The runtime went away mid-read. Rare, and still worth a
                // sentence rather than a blank list.
                Err(_) => Err("the runtime stopped before the page arrived".to_string()),
            }
        })
    }
}

impl ResultSource for Sources {
    fn rows(&self, ids: Vec<MessageId>) -> RowsFuture {
        let wanted = ids.len();
        let answer = self.ask(move |store| Box::pin(async move { store.message_rows(ids).await }));
        Box::pin(async move {
            match answer.recv().await {
                Ok(Ok(rows)) => {
                    // `wanted` and `rows` are logged together on purpose: the
                    // store drops ids it no longer holds, so a short answer is
                    // legitimate -- but it is also what a broken read looks
                    // like, and only the two numbers side by side tell them
                    // apart. Counts, never the query: a logged search is a
                    // logged mailbox.
                    tracing::debug!(wanted, rows = rows.len(), "search result page read");
                    Ok(rows.into_iter().map(row).collect())
                }
                Ok(Err(reason)) => Err(reason),
                Err(_) => Err("the runtime stopped before the results arrived".to_string()),
            }
        })
    }
}

impl MailboxSource for Sources {
    fn mailboxes(&self, account: AccountId) -> MailboxFuture {
        let answer = self.ask(move |store| Box::pin(async move { store.mailboxes(account).await }));
        Box::pin(async move {
            match answer.recv().await {
                Ok(Ok(mailboxes)) => {
                    tracing::debug!(count = mailboxes.len(), "folder list read");
                    Ok(mailboxes)
                }
                Ok(Err(reason)) => Err(reason),
                Err(_) => Err("the runtime stopped before the folders arrived".to_string()),
            }
        })
    }
}

/// What a page read was asked for, for the log line above.
///
/// An id and a role, never a name: a folder's name is the user's, and
/// `CLAUDE.md` puts mailbox names on the wrong side of the line that keeps
/// logs free of anybody's mail.
fn scope_name(scope: ListScope) -> String {
    match scope {
        ListScope::Mailbox(id) => format!("mailbox {}", id.get()),
        ListScope::Account(account) => format!("account {}", account.get()),
        ListScope::Flagged(account) => format!("flagged in account {}", account.get()),
        ListScope::Snoozed(account) => format!("snoozed in account {}", account.get()),
        ListScope::Thread(id) => format!("thread {}", id.get()),
    }
}

/// One conversation, as the list draws it.
///
/// The row *is* its representative message — the newest one in this folder —
/// so every surface that already takes a message keeps working: the reading
/// pane opens it, a drag exports it, a reply answers it. What the thread
/// contributes is what the row says about itself.
fn thread_row(summary: postio_runtime::store::ThreadSummary) -> Row {
    let seen = !summary.has_unread();
    Row {
        // The representative's id, deliberately. `thread` is what makes this
        // a conversation to the verbs; `id` is what makes it openable.
        id: summary.representative.id,
        thread: summary.id,
        from: summary.representative.from,
        // The conversation's subject, not the newest reply's: "Re: Re: Fwd:"
        // is not what the row is about.
        subject: summary.subject,
        preview: summary.representative.preview,
        // The conversation's recency, which is what the sort key is.
        received_at: summary.last_at,
        // Unread when anything in *this folder's* slice is unread. A thread
        // whose only unread member is filed elsewhere reads as handled here.
        seen,
        flagged: summary.flagged,
        answered: summary.representative.answered,
        draft: summary.representative.draft,
        has_attachments: summary.has_attachments,
        thread_count: summary.message_count,
        participants: summary.participants,
    }
}

/// One row, as the list draws it.
///
/// Field for field: the two types exist separately so that neither crate has
/// to depend on the other's, not because they disagree about what a row is.
fn row(summary: postio_runtime::store::MessageSummary) -> Row {
    Row {
        id: summary.id,
        thread: summary.thread,
        from: summary.from,
        subject: summary.subject,
        preview: summary.preview,
        received_at: summary.received_at,
        seen: summary.seen,
        flagged: summary.flagged,
        answered: summary.answered,
        draft: summary.draft,
        has_attachments: summary.has_attachments,
        thread_count: summary.thread_count,
        participants: Vec::new(),
    }
}
