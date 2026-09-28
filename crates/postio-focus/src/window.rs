//! Focus's one window: what it says while the store opens, the sentence when
//! it cannot, and the inbox once it has.
//!
//! The window comes first and the store behind it (#1114's rule for the
//! classic app): a person sees a window at once, and it says what it is
//! waiting for only once the wait is worth mentioning
//! ([`postio_ui::list_state::OPENING_THRESHOLD`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use postio_client::Client;
use postio_core::state::{Selection, ViewScope};
use postio_core::{Command, CommandId, Keymap, MessageTarget, SharedState};
use postio_model::ids::{AccountId, MessageId, ThreadId};
use postio_model::{FocusScope, ListScope};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use postio_ui::list_state::{OPENING_THRESHOLD, Waiting, describe_wait};
use postio_ui::selection::{Reach, SelectionState};

use crate::bulk::Bulk;
use crate::chrome::Chrome;
use crate::list::{Feed, FocusRow, ListPane, RowObject};
use postio_widgets::list_model::WindowedModel;

/// The window's pages, by name.
const BLANK: &str = "blank";
const OPENING: &str = "opening";
const UNAVAILABLE: &str = "unavailable";
const INBOX: &str = "inbox";

mod imp {
    use super::*;

    pub struct FocusWindow {
        pub pages: gtk::Stack,
        pub opening: adw::StatusPage,
        pub unavailable: adw::StatusPage,
        pub retry: gtk::Button,
        pub inbox: gtk::Box,
        /// The undo toast, and the overlay it appears over: the shared one
        /// both desktop apps say "Archived 3 messages" with.
        pub toast: postio_widgets::widgets::toast::Toast,
        /// The toast on screen, until it is dismissed -- by its timeout, its
        /// button, or a newer one.
        pub on_screen: RefCell<Option<adw::Toast>>,
        pub pane: RefCell<Option<ListPane>>,
        pub chrome: RefCell<Option<Rc<Chrome>>>,
        pub client: RefCell<Option<Client>>,
        /// What the store is being waited on for, while it is.
        pub waiting: Cell<Option<Waiting>>,
        /// When the wait began, for the threshold below which nothing is
        /// said.
        pub waiting_since: Cell<Option<std::time::Instant>>,
        /// What "Try again" does on the page that says why there is no mail.
        pub on_retry: RefCell<Option<Rc<dyn Fn()>>>,
        /// The keymap in force, as every surface's key hints read it.
        pub keymap: RefCell<Keymap>,
        /// Keys to commands, for Focus's commands alone (`crate::keys`).
        pub resolver: RefCell<Option<Resolver>>,
        /// What `a` would archive: not the cursor, which is GTK's
        /// selection model on the list (constitution II).
        pub picked: SelectionState,
        /// The conversations each picked row reaches: its own and the
        /// copies folded into it (T161), kept from when it was picked, so a
        /// row scrolled out of the window is still aimed at in full.
        pub reach: RefCell<HashMap<MessageId, Vec<ThreadId>>>,
        pub bulk: RefCell<Option<Rc<Bulk>>>,
        /// The host's view of this window's aim: a whole-view selection is
        /// a predicate the host resolves, and it reads it from here.
        pub state: RefCell<Option<SharedState>>,
        /// The enabled accounts, the aggregate a whole-view selection spans.
        pub accounts: RefCell<Vec<AccountId>>,
        /// Whether `!` has narrowed the list to the rows with a marker.
        pub has_action: Cell<bool>,
        /// The strip's counts, as the host last said.
        pub counts: Cell<Option<postio_client::protocol::FocusCounts>>,
        /// A message to put the cursor back on once the list it is changing
        /// to has landed: `!` keeps the cursor on the same message.
        pub keep: Cell<Option<MessageId>>,
        /// Where each account stands with its server, as sync has said.
        pub trackers: RefCell<postio_ui::status::Trackers>,
        /// The accounts sync has spoken about. Only those: a tracker says
        /// `Offline` for an account it has heard nothing of, which is not
        /// the same as the machine having no network.
        pub tracked: RefCell<Vec<AccountId>>,
        /// What the sign-in banner names for each account.
        pub facts: RefCell<Vec<postio_ui::focus_state::AccountFacts>>,
        /// When mail last finished arriving, in any account.
        pub last_synced: Cell<Option<chrono::DateTime<chrono::Utc>>>,
        /// The banner slot under the header strip.
        pub state_banner: RefCell<Option<Rc<crate::banner::StateBanner>>>,
    }

    impl Default for FocusWindow {
        fn default() -> Self {
            FocusWindow {
                pages: gtk::Stack::new(),
                opening: adw::StatusPage::new(),
                unavailable: adw::StatusPage::new(),
                retry: gtk::Button::with_label("Try again"),
                inbox: gtk::Box::new(gtk::Orientation::Vertical, 0),
                toast: postio_widgets::widgets::toast::Toast::new(),
                on_screen: RefCell::default(),
                pane: RefCell::default(),
                chrome: RefCell::default(),
                client: RefCell::default(),
                waiting: Cell::default(),
                waiting_since: Cell::default(),
                on_retry: RefCell::default(),
                keymap: RefCell::new(Keymap::defaults().clone()),
                resolver: RefCell::default(),
                picked: SelectionState::new(),
                reach: RefCell::default(),
                bulk: RefCell::default(),
                state: RefCell::default(),
                accounts: RefCell::default(),
                has_action: Cell::default(),
                counts: Cell::default(),
                keep: Cell::default(),
                trackers: RefCell::default(),
                tracked: RefCell::default(),
                facts: RefCell::default(),
                last_synced: Cell::default(),
                state_banner: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FocusWindow {
        const NAME: &'static str = "PostioFocusWindow";
        type Type = super::FocusWindow;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for FocusWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for FocusWindow {}
    impl WindowImpl for FocusWindow {}
    impl ApplicationWindowImpl for FocusWindow {}
    impl AdwApplicationWindowImpl for FocusWindow {}
}

glib::wrapper! {
    /// Focus's one window.
    pub struct FocusWindow(ObjectSubclass<imp::FocusWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager, gio::ActionGroup, gio::ActionMap;
}

impl FocusWindow {
    /// A window, for `application` when there is one.
    pub fn new(application: Option<&adw::Application>) -> Self {
        glib::Object::builder()
            .property("application", application)
            .property("title", "Postio Focus")
            .property("default-width", 1440)
            .property("default-height", 900)
            .build()
    }

    fn build(&self) {
        crate::style::install(&WidgetExt::display(self));
        self.add_css_class("focus-window");
        let imp = self.imp();
        imp.opening.add_css_class("focus-opening");
        imp.unavailable.add_css_class("focus-unavailable");
        imp.retry.add_css_class("pill");
        imp.retry.set_halign(gtk::Align::Center);
        imp.retry.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.retry()
        ));
        imp.unavailable.set_child(Some(&imp.retry));
        imp.inbox.add_css_class("focus-inbox");

        // Nothing at all until the wait is worth saying: a window that flashes
        // "Opening your mailbox" for the 150 ms a healthy start takes is the
        // flicker the budget forbids (`postio_ui::list_state`).
        imp.pages
            .add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some(BLANK));
        imp.pages.add_named(&imp.opening, Some(OPENING));
        imp.pages.add_named(&imp.unavailable, Some(UNAVAILABLE));
        imp.pages.add_named(&imp.inbox, Some(INBOX));
        imp.pages.set_visible_child_name(BLANK);
        imp.toast.overlay().set_child(Some(&imp.pages));
        self.set_content(Some(imp.toast.overlay()));

        // Capture, not bubble: a single-key binding has to be seen before the
        // focused widget consumes it, and whether it should is the
        // resolver's decision (the classic window's rule).
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| window.handle_key(key, state)
        ));
        self.add_controller(keys);

        // What the main menu's items run: a command by its id, through the
        // same `act` a key press reaches, and About.
        let run = gio::SimpleAction::new("run", Some(glib::VariantTy::STRING));
        run.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, parameter| {
                if let Some(id) = parameter
                    .and_then(|parameter| parameter.get::<String>())
                    .and_then(|id| id.parse::<CommandId>().ok())
                {
                    window.act(id);
                }
            }
        ));
        self.add_action(&run);
        // What the toast's Undo names: the same undo Ctrl+Z reaches.
        let undo = gio::SimpleAction::new("undo", None);
        undo.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.act(CommandId::Undo)
        ));
        self.add_action(&undo);
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.show_about()
        ));
        self.add_action(&about);
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name("Postio Focus")
            .version(env!("CARGO_PKG_VERSION"))
            .license_type(gtk::License::MitX11)
            .build();
        about.present(Some(self));
    }

    /// Take `keymap` as the keys in force: the resolver is rebuilt for
    /// Focus's commands, and what could not be bound is reported.
    pub fn set_keymap(&self, keymap: Keymap) {
        let (resolver, problems) = crate::keys::resolver(&keymap);
        for problem in &problems {
            tracing::warn!(problem, "a key binding was not honoured");
        }
        let imp = self.imp();
        imp.resolver.replace(Some(resolver));
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            chrome.set_keymap(&keymap);
        }
        if let Some(pane) = imp.pane.borrow().as_ref() {
            pane.set_keymap(keymap.clone());
        }
        if let Some(bulk) = imp.bulk.borrow().as_ref() {
            bulk.set_keymap(&keymap);
        }
        imp.keymap.replace(keymap);
    }

    /// The keymap in force.
    pub fn keymap(&self) -> Keymap {
        self.imp().keymap.borrow().clone()
    }

    /// Deliver one key press to the resolver, and act on what it means.
    ///
    /// Public because it is the whole keyboard path in one call: the
    /// controller on this window forwards to it, and a test presses a key
    /// through it without synthesizing a GDK event, which GTK4 gives no
    /// supported way to do.
    pub fn handle_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        let Some(chord) = postio_widgets::keys::chord(key, state) else {
            return glib::Propagation::Proceed;
        };
        let typing = gtk::prelude::GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>());
        let outcome = {
            let mut resolver = self.imp().resolver.borrow_mut();
            let Some(resolver) = resolver.as_mut() else {
                return glib::Propagation::Proceed;
            };
            resolver.press(
                &chord,
                self.key_context(),
                typing,
                std::time::Instant::now(),
            )
        };
        match outcome {
            Outcome::Command(id) => match id.parse::<CommandId>() {
                Ok(id) => {
                    self.act(id);
                    glib::Propagation::Stop
                }
                // A binding for a command this build does not know: leave the
                // key alone rather than swallow it.
                Err(_) => glib::Propagation::Proceed,
            },
            // Half a sequence is consumed, so its first key does not also
            // reach the widget underneath.
            Outcome::Pending(_) => glib::Propagation::Stop,
            Outcome::Unhandled => {
                tracing::debug!(chord = %chord, typing, "key resolved to nothing");
                glib::Propagation::Proceed
            }
        }
    }

    /// Which surface owns the keyboard.
    fn key_context(&self) -> KeyContext {
        KeyContext::List
    }

    /// Run the command `id` means here: the cursor and the selection are the
    /// window's own, and a verb on mail goes to the host aimed at them.
    pub fn act(&self, id: CommandId) {
        match id {
            CommandId::NextMessage => self.move_cursor(1),
            CommandId::PrevMessage => self.move_cursor(-1),
            CommandId::FirstMessage => self.cursor_to(Some(0)),
            CommandId::LastMessage => {
                let last = self.list_len().checked_sub(1);
                self.cursor_to(last);
            }
            CommandId::ToggleSelection => {
                if let Some(row) = self.cursor_row() {
                    self.imp()
                        .reach
                        .borrow_mut()
                        .insert(row.id(), row.threads());
                    self.imp().picked.toggle(row.id());
                }
            }
            CommandId::ExtendSelectionDown => self.extend(1),
            CommandId::ExtendSelectionUp => self.extend(-1),
            CommandId::SelectAll => self.imp().picked.select_all(Reach {
                accounts: self.imp().accounts.borrow().clone(),
                omitted: Vec::new(),
            }),
            CommandId::Back => self.clear_selection(),
            CommandId::Archive | CommandId::Delete | CommandId::ToggleRead => {
                self.send(Command::default_for(id));
            }
            // The last action this window took, whatever it was and however
            // long ago the toast went (FR-041): the host keeps the stack.
            CommandId::Undo => self.post(Command::Undo),
            CommandId::ToggleHasAction => self.toggle_has_action(),
            CommandId::Quit => self.close(),
            CommandId::Refresh => self.post(Command::Refresh),
            _ => tracing::debug!(command = %id, "no Focus surface answers this command yet"),
        }
    }

    /// How many rows the list has.
    fn list_len(&self) -> u32 {
        self.pane()
            .map(|pane| pane.feed().list().n_items())
            .unwrap_or(0)
    }

    /// Put the cursor on `position`, or on nothing, and bring it into view.
    fn cursor_to(&self, position: Option<u32>) {
        let Some(pane) = self.pane() else {
            return;
        };
        match position {
            Some(position) if position < pane.feed().list().n_items() => {
                pane.cursor().set_selected(position);
                pane.view()
                    .scroll_to(position, gtk::ListScrollFlags::NONE, None);
                if position == 0 {
                    pane.to_top();
                }
            }
            _ => {}
        }
    }

    /// Move the cursor `by` rows, and nothing else: nothing opens, nothing
    /// is marked read, the selection stays as it was (FR-016).
    fn move_cursor(&self, by: i32) {
        let Some(pane) = self.pane() else {
            return;
        };
        let len = pane.feed().list().n_items();
        if len == 0 {
            return;
        }
        let at = pane.cursor().selected();
        let next = if at == gtk::INVALID_LIST_POSITION {
            0
        } else {
            (i64::from(at) + i64::from(by)).clamp(0, i64::from(len) - 1) as u32
        };
        self.cursor_to(Some(next));
    }

    /// `J`/`K`: take the cursor's row into the selection and the next one
    /// with it, moving the cursor onto it.
    fn extend(&self, by: i32) {
        if let Some(row) = self.cursor_row() {
            self.imp()
                .reach
                .borrow_mut()
                .insert(row.id(), row.threads());
            self.imp().picked.extend_to(row.id());
        }
        self.move_cursor(by);
        if let Some(row) = self.cursor_row() {
            self.imp()
                .reach
                .borrow_mut()
                .insert(row.id(), row.threads());
            self.imp().picked.extend_to(row.id());
        }
    }

    /// Drop the selection; the cursor stays where it is.
    pub fn clear_selection(&self) {
        self.imp().picked.clear();
        self.imp().reach.borrow_mut().clear();
    }

    /// What is selected now: what `a` would archive.
    pub fn selection(&self) -> Selection {
        self.imp().picked.selection()
    }

    /// The row the cursor is on, once its page has landed.
    pub fn cursor_row(&self) -> Option<FocusRow> {
        let pane = self.pane()?;
        let at = pane.cursor().selected();
        if at == gtk::INVALID_LIST_POSITION {
            return None;
        }
        pane.feed()
            .list()
            .item(at)
            .and_downcast::<RowObject>()
            .and_then(|row| row.item())
    }

    /// Where a verb goes: the selection when there is one, the cursor's row
    /// otherwise. A row folded from several accounts is every copy
    /// (`MessageTarget::Threads`, T161); a message in no conversation is
    /// itself. A whole-view selection is a predicate the host resolves over
    /// the inboxes, never a list of what happens to be on screen.
    fn aims(&self) -> Vec<MessageTarget> {
        let imp = self.imp();
        match imp.picked.selection() {
            Selection::Everything { except } => {
                if let Some(state) = imp.state.borrow().as_ref() {
                    let accounts = imp.accounts.borrow().clone();
                    let (sink, _) = postio_core::bridge::event_channel();
                    state.update(&sink, |app| {
                        let mut events = app.open_view(ViewScope::Unified { accounts });
                        events.extend(app.select_all());
                        events
                    });
                    state.update(&sink, |app| {
                        let mut events = Vec::new();
                        for message in &except {
                            events.extend(app.toggle_selection(*message));
                        }
                        events
                    });
                }
                vec![MessageTarget::Selection]
            }
            Selection::These(picked) if !picked.is_empty() => {
                let reach = imp.reach.borrow();
                let mut threads = Vec::new();
                let mut lone = Vec::new();
                for message in picked {
                    match reach.get(&message) {
                        Some(theirs) if !theirs.is_empty() => {
                            threads.extend(theirs.iter().copied())
                        }
                        _ => lone.push(message),
                    }
                }
                let mut aims = Vec::new();
                if !threads.is_empty() {
                    aims.push(MessageTarget::Threads(threads));
                }
                if !lone.is_empty() {
                    aims.push(MessageTarget::Messages(lone));
                }
                aims
            }
            Selection::These(_) => match self.cursor_row() {
                Some(row) if !row.threads().is_empty() => {
                    vec![MessageTarget::Threads(row.threads())]
                }
                Some(row) => vec![MessageTarget::Messages(vec![row.id()])],
                None => Vec::new(),
            },
        }
    }

    /// Send `command` to the host as it is: a verb that aims at nothing.
    fn post(&self, command: Command) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: as `send`'s.
            if let Err(error) = client.send(command).await {
                tracing::warn!(%error, "Focus could not send a command: {error}");
            }
        });
    }

    /// Send `command` to the host, aimed as [`Self::aims`] says, and let
    /// the selection go: what it named has been acted on.
    fn send(&self, command: Command) {
        let aims = self.aims();
        if aims.is_empty() {
            return;
        }
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        self.clear_selection();
        glib::spawn_future_local(async move {
            for aim in aims {
                // POSTIO-GLIB-SAFE: the client's in-process transport hands
                // the command over and answers through a oneshot.
                if let Err(error) = client.send(command.clone().with_target(aim)).await {
                    tracing::warn!(%error, "Focus could not send a command: {error}");
                }
            }
        });
    }

    /// Say what the store is being waited on for -- once the wait has
    /// outlasted [`OPENING_THRESHOLD`], and not before.
    pub fn set_waiting_on(&self, waiting: Waiting) {
        let imp = self.imp();
        imp.waiting.set(Some(waiting));
        let since = imp
            .waiting_since
            .get()
            .unwrap_or_else(std::time::Instant::now);
        imp.waiting_since.set(Some(since));
        let waited = since.elapsed();
        if waited >= OPENING_THRESHOLD {
            self.show_waiting();
        } else {
            glib::timeout_add_local_once(
                OPENING_THRESHOLD - waited,
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.show_waiting()
                ),
            );
        }
    }

    fn show_waiting(&self) {
        let imp = self.imp();
        let Some(waiting) = imp.waiting.get() else {
            return;
        };
        let (title, description) = describe_wait(waiting);
        imp.opening.set_title(title);
        imp.opening.set_description(Some(description));
        imp.pages.set_visible_child_name(OPENING);
    }

    /// What the window says while it waits, empty when it says nothing.
    pub fn waiting_text(&self) -> String {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() != Some(OPENING) {
            return String::new();
        }
        format!(
            "{} {}",
            imp.opening.title(),
            imp.opening.description().unwrap_or_default()
        )
    }

    /// There is no mail to show, and `reason` says why -- among them another
    /// Postio having the store open. "Try again" runs `retry`.
    pub fn show_unavailable(&self, reason: &str, retry: impl Fn() + 'static) {
        let imp = self.imp();
        imp.waiting.set(None);
        imp.waiting_since.set(None);
        imp.unavailable
            .set_title("Postio can\u{2019}t open your mail");
        imp.unavailable.set_description(Some(reason));
        imp.on_retry.replace(Some(Rc::new(retry)));
        imp.pages.set_visible_child_name(UNAVAILABLE);
    }

    /// The sentence the window shows when it has no mail, if it is showing
    /// one.
    pub fn unavailable_reason(&self) -> Option<String> {
        let imp = self.imp();
        (imp.pages.visible_child_name().as_deref() == Some(UNAVAILABLE)).then(|| {
            imp.unavailable
                .description()
                .unwrap_or_default()
                .to_string()
        })
    }

    /// Press "Try again", as a click does.
    pub fn retry(&self) {
        let retry = self.imp().on_retry.borrow().clone();
        if let Some(retry) = retry {
            retry();
        }
    }

    /// Show the inbox, read through `client` with `keymap`'s keys, and
    /// follow what the store says from here on. `state` is the host's view
    /// of this window's aim, which a whole-view selection is written to.
    pub fn show_inbox(&self, client: Client, state: SharedState, keymap: Keymap) {
        self.imp().state.replace(Some(state));
        let imp = self.imp();
        imp.waiting.set(None);
        imp.waiting_since.set(None);
        let chrome = Chrome::new(&keymap);
        chrome.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        imp.inbox.append(chrome.top_bar());
        imp.inbox.append(chrome.strip());
        imp.chrome.replace(Some(Rc::clone(&chrome)));
        let state_banner = crate::banner::StateBanner::new();
        state_banner.connect_action(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |action| window.act(match action {
                postio_ui::focus_state::BannerAction::Retry => CommandId::Refresh,
                postio_ui::focus_state::BannerAction::UpdatePassword => {
                    CommandId::UpdateCredential
                }
            })
        ));
        imp.inbox.append(state_banner.widget());
        imp.state_banner.replace(Some(state_banner));
        self.show_state();
        self.set_keymap(keymap);
        let feed = Feed::new(client.clone());
        feed.connect_filled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.list_landed()
        ));
        let pane = ListPane::new(feed.clone(), self.keymap(), imp.picked.clone());
        imp.inbox.append(pane.widget());
        let bulk = Rc::new(Bulk::new(&self.keymap()));
        bulk.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        imp.inbox.append(bulk.widget());
        imp.bulk.replace(Some(Rc::clone(&bulk)));
        imp.picked.connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.selection_moved()
        ));
        // The accounts a whole-view selection spans.
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            client,
            async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
                // answers on its own runtime (ADR 0041).
                if let Ok(accounts) = client.accounts().await {
                    let enabled: Vec<&postio_model::Account> =
                        accounts.iter().filter(|account| account.enabled).collect();
                    window
                        .imp()
                        .accounts
                        .replace(enabled.iter().map(|account| account.id).collect());
                    window.imp().facts.replace(
                        enabled
                            .iter()
                            .map(|account| postio_ui::focus_state::AccountFacts {
                                id: account.id,
                                server: account.incoming.host.clone(),
                                address: account.address.address.clone(),
                            })
                            .collect(),
                    );
                    // When mail last arrived, from before this run: the
                    // newest folder's last completed sync.
                    let mut last = None;
                    for account in &enabled {
                        use postio_model::listing::MailStore as _;
                        // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
                        // answers on its own runtime (ADR 0041).
                        let read = client.mailboxes(account.id).await;
                        if let Ok(mailboxes) = read {
                            last = last.max(
                                mailboxes
                                    .iter()
                                    .filter_map(|mailbox| mailbox.last_synced_at)
                                    .max(),
                            );
                        }
                    }
                    if window.imp().last_synced.get().is_none() {
                        window.imp().last_synced.set(last);
                    }
                    window.show_state();
                }
            }
        ));
        imp.pane.replace(Some(pane));
        imp.client.replace(Some(client.clone()));
        imp.pages.set_visible_child_name(INBOX);
        feed.open(ListScope::Focus(FocusScope::Inbox));

        // Exactly one reader of the client's events, on the main loop:
        // clones of the receiver would compete for them (research R3).
        let events = client.events();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(envelope) = events.recv().await {
                let Some(window) = window.upgrade() else {
                    return;
                };
                window.hear(&envelope.event);
            }
        });
    }

    /// The selection changed: the bar says so, and the rows redraw their
    /// boxes.
    fn selection_moved(&self) {
        let imp = self.imp();
        let total = imp.pane.borrow().as_ref().map(|pane| pane.feed().total());
        let summary = postio_ui::selection::summary(&imp.picked.selection(), total, &[]);
        if let Some(bulk) = imp.bulk.borrow().as_ref() {
            bulk.set_summary(summary.as_deref());
        }
        if let Some(pane) = imp.pane.borrow().as_ref() {
            pane.redraw_rows();
        }
    }

    /// `!`: narrow the list to the rows with a marker, or back (FR-017).
    /// The selection goes, since what it named may not be shown; the cursor
    /// stays on the same message when that message is still shown.
    fn toggle_has_action(&self) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        let on = !imp.has_action.get();
        imp.has_action.set(on);
        imp.keep.set(self.cursor_row().map(|row| row.id()));
        self.clear_selection();
        pane.feed().list().set_single_heading(on.then(|| {
            postio_ui::focus_row::has_action_label(imp.counts.get().map(|counts| counts.has_action))
        }));
        pane.feed().open(ListScope::Focus(if on {
            FocusScope::HasAction
        } else {
            FocusScope::Inbox
        }));
        self.show_counts();
    }

    /// The list has landed or moved: put the cursor back where `!` left
    /// it, and ask the host for the strip's counts again.
    fn list_landed(&self) {
        let imp = self.imp();
        if let (Some(message), Some(pane)) = (imp.keep.get(), self.pane())
            && pane.feed().has_landed()
        {
            imp.keep.set(None);
            let list = pane.feed().list().clone();
            match list.position_of(message) {
                Some(position) => self.cursor_to(Some(position)),
                None if list.n_items() > 0 => self.cursor_to(Some(0)),
                None => {}
            }
        }
        self.update_counts();
    }

    /// Ask the host for the strip's counts, and show them when they come.
    fn update_counts(&self) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: the client's transport answers through a
            // oneshot.
            match client.focus_counts().await {
                Ok(counts) => {
                    if let Some(window) = window.upgrade() {
                        window.imp().counts.set(Some(counts));
                        window.show_counts();
                    }
                }
                Err(error) => tracing::warn!(%error, "Focus could not count its inbox: {error}"),
            }
        });
    }

    /// Say the counts the host last gave: the strip's, the toggle's, and
    /// how many of how many the filter is showing.
    fn show_counts(&self) {
        let imp = self.imp();
        let Some(chrome) = imp.chrome.borrow().clone() else {
            return;
        };
        let counts = imp.counts.get();
        if let Some(counts) = counts {
            chrome.set_counts(counts.conversations, counts.unread);
        }
        let has_action = counts.map(|counts| counts.has_action);
        let label = postio_ui::focus_row::has_action_label(has_action);
        let on = imp.has_action.get();
        let showing = match (on, counts) {
            (true, Some(counts)) => Some(postio_ui::focus_row::showing(
                counts.has_action,
                counts.conversations,
                postio_ui::hints::key(&self.keymap(), CommandId::ToggleHasAction).as_deref(),
            )),
            _ => None,
        };
        chrome.set_has_action(on, &label, showing.as_deref());
        if on
            && let Some(pane) = self.pane()
            && pane.feed().list().single_heading().as_deref() != Some(label.as_str())
        {
            pane.feed().list().set_single_heading(Some(label));
        }
    }

    /// The window's chrome, once the inbox is showing.
    pub fn chrome(&self) -> Option<Rc<Chrome>> {
        self.imp().chrome.borrow().clone()
    }

    /// What the store just said.
    fn hear(&self, event: &postio_core::Event) {
        use postio_core::Event;
        match event {
            // What this window's own commands say about themselves: only the
            // client that sent a command hears these (postio-host).
            Event::ActionCompleted {
                description,
                undoable,
            } => {
                self.imp()
                    .toast
                    .show_action_completed(description, *undoable);
                self.follow_toast();
            }
            Event::UndoPerformed { description } => {
                self.imp().toast.show_undo_performed(description);
                self.follow_toast();
            }
            Event::CommandRejected { reason, .. } => {
                self.imp().toast.show_notice(reason);
                self.follow_toast();
            }
            _ => {}
        }
        self.hear_sync(event);
        if let Some(pane) = self.imp().pane.borrow().as_ref() {
            pane.feed().handle(event);
        }
    }

    /// What sync said about an account: the banner and the label follow.
    fn hear_sync(&self, event: &postio_core::Event) {
        use postio_core::Event;
        let imp = self.imp();
        // The first word about an account is news even when it changes
        // nothing in its tracker: a tracker starts at `Offline`, so an
        // account whose first report is `Offline` would otherwise never
        // be drawn as such.
        let first = match event {
            Event::ConnectionChanged { account, .. }
            | Event::SyncProgress { account, .. }
            | Event::BackfillProgress { account, .. } => self.note_account(*account),
            _ => false,
        };
        let changed = imp.trackers.borrow_mut().apply(event, None);
        if !(first || changed) {
            return;
        }
        if let Event::SyncProgress { done, total, .. } = event
            && done >= total
        {
            imp.last_synced.set(Some(chrono::Utc::now()));
        }
        self.show_state();
    }

    /// Follow `account`'s sync state from here on; whether it is new.
    fn note_account(&self, account: AccountId) -> bool {
        let mut tracked = self.imp().tracked.borrow_mut();
        let new = !tracked.contains(&account);
        if new {
            tracked.push(account);
        }
        new
    }

    /// Draw the banner and the sync label for where every account stands.
    fn show_state(&self) {
        let imp = self.imp();
        let statuses = imp.trackers.borrow().statuses(&imp.tracked.borrow());
        let banner = postio_ui::focus_state::banner(&statuses, &imp.facts.borrow());
        if let Some(slot) = imp.state_banner.borrow().as_ref() {
            slot.show(banner.as_ref());
        }
        let label = postio_ui::focus_state::sync_label_here(&statuses, imp.last_synced.get());
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            chrome.set_sync(&label.text, label.icon);
        }
    }

    /// The banner under the strip, while one shows: what it says, its
    /// button's label, and the first sync's progress.
    pub fn banner_showing(&self) -> Option<(String, Option<String>, Option<f64>)> {
        self.imp()
            .state_banner
            .borrow()
            .as_ref()
            .and_then(|slot| slot.showing())
    }

    /// What the top bar's sync label says.
    pub fn sync_said(&self) -> String {
        self.chrome()
            .map(|chrome| chrome.sync_said())
            .unwrap_or_default()
    }

    /// The list pane, once the inbox is showing.
    pub fn pane(&self) -> Option<ListPane> {
        self.imp().pane.borrow().clone()
    }

    /// The rows a person can see, top to bottom, each as it reads.
    pub fn rows_on_screen(&self) -> Vec<String> {
        self.pane()
            .map(|pane| {
                pane.rows_on_screen()
                    .iter()
                    .map(|row| row.spoken())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Keep track of the toast just shown until it goes.
    fn follow_toast(&self) {
        let Some(toast) = self.imp().toast.showing() else {
            return;
        };
        toast.connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gone| {
                let mut on_screen = window.imp().on_screen.borrow_mut();
                if on_screen.as_ref() == Some(gone) {
                    on_screen.take();
                }
            }
        ));
        self.imp().on_screen.replace(Some(toast));
    }

    /// The words of the toast on screen, if one is.
    pub fn toast_showing(&self) -> Option<String> {
        self.imp()
            .on_screen
            .borrow()
            .as_ref()
            .and_then(|toast| toast.title().map(|title| title.to_string()))
    }

    /// Take the toast on screen away, as its timeout would.
    pub fn dismiss_toast(&self) {
        let toast = self.imp().on_screen.borrow().clone();
        if let Some(toast) = toast {
            toast.dismiss();
        }
    }
}
