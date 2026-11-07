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
use postio_core::{CommandId, Event};
use postio_focus::{Effect, FocusController, Input, Intent, Policy, Request, Ticket};
use postio_model::{ListScope, MessageId, ThreadId};
use postio_widgets::list_model::{PageSource, WindowedModel};

use super::model::FocusList;

/// The conversations each selected row stands for.
pub type SelectionReach = std::collections::HashMap<MessageId, Vec<ThreadId>>;

/// What the window does with an intent it draws.
type IntentHandler = Box<dyn Fn(&Intent)>;

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
    /// Called with each intent that is the window's to draw: the cursor,
    /// the selection, the heading, the strip's counts, the empty page.
    on_intent: RefCell<Vec<IntentHandler>>,
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
                on_intent: RefCell::new(Vec::new()),
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

    /// Call `handler` with each intent the window draws: where the cursor
    /// is, what is selected, the list's heading, the strip's counts, the
    /// page that stands in for an empty list.
    pub fn connect_intent(&self, handler: impl Fn(&Intent) + 'static) {
        self.inner.on_intent.borrow_mut().push(Box::new(handler));
    }

    /// Run `id` through the controller: the cursor's and the selection's
    /// commands, `!`, and Back's last rung.
    pub fn command(&self, id: CommandId) {
        self.input(Input::Command(id));
    }

    /// Tell the controller `input`, over the rows as they are held, and do
    /// what it says.
    pub fn input(&self, input: Input) {
        let effects = self
            .inner
            .focus
            .borrow_mut()
            .handle_on(input, &self.inner.list);
        Rc::clone(&self.inner).apply(effects);
    }

    /// The search result at `position`, drawn; `None` while its page is on
    /// its way, which [`want_result`](Self::want_result) asks for.
    pub fn result_row(&self, position: u64) -> Option<postio_focus::ResultRow> {
        self.inner.focus.borrow().result_row(position)
    }

    /// The People tab's row at `position`, drawn; `None` past the last, or
    /// before the rows are read.
    pub fn result_person(&self, position: u64) -> Option<postio_focus::PersonRow> {
        self.inner.focus.borrow().result_person(position)
    }

    /// The Files tab's card at `position`, drawn; `None` past the last, or
    /// before the cards are read.
    pub fn result_file(&self, position: u64) -> Option<postio_focus::FileCard> {
        self.inner.focus.borrow().result_file(position)
    }

    /// The results view wants the row at `position`, which
    /// [`result_row`](Self::result_row) could not give: its page is read,
    /// and `Intent::ResultsPage` says when it lands.
    pub fn want_result(&self, position: u64) {
        let effects = self.inner.focus.borrow_mut().results_wanted(position);
        Rc::clone(&self.inner).apply(effects);
    }

    /// `Tab` in the bar's field: whether the controller used it, stepping
    /// into the chips; `false` leaves the key to the toolkit.
    pub fn bar_tab(&self) -> bool {
        let effects = self
            .inner
            .focus
            .borrow_mut()
            .handle_on(Input::BarTab, &self.inner.list);
        let used = !effects.is_empty();
        Rc::clone(&self.inner).apply(effects);
        used
    }

    /// Whether `message` is open among a search's hits rather than as the
    /// list's row (spec 009 slice 8).
    pub fn among_hits(&self, message: MessageId) -> bool {
        self.inner.focus.borrow().among_hits(message)
    }

    /// The folders popover's places whose names hold `filter`, each with
    /// the token [`Input::OpenPlace`] goes to it by, from the places the
    /// controller last read.
    pub fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.inner.focus.borrow().places(filter)
    }

    /// The Contacts list's row at `index`, or `None` while its page is on its
    /// way: a miss asks the store for it, and `ContactsRows` says when to
    /// draw again.
    pub fn contacts_row(&self, index: u32) -> Option<postio_ui::contacts_view::RowView> {
        let effects = {
            let mut focus = self.inner.focus.borrow_mut();
            if let Some(row) = focus.contacts_row(index) {
                return Some(row);
            }
            focus.contacts_wanted(index)
        };
        Rc::clone(&self.inner).apply(effects);
        None
    }

    /// How many people the Contacts list holds.
    pub fn contacts_total(&self) -> u32 {
        self.inner.focus.borrow().contacts_total()
    }

    /// Where the Contacts cursor is.
    pub fn contacts_cursor(&self) -> Option<u32> {
        self.inner.focus.borrow().contacts_cursor()
    }

    /// The card beside the Contacts list, once it has been read.
    pub fn contacts_card(&self) -> Option<postio_ui::contacts_view::CardView> {
        self.inner.focus.borrow().contacts_card()
    }

    /// The Edit contact form, while one is open.
    pub fn contact_edit(&self) -> Option<postio_ui::contact_edit::EditView> {
        self.inner.focus.borrow().contact_edit()
    }

    /// The message the open message surface shows, while it is on top.
    pub fn reading(&self) -> Option<MessageId> {
        self.inner.focus.borrow().reading()
    }

    /// The surface on top of the list, by the controller's stack.
    pub fn top_surface(&self) -> Option<postio_focus::SurfaceKind> {
        self.inner.focus.borrow().top_surface()
    }

    /// Whether the controller answers `id` itself over what is on screen:
    /// while a search's results are up, their keys and the verbs on a row.
    pub fn answers(&self, id: CommandId) -> bool {
        self.inner.focus.borrow().answers(id)
    }

    /// Who runs `id` with the surfaces now over the list.
    pub fn route(&self, id: CommandId) -> postio_focus::Route {
        self.inner.focus.borrow().route(id)
    }

    /// The key context in force: the top surface's, or the list's.
    pub fn key_context(&self) -> postio_ui::keymap::KeyContext {
        self.inner.focus.borrow().key_context()
    }

    /// A Ctrl-click (`range` false: `x` on that row) or a Shift-click (the
    /// walk from the anchor to it) on `position`.
    pub fn pick(&self, position: u32, range: bool) {
        self.input(Input::Pick { position, range });
    }

    /// The list has landed or moved: the cursor goes to the first row of
    /// a place just opened, or back to what `!` kept; the strip's counts
    /// are asked for again.
    pub fn landed(&self) {
        let effects = {
            let mut focus = self.inner.focus.borrow_mut();
            let opened = focus.take_opened();
            focus.landed(&self.inner.list, opened)
        };
        Rc::clone(&self.inner).apply(effects);
    }

    /// The list view put the cursor on `position`, or took it off the list:
    /// the controller notes where it is and draws nothing.
    pub fn cursor_is(&self, position: Option<u32>) {
        self.inner.focus.borrow_mut().cursor_is(position);
    }

    /// Drop the selection; the cursor stays.
    pub fn clear_selection(&self) {
        let effects = self.inner.focus.borrow_mut().clear_selection();
        Rc::clone(&self.inner).apply(effects);
    }

    /// The window takes the list to a place of its own: the selection
    /// goes, `!` is off, the days' headings come back.
    pub fn leave_list(&self) {
        let effects = self.inner.focus.borrow_mut().leave_list();
        Rc::clone(&self.inner).apply(effects);
    }

    /// The header strip names `name`: what an empty folder says it is.
    pub fn note_place(&self, name: &str) {
        self.inner.focus.borrow_mut().note_place(name);
    }

    /// What is selected: what `e` would archive.
    pub fn selection(&self) -> postio_core::state::Selection {
        self.inner.focus.borrow().selection()
    }

    /// The conversations each selected row stands for.
    pub fn selection_reach(&self) -> SelectionReach {
        self.inner.focus.borrow().selection_reach().clone()
    }

    /// Where a verb would go now: the selection when there is one, the
    /// cursor's row otherwise.
    pub fn aim(&self) -> postio_focus::Aimed {
        self.inner.focus.borrow().aim(&self.inner.list)
    }

    /// Whether an undo is waiting for the rows it brings back to land.
    pub fn restoring(&self) -> bool {
        self.inner.focus.borrow().restoring()
    }

    /// Whether the last landing put the cursor on a row an undo brought
    /// back: true once per undo.
    pub fn take_restored(&self) -> bool {
        self.inner.focus.borrow_mut().take_restored()
    }

    /// Whether `!` has narrowed the list to the rows with a marker.
    pub fn has_action(&self) -> bool {
        self.inner.focus.borrow().has_action()
    }

    /// The strip's counts, as the host last said.
    pub fn counts(&self) -> Option<postio_client::protocol::FocusCounts> {
        self.inner.focus.borrow().counts()
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
                Effect::Timer { token, after } => {
                    let inner = Rc::downgrade(&self);
                    glib::timeout_add_local_once(after, move || {
                        let Some(inner) = inner.upgrade() else {
                            return;
                        };
                        let effects = inner
                            .focus
                            .borrow_mut()
                            .handle_on(Input::Timer(token), &inner.list);
                        inner.apply(effects);
                    });
                }
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
            intent => {
                for handler in self.on_intent.borrow().iter() {
                    handler(&intent);
                }
            }
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
