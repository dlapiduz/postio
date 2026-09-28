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
use postio_model::listing::{ListPage, MailStore, PageRequest};
use postio_model::mailbox::MailboxRole;
use postio_model::{FocusScope, ListScope};
use postio_ui::paging::{Fetch, Paging, Plan};
use postio_ui::surfaced::{Slot, Spliced};
use postio_widgets::list_model::{PageSource, WindowedModel};

use super::item::FocusRow;
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
    /// Every page asked of the store, in order: "only the visible window is
    /// read" (US1 scenario 7) is a question about which pages.
    pages_asked: RefCell<Vec<u32>>,
    /// Whether the first page of the scope in view has landed.
    landed: Cell<bool>,
    /// Called when a page lands or the list is re-read.
    on_filled: RefCell<Vec<Box<dyn Fn()>>>,
    /// The rows Focus's inbox surfaces among its conversations, in the
    /// order the host gave them: fired reminders (T095).
    surfaced: RefCell<Vec<FocusRow>>,
    /// Where they sit.
    spliced: RefCell<Spliced>,
    /// How many conversations the store holds, without the surfaced rows.
    stored: Cell<u32>,
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
                pages_asked: RefCell::new(Vec::new()),
                landed: Cell::new(false),
                on_filled: RefCell::new(Vec::new()),
                surfaced: RefCell::default(),
                spliced: RefCell::default(),
                stored: Cell::new(0),
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

    /// Every page this feed has asked the store for, in order.
    pub fn pages_asked(&self) -> Vec<u32> {
        self.inner.pages_asked.borrow().clone()
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
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            if let Ok(accounts) = inner.client.accounts().await {
                for account in accounts.iter().filter(|account| account.enabled) {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
                    // answers on its own runtime (ADR 0041).
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
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let total = match inner.client.list_count(scope).await {
                Ok(total) => total,
                Err(error) => {
                    tracing::warn!(%error, "Focus could not count its list: {error}");
                    0
                }
            };
            Rc::clone(&inner).read_surfaced().await;
            if inner.paging.borrow().scope() != Some(scope) {
                // Another place was opened while this one was being counted.
                return;
            }
            inner.stored.set(total);
            inner.total.set(inner.spliced.borrow().total(total));
            let source: Rc<dyn PageSource> = Rc::new(Source(Rc::clone(&inner)));
            inner.list.replace_source(source, false);
            // Asked for here rather than left to the view: rows already on
            // screen stay until this page lands and the list changes over,
            // and nothing else would ask for it (the classic feed's rule).
            // An empty scope's first page is empty, which is the change-over
            // to nothing.
            Rc::clone(&inner).request(0);
        });
    }

    /// Whether the first page of the scope in view has landed: a list still
    /// waiting for it is loading, not empty.
    pub fn has_landed(&self) -> bool {
        self.inner.landed.get()
    }

    /// Whether `mailbox` is one of the inboxes Focus's own inbox is made of.
    pub fn is_inbox(&self, mailbox: postio_model::MailboxId) -> bool {
        self.inner.paging.borrow().is_inbox(mailbox)
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
        let surfaced_moved = matches!(event, Event::SurfacedChanged);
        let plan = self.inner.paging.borrow().plan(event);
        match plan {
            Plan::Ignore if !surfaced_moved => {}
            _ if self.inner.splices() => {
                // What is surfaced may have moved with the mail -- an
                // archived reminder's row goes with its conversation -- so
                // it is read again before the pages are.
                let inner = Rc::clone(&self.inner);
                glib::spawn_future_local(async move {
                    Rc::clone(&inner).read_surfaced().await;
                    inner.list.refresh();
                });
            }
            Plan::Ignore => {}
            Plan::InsertAtTop(_) | Plan::Refetch(_) | Plan::Reload => self.inner.list.refresh(),
        }
    }
}

impl Inner {
    /// Whether the scope in view has rows spliced among its conversations:
    /// Focus's own inbox.
    fn splices(&self) -> bool {
        self.paging.borrow().scope() == Some(ListScope::Focus(FocusScope::Inbox))
    }

    /// Read the surfaced rows, when the scope in view has them, and place
    /// them; none otherwise.
    async fn read_surfaced(self: Rc<Self>) {
        let mut rows = Vec::new();
        let mut positions = Vec::new();
        if self.splices() {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = self.client.surfaced().await;
            match read {
                Ok(surfaced) => {
                    for row in &surfaced {
                        if let Some(focus_row) = FocusRow::surfaced(row) {
                            positions.push(row.position());
                            rows.push(focus_row);
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "Focus could not read its surfaced rows: {error}");
                }
            }
        }
        self.spliced.replace(Spliced::new(&positions));
        self.surfaced.replace(rows);
        self.total
            .set(self.spliced.borrow().total(self.stored.get()));
    }

    /// Ask the store for `page` of the scope in view, and deliver the answer
    /// when it lands.
    fn request(self: Rc<Self>, page: u32) {
        let Some(Fetch::Scope(request)) = self.paging.borrow().fetch_for(page) else {
            self.list.give_up(self.list.generation(), page);
            return;
        };
        // The page's positions, in the store's terms: which of them are
        // surfaced rows, and which run of conversations fills the rest.
        // The store's total may have moved since it was last read, so the
        // run asked for assumes conversations fill every position that is
        // not a surfaced row, and the answer's own total places them.
        let (start, count) = (request.offset, request.limit);
        let asked = self.spliced.borrow().page(start, count, u32::MAX);
        let surfaced = self.surfaced.borrow().clone();
        self.list.note_pending(page);
        self.pages_asked.borrow_mut().push(page);
        let generation = self.list.generation();
        let client = self.client.clone();
        let wanted = PageRequest {
            scope: request.scope,
            offset: asked.offset,
            limit: asked.limit.max(1),
        };
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: the client's in-process transport runs the
            // read on the host's runtime and answers through a oneshot, which
            // any executor can await (postio-host's `Local`).
            match client.list_page(wanted).await {
                Ok(ListPage::Threads(answer)) => {
                    // The page's label pills, for every row at once: one
                    // round trip, one statement at the store (T043).
                    let threads: Vec<_> = answer
                        .rows
                        .iter()
                        .flat_map(|row| row.id.into_iter().chain(row.copies.iter().copied()))
                        .collect();
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
                    // answers on its own runtime (ADR 0041).
                    let mut labelled = match client.thread_labels(threads).await {
                        Ok(labelled) => labelled,
                        Err(error) => {
                            tracing::warn!(page, %error, "Focus could not read a page's labels");
                            Vec::new()
                        }
                    };
                    let stored: Vec<FocusRow> = answer
                        .rows
                        .into_iter()
                        .map(|summary| {
                            let mine: Vec<_> = summary
                                .id
                                .into_iter()
                                .chain(summary.copies.iter().copied())
                                .collect();
                            let mut labels = Vec::new();
                            labelled.retain(|(thread, label)| {
                                if mine.contains(thread) {
                                    if !labels
                                        .iter()
                                        .any(|held: &postio_model::Label| held.id == label.id)
                                    {
                                        labels.push(label.clone());
                                    }
                                    false
                                } else {
                                    true
                                }
                            });
                            FocusRow::Conversation(super::item::Conversation { summary, labels })
                        })
                        .collect();
                    let placed = self.spliced.borrow().page(start, count, answer.total);
                    let rows: Vec<FocusRow> = placed
                        .slots
                        .iter()
                        .filter_map(|slot| match slot {
                            Slot::Surfaced(index) => surfaced.get(*index).cloned(),
                            Slot::Stored(index) => stored.get(*index).cloned(),
                        })
                        .collect();
                    let total = self.spliced.borrow().total(answer.total);
                    if generation == self.list.generation() {
                        self.stored.set(answer.total);
                        self.total.set(total);
                    }
                    self.list.deliver_page(generation, total, page, rows);
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
