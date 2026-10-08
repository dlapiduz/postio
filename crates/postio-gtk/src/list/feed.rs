//! Filling Focus's list from the store's host, and keeping it in step.
//!
//! The rules are the controller's (`postio_focus`, ADR 0045): what opening a
//! place reads, how a page is shaped and spliced, what an engine event
//! re-reads. This is their GTK driver. It hands the controller what happened,
//! runs each request it asks for through `postio_focus::perform` on GTK's own
//! loop -- the client's in-process transport answers through a oneshot, so
//! the loop is never inside a query -- and applies what comes back to the
//! list model.
//!
//! Staleness is the model's: a page is asked for under the model's
//! generation, and the answer is delivered against it.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use postio_client::Client;
use postio_config::paths::Platform;
use postio_core::Event;
use postio_focus::{Effect, FocusController, Input, Intent, Policy, Request, Ticket};
use postio_model::ListScope;
use postio_widgets::list_model::{PageSource, WindowedModel};

use super::model::FocusList;

/// The list, and where its pages come from.
#[derive(Clone)]
pub struct Feed {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    list: FocusList,
    /// Focus's rules for the list. Borrowed only to hand it something and
    /// take its effects: never while they are applied, because applying them
    /// calls the model, and the model calls back.
    focus: RefCell<FocusController>,
    /// Called when a page lands or the list is re-read.
    on_filled: RefCell<Vec<Box<dyn Fn()>>>,
}

/// The `PageSource` the model holds: a handle on the feed, so a request can
/// take an owned `Rc` into the future it spawns.
struct Source(Rc<Inner>);

impl PageSource for Source {
    fn total(&self) -> u32 {
        self.0.focus.borrow().list_total()
    }

    fn request(&self, page: u32) {
        let stamp = self.0.list.generation();
        let effects = self.0.focus.borrow_mut().page_wanted(page, stamp);
        Rc::clone(&self.0).apply(effects);
    }
}

impl Feed {
    /// A feed reading through `client`, with nothing open yet.
    pub fn new(client: Client) -> Self {
        Feed {
            inner: Rc::new(Inner {
                client,
                list: FocusList::default(),
                focus: RefCell::new(FocusController::new(Policy::for_platform(
                    Platform::Freedesktop,
                ))),
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
        self.inner.focus.borrow().scope()
    }

    /// How many rows the scope in view has, as the store last said.
    pub fn total(&self) -> u32 {
        self.inner.focus.borrow().list_total()
    }

    /// The cursor's `index` among the messages of a list of `total` rows,
    /// and how many messages it holds: digests are not counted, matching
    /// the strip, which counts what the store holds.
    pub fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        self.inner.focus.borrow().message_place(index, total)
    }

    /// Every page this feed has asked the store for, in order.
    pub fn pages_asked(&self) -> Vec<u32> {
        self.inner.focus.borrow().pages_asked().to_vec()
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
        let effects = self.inner.focus.borrow_mut().open(scope);
        Rc::clone(&self.inner).apply(effects);
    }

    /// Whether a scope was opened and its first page has landed since this
    /// was last asked: true once per opening, so the cursor goes to the new
    /// list's first row once and a later re-read leaves it where it is.
    pub fn take_opened(&self) -> bool {
        self.inner.focus.borrow_mut().take_opened()
    }

    /// Whether the first page of the scope in view has landed: a list still
    /// waiting for it is loading, not empty.
    pub fn has_landed(&self) -> bool {
        self.inner.focus.borrow().has_landed()
    }

    /// Whether `mailbox` is one of the inboxes Focus's own inbox is made of.
    pub fn is_inbox(&self, mailbox: postio_model::MailboxId) -> bool {
        self.inner.focus.borrow().is_inbox(mailbox)
    }

    /// Bring the list into step with `event`, by the controller's rules.
    pub fn handle(&self, event: &Event) {
        let effects = self
            .inner
            .focus
            .borrow_mut()
            .handle(Input::Event(event.clone()));
        Rc::clone(&self.inner).apply(effects);
    }
}

impl Inner {
    /// Do what the controller said, in order.
    fn apply(self: Rc<Self>, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Show(intent) => Rc::clone(&self).show(intent),
                Effect::Ask(ticket, request) => Rc::clone(&self).ask(ticket, request),
                // The feed sets no timers.
                _ => {}
            }
        }
    }

    fn show(self: Rc<Self>, intent: Intent) {
        match intent {
            Intent::ReplaceSource { .. } => {
                let source: Rc<dyn PageSource> = Rc::new(Source(Rc::clone(&self)));
                self.list.replace_source(source, false);
                // Asked for here rather than left to the view: rows already
                // on screen stay until this page lands and the list changes
                // over, and nothing else would ask for it. An empty scope's
                // first page is empty, which is the change-over to nothing.
                let stamp = self.list.generation();
                let effects = self.focus.borrow_mut().page_wanted(0, stamp);
                self.apply(effects);
            }
            Intent::PagePending { page, .. } => {
                self.list.note_pending(page);
            }
            Intent::DeliverPage {
                stamp,
                page,
                total,
                rows,
            } => self.list.deliver_page(stamp, total, page, rows),
            Intent::AbandonPage { stamp, page } => self.list.abandon_page(stamp, page),
            Intent::GiveUp { stamp, page } => self.list.give_up(stamp, page),
            Intent::RefreshList => self.list.refresh(),
            Intent::Filled => {
                for handler in self.on_filled.borrow().iter() {
                    handler();
                }
            }
            // Not the feed's: the window draws the rest.
            _ => {}
        }
    }

    fn ask(self: Rc<Self>, ticket: Ticket, request: Request) {
        // A post is said now, before anything after it is asked.
        let request = match postio_focus::perform_now(&self.client, request) {
            Ok(reply) => {
                let effects = self.focus.borrow_mut().handle(Input::Reply(ticket, reply));
                self.apply(effects);
                return;
            }
            Err(request) => request,
        };
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: perform awaits only client calls, each a
            // oneshot receive; the host answers on its own runtime (ADR 0041).
            let reply = postio_focus::perform(&self.client, request).await;
            let effects = self.focus.borrow_mut().handle(Input::Reply(ticket, reply));
            self.apply(effects);
        });
    }
}
