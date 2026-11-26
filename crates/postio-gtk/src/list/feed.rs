//! The GTK driver for the controller (`postio_focus`, ADR 0045): hands it what happened, runs each
//! request through `postio_focus::perform` on GTK's loop (the client's in-process transport answers
//! through a oneshot, so the loop is never inside a query), and applies the result to the list
//! model. Staleness is the model's: a page is asked for under its generation.

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
use super::spliced::SplicedList;

pub type SelectionReach = std::collections::HashMap<MessageId, Vec<ThreadId>>;

type IntentHandler = Box<dyn Fn(&Intent)>;

#[derive(Clone)]
pub struct Feed {
    inner: Rc<Inner>,
}

struct Inner {
    client: Client,
    list: FocusList,
    shown: SplicedList,
    /// Borrowed only to hand it something and take its effects: never while they are applied,
    /// because applying them calls the model, and the model calls back.
    focus: RefCell<FocusController>,
    on_filled: RefCell<Vec<Box<dyn Fn()>>>,
    on_intent: RefCell<Vec<IntentHandler>>,
}

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
    pub fn new(client: Client) -> Self {
        let list = FocusList::default();
        Feed {
            inner: Rc::new(Inner {
                client,
                shown: SplicedList::new(&list),
                list,
                focus: RefCell::new(FocusController::new(Policy::for_platform(
                    Platform::Freedesktop,
                ))),
                on_filled: RefCell::new(Vec::new()),
                on_intent: RefCell::new(Vec::new()),
            }),
        }
    }

    pub fn client(&self) -> Client {
        self.inner.client.clone()
    }

    pub fn list(&self) -> &FocusList {
        &self.inner.list
    }

    pub fn shown(&self) -> &SplicedList {
        &self.inner.shown
    }

    pub fn scope(&self) -> Option<ListScope> {
        self.inner.focus.borrow().scope()
    }

    pub fn total(&self) -> u32 {
        self.inner.focus.borrow().list_total()
    }

    /// Digests are not counted, matching the strip, which counts what the store holds.
    pub fn message_place(&self, index: u32, total: u32) -> (u32, u32) {
        self.inner.focus.borrow().message_place(index, total)
    }

    pub fn pages_asked(&self) -> Vec<u32> {
        self.inner.focus.borrow().pages_asked().to_vec()
    }

    pub fn connect_filled(&self, handler: impl Fn() + 'static) {
        self.inner.on_filled.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_intent(&self, handler: impl Fn(&Intent) + 'static) {
        self.inner.on_intent.borrow_mut().push(Box::new(handler));
    }

    pub fn command(&self, id: CommandId) {
        self.input(Input::Command(id));
    }

    pub fn input(&self, input: Input) {
        let effects = self
            .inner
            .focus
            .borrow_mut()
            .handle_on(input, &self.inner.list);
        Rc::clone(&self.inner).apply(effects);
    }

    /// `None` while its page is on its way.
    pub fn result_row(&self, position: u64) -> Option<postio_focus::ResultRow> {
        self.inner.focus.borrow().result_row(position)
    }

    /// `None` past the last, or before the rows are read.
    pub fn result_person(&self, position: u64) -> Option<postio_focus::PersonRow> {
        self.inner.focus.borrow().result_person(position)
    }

    /// `None` past the last, or before the cards are read.
    pub fn result_file(&self, position: u64) -> Option<postio_focus::FileCard> {
        self.inner.focus.borrow().result_file(position)
    }

    pub fn want_result(&self, position: u64) {
        let effects = self.inner.focus.borrow_mut().results_wanted(position);
        Rc::clone(&self.inner).apply(effects);
    }

    /// Whether the controller used it, stepping into the chips; `false` leaves the key to the
    /// toolkit.
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

    pub fn among_hits(&self, message: MessageId) -> bool {
        self.inner.focus.borrow().among_hits(message)
    }

    pub fn places(&self, filter: &str) -> Vec<(u64, postio_ui::places::Entry)> {
        self.inner.focus.borrow().places(filter)
    }

    pub fn folder_names(&self) -> Vec<(postio_model::MailboxId, String)> {
        self.inner.focus.borrow().folder_names().to_vec()
    }

    /// `None` while its page is on its way: a miss asks the store for it.
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

    pub fn contacts_total(&self) -> u32 {
        self.inner.focus.borrow().contacts_total()
    }

    pub fn contacts_cursor(&self) -> Option<u32> {
        self.inner.focus.borrow().contacts_cursor()
    }

    pub fn contacts_card(&self) -> Option<postio_ui::contacts_view::CardView> {
        self.inner.focus.borrow().contacts_card()
    }

    pub fn contact_edit(&self) -> Option<postio_ui::contact_edit::EditView> {
        self.inner.focus.borrow().contact_edit()
    }

    pub fn reading(&self) -> Option<MessageId> {
        self.inner.focus.borrow().reading()
    }

    pub fn top_surface(&self) -> Option<postio_focus::SurfaceKind> {
        self.inner.focus.borrow().top_surface()
    }

    pub fn focused_window(&self) -> postio_focus::Window {
        self.inner.focus.borrow().focused_window()
    }

    pub fn answers(&self, id: CommandId) -> bool {
        self.inner.focus.borrow().answers(id)
    }

    pub fn route(&self, id: CommandId) -> postio_focus::Route {
        self.inner.focus.borrow().route(id)
    }

    pub fn keymap(&self) -> postio_core::Keymap {
        self.inner.focus.borrow().keymap().clone()
    }

    pub fn key(&self, id: CommandId) -> Option<String> {
        postio_ui::hints::key(self.inner.focus.borrow().keymap(), id)
    }

    pub fn key_context(&self) -> postio_ui::keymap::KeyContext {
        self.inner.focus.borrow().key_context()
    }

    pub fn pick(&self, position: u32, range: bool) {
        self.input(Input::Pick { position, range });
    }

    pub fn landed(&self) {
        let effects = {
            let mut focus = self.inner.focus.borrow_mut();
            let opened = focus.take_opened();
            focus.landed(&self.inner.list, opened)
        };
        Rc::clone(&self.inner).apply(effects);
    }

    /// The controller notes where it is and draws nothing.
    pub fn cursor_is(&self, position: Option<u32>) {
        self.inner.focus.borrow_mut().cursor_is(position);
    }

    pub fn clear_selection(&self) {
        let effects = self.inner.focus.borrow_mut().clear_selection();
        Rc::clone(&self.inner).apply(effects);
    }

    pub fn leave_list(&self) {
        let effects = self.inner.focus.borrow_mut().leave_list();
        Rc::clone(&self.inner).apply(effects);
    }

    pub fn note_place(&self, name: &str) {
        self.inner.focus.borrow_mut().note_place(name);
    }

    pub fn selection(&self) -> postio_core::state::Selection {
        self.inner.focus.borrow().selection()
    }

    pub fn selection_reach(&self) -> SelectionReach {
        self.inner.focus.borrow().selection_reach().clone()
    }

    pub fn aim(&self) -> postio_focus::Aimed {
        self.inner.focus.borrow().aim(&self.inner.list)
    }

    pub fn walking(&self, message: MessageId, row: Option<MessageId>) -> bool {
        self.inner.focus.borrow().walking(message, row)
    }

    pub fn aim_for(&self, action: CommandId) -> postio_focus::Aimed {
        self.inner.focus.borrow().aim_for(&self.inner.list, action)
    }

    pub fn restoring(&self) -> bool {
        self.inner.focus.borrow().restoring()
    }

    /// True once per undo.
    pub fn take_restored(&self) -> bool {
        self.inner.focus.borrow_mut().take_restored()
    }

    pub fn has_action(&self) -> bool {
        self.inner.focus.borrow().has_action()
    }

    pub fn counts(&self) -> Option<postio_client::protocol::FocusCounts> {
        self.inner.focus.borrow().counts()
    }

    /// The rows already on screen stay until the new scope's first page lands, so a change of place
    /// never flashes empty.
    pub fn open(&self, scope: ListScope) {
        let effects = self.inner.focus.borrow_mut().open(scope);
        Rc::clone(&self.inner).apply(effects);
    }

    /// True once per opening, so the cursor goes to the new list's first row once and a later
    /// re-read leaves it where it is.
    pub fn take_opened(&self) -> bool {
        self.inner.focus.borrow_mut().take_opened()
    }

    pub fn has_landed(&self) -> bool {
        self.inner.focus.borrow().has_landed()
    }

    pub fn is_inbox(&self, mailbox: postio_model::MailboxId) -> bool {
        self.inner.focus.borrow().is_inbox(mailbox)
    }

    pub fn handle(&self, event: &Event) {
        let effects = self
            .inner
            .focus
            .borrow_mut()
            .handle_on(Input::Event(event.clone()), &self.inner.list);
        Rc::clone(&self.inner).apply(effects);
    }
}

impl Inner {
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
                // Rows already on screen stay until this page lands, and nothing else would ask for
                // it. An empty scope's first page is empty: the change-over to nothing.
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
            Intent::Splice { anchor, children } => self.shown.splice(anchor, children),
            Intent::Unsplice { .. } => self.shown.unsplice(),
            Intent::Filled => {
                for handler in self.on_filled.borrow().iter() {
                    handler();
                }
            }
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
                let effects = self
                    .focus
                    .borrow_mut()
                    .handle_on(Input::Reply(ticket, reply), &self.list);
                self.apply(effects);
                return;
            }
            Err(request) => request,
        };
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: perform awaits only client calls, each a
            // oneshot receive; the host answers on its own runtime (ADR 0041).
            let reply = postio_focus::perform(&self.client, request).await;
            let effects = self
                .focus
                .borrow_mut()
                .handle_on(Input::Reply(ticket, reply), &self.list);
            self.apply(effects);
        });
    }
}
