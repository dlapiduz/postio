//! Filling Focus's list from the store's host, and keeping it in step.
//!
//! The list asks for the pages it is about to draw ([`PageSource`]), and this
//! reads them through the client -- the one way Focus reaches mail (ADR 0041)
//! -- and hands them back on the main loop. The client's in-process
//! transport answers through a oneshot, so its futures are awaited on GTK's
//! own loop and the loop is never inside a query.
//!
//! What an event does to the list is `postio_ui::paging`'s one policy: Focus's
//! inbox never inserts an arrival at the top, because Focus may hold it for a
//! digest or file it away first, and only a re-read of what is on screen can
//! say which (`ListScope::reaction`).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use postio_client::Client;
use postio_core::Event;
use postio_model::ListScope;
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_ui::paging::{Fetch, Paging, Plan};
use postio_widgets::list_model::{PageSource, WindowedModel};

use super::item::FocusItem;
use super::model::FocusList;

/// The list, and where its pages come from.
#[derive(Clone)]
pub struct Feed {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    list: FocusList,
    paging: RefCell<Paging>,
    /// The row count the store last gave for the scope in view.
    total: Cell<u32>,
    /// Pages asked of the store, ever: "only the visible window is read" is
    /// a count (US1 scenario 7), and this is it at the list's end.
    pages_read: Cell<u64>,
    /// Whether the first page of the scope in view has landed.
    landed: Cell<bool>,
    /// Called when a page lands or the list is re-read.
    on_filled: RefCell<Vec<Box<dyn Fn()>>>,
}

/// The `PageSource` the model holds: a handle on the feed, so a request can
/// take an owned `Rc` into the future it spawns.
struct Source(Rc<Inner>);

impl PageSource for Source {
    fn total(&self) -> u32 {
        self.0.total.get()
    }

    fn request(&self, page: u32) {
        Rc::clone(&self.0).request(page);
    }
}

impl Feed {
    /// A feed reading through `client`, with nothing open yet.
    pub fn new(client: Client) -> Self {
        Feed {
            inner: Rc::new(Inner {
                client,
                list: FocusList::default(),
                paging: RefCell::new(Paging::default()),
                total: Cell::new(0),
                pages_read: Cell::new(0),
                landed: Cell::new(false),
                on_filled: RefCell::new(Vec::new()),
            }),
        }
    }

    /// The list model a view draws.
    pub fn list(&self) -> &FocusList {
        &self.inner.list
    }

    /// The scope in view, once one is open.
    pub fn scope(&self) -> Option<ListScope> {
        self.inner.paging.borrow().scope()
    }

    /// How many rows the scope in view has, as the store last said.
    pub fn total(&self) -> u32 {
        self.inner.total.get()
    }

    /// How many pages this feed has asked the store for.
    pub fn pages_read(&self) -> u64 {
        self.inner.pages_read.get()
    }

    /// Call `handler` each time rows land: a first page, a re-read.
    pub fn connect_filled(&self, handler: impl Fn() + 'static) {
        self.inner.on_filled.borrow_mut().push(Box::new(handler));
    }

    /// Show `scope`: count it, then let the list ask for what it draws.
    ///
    /// The rows already on screen stay until the new scope's first page
    /// lands (`replace_source`), so a change of place never flashes empty.
    pub fn open(&self, scope: ListScope) {
        self.inner.paging.borrow_mut().open(scope);
        self.inner.landed.set(false);
        let inner = Rc::clone(&self.inner);
        glib::spawn_future_local(async move {
            // Which folders are inboxes: what lets Focus's inbox ignore mail
            // moving anywhere else rather than re-read on every arrival.
            let mut folders = Vec::new();
            if let Ok(accounts) = inner.client.accounts().await {
                for account in accounts.iter().filter(|account| account.enabled) {
                    if let Ok(mailboxes) = inner.client.mailboxes(account.id).await {
                        folders.extend(
                            mailboxes
                                .iter()
                                .map(|mailbox| (mailbox.id, mailbox.role == MailboxRole::Inbox)),
                        );
                    }
                }
            }
            inner.paging.borrow_mut().set_folders(folders);
            let total = match inner.client.list_count(scope).await {
                Ok(total) => total,
                Err(error) => {
                    tracing::warn!(%error, "Focus could not count its list: {error}");
                    0
                }
            };
            if inner.paging.borrow().scope() != Some(scope) {
                // Another place was opened while this one was being counted.
                return;
            }
            inner.total.set(total);
            let source: Rc<dyn PageSource> = Rc::new(Source(Rc::clone(&inner)));
            inner.list.replace_source(source, false);
            if total == 0 {
                inner.landed.set(true);
                inner.filled();
            }
        });
    }

    /// Whether the first page of the scope in view has landed: a list still
    /// waiting for it is loading, not empty.
    pub fn has_landed(&self) -> bool {
        self.inner.landed.get()
    }

    /// Bring the list into step with `event`, by `postio_ui::paging`'s table.
    pub fn handle(&self, event: &Event) {
        if let Event::MessagesRemoved {
            mailbox, messages, ..
        } = event
        {
            // Said before the list re-reads, so the store's own caches have
            // already let go of what left.
            self.inner.client.note_removed(*mailbox, messages.clone());
        }
        let plan = self.inner.paging.borrow().plan(event);
        match plan {
            Plan::Ignore => {}
            Plan::InsertAtTop(_) | Plan::Refetch(_) | Plan::Reload => self.inner.list.refresh(),
        }
    }
}

impl Inner {
    /// Ask the store for `page` of the scope in view, and deliver the answer
    /// when it lands.
    fn request(self: Rc<Self>, page: u32) {
        let Some(Fetch::Scope(request)) = self.paging.borrow().fetch_for(page) else {
            self.list.give_up(self.list.generation(), page);
            return;
        };
        self.list.note_pending(page);
        self.pages_read.set(self.pages_read.get() + 1);
        let generation = self.list.generation();
        let client = self.client.clone();
        let wanted = PageRequest {
            scope: request.scope,
            offset: request.offset,
            limit: request.limit,
        };
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: the client's in-process transport runs the
            // read on the host's runtime and answers through a oneshot, which
            // any executor can await (postio-host's `Local`).
            match client.list_page(wanted).await {
                Ok(ListPage::Threads(answer)) => {
                    let rows = answer
                        .rows
                        .into_iter()
                        .map(FocusItem::Conversation)
                        .collect();
                    if generation == self.list.generation() {
                        self.total.set(answer.total);
                    }
                    self.list.deliver_page(generation, answer.total, page, rows);
                    self.landed.set(true);
                    self.filled();
                }
                Ok(ListPage::Messages(_)) => {
                    // Every scope Focus opens lists conversations; a scope
                    // that lists messages is a place this list does not draw.
                    tracing::warn!(page, "Focus asked a scope that lists messages");
                    self.list.give_up(generation, page);
                }
                Err(error) => {
                    tracing::warn!(page, %error, "Focus could not read a page: {error}");
                    if self.paging.borrow_mut().retry(page) {
                        self.list.abandon_page(generation, page);
                        Rc::clone(&self).request(page);
                    } else {
                        self.list.give_up(generation, page);
                    }
                }
            }
        });
    }

    fn filled(&self) {
        for handler in self.on_filled.borrow().iter() {
            handler();
        }
    }
}
