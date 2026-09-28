//! Focus's one window: what it says while the store opens, the sentence when
//! it cannot, and the inbox once it has.
//!
//! The window comes first and the store behind it (#1114's rule for the
//! classic app): a person sees a window at once, and it says what it is
//! waiting for only once the wait is worth mentioning
//! ([`postio_ui::list_state::OPENING_THRESHOLD`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, glib};
use postio_client::Client;
use postio_core::{CommandId, Keymap};
use postio_model::{FocusScope, ListScope};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use postio_ui::list_state::{OPENING_THRESHOLD, Waiting, describe_wait};

use crate::chrome::Chrome;
use crate::list::{Feed, ListPane};

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
        pub toasts: adw::ToastOverlay,
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
    }

    impl Default for FocusWindow {
        fn default() -> Self {
            FocusWindow {
                pages: gtk::Stack::new(),
                opening: adw::StatusPage::new(),
                unavailable: adw::StatusPage::new(),
                retry: gtk::Button::with_label("Try again"),
                inbox: gtk::Box::new(gtk::Orientation::Vertical, 0),
                toasts: adw::ToastOverlay::new(),
                pane: RefCell::default(),
                chrome: RefCell::default(),
                client: RefCell::default(),
                waiting: Cell::default(),
                waiting_since: Cell::default(),
                on_retry: RefCell::default(),
                keymap: RefCell::new(Keymap::defaults().clone()),
                resolver: RefCell::default(),
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
        imp.toasts.set_child(Some(&imp.pages));
        self.set_content(Some(&imp.toasts));

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

    /// Run the command `id` means here, with the registry's default target.
    pub fn act(&self, id: CommandId) {
        match id {
            CommandId::Quit => self.close(),
            _ => tracing::debug!(command = %id, "no Focus surface answers this command yet"),
        }
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
    /// follow what the store says from here on.
    pub fn show_inbox(&self, client: Client, keymap: Keymap) {
        let imp = self.imp();
        imp.waiting.set(None);
        imp.waiting_since.set(None);
        let chrome = Chrome::new(&keymap);
        chrome.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        chrome.set_sync("Not synced yet", "emblem-synchronizing-symbolic");
        imp.inbox.append(chrome.top_bar());
        imp.inbox.append(chrome.strip());
        imp.chrome.replace(Some(Rc::clone(&chrome)));
        self.set_keymap(keymap);
        let feed = Feed::new(client.clone());
        feed.connect_filled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.update_counts()
        ));
        let pane = ListPane::new(feed.clone());
        imp.inbox.append(pane.widget());
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

    /// Bring the strip's counts into step with the list.
    fn update_counts(&self) {
        let imp = self.imp();
        let total = imp.pane.borrow().as_ref().map(|pane| pane.feed().total());
        if let (Some(total), Some(chrome)) = (total, imp.chrome.borrow().as_ref()) {
            chrome.set_counts(total);
        }
    }

    /// The window's chrome, once the inbox is showing.
    pub fn chrome(&self) -> Option<Rc<Chrome>> {
        self.imp().chrome.borrow().clone()
    }

    /// What the store just said.
    fn hear(&self, event: &postio_core::Event) {
        if let Some(pane) = self.imp().pane.borrow().as_ref() {
            pane.feed().handle(event);
        }
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

    /// The toast overlay, for what the window announces.
    pub fn toasts(&self) -> &adw::ToastOverlay {
        &self.imp().toasts
    }
}
