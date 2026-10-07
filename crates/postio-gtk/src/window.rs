//! Focus's one window: what it says while the store opens, the sentence when
//! it cannot, and the inbox once it has.
//!
//! The window comes first and the store behind it (#1114's rule): a person sees a window at once, and it says what it is
//! waiting for only once the wait is worth mentioning
//! ([`postio_ui::list_state::OPENING_THRESHOLD`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::translate::IntoGlib;
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
use postio_widgets::widgets::pickers::{Picker, When, WhenPicker};

/// Where a chosen link or part is opened.
type Launcher = Rc<dyn Fn(&str)>;
/// Asks where to save, and answers with the place chosen -- `None` when the
/// person dismissed it (T240).
type FilePicker = Rc<dyn Fn(crate::chooser::SavePick, Box<dyn FnOnce(Option<std::path::PathBuf>)>)>;

/// Where `EditConfig` opens `config.toml`, when a test says.
type Editor = Rc<dyn Fn(&std::path::Path)>;

/// What the page offering a fresh store says (T215): what happened, why
/// trying again would not help, what a fresh store keeps and what it does
/// not, and that the old one is set aside rather than deleted.
const START_OVER: &str = "This version of Postio can\u{2019}t read the store an earlier \
     build wrote, and no update carries it forward, so trying again won\u{2019}t help. \
     A fresh store keeps your accounts and settings and syncs your mail again from the \
     server. Snoozes, reminders, Focus\u{2019}s filing history, and drafts or changes \
     not yet sent stay in the old store, which is set aside, not deleted.";

/// The window's pages, by name.
/// What a window with nothing saved opens at.
const DEFAULT_GEOMETRY: postio_widgets::state::Geometry =
    postio_widgets::state::Geometry::new(1440, 900);
const BLANK: &str = "blank";
const OPENING: &str = "opening";
const UNAVAILABLE: &str = "unavailable";
const INBOX: &str = "inbox";
/// What decides a notification for new mail: the mailbox, the messages
/// an arrival names, and where the person is looking.
pub type Notifier = Rc<
    dyn Fn(
        postio_model::MailboxId,
        Vec<MessageId>,
        postio_ui::notify::Attention,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<postio_ui::notify::Notification>>>,
    >,
>;

/// Where a test takes notifications instead of the desktop.
type NotificationSink = Rc<dyn Fn(&postio_ui::notify::Notification)>;

/// The stop-digesting confirmation (US10 scenario 5).
const STOP_DIALOG: &str = "focus-stop-digesting";

/// The sweep's confirmation (FR-118).
const SWEEP_DIALOG: &str = "focus-sweep";

/// The digest rules list's page (`g d`, T139).
const RULES: &str = "rules";

/// Removing a digest rule's confirmation (FR-126).
const REMOVE_RULE_DIALOG: &str = "focus-remove-rule";

/// The Filtered view's page (screen 21).
const FILTERED: &str = "filtered";
/// The inbox's list, and the empty inbox in its place.
const LIST: &str = "list";
/// The key map dialog's name, so it can be told from another dialog.
const KEY_MAP: &str = "focus-key-map";
const EMPTY: &str = "empty";

mod imp {
    use super::*;

    pub struct FocusWindow {
        pub pages: gtk::Stack,
        /// The pages, with what lies over the whole window: the command bar.
        pub stage: gtk::Overlay,
        pub opening: adw::StatusPage,
        pub unavailable: adw::StatusPage,
        pub retry: gtk::Button,
        /// The bar over every page before the inbox's own: the window's
        /// close button, with nothing to act on yet (T216). The inbox's top
        /// bar takes over once there is mail.
        pub bar_before_mail: gtk::WindowHandle,
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
        /// What the one button on the page that says why there is no mail
        /// does: try again, or start a fresh store (T215).
        pub on_retry: RefCell<Option<Rc<dyn Fn()>>>,
        /// The sentence that page says, as given: the page shows it as
        /// escaped markup.
        pub refusal: RefCell<String>,
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
        /// `[focus]`: whether filtering is on, and the digest rules.
        pub focus_config: RefCell<postio_config::FocusConfig>,
        /// The list, or the empty inbox in its place.
        pub list_or_empty: RefCell<Option<gtk::Stack>>,
        /// The empty inbox's page.
        pub empty: RefCell<Option<Rc<crate::empty::EmptyInbox>>>,
        /// The open-email dialog, built on the first open and reused.
        pub reading: RefCell<Option<Rc<crate::open::OpenMessage>>>,
        /// The pane beside the list that an open message is drawn in when
        /// the person reads beside the list (T232).
        pub reading_pane: RefCell<Option<Rc<crate::reading_pane::ReadingPane>>>,
        /// The list and the reading pane, side by side.
        pub split: RefCell<Option<gtk::Box>>,
        /// Where an open message is drawn now: the setting, as far as the
        /// window's width and the page on screen allow.
        pub placement: Cell<postio_ui::focus_dialog::Placement>,
        /// Where a chosen link or part is opened: the desktop, unless a
        /// test has said otherwise.
        pub launcher: RefCell<Option<super::Launcher>>,
        pub file_picker: RefCell<Option<super::FilePicker>>,
        /// What the open-with chooser on screen offers.
        pub choices: RefCell<Vec<crate::chooser::Choice>>,
        /// Where remote images are fetched: the host's runtime.
        pub runtime: RefCell<Option<tokio::runtime::Handle>>,
        /// The command bar, over the list.
        pub bar: RefCell<Option<Rc<crate::bar::Bar>>>,
        /// The pinned saved searches, in order: each name and its query.
        pub saved: RefCell<Vec<(String, String)>>,
        /// The folders popover, built the first time it opens.
        pub places: RefCell<Option<Rc<crate::places::Places>>>,
        /// Whether the list shows Focus's own inbox, rather than a folder.
        pub at_inbox: Cell<bool>,
        /// The place the list shows when it is not the inbox: what its
        /// strip counts and what it says when it has nothing in it.
        pub place: RefCell<Option<postio_ui::focus_state::EmptyPlace>>,
        /// The last key press `handle_key` was given, by its event time and
        /// key: so a press the window's controller and a dialog's both see
        /// is handled once (`keys_under_dialogs`).
        pub last_key: Cell<Option<(u32, u32)>>,
        /// The dialogs over the window, held so its changes keep being
        /// heard: libadwaita hands out a model of its own each time.
        pub dialogs: RefCell<Option<gtk::gio::ListModel>>,
        /// The snooze picker, built the first time `s` opens it.
        pub snooze: RefCell<Option<Rc<WhenPicker>>>,
        /// The row's right-click menu (T199), built on its first use.
        pub row_menu: RefCell<Option<Rc<crate::row_menu::RowMenu>>>,
        /// Whether the open row menu is for its row alone, outside the
        /// selection: a verb then lets the selection go first (T199).
        pub row_menu_alone: Cell<bool>,
        /// Commands `act` had no arm for, since a test last took them: the
        /// seam `registry_parity` reads to prove every offered command is
        /// answered.
        pub unanswered: RefCell<Vec<CommandId>>,
        /// The remind picker, built the first time `h` opens it.
        pub remind: RefCell<Option<Rc<WhenPicker>>>,
        /// The label picker, built the first time `l` opens it.
        pub labels: RefCell<Option<Rc<crate::label_picker::LabelPicker>>>,
        /// Decides a notification for new mail: the host's (FR-153).
        pub notifier: RefCell<Option<super::Notifier>>,
        /// Where a test takes notifications instead of the desktop.
        pub notification_sink: RefCell<Option<super::NotificationSink>>,
        /// Where `config.toml` is: what `Ctrl+S` writes a saved search to.
        pub config_path: RefCell<Option<std::path::PathBuf>>,
        /// The start being measured, from `startup::time` until the frame
        /// with mail in it closes it.
        pub timeline: RefCell<Option<postio_widgets::startup::Timeline>>,
        /// The digest rules list, built the first time `g d` opens it.
        pub rules_view: RefCell<Option<Rc<crate::rules::RulesView>>>,
        /// The digest rule dialog, built the first time `d` opens it.
        pub rule_dialog: RefCell<Option<Rc<crate::rule_dialog::RuleDialog>>>,
        /// The digest window, built the first time a digest opens.
        pub digest_window: RefCell<Option<Rc<crate::digest::DigestWindow>>>,
        /// The Filtered view, built the first time `g f` opens it.
        pub filtered: RefCell<Option<Rc<crate::filtered::FilteredView>>>,
        /// A `postio://` link that arrived before the store was open.
        pub pending_link: RefCell<Option<String>>,
        /// A `mailto:` link that arrived before the composer was mounted.
        pub pending_mailto: RefCell<Option<postio_model::mailto::Mailto>>,
        /// The move picker, built the first time `m` opens it.
        pub moves: RefCell<Option<Rc<crate::move_picker::MovePicker>>>,
        /// The composer, in its dialog (US3), once an account is known.
        pub compose: RefCell<Option<Rc<crate::compose::Compose>>>,
        /// Whether this window's last command answered an invitation: its
        /// toast lasts the answer's window (FR-102), not an ordinary one's.
        pub answering: Cell<bool>,
        /// Whether the composer's editing surface should start as soon as
        /// the composer is mounted: asked for before an account was known.
        pub warm: Cell<bool>,
        /// The capture sheet (US15), built the first time `t` or `n` opens
        /// it.
        pub capture: RefCell<Option<Rc<crate::capture::CaptureSheet>>>,
        /// The add-account form's dialog, while it is open (T171, T172):
        /// Focus's first run, and what `c` and `CommandId::AddAccount` offer
        /// while there is no account to write from.
        pub adding_account: RefCell<Option<adw::Dialog>>,
        /// Brings every enabled account's connection up, once there is one
        /// to bring up: the host's `start_syncing`, set from
        /// `startup::adopt_at` (ADR 0041). Called again once an account is
        /// added after a first run that started with none (T171).
        pub start_syncing: RefCell<Option<Rc<dyn Fn()>>>,
        /// Settings (T234), built on its first open and kept
        /// (`crate::settings`).
        pub settings: RefCell<Option<Rc<crate::settings::Settings>>>,
        /// What Settings asks of the process that opened the store, set by
        /// `startup::adopt_at`.
        pub settings_seams: RefCell<Option<crate::settings::Seams>>,
        /// Where `EditConfig` opens `config.toml`: the person's editor,
        /// unless a test has said otherwise (T235).
        pub editor: RefCell<Option<super::Editor>>,
        /// `[compose]` as the file last said: where a signature goes (T235).
        pub compose_config: RefCell<postio_config::ComposeConfig>,
        /// `[reader]`'s zoom as the file last said, once it has said one
        /// (T235).
        pub zoom: Cell<Option<u16>>,
    }

    impl Default for FocusWindow {
        fn default() -> Self {
            FocusWindow {
                pages: gtk::Stack::new(),
                stage: gtk::Overlay::new(),
                opening: adw::StatusPage::new(),
                unavailable: adw::StatusPage::new(),
                retry: gtk::Button::with_label("Try again"),
                bar_before_mail: gtk::WindowHandle::new(),
                inbox: gtk::Box::new(gtk::Orientation::Vertical, 0),
                toast: postio_widgets::widgets::toast::Toast::new(),
                on_screen: RefCell::default(),
                pane: RefCell::default(),
                chrome: RefCell::default(),
                client: RefCell::default(),
                waiting: Cell::default(),
                waiting_since: Cell::default(),
                on_retry: RefCell::default(),
                refusal: RefCell::default(),
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
                focus_config: RefCell::default(),
                list_or_empty: RefCell::default(),
                empty: RefCell::default(),
                reading: RefCell::default(),
                reading_pane: RefCell::default(),
                split: RefCell::default(),
                placement: Cell::default(),
                launcher: RefCell::default(),
                file_picker: RefCell::default(),
                choices: RefCell::default(),
                runtime: RefCell::default(),
                bar: RefCell::default(),
                saved: RefCell::default(),
                places: RefCell::default(),
                at_inbox: Cell::new(true),
                place: RefCell::default(),
                last_key: Cell::new(None),
                dialogs: RefCell::default(),
                snooze: RefCell::default(),
                row_menu: RefCell::default(),
                row_menu_alone: Cell::new(false),
                unanswered: RefCell::default(),
                remind: RefCell::default(),
                labels: RefCell::default(),
                moves: RefCell::default(),
                pending_link: RefCell::default(),
                pending_mailto: RefCell::default(),
                compose: RefCell::default(),
                warm: Cell::default(),
                answering: Cell::default(),
                filtered: RefCell::default(),
                digest_window: RefCell::default(),
                rule_dialog: RefCell::default(),
                rules_view: RefCell::default(),
                config_path: RefCell::default(),
                timeline: RefCell::default(),
                notifier: RefCell::default(),
                notification_sink: RefCell::default(),
                capture: RefCell::default(),
                adding_account: RefCell::default(),
                start_syncing: RefCell::default(),
                settings: RefCell::default(),
                settings_seams: RefCell::default(),
                editor: RefCell::default(),
                compose_config: RefCell::default(),
                zoom: Cell::default(),
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
    ///
    /// It opens at the size and maximised state it was closed at, from
    /// `$XDG_STATE_HOME/postio/window.ini`; a file that is missing or cannot
    /// be read means 1440 by 900 (row 10, T246).
    pub fn new(application: Option<&adw::Application>) -> Self {
        let saved = postio_widgets::state::Geometry::load(DEFAULT_GEOMETRY);
        glib::Object::builder()
            .property("application", application)
            .property("title", crate::app::NAME)
            .property("default-width", saved.width)
            .property("default-height", saved.height)
            .property("maximized", saved.maximized)
            .build()
    }

    /// Remember this window's size and maximised state for the next start.
    /// Best-effort: a state file that cannot be written is one line in the
    /// log and nothing more.
    fn save_geometry(&self) {
        let (width, height) = self.default_size();
        let saved = postio_widgets::state::Geometry {
            width,
            height,
            maximized: self.is_maximized(),
        };
        if let Err(error) = saved.save() {
            tracing::warn!(%error, "cannot save the window's size");
        }
    }

    fn build(&self) {
        crate::style::install(&WidgetExt::display(self));
        postio_widgets::style::install_icons(&WidgetExt::display(self));
        gtk::Window::set_default_icon_name(crate::app::ICON_NAME);
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

        // Before there is mail there is still a window to close: the same
        // close button the inbox's top bar ends with, in the same bar, and
        // the same Quit it runs (T216). Until this, nothing before the inbox
        // drew one, and the page saying the store would not open was a
        // window nobody could close.
        let close = postio_widgets::widgets::close_button();
        close.add_css_class("focus-close");
        close.add_css_class("circular");
        close.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.act(CommandId::Quit)
        ));
        let bar = gtk::CenterBox::new();
        bar.add_css_class("focus-top-bar");
        bar.set_end_widget(Some(&close));
        imp.bar_before_mail.set_child(Some(&bar));
        let before_mail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        before_mail.append(&imp.bar_before_mail);
        imp.pages.set_vexpand(true);
        before_mail.append(&imp.pages);
        imp.stage.set_child(Some(&before_mail));
        imp.toast.overlay().set_child(Some(&imp.stage));
        self.set_content(Some(imp.toast.overlay()));

        self.drop_focus_that_leaves();
        self.connect_is_active_notify(|window| window.focus_changed(window.is_active()));
        self.connect_close_request(|window| {
            window.save_geometry();
            glib::Propagation::Proceed
        });
        // Leaving the inbox's page for Filtered or the digest rules takes the
        // reading pane with it; coming back brings it (T232).
        imp.pages.connect_visible_child_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.place_reading()
        ));

        // Capture, not bubble: a single-key binding has to be seen before the
        // focused widget consumes it, and whether it should is the
        // resolver's decision.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, key, _, state| {
                window
                    .imp()
                    .last_key
                    .set(Some((controller.current_event_time(), key.into_glib())));
                window.handle_key(key, state)
            }
        ));
        self.add_controller(keys);
        self.keys_under_dialogs();

        // The pointer, before a dialog's scrim takes it: see
        // `click_through_dialog`.
        let clicks = gtk::GestureClick::new();
        clicks.set_propagation_phase(gtk::PropagationPhase::Capture);
        clicks.connect_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |gesture, _, x, y| {
                if window.click_through_dialog(x, y) {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                }
            }
        ));
        self.add_controller(clicks);

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
        // The main menu's "Read beside the list": `F8` for the pointer, a
        // check item whose state follows `[focus] reading` (T232).
        let reading = gio::SimpleAction::new_stateful(
            crate::chrome::READING_PANE_ACTION.trim_start_matches("win."),
            None,
            &false.to_variant(),
        );
        reading.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.act(CommandId::ToggleReadingPane)
        ));
        self.add_action(&reading);
        let about = gio::SimpleAction::new("about", None);
        about.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.show_about()
        ));
        self.add_action(&about);
    }

    /// Take the keyboard off a widget that leaves the window (T200).
    ///
    /// GTK delivers a key from the window's focus up through its parents, and
    /// leaves the focus where it was when its widget is removed: a row that
    /// went with its message, or a list that redrew. The key then starts at a
    /// widget with no parents and never reaches the window, so `j`, `z` and
    /// `Esc` all do nothing until a click. When the widget holding the
    /// keyboard leaves, the window has it instead, which is where every key
    /// goes when nothing does.
    fn drop_focus_that_leaves(&self) {
        // The widget being watched, and the watch: one at a time, moved with
        // the focus.
        let watch: std::rc::Rc<std::cell::RefCell<Option<(gtk::Widget, glib::SignalHandlerId)>>> =
            std::rc::Rc::default();
        self.connect_focus_widget_notify(move |window| {
            if let Some((old, id)) = watch.borrow_mut().take() {
                old.disconnect(id);
            }
            let Some(focus) = gtk::prelude::GtkWindowExt::focus(window) else {
                return;
            };
            let id = focus.connect_root_notify(glib::clone!(
                #[weak]
                window,
                move |gone| {
                    if gone.root().is_some() {
                        return;
                    }
                    let gone = gone.clone();
                    glib::idle_add_local_once(move || {
                        if gtk::prelude::GtkWindowExt::focus(&window).as_ref() == Some(&gone)
                            && gone.root().is_none()
                        {
                            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
                        }
                    });
                }
            ));
            watch.borrow_mut().replace((focus, id));
        });
    }

    /// Give every dialog over the window the window's keyboard (T195).
    ///
    /// GTK runs a key press through the widgets from the focus up to the
    /// innermost dialog presented over the window and stops there, so the
    /// window's own controller never sees a key while a dialog is up: `j`
    /// and `k` did nothing in the open message, and the arrows fell through
    /// to GTK's focus moves, which scrolled the column to whichever control
    /// took the focus (T196). Each dialog gets a controller of its own that
    /// asks the window, as the window's would have. The composer's too: its
    /// host's `handle_key` takes nothing, so before T200 Esc, Ctrl+Return and
    /// the composer's other keys reached nothing while it was open; the
    /// window's `handle_key` has a composer branch for exactly them.
    fn keys_under_dialogs(&self) {
        let dialogs = self.dialogs();
        dialogs.connect_items_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dialogs, at, _, added| {
                for index in at..at + added {
                    let Some(dialog) = dialogs.item(index).and_downcast::<adw::Dialog>() else {
                        continue;
                    };
                    window.give_keys(&dialog);
                }
            }
        ));
        self.imp().dialogs.replace(Some(dialogs));
    }

    /// `dialog`'s key controller, once: what [`Self::keys_under_dialogs`]
    /// adds.
    fn give_keys(&self, dialog: &adw::Dialog) {
        const KEYS: &str = "focus-window-keys";
        let controllers = dialog.observe_controllers();
        let has = (0..controllers.n_items()).any(|index| {
            controllers
                .item(index)
                .and_downcast::<gtk::EventController>()
                .is_some_and(|controller| controller.name().as_deref() == Some(KEYS))
        });
        if has {
            return;
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_name(Some(KEYS));
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |controller, key, _, state| {
                let press = (controller.current_event_time(), key.into_glib());
                // A press with no time is one a test emitted, not a repeat.
                if press.0 != 0 && window.imp().last_key.get() == Some(press) {
                    return glib::Propagation::Proceed;
                }
                window.handle_key(key, state)
            }
        ));
        dialog.add_controller(keys);
    }

    fn show_about(&self) {
        let about = adw::AboutDialog::builder()
            .application_name(crate::app::NAME)
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
        if let Some(menu) = imp.row_menu.borrow().as_ref() {
            menu.set_keymap(&keymap);
        }
        if let Some(reading) = imp.reading.borrow().as_ref() {
            reading.set_keymap(&keymap);
        }
        if let Some(pane) = imp.reading_pane.borrow().as_ref() {
            pane.set_keymap(&keymap);
        }
        if let Some(bar) = imp.bar.borrow().as_ref() {
            bar.set_keymap(&keymap);
        }
        if let Some(places) = imp.places.borrow().as_ref() {
            places.set_keymap(&keymap);
        }
        for picker in [&imp.snooze, &imp.remind] {
            if let Some(picker) = picker.borrow().as_ref() {
                picker.set_keymap(&keymap);
            }
        }
        if let Some(labels) = imp.labels.borrow().as_ref() {
            labels.set_keymap(&keymap);
        }
        if let Some(moves) = imp.moves.borrow().as_ref() {
            moves.set_keymap(&keymap);
        }
        if let Some(compose) = imp.compose.borrow().as_ref() {
            compose.set_keymap(&keymap);
        }
        if let Some(capture) = imp.capture.borrow().as_ref() {
            capture.set_keymap(&keymap);
        }
        if let Some(settings) = imp.settings.borrow().as_ref() {
            settings.set_keymap(&keymap, &problems);
        }
        imp.keymap.replace(keymap);
        // Every cap was drawn again: its control's shortcut follows it.
        crate::a11y::teach_shortcuts(self);
        // An open key map is drawn from the keymap: draw it again.
        if let Some(open) = self.key_map() {
            open.force_close();
            self.show_key_map();
        }
    }

    /// The keymap in force.
    pub fn keymap(&self) -> Keymap {
        self.imp().keymap.borrow().clone()
    }

    /// A key while there is no inbox yet -- the store is opening, or would
    /// not open: Quit, which is all there is to do, from the keymap the
    /// inbox will use (T216). The rest are the page's own, its button's
    /// Return among them.
    fn key_before_mail(&self, chord: &postio_ui::keymap::Chord) -> glib::Propagation {
        let (mut resolver, _) = crate::keys::resolver(&self.keymap());
        match resolver.press(chord, KeyContext::List, false, std::time::Instant::now()) {
            Outcome::Command(id) if id.parse::<CommandId>() == Ok(CommandId::Quit) => {
                self.act(CommandId::Quit);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// Deliver one key press to the resolver, and act on what it means.
    ///
    /// Public because it is the whole keyboard path in one call: the
    /// controller on this window forwards to it, and a test presses a key
    /// through it without synthesizing a GDK event, which GTK4 gives no
    /// supported way to do.
    pub fn handle_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        // A picker at the row has the keyboard: its keys are the picker
        // context's, and what it does not use goes on to its field.
        // The row menu has the keyboard while it is up (T199): Escape
        // closes it, a verb's key runs that verb from it, and the arrows and
        // Enter walk and press its items as GTK does.
        if let Some(menu) = self.row_menu().filter(|menu| menu.is_open()) {
            let command = postio_widgets::keys::chord(key, state).and_then(|chord| {
                let mut resolver = self.imp().resolver.borrow_mut();
                match resolver.as_mut()?.press(
                    &chord,
                    KeyContext::List,
                    false,
                    std::time::Instant::now(),
                ) {
                    Outcome::Command(id) => id.parse::<CommandId>().ok(),
                    _ => None,
                }
            });
            return if menu.press(command, key) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            };
        }
        if let Some(picker) = self.open_picker() {
            return if picker.press(key, state) {
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            };
        }
        // A Keyboard row in Settings waiting for its key takes every key,
        // Escape included, until it has one (T234).
        if self.settings_takes_every_key() {
            return glib::Propagation::Proceed;
        }
        let Some(chord) = postio_widgets::keys::chord(key, state) else {
            return glib::Propagation::Proceed;
        };
        if self.imp().resolver.borrow().is_none() {
            return self.key_before_mail(&chord);
        }
        let typing = self.is_typing();
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
        // The composer's dialog has the keyboard for the composer's own
        // commands: Send, Esc keeping the draft, and the rest (US3).
        if let Some(compose) = self.compose().filter(|compose| compose.is_showing()) {
            return match outcome {
                Outcome::Command(id) => match id.parse::<CommandId>() {
                    Ok(id) => {
                        compose.dispatch(id);
                        glib::Propagation::Stop
                    }
                    Err(_) => glib::Propagation::Proceed,
                },
                Outcome::Pending(_) => glib::Propagation::Stop,
                Outcome::Unhandled => glib::Propagation::Proceed,
            };
        }
        // A dialog over the window has the keyboard: only the keys that
        // close it are the window's, and the rest go on to the dialog
        // rather than moving the list underneath.
        if let Some(dialog) = self.visible_dialog() {
            if dialog.widget_name() == crate::open::DIALOG_NAME {
                // The arrows and the paging keys read the message: they are
                // the dialog's own, though the list's `Down` and `Up` are
                // the same chords, and the list is not what is being read.
                if self.scroll_reading(key, state) {
                    return glib::Propagation::Stop;
                }
                return self.reading_key(outcome);
            }
            if dialog.widget_name() == crate::digest::DIALOG_NAME {
                return self.digest_key(outcome);
            }
            if dialog.widget_name() == crate::capture::DIALOG_NAME {
                return self.capture_key(outcome);
            }
            if dialog.widget_name() == crate::settings::DIALOG_NAME {
                return self.settings_key(outcome);
            }
            return match outcome {
                Outcome::Command(id)
                    if matches!(
                        id.parse::<CommandId>(),
                        Ok(CommandId::CheatSheet | CommandId::Back)
                    ) =>
                {
                    dialog.close();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            };
        }
        // A message open beside the list has the open message's keys, as
        // the dialog does; the list keeps the rest, `/` and `x` among them
        // (T232).
        if self.reading_beside() {
            if self.scroll_reading(key, state) {
                return glib::Propagation::Stop;
            }
            if let Outcome::Command(id) = &outcome
                && let Ok(command) = id.parse::<CommandId>()
                && self.reading_key(outcome.clone()) == glib::Propagation::Proceed
            {
                self.act(command);
            }
            return glib::Propagation::Stop;
        }
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

    /// Whether the keyboard is on text entry: what the resolver is told, so
    /// a letter types rather than runs. A storyboard's observation reads the
    /// same answer.
    fn is_typing(&self) -> bool {
        gtk::prelude::GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
            || self.composer_body_has_keyboard()
    }

    /// Which surface owns the keyboard: the open message, or the list.
    pub fn key_context(&self) -> KeyContext {
        if self.compose().is_some_and(|compose| compose.is_showing()) {
            return KeyContext::Composer;
        }
        if self.settings_on_an_account() {
            // Settings' account list, the keyboard on a row (T258).
            KeyContext::Accounts
        } else if self
            .visible_dialog()
            .is_some_and(|dialog| dialog.widget_name() == crate::open::DIALOG_NAME)
            || (self.visible_dialog().is_none() && self.reading_beside())
        {
            KeyContext::Reader
        } else if self.capture().is_some() {
            KeyContext::Capture
        } else if self.digest().is_some() {
            KeyContext::Digest
        } else if self.bar().is_some_and(|bar| bar.is_open()) {
            // The bar is `Context::Search`: `Ctrl+S` saves what it holds,
            // `Ctrl+Backspace` goes back to the words (T086).
            KeyContext::Search
        } else if self.filtered().is_some() {
            KeyContext::Filtered
        } else {
            KeyContext::List
        }
    }

    /// A key while the capture sheet is up: its own commands, and `Esc`.
    fn capture_key(&self, outcome: Outcome) -> glib::Propagation {
        match outcome {
            Outcome::Command(id) => match (id.parse::<CommandId>(), self.capture()) {
                (Ok(id), Some(sheet)) => {
                    sheet.run(id);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            },
            Outcome::Pending(_) => glib::Propagation::Stop,
            Outcome::Unhandled => glib::Propagation::Proceed,
        }
    }

    /// The capture sheet, once `t` or `n` has opened it.
    pub fn capture(&self) -> Option<Rc<crate::capture::CaptureSheet>> {
        self.imp().capture.borrow().clone()
    }

    /// `t` or `n`: the capture sheet for the message aimed at, as a task or
    /// a note (US15). With no vault configured, it says so instead.
    fn open_capture(&self, mode: crate::capture::Mode) {
        let imp = self.imp();
        if imp.focus_config.borrow().vault.is_none() {
            imp.toast.show_notice(crate::capture::NO_VAULT);
            self.follow_toast();
            return;
        }
        let (Some(client), Some(source)) = (imp.client.borrow().clone(), self.capture_source())
        else {
            return;
        };
        let sheet = imp.capture.borrow().clone();
        let sheet = sheet.unwrap_or_else(|| {
            let sheet = crate::capture::CaptureSheet::new(client, &self.keymap());
            sheet.connect_written(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |said| {
                    window.imp().toast.show_notice(&said);
                    window.follow_toast();
                }
            ));
            imp.capture.replace(Some(Rc::clone(&sheet)));
            sheet
        });
        sheet.open(self, source, mode);
        crate::a11y::teach_shortcuts(sheet.dialog());
        crate::motion::keep_to_budget(sheet.dialog());
    }

    /// What a capture is made from: the message open over the list when one
    /// is, the cursor's row otherwise, with its marker's sentence and day.
    fn capture_source(&self) -> Option<postio_ui::capture::Source> {
        let message = self.aimed_message()?;
        let title = self
            .reading()
            .map(|reading| reading.title())
            .unwrap_or_default();
        postio_ui::capture::source(
            message,
            self.cursor_row().as_ref(),
            &title,
            postio_ui::clock::now(),
        )
    }

    /// Scroll the open message for `key`, when it is one that scrolls:
    /// the arrows a line, `Page_Up`/`Page_Down` and `space` a screen, `Home`
    /// and `End` to the ends. Whether it did.
    fn scroll_reading(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        let Some(reading) = self.reading() else {
            return false;
        };
        let reader = reading.reader();
        let plain = !state.intersects(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        );
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        match key {
            gdk::Key::Down if plain && !shift => reader.scroll_lines(1),
            gdk::Key::Up if plain && !shift => reader.scroll_lines(-1),
            gdk::Key::Page_Down if plain => reader.page_down(),
            gdk::Key::Page_Up if plain => reader.page_up(),
            gdk::Key::space if plain && !shift => reader.page_down(),
            gdk::Key::space if plain => reader.page_up(),
            gdk::Key::Home if plain && !shift => reader.scroll_to_edge(false),
            gdk::Key::End if plain && !shift => reader.scroll_to_edge(true),
            _ => return false,
        }
        true
    }

    /// A key while a message is open over the list (US2 scenarios 1-3):
    /// `Esc` closes it and leaves the list as it was, `j`/`k` step the list
    /// and move its cursor behind the dialog, and `[`/`]` step the thread.
    fn reading_key(&self, outcome: Outcome) -> glib::Propagation {
        let Outcome::Command(id) = outcome else {
            return match outcome {
                Outcome::Pending(_) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            };
        };
        let Some(reading) = self.reading() else {
            return glib::Propagation::Proceed;
        };
        match id.parse::<CommandId>() {
            // Escape closes More's menu first, as it does any popover: the
            // keyboard goes back to More and the message stays open.
            Ok(CommandId::Back) if reading.more_open() => reading.close_more(),
            // Escape closes find first, the message only after (T203).
            Ok(CommandId::Back) if reading.reader().finding() => reading.reader().close_find(),
            Ok(CommandId::Back) => reading.close(),
            Ok(CommandId::FindInMessage) => reading.reader().find_in_message(),
            // App colours or the original, for this message (T213).
            Ok(CommandId::SwitchTreatment) => {
                reading.reader().switch_treatment();
            }
            Ok(CommandId::FindNext) => reading.reader().find_step(true),
            Ok(CommandId::FindPrevious) => reading.reader().find_step(false),
            Ok(CommandId::NextMessage) => {
                self.move_cursor(1);
                self.open_message();
            }
            Ok(CommandId::PrevMessage) => {
                self.move_cursor(-1);
                self.open_message();
            }
            Ok(CommandId::ViewSource) => self.view_source(),
            // The size of the message on screen, in the dialog and beside
            // the list (T242), and the banner's buttons (T247).
            Ok(
                id @ (CommandId::ZoomIn
                | CommandId::ZoomOut
                | CommandId::ZoomReset
                | CommandId::ShowImages
                | CommandId::AlwaysShowImages
                | CommandId::Unsubscribe),
            ) => self.act(id),
            Ok(CommandId::DismissMarker) => self.dismiss_marker(),
            Ok(CommandId::MoreActions) => reading.show_more(),
            // The open message moves between the dialog and the pane (T232).
            Ok(CommandId::ToggleReadingPane) => self.toggle_reading_pane(),
            // Edit: `Return` on a draft on its way or stopped, already open,
            // writes it (T239).
            Ok(CommandId::OpenMessage) if self.offered_on_open_draft(CommandId::OpenMessage) => {
                self.act(CommandId::OpenMessage);
            }
            // `Return` on the row already open beside the list: it is open.
            Ok(CommandId::OpenMessage) if reading.in_pane() => self.open_message(),
            Ok(CommandId::OpenAttachmentOrLink) => self.offer_choices(None),
            // What settles a send (T239).
            Ok(id @ (CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent)) => {
                self.act(id);
            }
            // Read or unread, set by the person: the read clock stops, so
            // a message kept unread stays unread while it is open (T237).
            Ok(CommandId::ToggleRead) => {
                reading.cancel_dwell();
                self.act(CommandId::ToggleRead);
            }
            // Screen 04's toolbar verbs and the Invite card's answers, for
            // the message on screen (US3, US8).
            Ok(
                id @ (CommandId::Reply
                | CommandId::ReplyAll
                | CommandId::Forward
                | CommandId::Compose
                | CommandId::AcceptInvite
                | CommandId::DeclineInvite),
            ) => self.act(id),
            // The message on screen goes away, and the dialog goes on to
            // the next one (T190).
            // So does waking one, from the Snoozed list it is leaving (T238).
            Ok(id @ (CommandId::Archive | CommandId::Delete | CommandId::Unsnooze))
                if id != CommandId::Unsnooze
                    || self.pane().is_some_and(|pane| {
                        matches!(
                            pane.feed().scope(),
                            Some(ListScope::Focus(FocusScope::Snoozed))
                        )
                    }) =>
            {
                let gone = self
                    .pane()
                    .zip(self.cursor_row())
                    .map(|(pane, row)| (pane.cursor().selected(), row.id()));
                self.act(id);
                if let Some((index, message)) = gone {
                    self.step_past(index, message);
                }
            }
            // The action row's other verbs act on the message on screen:
            // their pickers open over it, and Undo takes back the last act.
            Ok(
                id @ (CommandId::AddLabel
                | CommandId::Move
                | CommandId::Snooze
                | CommandId::RemindIfNoReply
                | CommandId::Undo),
            ) => self.act(id),
            Ok(CommandId::PrevInConversation) => reading.step_thread(-1),
            Ok(CommandId::NextInConversation) => reading.step_thread(1),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// The row at `index` was `message`, and is being archived or deleted
    /// under the open dialog: once the list has let it go, show what took
    /// its place -- the next message, or the previous one when it was the
    /// last -- and close the dialog when the list is empty (T190).
    fn step_past(&self, index: u32, message: MessageId) {
        let window = self.downgrade();
        let mut waited = 0;
        glib::timeout_add_local(std::time::Duration::from_millis(20), move || {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(pane) = window.pane() else {
                return glib::ControlFlow::Break;
            };
            let list = pane.feed().list();
            let len = list.n_items();
            let still_there = list
                .item(index)
                .and_downcast::<RowObject>()
                .and_then(|row| row.item())
                .is_some_and(|row| row.id() == message);
            if still_there && waited < 250 {
                waited += 1;
                return glib::ControlFlow::Continue;
            }
            if still_there {
                // The action never landed: leave the dialog as it was.
                return glib::ControlFlow::Break;
            }
            if let Some(reading) = window.reading().filter(|reading| reading.is_open()) {
                if len == 0 {
                    reading.close();
                } else {
                    window.cursor_to(Some(index.min(len - 1)));
                    window.open_message();
                }
            }
            glib::ControlFlow::Break
        });
    }

    /// Whether the composer's body has the keyboard: a `WebView`, which no
    /// type test says is being typed in (#602).
    pub(crate) fn composer_body_has_keyboard(&self) -> bool {
        self.compose().is_some_and(|compose| {
            compose.composer().focused_field() == Some(postio_widgets::composer::Field::Body)
        })
    }

    /// The command `key` runs in `context`, resolved against the keys in
    /// force, without running it: what a detached composer's window asks.
    pub fn command_in(
        &self,
        key: gdk::Key,
        state: gdk::ModifierType,
        context: KeyContext,
        typing: bool,
    ) -> Option<CommandId> {
        let chord = postio_widgets::keys::chord(key, state)?;
        let mut resolver = self.imp().resolver.borrow_mut();
        match resolver
            .as_mut()?
            .press(&chord, context, typing, std::time::Instant::now())
        {
            Outcome::Command(id) => id.parse::<CommandId>().ok(),
            _ => None,
        }
    }

    /// The composer and its dialog, once an account is known.
    fn compose(&self) -> Option<Rc<crate::compose::Compose>> {
        self.imp().compose.borrow().clone()
    }

    /// Mount the composer for `account`, writing through `client`.
    fn mount_compose(&self, client: &Client, account: AccountId) {
        if self.imp().compose.borrow().is_some() {
            return;
        }
        let window = self.downgrade();
        let current: crate::compose::Current =
            Rc::new(move || window.upgrade().and_then(|window| window.aimed_message()));
        let compose = crate::compose::Compose::new(self, client, account, current);
        if self.imp().warm.get() {
            compose.warm();
        }
        // Where its signature goes, as `[compose]` says (T235).
        crate::settings::place_signatures(compose.composer(), &self.imp().compose_config.borrow());
        if let Some(pane) = self.imp().reading_pane.borrow().as_ref()
            && self.imp().placement.get() == postio_ui::focus_dialog::Placement::Pane
        {
            compose.place(Some(pane.compose_slot()));
        }
        self.imp().compose.replace(Some(Rc::clone(&compose)));
        if let Some(mailto) = self.imp().pending_mailto.take() {
            compose.open_mailto(mailto);
        }
    }

    /// Refresh which accounts are enabled, mount the composer for the
    /// first one once there is one, and offer the first-run form
    /// (T171) while there still is none -- once when the inbox is shown,
    /// and again once an account has just been saved.
    fn refresh_accounts(&self, client: Client) {
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let Ok(accounts) = client.accounts().await else {
                return;
            };
            let Some(window) = window.upgrade() else {
                return;
            };
            let enabled: Vec<&postio_model::Account> =
                accounts.iter().filter(|account| account.enabled).collect();
            window
                .imp()
                .accounts
                .replace(enabled.iter().map(|account| account.id).collect());
            // A new message is written from the first account.
            if let Some(account) = enabled.first() {
                window.mount_compose(&client, account.id);
            }
            window.imp().facts.replace(
                enabled
                    .iter()
                    .map(|account| postio_ui::focus_state::AccountFacts {
                        id: account.id,
                        server: account.incoming.host.clone(),
                        address: account.address.address.clone(),
                        name: if account.display_name.is_empty() {
                            account.address.address.clone()
                        } else {
                            account.display_name.clone()
                        },
                    })
                    .collect(),
            );
            // No account at all: Focus's first run (T171), or a `c` or
            // `CommandId::AddAccount` while none has been added yet.
            if enabled.is_empty() {
                window.open_add_account();
                return;
            }
            // When mail last arrived, from before this run: the newest
            // folder's last completed sync.
            let mut last = None;
            for account in &enabled {
                use postio_model::listing::MailStore as _;
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime (ADR 0041).
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
        });
    }

    /// The add-account form over this window (T171, T172,
    /// `CommandId::AddAccount`): the shared form both desktop apps drive
    /// (`postio_widgets::present::onboarding::add_account`, T165). Saving
    /// refreshes the accounts Focus knows -- mounting the composer among
    /// them -- and brings every account's connection up, in that order.
    ///
    /// With no account at all this is the first run: once the account is
    /// saved the form asks how much history to sync (#876, T243) before the
    /// connection comes up. An account added
    /// to a window that has one joins a window already syncing.
    ///
    /// A second call while the form is already open reuses it rather than
    /// stacking a second wizard over the first.
    fn open_add_account(&self) {
        let imp = self.imp();
        let first_run = imp.accounts.borrow().is_empty();
        if imp.adding_account.borrow().is_some() {
            return;
        }
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let open_link = postio_widgets::present::onboarding::open_in_browser(self);
        let saved = {
            let window = self.downgrade();
            let client = client.clone();
            move |_submission: &postio_widgets::onboarding::Submission| {
                let Some(window) = window.upgrade() else {
                    return;
                };
                window.refresh_accounts(client.clone());
                if let Some(start) = window.imp().start_syncing.borrow().clone() {
                    start();
                }
            }
        };
        let dialog = if first_run {
            postio_widgets::present::onboarding::add_account_asking_history(
                self, &client, open_link, saved,
            )
        } else {
            postio_widgets::present::onboarding::add_account(self, &client, open_link, saved)
        };
        dialog.connect_closed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                window.imp().adding_account.replace(None);
            }
        ));
        imp.adding_account.replace(Some(dialog));
    }

    /// A pointer press at (`x`, `y`) in the window, before any dialog over it
    /// sees it. Whether the window took it.
    ///
    /// Only the add-account form's dialog is a way in that must not shut the
    /// window's own close button out: it covers the window on a first run,
    /// where closing the app is the one other thing a person can want, and a
    /// modal dialog gives its scrim the click. A click on the button while
    /// that form is up closes the app.
    pub fn click_through_dialog(&self, x: f64, y: f64) -> bool {
        let imp = self.imp();
        if imp.adding_account.borrow().is_none() {
            return false;
        }
        let Some(chrome) = imp.chrome.borrow().clone() else {
            return false;
        };
        let Some(bounds) = chrome.close_button().compute_bounds(self) else {
            return false;
        };
        let (x, y) = (x as f32, y as f32);
        if x < bounds.x()
            || x > bounds.x() + bounds.width()
            || y < bounds.y()
            || y > bounds.y() + bounds.height()
        {
            return false;
        }
        // A window asked to close with a dialog over it closes the dialog
        // and stays; the app is closing, so the form goes first.
        if let Some(form) = imp.adding_account.take() {
            form.force_close();
        }
        self.act(CommandId::Quit);
        true
    }

    /// The add-account form's dialog, while it is open.
    pub fn add_account_dialog(&self) -> Option<adw::Dialog> {
        self.imp().adding_account.borrow().clone()
    }

    /// What `c` and the top bar's compose button say while there is no
    /// account to write from (T172): the composer is mounted only once one
    /// is known (`Self::mount_compose`), so this says what is missing and
    /// offers the same form T171 opens on a first run, rather than doing
    /// nothing.
    fn offer_add_account_for_compose(&self) {
        let window = self.downgrade();
        self.imp().toast.show_prompt(
            "There's no account to write from yet.",
            "Add account",
            move || {
                if let Some(window) = window.upgrade() {
                    window.open_add_account();
                }
            },
        );
        self.follow_toast();
    }

    /// Open the draft behind the Drafts row `message` for editing, in the
    /// composer's dialog: what a Drafts row does when it is opened, whichever
    /// app left the draft there (US11 scenario 3).
    pub fn open_draft(&self, message: MessageId) {
        if let Some(compose) = self.compose() {
            compose.open_draft(message);
        }
    }

    /// Start the composer's editing surface, while nobody is waiting on it.
    pub fn warm_composer(&self) {
        self.imp().warm.set(true);
        if let Some(compose) = self.compose() {
            compose.warm();
        }
    }

    /// Run the command `id` means here: the cursor and the selection are the
    /// window's own, and a verb on mail goes to the host aimed at them.
    pub fn act(&self, id: CommandId) {
        // A stall after this is blamed on it (`postio_widgets::jank`).
        postio_widgets::jank::note_action(id);
        // The rules list takes the list's keys for its own rows; a verb on
        // mail has nothing under it there.
        if self.rules().is_some() && self.rules_act(id) {
            return;
        }
        // So does Filtered: its rows are walked and opened as the list's
        // are, and `g i` goes back (screen 21's footer).
        if let Some(view) = self.filtered() {
            match id {
                CommandId::NextMessage => return view.step(1),
                CommandId::PrevMessage => return view.step(-1),
                CommandId::OpenMessage => return view.open_focused(),
                CommandId::GoToInbox => return self.leave_filtered(),
                _ => {}
            }
        }
        match id {
            CommandId::NextMessage => self.move_cursor(1),
            CommandId::PrevMessage => self.move_cursor(-1),
            CommandId::FirstMessage => self.cursor_to(Some(0)),
            CommandId::LastMessage => {
                let last = self.list_len().checked_sub(1);
                self.cursor_to(last);
            }
            CommandId::ToggleSelection => {
                if let Some(row) = self
                    .cursor_row()
                    .filter(|row| !matches!(row, FocusRow::Digest(_)))
                {
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
            CommandId::Back => {
                if let Some(places) = self.places().filter(|places| places.is_open()) {
                    places.close();
                } else if let Some(bar) = self.bar().filter(|bar| bar.is_open()) {
                    bar.close();
                } else if self.filtered().is_some() {
                    self.leave_filtered();
                } else {
                    self.clear_selection();
                }
            }
            CommandId::SavedSearch1
            | CommandId::SavedSearch2
            | CommandId::SavedSearch3
            | CommandId::SavedSearch4 => {
                let index = match id {
                    CommandId::SavedSearch1 => 0,
                    CommandId::SavedSearch2 => 1,
                    CommandId::SavedSearch3 => 2,
                    _ => 3,
                };
                let opened = self.bar().is_some_and(|bar| bar.open_saved(index));
                if !opened {
                    self.imp()
                        .toast
                        .show_notice(&postio_ui::focus_target::no_saved_search(index));
                    self.follow_toast();
                }
            }
            // `/` is for mail, `Ctrl K` for commands: one bar, opened two
            // ways (spec C24).
            CommandId::Search => {
                if let Some(bar) = self.bar() {
                    bar.open();
                }
            }
            CommandId::CommandPalette => {
                if let Some(bar) = self.bar() {
                    bar.open_commands();
                }
            }
            CommandId::Archive if self.digest_at_cursor().is_some() => {
                if let Some(digest) = self.digest_at_cursor() {
                    self.archive_digest(digest.delivery);
                }
            }
            // The host's own verbs: `postio_ui::focus_target::dispatch` says
            // which are aimed at mail and which are not. Undo is the last
            // action this window took, whatever it was and however long ago
            // the toast went (FR-041): the host keeps the stack.
            // A toast with an Undo of its own -- a send, queued -- is the
            // last thing said, so Undo takes that back first (#1752).
            CommandId::Undo if self.imp().toast.activate_undo() => {}
            CommandId::Archive
            | CommandId::Delete
            | CommandId::ToggleRead
            | CommandId::Flag
            | CommandId::Unsnooze
            | CommandId::Undo
            | CommandId::Refresh => self.dispatch(id),
            CommandId::ToggleHasAction => self.toggle_has_action(),
            CommandId::Quit => self.close(),
            CommandId::UpdateCredential => self.update_credential(),
            CommandId::CheatSheet => self.show_key_map(),
            // Settings (T234) and its file in the person's editor (T235):
            // `crate::settings`.
            CommandId::Settings => self.toggle_settings(),
            CommandId::EditConfig => self.edit_config(),
            // The account verbs, on the row Settings has focused (T258).
            CommandId::ToggleAccountEnabled
            | CommandId::RemoveAccount
            | CommandId::RebuildAccountIndex
            | CommandId::SetDefaultAccount
            | CommandId::MapMailboxRole => self.account_verb(id),
            // Edit, on a draft on its way or stopped that is open (T239):
            // the composer takes a waiting send off the queue before
            // anything is edited.
            CommandId::OpenMessage if self.offered_on_open_draft(id) => {
                if let Some(reading) = self.reading()
                    && let Some(message) = reading.shown()
                {
                    reading.close();
                    self.open_draft(message);
                }
            }
            CommandId::OpenMessage => match self.digest_at_cursor() {
                Some(digest) => self.open_digest(digest),
                None => self.open_message(),
            },
            CommandId::GoToFolders => self.open_places(),
            CommandId::GoToInbox => self.go_to_inbox(),
            CommandId::ViewSource => self.view_source(),
            CommandId::GoToFiltered => self.show_filtered(),
            CommandId::SweepInbox => self.ask_sweep(),
            CommandId::DismissMarker => self.dismiss_marker(),
            CommandId::MoreActions => {
                if let Some(reading) = self.reading() {
                    reading.show_more();
                }
            }
            CommandId::DigestRule => self.new_digest_rule(),
            CommandId::ToggleReadingPane => self.toggle_reading_pane(),
            CommandId::BackToWords => {
                if let Some(bar) = self.bar() {
                    bar.back_to_words();
                }
            }
            CommandId::SaveSearch => self.save_search(),
            CommandId::GoToDigestRules => self.show_rules(),
            CommandId::RestoreFiltered => {
                if let Some(message) = self.filtered().and_then(|view| view.focused()) {
                    self.restore_filtered(message);
                }
            }
            CommandId::FilteredTab1
            | CommandId::FilteredTab2
            | CommandId::FilteredTab3
            | CommandId::FilteredTab4
            | CommandId::FilteredTab5
            | CommandId::FilteredTab6
            | CommandId::FilteredTab7 => {
                let index = postio_ui::filtered::TAB_COMMANDS
                    .iter()
                    .position(|tab| *tab == id);
                if let (Some(view), Some(index)) = (self.filtered(), index) {
                    view.set_tab(index);
                }
            }
            CommandId::Snooze => self.open_when(When::Snooze),
            CommandId::RemindIfNoReply => self.open_when(When::Remind),
            CommandId::AddLabel => self.open_labels(),
            CommandId::Move => self.open_moves(),
            // Answered from the row (US8): the invitation the cursor is on.
            CommandId::AcceptInvite | CommandId::DeclineInvite => {
                if let Some(message) = self.aimed_message() {
                    self.answer(message, id);
                }
            }
            // `g t`: the Drafts folder, where Enter opens a draft to edit.
            CommandId::GoToDrafts => self.go_to_role(postio_model::MailboxRole::Drafts),
            // The rest of the go-to keys (T236): a folder by its role, a
            // view by its scope.
            CommandId::GoToSent => self.go_to_role(postio_model::MailboxRole::Sent),
            CommandId::GoToArchive => self.go_to_role(postio_model::MailboxRole::Archive),
            CommandId::GoToSnoozed => self.go_to_view(postio_model::MailboxRole::Snoozed),
            CommandId::GoToFlagged => self.go_to_view(postio_model::MailboxRole::Flagged),
            CommandId::GoToJunk => self.go_to_role(postio_model::MailboxRole::Junk),
            CommandId::GoToTrash => self.go_to_role(postio_model::MailboxRole::Trash),
            // A view over Drafts, of the first account, as the popover's row
            // for it is of the account it lists.
            CommandId::GoToOutbox => {
                let account = self.imp().accounts.borrow().first().copied();
                if let Some(account) = account {
                    self.go_to(
                        postio_ui::finder::Destination::Outbox(account),
                        postio_ui::places::OUTBOX,
                    );
                }
            }
            // A Focus row is a whole conversation, so `A` is `a` here.
            CommandId::ArchiveThread => self.act(CommandId::Archive),
            // A send on its way or stopped, from the list or the open
            // message (T239): the draft behind the message aimed at.
            CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent => {
                // One the open message offers moves the message out of the
                // list it was opened from, so it closes; the cursor stays,
                // and the list's next row -- a draft being written, often --
                // is not opened in its place.
                let offered = self.offered_on_open_draft(id);
                self.settle_send(id);
                if offered && let Some(reading) = self.reading() {
                    reading.close();
                }
            }
            // The capture sheet (US15), from the row or the open message.
            CommandId::CaptureTask => self.open_capture(crate::capture::Mode::Task),
            CommandId::CaptureNote => self.open_capture(crate::capture::Mode::Note),
            // The shared composer, in its dialog (US3). With no
            // account there is nothing to write from yet (T172).
            CommandId::Compose => match self.compose() {
                Some(compose) => compose.dispatch(id),
                None => self.offer_add_account_for_compose(),
            },
            // Not on mail still on its way out (#1749): a notice says why.
            CommandId::Reply | CommandId::ReplyAll
                if postio_ui::focus_target::refuses_reply(self.aimed_send_state()) =>
            {
                self.imp()
                    .toast
                    .show_notice(postio_ui::focus_target::NO_REPLY_TO_OUTGOING);
                self.follow_toast();
            }
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward => {
                if let Some(compose) = self.compose() {
                    compose.dispatch(id);
                }
            }
            // Focus's first run (T171), and what `c` offers with no
            // account (T172).
            CommandId::AddAccount => self.open_add_account(),
            // The open message's size and its banner's buttons (T242,
            // T247): nothing happens with no message open or nothing held
            // back.
            CommandId::ZoomIn | CommandId::ZoomOut | CommandId::ZoomReset => {
                if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
                    match id {
                        CommandId::ZoomIn => reading.reader().zoom_in(),
                        CommandId::ZoomOut => reading.reader().zoom_out(),
                        _ => reading.reader().zoom_reset(),
                    }
                }
            }
            // `U` leaves the list on any message the shared rule allows, band
            // or not: the band only shows for list mail (T261), the key is
            // not tied to it.
            CommandId::Unsubscribe => {
                if let Some(reading) = self.reading().filter(|reading| reading.is_open())
                    && postio_ui::reader::header::ReaderAction::unsubscribable(reading.send_state())
                    && let Some(message) = reading.shown()
                {
                    self.unsubscribe(message);
                }
            }
            CommandId::ShowImages | CommandId::AlwaysShowImages => {
                if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
                    reading.reader().run_banner_command(id);
                }
            }
            _ => {
                tracing::debug!(command = %id, "no Focus surface answers this command yet");
                self.imp().unanswered.borrow_mut().push(id);
            }
        }
    }

    /// The commands `act` has had no arm for since this was last called.
    pub fn take_unanswered(&self) -> Vec<CommandId> {
        std::mem::take(&mut *self.imp().unanswered.borrow_mut())
    }

    /// How many rows the list has.
    fn list_len(&self) -> u32 {
        self.pane()
            .map(|pane| pane.feed().list().n_items())
            .unwrap_or(0)
    }

    /// Give the keyboard to the list: the one place it rests when nothing
    /// else is being typed into. At launch, after the bar closes, after a
    /// place is chosen and after an overlay goes, the window's real focus is
    /// the list view -- not the top bar's Compose button, nor the header's
    /// place button, where Space or Return would press a control instead of
    /// acting on the row under the cursor. The view itself, not a row of
    /// it: the cursor is the row that is drawn, and a row that also held
    /// GTK's focus would be a second ring.
    pub fn focus_list(&self) {
        if let Some(pane) = self.pane() {
            gtk::prelude::GtkWindowExt::set_focus(self, Some(pane.view()));
        }
    }

    /// [`Self::focus_list`] once what is closing has had its say: a popover
    /// hands the keyboard back to what had it before, on the way out.
    fn focus_list_soon(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                if window.keyboard_is_free() {
                    window.focus_list();
                }
            }
        ));
    }

    /// Whether nothing is holding the keyboard for itself: no dialog over
    /// the window, no open bar or popover, and nothing being typed.
    fn keyboard_is_free(&self) -> bool {
        self.visible_dialog().is_none()
            && !self.is_typing()
            && self.bar().is_none_or(|bar| !bar.is_open())
            && self.places().is_none_or(|places| !places.is_open())
    }

    /// Put the cursor on `position`, or on nothing, and bring it into view.
    fn cursor_to(&self, position: Option<u32>) {
        let Some(pane) = self.pane() else {
            return;
        };
        // A click on a row gives GTK's focus to that row, and the cursor
        // moving on would leave it behind with a ring of its own.
        if let Some(focus) = gtk::prelude::GtkWindowExt::focus(self)
            && focus != *pane.view().upcast_ref::<gtk::Widget>()
            && focus.is_ancestor(pane.view())
        {
            self.focus_list();
        }
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

    /// A Ctrl-click (`Toggle`: `x` on that row) or Shift-click (`Range`: the
    /// `Shift`+`j`/`k` walk from the anchor to that row), through the same
    /// commands the keys run.
    fn pick(&self, row: &FocusRow, pick: crate::list::row::Pick) {
        use crate::list::row::Pick;
        let Some(target) = self
            .pane()
            .and_then(|pane| pane.feed().list().position_of(row.id()))
        else {
            return;
        };
        match pick {
            Pick::Toggle => {
                self.cursor_to(Some(target));
                self.act(CommandId::ToggleSelection);
            }
            Pick::Range => {
                // From the anchor (the cursor, with none yet) to the row.
                let from = self
                    .imp()
                    .picked
                    .anchor()
                    .and_then(|anchor| {
                        self.pane()
                            .and_then(|pane| pane.feed().list().position_of(anchor))
                    })
                    .or_else(|| self.pane().map(|pane| pane.cursor().selected()))
                    .filter(|from| *from != gtk::INVALID_LIST_POSITION)
                    .unwrap_or(target);
                self.cursor_to(Some(from));
                let (step, command) = if target >= from {
                    (1, CommandId::ExtendSelectionDown)
                } else {
                    (-1, CommandId::ExtendSelectionUp)
                };
                let mut at = from;
                while at != target {
                    self.act(command);
                    at = (i64::from(at) + step) as u32;
                }
            }
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
        let cursor = self.cursor_row();
        let aim = postio_ui::focus_target::aim(
            &imp.picked.selection(),
            &imp.reach.borrow(),
            cursor.as_ref(),
        );
        match aim {
            postio_ui::focus_target::Aim::Everything { except } => {
                if let Some(state) = imp.state.borrow().as_ref() {
                    let accounts = imp.accounts.borrow().clone();
                    let (sink, _) = postio_core::bridge::event_channel();
                    state.update(&sink, |app| {
                        let mut events = app.open_view(ViewScope::Focus { accounts });
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
            postio_ui::focus_target::Aim::Targets(targets) => targets,
        }
    }

    /// Send the host command `id` means, as `postio_ui::focus_target` says:
    /// aimed at mail, or as it is.
    fn dispatch(&self, id: CommandId) {
        use postio_ui::focus_target::Dispatch;
        match postio_ui::focus_target::dispatch(id) {
            Some(Dispatch::OnMail(command)) => self.send(command),
            Some(Dispatch::Plain(command)) => self.post(command),
            None => {}
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
        self.show_refusal(
            "Postio can\u{2019}t open your mail",
            reason,
            "Try again",
            Rc::new(retry),
        );
    }

    /// The store was written at a schema no migration carries forward, so
    /// trying again would meet the same file (T215): say so, say what a
    /// fresh store keeps and what stays behind before it is chosen, and
    /// offer it. "Start a fresh store" runs `start_over`.
    pub fn show_start_over(&self, start_over: impl Fn() + 'static) {
        self.show_refusal(
            "Your mail store is from another version of Postio",
            START_OVER,
            "Start a fresh store",
            Rc::new(start_over),
        );
    }

    /// Say a fresh store is being started, with the button that started it
    /// held until the store opens or says why not.
    pub fn show_starting_over(&self) {
        let imp = self.imp();
        imp.retry.set_label("Starting a fresh store\u{2026}");
        imp.retry.set_sensitive(false);
    }

    fn show_refusal(&self, title: &str, reason: &str, action: &str, run: Rc<dyn Fn()>) {
        let imp = self.imp();
        imp.waiting.set(None);
        imp.waiting_since.set(None);
        imp.unavailable.set_title(title);
        imp.unavailable
            .set_description(Some(&glib::markup_escape_text(reason)));
        imp.refusal.replace(reason.to_owned());
        imp.retry.set_label(action);
        imp.retry.set_sensitive(true);
        imp.on_retry.replace(Some(run));
        // The inbox's own top bar took the close button when it opened and
        // is behind this page now: the one before mail comes back, so the
        // page that says why there is no mail can always be closed.
        if imp.bar_before_mail.parent().is_none()
            && let Some(before_mail) = imp.stage.child().and_downcast::<gtk::Box>()
        {
            before_mail.prepend(&imp.bar_before_mail);
        }
        imp.pages.set_visible_child_name(UNAVAILABLE);
        imp.retry.grab_focus();
    }

    /// Whether the window has a close button on show: the inbox's top bar
    /// has it over the inbox, the bar before mail over every other page.
    pub fn close_button_showing(&self) -> bool {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() == Some(INBOX) {
            imp.chrome
                .borrow()
                .as_ref()
                .is_some_and(|chrome| chrome.close_button().is_drawable())
        } else {
            imp.bar_before_mail.parent().is_some() && imp.bar_before_mail.is_drawable()
        }
    }

    /// The sentence the window shows when it has no mail, if it is showing
    /// one.
    pub fn unavailable_reason(&self) -> Option<String> {
        let imp = self.imp();
        (imp.pages.visible_child_name().as_deref() == Some(UNAVAILABLE))
            .then(|| imp.refusal.borrow().clone())
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
        // The inbox's own top bar has the close button from here on: out of
        // the tree, not hidden, so the window has one close button.
        if let Some(holder) = imp.bar_before_mail.parent().and_downcast::<gtk::Box>() {
            holder.remove(&imp.bar_before_mail);
        }
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
            move |action| window.act(banner_command(action))
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
        // A row's drawn action does what its key does, for that row.
        pane.connect_row_action(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |row, command| match command {
                CommandId::AcceptInvite | CommandId::DeclineInvite => {
                    window.answer(row.id(), command)
                }
                _ => {
                    if let Some(position) = window
                        .pane()
                        .and_then(|pane| pane.feed().list().position_of(row.id()))
                    {
                        window.cursor_to(Some(position));
                    }
                    window.act(command);
                }
            }
        ));
        // Ctrl-click and Shift-click are `x` and `Shift`+`j`/`k` for the
        // row clicked: the cursor goes to it and the same commands run.
        pane.connect_row_pick(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |row, pick| window.pick(row, pick)
        ));
        // Double-click (GTK's `activate` on the list) opens the row, as Enter
        // does: the cursor goes to it and the one `OpenMessage` command runs.
        pane.view().connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, position| {
                window.cursor_to(Some(position));
                window.act(CommandId::OpenMessage);
            }
        ));
        // While a message is open beside the list, the pane shows the
        // cursor's row wherever the cursor goes: a click on a row is the
        // pointer's `j`/`k` (T232).
        pane.cursor().connect_selected_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.follow_cursor()
        ));
        // A row pulled out of the window offers its message as a file (T245).
        self.connect_drag_out(&pane);
        // A right-click opens the row's menu (T199).
        pane.connect_row_menu(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |position, at| window.open_row_menu(position, at)
        ));
        let empty = crate::empty::EmptyInbox::new();
        empty.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        let list_or_empty = gtk::Stack::new();
        list_or_empty.set_vexpand(true);
        list_or_empty.add_named(pane.widget(), Some(LIST));
        list_or_empty.add_named(empty.widget(), Some(EMPTY));
        list_or_empty.set_visible_child_name(LIST);
        // The command bar lies over the window, never beside the list.
        let bar = crate::bar::Bar::new(client.clone(), &self.keymap());
        bar.set_saved(imp.saved.borrow().clone());
        bar.set_digesting(!imp.focus_config.borrow().digests.is_empty());
        bar.connect_action(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |action| window.bar_action(action)
        ));
        // Closing it hands the keyboard to the list.
        bar.set_home(pane.view().clone().upcast());
        // In place (spec C24): the bar's input is drawn over the top bar's
        // own field and its results hang below, so the bar lies over the
        // whole window rather than over the list.
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            bar.set_field(chrome.field().clone().upcast());
        }
        imp.stage.add_overlay(bar.widget());
        list_or_empty.set_vexpand(true);
        // The list, and the reading pane beside it (T232): hidden until the
        // person reads beside the list.
        let list_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_column.set_hexpand(true);
        list_column.append(&list_or_empty);
        let reading_pane = crate::reading_pane::ReadingPane::new(&self.keymap());
        reading_pane.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        reading_pane.widget().set_visible(false);
        let split = self.split(&list_column, &reading_pane.widget());
        split.set_vexpand(true);
        imp.inbox.append(&split);
        imp.split.replace(Some(split));
        imp.reading_pane.replace(Some(reading_pane));
        imp.bar.replace(Some(bar));
        imp.list_or_empty.replace(Some(list_or_empty));
        imp.empty.replace(Some(empty));
        let bulk = Rc::new(Bulk::new(&self.keymap()));
        bulk.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.act(id)
        ));
        // At the window's foot, under the list and the pane alike: its verbs
        // want the window's width, not the list's.
        imp.inbox.append(bulk.widget());
        imp.bulk.replace(Some(Rc::clone(&bulk)));
        imp.picked.connect_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.selection_moved()
        ));
        // The accounts a whole-view selection spans.
        self.refresh_accounts(client.clone());
        pane.set_capture(imp.focus_config.borrow().vault.is_some());
        imp.pane.replace(Some(pane));
        imp.client.replace(Some(client.clone()));
        if let Some(uri) = imp.pending_link.take() {
            self.open_link(&uri);
        }
        imp.pages.set_visible_child_name(INBOX);
        crate::a11y::teach_shortcuts(self);
        crate::motion::keep_to_budget(self);
        self.place_reading();
        feed.open(ListScope::Focus(FocusScope::Inbox));
        // Opened with the keyboard in the list, not on the first control GTK
        // finds in the top bar.
        self.focus_list();

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
        // It narrows the inbox: a place has no marked rows of its own to
        // narrow to, and the toggle is not offered there.
        if imp.place.borrow().is_some() {
            return;
        }
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
        // A place counts what the list now holds.
        self.show_counts();
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

    /// Answer the invitation `row` carries, as `id` says: through the host,
    /// which queues the reply for the answer's window (FR-102).
    fn answer(&self, message: MessageId, id: CommandId) {
        let message = Some(message);
        let command = match id {
            CommandId::DeclineInvite => Command::DeclineInvite { message },
            _ => Command::AcceptInvite { message },
        };
        self.imp().answering.set(true);
        self.post(command);
    }

    /// The one message a reply or an answer is about: the message open over
    /// the list when one is, the cursor's row otherwise.
    fn aimed_message(&self) -> Option<MessageId> {
        let open = self
            .reading()
            .filter(|reading| reading.is_open())
            .map(|reading| reading.shown());
        postio_ui::focus_target::aimed_message(open, self.cursor_row().as_ref())
    }

    /// The send state of the message a verb is aimed at: the open message's,
    /// else the cursor's row.
    fn aimed_send_state(&self) -> Option<postio_model::DraftState> {
        if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            return reading.send_state();
        }
        self.cursor_row()?
            .as_conversation()
            .and_then(|row| row.summary.representative.send_state)
    }

    /// Whether the open message is a draft on its way or stopped whose
    /// action row offers `id` (T239).
    fn offered_on_open_draft(&self, id: CommandId) -> bool {
        self.reading()
            .filter(|reading| reading.is_open())
            .and_then(|reading| postio_ui::focus_dialog::send_verbs(reading.send_state()))
            .is_some_and(|verbs| verbs.contains(&id))
    }

    /// Cancel, retry or settle the send of the draft behind the message
    /// aimed at (T239), naming the draft so the host acts on that one and
    /// says so in the toast, or says why not. A message that is no draft
    /// says that instead of doing nothing.
    fn settle_send(&self, id: CommandId) {
        let (Some(message), Some(client)) =
            (self.aimed_message(), self.imp().client.borrow().clone())
        else {
            return;
        };
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            let behind = client.draft_behind(message).await.ok().flatten();
            let Some(draft) = behind.map(|draft| Some(draft.id)) else {
                if let Some(window) = window.upgrade() {
                    window
                        .imp()
                        .toast
                        .show_notice(postio_ui::focus_target::NOT_BEING_SENT);
                    window.follow_toast();
                }
                return;
            };
            let command = postio_ui::focus_target::settle_command(id, draft);
            // POSTIO-GLIB-SAFE: as above.
            if let Err(error) = client.send(command).await {
                tracing::warn!(%error, "Focus could not send a command: {error}");
            }
        });
    }

    /// `g t`, `g s`, `g r`: list the folder of `role` in the account Focus
    /// writes from -- the first with one. A role, not a name: what a provider
    /// calls its sent mail does not matter.
    fn go_to_role(&self, role: postio_model::MailboxRole) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let accounts = self.imp().accounts.borrow().clone();
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            use postio_model::listing::MailStore as _;
            for account in accounts {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let Ok(folders) = client.mailboxes(account).await else {
                    continue;
                };
                if let Some(folder) = folders.iter().find(|folder| folder.role == role)
                    && let Some(window) = window.upgrade()
                {
                    window.go_to(
                        postio_ui::finder::Destination::Mailbox(folder.id),
                        &crate::places::place_name(folder),
                    );
                    return;
                }
            }
        });
    }

    /// `g z`, `g *`: list a view -- Snoozed or Flagged -- which is a scope
    /// over mail filed elsewhere, not a folder with an id of its own. Of every
    /// enabled account, as Focus's inbox is.
    fn go_to_view(&self, role: postio_model::MailboxRole) {
        let (Some(pane), Some(chrome)) = (self.pane(), self.chrome()) else {
            return;
        };
        let (scope, name) = match role {
            postio_model::MailboxRole::Snoozed => {
                (ListScope::Focus(FocusScope::Snoozed), "Snoozed")
            }
            _ => (ListScope::Focus(FocusScope::Flagged), "Flagged"),
        };
        self.imp().at_inbox.set(false);
        self.imp().has_action.set(false);
        self.imp().place.replace(Some(match role {
            postio_model::MailboxRole::Snoozed => postio_ui::focus_state::EmptyPlace::Snoozed,
            _ => postio_ui::focus_state::EmptyPlace::Flagged,
        }));
        self.clear_selection();
        pane.feed().list().set_single_heading(None);
        pane.feed().open(scope);
        chrome.set_place(name);
        self.show_counts();
        self.focus_list_soon();
    }

    /// Say the counts the host last gave: the strip's, the toggle's, and
    /// how many of how many the filter is showing.
    fn show_counts(&self) {
        let imp = self.imp();
        let Some(chrome) = imp.chrome.borrow().clone() else {
            return;
        };
        chrome.set_digest_rules(imp.focus_config.borrow().digests.len());
        // A place that is not the inbox counts what it lists. The host's
        // counts are the inbox's, and beside "Flagged" they described a
        // place the person had left.
        if imp.place.borrow().is_some() {
            let listed = self
                .pane()
                .filter(|pane| pane.feed().has_landed())
                .map(|pane| pane.feed().total());
            chrome.set_place_counts(listed);
            chrome.set_filtered_today(None);
            self.show_empty_or_list();
            return;
        }
        let counts = imp.counts.get();
        if let Some(counts) = counts {
            chrome.set_counts(counts.conversations, counts.unread);
            // Filtering off files nothing new, and says nothing about it;
            // the strip says nothing of a day with nothing filtered (C10).
            let filtered = imp
                .focus_config
                .borrow()
                .filtering
                .then_some(counts.filtered_today);
            chrome.set_filtered_today(filtered.filter(|count| *count > 0));
            if let Some(places) = imp.places.borrow().as_ref() {
                places.set_filtered_today(filtered);
            }
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
        self.show_empty_or_list();
        if on
            && let Some(pane) = self.pane()
            && pane.feed().list().single_heading().as_deref() != Some(label.as_str())
        {
            pane.feed().list().set_single_heading(Some(label));
        }
    }

    /// The empty inbox when there is nothing in it, the list otherwise
    /// (screen 16). Only the inbox: the has-action filter showing nothing
    /// is the list's own business.
    fn show_empty_or_list(&self) {
        let imp = self.imp();
        let Some(stack) = imp.list_or_empty.borrow().clone() else {
            return;
        };
        // A place with nothing in it says why, once its first page has
        // landed: a list still waiting for it is loading, not empty.
        let empty_place = imp.place.borrow().clone().filter(|_| {
            self.pane()
                .is_some_and(|pane| pane.feed().has_landed() && pane.feed().total() == 0)
        });
        let empty = match imp.counts.get() {
            Some(counts)
                if counts.conversations == 0
                    && !imp.has_action.get()
                    && imp.at_inbox.get()
                    && imp.place.borrow().is_none() =>
            {
                Some(counts)
            }
            _ => None,
        };
        match (empty, imp.empty.borrow().as_ref()) {
            _ if empty_place.is_some() && imp.empty.borrow().is_some() => {
                if let (Some(place), Some(page)) = (empty_place, imp.empty.borrow().as_ref()) {
                    page.show(&postio_ui::focus_state::empty_place(&place, &self.keymap()));
                }
                stack.set_visible_child_name(EMPTY);
            }
            (Some(counts), Some(page)) => {
                // "Empty" is only said once a pass has finished (T220).
                let statuses = imp.trackers.borrow().statuses(&imp.tracked.borrow());
                let saying = postio_ui::focus_state::inbox_saying(&statuses, imp.last_synced.get());
                page.show(
                    &postio_ui::focus_state::empty_inbox(
                        &imp.focus_config.borrow(),
                        counts.filtered_today,
                        &self.keymap(),
                        &postio_ui::clock::now(),
                    )
                    .saying(&saying, &self.keymap(), &chrono::Local),
                );
                stack.set_visible_child_name(EMPTY);
            }
            _ => stack.set_visible_child_name(LIST),
        }
        // An empty inbox takes the window: there is nothing to read beside it.
        let beside = self.imp().placement.get() == postio_ui::focus_dialog::Placement::Pane;
        if beside != (stack.visible_child_name().as_deref() == Some(LIST)) {
            self.place_reading();
        }
    }

    /// `[focus]`, for what the empty inbox names.
    pub fn set_focus_config(&self, focus: postio_config::FocusConfig) {
        if let Some(bar) = self.bar() {
            bar.set_digesting(!focus.digests.is_empty());
        }
        let capture = focus.vault.is_some();
        let beside = focus.reading == postio_config::Reading::Pane;
        self.imp().focus_config.replace(focus);
        if let Some(menu) =
            self.lookup_action(crate::chrome::READING_PANE_ACTION.trim_start_matches("win."))
        {
            menu.change_state(&beside.to_variant());
        }
        self.show_capture(capture);
        self.show_empty_or_list();
        self.show_counts();
        self.place_reading();
    }

    /// Whether rows and the marker card offer Task: once a vault is
    /// configured (spec C9, milestone 3).
    fn show_capture(&self, capture: bool) {
        if let Some(pane) = self.pane() {
            pane.set_capture(capture);
        }
        if let Some(reading) = self.reading() {
            reading.set_capture(capture);
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
                // An answer's Undo works while its reply waits, so its toast
                // stays exactly that long (FR-102).
                if self.imp().answering.replace(false) {
                    let window = postio_session::actions::RSVP_WINDOW.as_secs();
                    self.imp().toast.show_action_completed_for(
                        description,
                        *undoable,
                        u32::try_from(window).unwrap_or(u32::MAX),
                    );
                } else {
                    self.imp()
                        .toast
                        .show_action_completed(description, *undoable);
                }
                self.follow_toast();
            }
            Event::UndoPerformed { description } => {
                self.imp().toast.show_undo_performed(description);
                self.follow_toast();
            }
            Event::CommandRejected { reason, .. } => {
                self.imp().answering.set(false);
                self.imp().toast.show_notice(reason);
                self.follow_toast();
            }
            Event::NewMail {
                mailbox, messages, ..
            } => self.announce(*mailbox, messages.clone()),
            _ => {}
        }
        self.hear_sync(event);
        if let Some(settings) = self.imp().settings.borrow().as_ref() {
            settings.hear(event);
        }
        if let Some(pane) = self.imp().pane.borrow().as_ref() {
            pane.feed().handle(event);
        }
        // What is filtered moves with a restore, its undo, and mail filed
        // away while the view is open.
        if matches!(
            event,
            Event::MessageListChanged { .. }
                | Event::UndoPerformed { .. }
                | Event::ActionCompleted { .. }
        ) && let Some(view) = self.filtered()
        {
            view.refresh();
        }
    }

    /// Hand every notification to `sink` instead of the desktop: what a
    /// test records.
    pub fn set_notification_sink(&self, sink: impl Fn(&postio_ui::notify::Notification) + 'static) {
        self.imp().notification_sink.replace(Some(Rc::new(sink)));
    }

    /// Decide notifications with `notifier`: the host's, which names only
    /// what stayed in Focus's inbox (FR-153).
    pub fn set_notifier(&self, notifier: Notifier) {
        self.imp().notifier.replace(Some(notifier));
    }

    /// Bring every enabled account's connection up, through `start`: the
    /// host's own `start_syncing` (ADR 0041), set once from
    /// `startup::adopt_at`. Called again once an account is saved on a
    /// first run that began with none (T171), so its engine comes up
    /// without a restart.
    pub fn set_start_syncing(&self, start: Rc<dyn Fn()>) {
        self.imp().start_syncing.replace(Some(start));
    }

    /// New mail in `mailbox`: a notification, if the host decides it is
    /// worth one while the person is looking where they are.
    fn announce(&self, mailbox: postio_model::MailboxId, messages: Vec<MessageId>) {
        let Some(notifier) = self.imp().notifier.borrow().clone() else {
            return;
        };
        let imp = self.imp();
        // Looking at Focus's inbox is looking at every inbox it is made of.
        let looking = imp.pages.visible_child_name().as_deref() == Some(INBOX)
            && imp.at_inbox.get()
            && self
                .pane()
                .is_some_and(|pane| pane.feed().is_inbox(mailbox));
        let attention = postio_ui::notify::Attention {
            showing: looking.then_some(mailbox),
            active: gtk::prelude::GtkWindowExt::is_active(self),
        };
        let deciding = notifier(mailbox, messages, attention);
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: the notifier's future awaits the host's
                // runtime through a join handle, a oneshot.
                let decided = deciding.await;
                if let Some(notification) = decided {
                    window.deliver(&notification);
                }
            }
        ));
    }

    /// Post `notification`, replacing the one already showing for its
    /// folder, or hand it to the test's sink.
    fn deliver(&self, notification: &postio_ui::notify::Notification) {
        let sink = self.imp().notification_sink.borrow().clone();
        if let Some(sink) = sink {
            sink(notification);
            return;
        }
        let Some(application) = gtk::prelude::GtkWindowExt::application(self) else {
            return;
        };
        let built = gio::Notification::new(&notification.title);
        built.set_body(Some(&notification.body));
        application.send_notification(Some(&notification.identifier), &built);
    }

    /// `-` (T118): take the marker off the open message, or off what a
    /// verb would aim at in the list. One undo puts it back.
    fn dismiss_marker(&self) {
        if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            let Some(message) = reading.shown() else {
                return;
            };
            reading.clear_marker();
            self.post(Command::DismissMarker {
                target: MessageTarget::Messages(vec![message]),
                dismissed: true,
            });
            return;
        }
        self.send(Command::DismissMarker {
            target: MessageTarget::Selection,
            dismissed: true,
        });
    }

    /// The digest the cursor is on, when it is on one.
    fn digest_at_cursor(&self) -> Option<crate::list::Digest> {
        match self.cursor_row()? {
            FocusRow::Digest(digest) => Some(digest),
            _ => None,
        }
    }

    /// Archive a whole delivery: one undo (FR-125).
    fn archive_digest(&self, delivery: postio_model::DeliveryId) {
        self.post(Command::ArchiveDigest {
            delivery,
            archived: true,
        });
    }

    /// Open `digest`'s window over the inbox (T137): its plain list.
    fn open_digest(&self, digest: crate::list::Digest) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let window = imp.digest_window.borrow().clone();
        let window = window.unwrap_or_else(|| {
            let window = crate::digest::DigestWindow::new(
                client,
                &self.keymap(),
                &crate::open::allowlist_path(),
            );
            window.connect_action(glib::clone!(
                #[weak(rename_to = focus)]
                self,
                move |action| focus.digest_action(action)
            ));
            imp.digest_window.replace(Some(Rc::clone(&window)));
            window
        });
        let rule_when = imp
            .focus_config
            .borrow()
            .digests
            .iter()
            .find(|rule| rule.name.trim() == digest.rule.trim())
            .and_then(postio_ui::digest::rule_when);
        window.show(self, digest, rule_when);
        crate::a11y::teach_shortcuts(window.dialog());
        crate::motion::keep_to_budget(window.dialog());
    }

    /// The digest window, while it is open.
    pub fn digest(&self) -> Option<Rc<crate::digest::DigestWindow>> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::digest::DIALOG_NAME)?;
        self.imp().digest_window.borrow().clone()
    }

    /// What the digest window asked for.
    fn digest_action(&self, action: crate::digest::DigestAction) {
        use crate::digest::DigestAction;
        let Some(window) = self.imp().digest_window.borrow().clone() else {
            return;
        };
        match action {
            DigestAction::ArchiveAll => {
                if let Some(digest) = window.digest() {
                    window.close();
                    self.archive_digest(digest.delivery);
                }
            }
            DigestAction::EditRule => {
                if let Some(digest) = window.digest() {
                    self.edit_digest_rule(&digest.rule);
                }
            }
            DigestAction::Open { message, subject } => {
                if let Some(reading) = self.reading_dialog() {
                    // Over the digest, not in the pane behind it.
                    self.place_reading();
                    reading.show_found(self, message, &subject);
                }
            }
        }
    }

    /// A key while a digest is open (US10): its keys are `Context::Digest`'s.
    fn digest_key(&self, outcome: Outcome) -> glib::Propagation {
        let Outcome::Command(id) = outcome else {
            return match outcome {
                Outcome::Pending(_) => glib::Propagation::Stop,
                _ => glib::Propagation::Proceed,
            };
        };
        let Some(window) = self.imp().digest_window.borrow().clone() else {
            return glib::Propagation::Proceed;
        };
        match id.parse::<CommandId>() {
            Ok(CommandId::Back) => {
                if !window.back() {
                    window.close();
                }
            }
            Ok(CommandId::ArchiveThread) => {
                self.digest_action(crate::digest::DigestAction::ArchiveAll)
            }
            Ok(CommandId::DigestRule) => self.digest_action(crate::digest::DigestAction::EditRule),
            Ok(CommandId::NextReference) => window.step_reference(1),
            Ok(CommandId::PrevReference) => window.step_reference(-1),
            Ok(CommandId::ToggleDigestSummary) => window.toggle_summary(),
            Ok(CommandId::StopDigestingSender) => {
                if let Some(message) = window.focused() {
                    self.ask_stop_digesting(&message);
                }
            }
            Ok(CommandId::Unsubscribe) => {
                if let Some(message) = window.focused() {
                    self.unsubscribe(message.id);
                }
            }
            Ok(CommandId::Undo) => self.act(CommandId::Undo),
            Ok(CommandId::NextMessage) => window.step(1),
            Ok(CommandId::PrevMessage) => window.step(-1),
            Ok(CommandId::OpenMessage) => window.open_focused(),
            Ok(CommandId::GoToInbox) => window.close(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// `D` in a digest (US10 scenario 5): ask, then stop digesting the
    /// message's sender -- out of `config.toml`, releasing what the rule
    /// holds from them.
    fn ask_stop_digesting(&self, message: &postio_model::listing::MessageSummary) {
        let sender = message
            .from
            .as_ref()
            .map(|from| from.address.clone())
            .unwrap_or_default();
        let dialog = adw::AlertDialog::new(
            Some(&postio_ui::focus_target::stop_digesting_title(&sender)),
            Some(postio_ui::focus_target::STOP_DIGESTING_BODY),
        );
        dialog.set_widget_name(STOP_DIALOG);
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("stop", "Stop digesting");
        dialog.set_response_appearance("stop", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("stop"));
        dialog.set_close_response("cancel");
        let id = message.id;
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, response| {
                    if response == "stop" {
                        window.post(Command::StopDigestingSender {
                            target: MessageTarget::Messages(vec![id]),
                            stopped: true,
                            kept: None,
                        });
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// The stop-digesting confirmation, while it is up.
    pub fn stop_digesting_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == STOP_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

    /// `U`: leave the list `message` came from -- only ever on this key.
    fn unsubscribe(&self, message: MessageId) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let left = client.unsubscribe(message).await;
                let said = match left {
                    Ok(list) => postio_ui::focus_target::unsubscribed(&list),
                    Err(error) => error.to_string(),
                };
                window.imp().toast.show_notice(&said);
                window.follow_toast();
            }
        ));
    }

    /// Edit the digest rule called `name` (T138), from its digest's `d`.
    fn edit_digest_rule(&self, name: &str) {
        let rule = self
            .imp()
            .focus_config
            .borrow()
            .digests
            .iter()
            .find(|rule| rule.name.trim() == name.trim())
            .cloned();
        let Some(rule) = rule else {
            self.imp()
                .toast
                .show_notice(postio_ui::focus_target::RULE_MISSING);
            self.follow_toast();
            return;
        };
        if let Some(dialog) = self.rule_dialog_built() {
            dialog.open_edit(self, &rule);
            crate::a11y::teach_shortcuts(dialog.dialog());
            crate::motion::keep_to_budget(dialog.dialog());
        }
    }

    /// `d` on a message, or "Digest these…" in the bulk bar (T138): a new
    /// rule for the senders of what a verb would aim at.
    fn new_digest_rule(&self) {
        let senders = self.aimed_senders();
        if senders.is_empty() {
            return;
        }
        let like_this = self.aimed_message_for_like_this();
        if let Some(dialog) = self.rule_dialog_built() {
            dialog.open_new(self, &senders, like_this);
            crate::a11y::teach_shortcuts(dialog.dialog());
            crate::motion::keep_to_budget(dialog.dialog());
        }
    }

    /// The one message "Digest mail like this" would check other mail
    /// against: the cursor's, when nothing beyond it is selected, and only
    /// when the user has brought a model with `like_this` on (US14, FR-171)
    /// -- whether it connects is a further question the dialog leaves to
    /// `Client::digest_like_this`.
    fn aimed_message_for_like_this(&self) -> Option<MessageId> {
        let enabled = self
            .imp()
            .focus_config
            .borrow()
            .model_for(postio_config::model::ModelFeature::LikeThis)
            .is_some();
        postio_ui::focus_target::like_this_message(
            &self.selection(),
            enabled,
            self.cursor_row().as_ref(),
        )
    }

    /// The senders of the selection, or of the cursor's conversation, once
    /// each.
    fn aimed_senders(&self) -> Vec<postio_model::EmailAddress> {
        let resident = self
            .pane()
            .map(|pane| {
                let list = pane.feed().list();
                (0..list.n_items())
                    .filter_map(|position| {
                        list.item(position)
                            .and_downcast::<RowObject>()
                            .and_then(|row| row.item())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let rows =
            postio_ui::focus_target::aimed_rows(&self.selection(), self.cursor_row(), resident);
        postio_ui::focus_target::senders(&rows)
    }

    /// Measure this window's start on `timeline` (`startup::time`).
    pub fn set_timeline(&self, timeline: postio_widgets::startup::Timeline) {
        self.imp().timeline.replace(Some(timeline));
    }

    /// The start being measured, if one is.
    pub fn timeline(&self) -> Option<postio_widgets::startup::Timeline> {
        self.imp().timeline.borrow().clone()
    }

    /// Where `config.toml` is, for the corrections the window writes itself.
    pub fn set_config_path(&self, path: Option<std::path::PathBuf>) {
        self.imp().config_path.replace(path);
    }

    /// `Ctrl+S` in the bar (T086): save its query to `config.toml` as a
    /// pinned search -- `[filters]` alone, the rest of the file as it
    /// was -- and show it in the saved row.
    fn save_search(&self) {
        let Some(bar) = self.bar().filter(|bar| bar.is_open()) else {
            return;
        };
        let query = bar.query();
        if query.is_empty() {
            return;
        }
        let Some(path) = self.imp().config_path.borrow().clone() else {
            self.imp()
                .toast
                .show_notice(postio_ui::focus_target::NO_CONFIG_TO_SAVE);
            self.follow_toast();
            return;
        };
        let original = std::fs::read_to_string(&path).unwrap_or_default();
        let mut config = postio_config::Config::from_toml_str(&original).unwrap_or_default();
        config.save_filter(&query);
        let written = postio_config::patch_filters(&original, &config.filters)
            .and_then(|patched| postio_config::Config::write_text_to_path(&patched, &path));
        let said = match written {
            Ok(()) => {
                self.set_saved_searches(postio_session::focus::saved_searches(&config));
                postio_ui::focus_target::search_saved(&query)
            }
            Err(error) => {
                tracing::warn!(%error, "Focus could not save the search");
                postio_ui::focus_target::SEARCH_NOT_WRITTEN.to_owned()
            }
        };
        self.imp().toast.show_notice(&said);
        self.follow_toast();
    }

    /// The digest rules list, while it is the page on screen.
    pub fn rules(&self) -> Option<Rc<crate::rules::RulesView>> {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() != Some(RULES) {
            return None;
        }
        imp.rules_view.borrow().clone()
    }

    /// `g d`: every digest rule, in place of the inbox (T139).
    fn show_rules(&self) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let view = imp.rules_view.borrow().clone();
        let view = view.unwrap_or_else(|| {
            let view = crate::rules::RulesView::new(client, &self.keymap());
            view.connect_action(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |action| window.rules_action(action)
            ));
            imp.pages.add_named(view.widget(), Some(RULES));
            imp.rules_view.replace(Some(Rc::clone(&view)));
            view
        });
        imp.pages.set_visible_child_name(RULES);
        // `RulesView::show` teaches its own caps: it redraws rows again
        // once `read_holds`'s async fetch lands, and a call only here
        // would miss those.
        view.open(imp.focus_config.borrow().digests.clone());
    }

    /// A command while the rules list is on screen; whether it was the
    /// list's. What only the inbox answers is refused there, so a key does
    /// not act on mail nobody can see.
    fn rules_act(&self, id: CommandId) -> bool {
        let Some(view) = self.rules() else {
            return false;
        };
        match id {
            CommandId::Back | CommandId::GoToInbox => self.leave_filtered(),
            CommandId::NextMessage => view.step(1),
            CommandId::PrevMessage => view.step(-1),
            CommandId::OpenMessage => {
                if let Some((name, _)) = view.focused() {
                    self.edit_digest_rule(&name);
                }
            }
            CommandId::Delete => {
                if let Some((name, holds)) = view.focused() {
                    self.ask_remove_rule(&name, holds);
                }
            }
            CommandId::GoToDigestRules
            | CommandId::GoToFiltered
            | CommandId::Undo
            | CommandId::CheatSheet
            | CommandId::Quit
            | CommandId::Search
            | CommandId::CommandPalette => return false,
            _ => {}
        }
        true
    }

    /// What the rules list asked for.
    fn rules_action(&self, action: crate::rules::RulesAction) {
        use crate::rules::RulesAction;
        match action {
            RulesAction::Back => self.leave_filtered(),
            RulesAction::Edit(name) => self.edit_digest_rule(&name),
            RulesAction::Remove { name, holds } => self.ask_remove_rule(&name, holds),
        }
    }

    /// `Delete` on a rule (FR-126): ask, then take it out of `config.toml`
    /// and release what it holds into the inbox. No undo takes that mail
    /// back into the rule, so this is the one place the list asks first.
    fn ask_remove_rule(&self, name: &str, holds: u32) {
        let dialog = adw::AlertDialog::new(
            Some(&postio_ui::focus_target::remove_rule_title(name)),
            Some(&postio_ui::digest::remove_body(holds)),
        );
        dialog.set_widget_name(REMOVE_RULE_DIALOG);
        dialog.add_response("cancel", "Cancel");
        dialog.add_response("remove", "Remove rule");
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        dialog.set_close_response("cancel");
        let name = name.to_owned();
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, response| {
                    if response == "remove" {
                        window.remove_rule(name.clone());
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// Removing a rule's confirmation, while it is up.
    pub fn remove_rule_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == REMOVE_RULE_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

    /// Take the rule called `name` out of `config.toml`, releasing what it
    /// held, and say how much came back.
    fn remove_rule(&self, name: String) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let removed = client.delete_digest_rule(name.clone()).await;
                let said = match removed {
                    Ok(released) => {
                        window
                            .imp()
                            .focus_config
                            .borrow_mut()
                            .digests
                            .retain(|rule| rule.name != name);
                        if let Some(view) = window.imp().rules_view.borrow().as_ref() {
                            view.forget(&name);
                        }
                        postio_ui::focus_target::rule_removed(&name, released)
                    }
                    Err(error) => error.to_string(),
                };
                window.imp().toast.show_notice(&said);
                window.follow_toast();
            }
        ));
    }

    /// The rule dialog, built the first time it is asked for.
    fn rule_dialog_built(&self) -> Option<Rc<crate::rule_dialog::RuleDialog>> {
        let imp = self.imp();
        let client = imp.client.borrow().clone()?;
        let dialog = imp.rule_dialog.borrow().clone();
        Some(dialog.unwrap_or_else(|| {
            let dialog = crate::rule_dialog::RuleDialog::new(client, &self.keymap());
            dialog.connect_saved(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |name| {
                    window
                        .imp()
                        .toast
                        .show_notice(&postio_ui::focus_target::rule_saved(&name));
                    window.follow_toast();
                }
            ));
            imp.rule_dialog.replace(Some(Rc::clone(&dialog)));
            dialog
        }))
    }

    /// The rule dialog, while it is open.
    pub fn rule_dialog(&self) -> Option<Rc<crate::rule_dialog::RuleDialog>> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::rule_dialog::DIALOG_NAME)?;
        self.imp().rule_dialog.borrow().clone()
    }

    /// The sweep's confirmation, while it is up.
    pub fn sweep_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == SWEEP_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

    /// `F` (FR-118): say how much of the inbox the filtering rules would
    /// file away, and sweep only when the person says so.
    fn ask_sweep(&self) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime (ADR 0041).
                let counted = client.sweep_preview().await;
                let count = match counted {
                    Ok(count) => count,
                    Err(error) => {
                        window.imp().toast.show_notice(&error.to_string());
                        window.follow_toast();
                        return;
                    }
                };
                if count == 0 {
                    window
                        .imp()
                        .toast
                        .show_notice(postio_ui::filtered::SWEEP_NOTHING);
                    window.follow_toast();
                    return;
                }
                let undo = postio_ui::hints::key(&window.keymap(), CommandId::Undo);
                let dialog = adw::AlertDialog::new(
                    Some(postio_ui::filtered::SWEEP_HEADING),
                    Some(&postio_ui::filtered::sweep_body(count, undo.as_deref())),
                );
                dialog.set_widget_name(SWEEP_DIALOG);
                dialog.add_response("cancel", "Cancel");
                dialog.add_response("sweep", &postio_ui::filtered::sweep_action(count));
                dialog.set_response_appearance("sweep", adw::ResponseAppearance::Suggested);
                dialog.set_default_response(Some("sweep"));
                dialog.set_close_response("cancel");
                dialog.connect_response(
                    None,
                    glib::clone!(
                        #[weak]
                        window,
                        move |_, response| {
                            if response == "sweep" {
                                window.post(Command::SweepInbox);
                            }
                        }
                    ),
                );
                dialog.present(Some(&window));
            }
        ));
    }

    /// The Filtered view, while it is the page on screen.
    pub fn filtered(&self) -> Option<Rc<crate::filtered::FilteredView>> {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() != Some(FILTERED) {
            return None;
        }
        imp.filtered.borrow().clone()
    }

    /// `g f`: the Filtered view in place of the inbox (screen 21).
    fn show_filtered(&self) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let view = imp.filtered.borrow().clone();
        let view = view.unwrap_or_else(|| {
            let view = crate::filtered::FilteredView::new(client, &self.keymap());
            view.connect_action(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |action| window.filtered_action(action)
            ));
            imp.pages.add_named(view.widget(), Some(FILTERED));
            imp.filtered.replace(Some(Rc::clone(&view)));
            view
        });
        // `FilteredView::show_rows` teaches its own caps: `open` reads
        // its rows asynchronously, and a call only here would run before
        // any row exists.
        view.open();
        imp.pages.set_visible_child_name(FILTERED);
        view.focus_list();
    }

    /// Back from Filtered to the inbox, as it was.
    fn leave_filtered(&self) {
        self.imp().pages.set_visible_child_name(INBOX);
        self.focus_list();
    }

    /// What the Filtered view asked for.
    fn filtered_action(&self, action: crate::filtered::FilteredAction) {
        use crate::filtered::FilteredAction;
        match action {
            FilteredAction::Back => self.leave_filtered(),
            FilteredAction::Open { message, subject } => {
                if let Some(reading) = self.reading_dialog() {
                    reading.show_found(self, message, &subject);
                }
            }
            FilteredAction::Restore(message) => self.restore_filtered(message),
            FilteredAction::Sweep => self.ask_sweep(),
        }
    }

    /// Put `message` back in the inbox, and never filter its sender (FR-116).
    fn restore_filtered(&self, message: MessageId) {
        self.post(Command::RestoreFiltered {
            target: MessageTarget::Messages(vec![message]),
            restored: true,
        });
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
            imp.last_synced.set(Some(postio_ui::clock::now().to_utc()));
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
        // What an inbox with no rows says follows the same news.
        self.show_empty_or_list();
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

    /// "Update password…": the credential dialog both desktop apps share,
    /// for the account the sign-in banner names (US6 scenario 3). Once the
    /// new password is saved, sync tries again at once.
    fn update_credential(&self) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let statuses = imp.trackers.borrow().statuses(&imp.tracked.borrow());
        let Some(postio_ui::focus_state::Banner::SignIn { address, .. }) =
            postio_ui::focus_state::banner(&statuses, &imp.facts.borrow())
        else {
            return;
        };
        let window = self.clone();
        glib::spawn_future_local(async move {
            let open_link = postio_widgets::present::onboarding::open_in_browser(&window);
            let retry = {
                let window = window.downgrade();
                move || {
                    if let Some(window) = window.upgrade() {
                        window.post(Command::Refresh);
                    }
                }
            };
            let opening = postio_widgets::present::onboarding::update_credential(
                &window,
                &client,
                move |account| account.address.address.eq_ignore_ascii_case(&address),
                open_link,
                retry,
            );
            // POSTIO-GLIB-SAFE: reading the account is a client call, a
            // oneshot receive; the host answers on its own runtime.
            opening.await;
        });
    }

    /// Go to the message a `postio://` link names (T159, US15 scenario 2):
    /// open it over the list and do nothing else to it. A link that is not
    /// one of Postio's, or names a message not in this store, is refused
    /// with a sentence. A link that arrives before the store is open waits
    /// for it.
    pub fn open_link(&self, uri: &str) {
        // The desktop hands over every scheme the entry registers: a
        // `mailto:` link starts a message (row 46, T244).
        if let Some(mailto) = postio_model::mailto::Mailto::parse(uri) {
            return self.open_mailto(mailto);
        }
        let Some(message) = postio_ui::links::message(uri) else {
            self.imp().toast.show_notice(postio_ui::links::UNKNOWN);
            self.follow_toast();
            return;
        };
        let Some(client) = self.imp().client.borrow().clone() else {
            self.imp().pending_link.replace(Some(uri.to_owned()));
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                use postio_model::listing::MailStore as _;
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime (ADR 0041).
                let found = client.message_rows(vec![message]).await;
                let Some(row) = found.ok().and_then(|rows| rows.into_iter().next()) else {
                    window.imp().toast.show_notice(postio_ui::links::GONE);
                    window.follow_toast();
                    return;
                };
                if let Some(reading) = window.reading_dialog() {
                    reading.show_found(&window, message, &row.subject.unwrap_or_default());
                }
            }
        ));
    }

    /// A `mailto:` link: a new message with its recipients, subject and body
    /// filled in, in the composer's dialog. A link that arrives before the
    /// composer is mounted -- a cold launch from a browser -- waits for it.
    fn open_mailto(&self, mailto: postio_model::mailto::Mailto) {
        match self.compose() {
            Some(compose) => compose.open_mailto(mailto),
            None => {
                self.imp().pending_mailto.replace(Some(mailto));
            }
        }
    }

    /// The list's column and the reading pane, side by side (T232): the pane
    /// as wide as `focus_dialog::pane_width` says for the split's width, the
    /// list the rest. A layout of its own, so the widths come from the
    /// allocation itself and never from a size request the window would
    /// then refuse to shrink below; and the place a crossing of the
    /// narrowest width a pane fits is noticed, to move an open message
    /// between pane and dialog.
    fn split(&self, list: &gtk::Box, pane: &gtk::Widget) -> gtk::Box {
        let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        split.add_css_class("focus-split");
        split.append(list);
        split.append(pane);
        let fits: Rc<Cell<Option<bool>>> = Rc::default();
        let window = self.downgrade();
        split.set_layout_manager(Some(crate::split::SplitLayout::new(move |width| {
            let now = postio_ui::focus_dialog::pane_width(width).is_some();
            if fits.replace(Some(now)) != Some(now) {
                // Not from inside an allocation: once it is done.
                let window = window.clone();
                glib::idle_add_local_once(move || {
                    if let Some(window) = window.upgrade() {
                        window.place_reading();
                    }
                });
            }
        })));
        split
    }

    /// Where an open message goes now, and put it there (T232): beside the
    /// list when the person reads beside it, the window has room for a pane
    /// and the inbox's list is what is on screen; over it otherwise. An open
    /// message, and an open composer, move with it and stay open.
    fn place_reading(&self) {
        use postio_ui::focus_dialog::{self, Placement};
        let imp = self.imp();
        let (Some(pane), Some(split)) = (
            imp.reading_pane.borrow().clone(),
            imp.split.borrow().clone(),
        ) else {
            return;
        };
        let chosen = match imp.focus_config.borrow().reading {
            postio_config::Reading::Pane => Placement::Pane,
            postio_config::Reading::Dialog => Placement::Dialog,
        };
        let width = match split.width() {
            0 => self.width(),
            width => width,
        };
        // The pane is the inbox list's: Filtered, the digest rules and a
        // digest over the window have no list beside it, and an empty inbox
        // has nothing to open.
        let listing = imp.pages.visible_child_name().as_deref() == Some(INBOX)
            && self.digest().is_none()
            && imp
                .list_or_empty
                .borrow()
                .as_ref()
                .is_some_and(|stack| stack.visible_child_name().as_deref() == Some(LIST));
        let placement = if listing {
            focus_dialog::placement(chosen, width)
        } else {
            Placement::Dialog
        };
        imp.placement.set(placement);
        let beside = placement == Placement::Pane;
        pane.widget().set_visible(beside);
        if let Some(reading) = self.reading() {
            reading.place(beside.then(|| pane.message_slot()));
        }
        if let Some(compose) = self.compose() {
            compose.place(beside.then(|| pane.compose_slot()));
        }
        self.show_pane_page();
    }

    /// Show in the reading pane what is open: the composer, which takes the
    /// open message's place, then the message, then nothing (T232).
    pub(crate) fn show_pane_page(&self) {
        use crate::reading_pane::Page;
        let Some(pane) = self.imp().reading_pane.borrow().clone() else {
            return;
        };
        let page = if self.compose().is_some_and(|compose| compose.in_pane()) {
            Page::Compose
        } else if self
            .reading()
            .is_some_and(|reading| reading.in_pane() && reading.is_open())
        {
            Page::Message
        } else {
            Page::Empty
        };
        pane.show(page);
    }

    /// Whether a message is open in the reading pane beside the list.
    fn reading_beside(&self) -> bool {
        self.reading()
            .is_some_and(|reading| reading.in_pane() && reading.is_open())
    }

    /// The reading pane, once the inbox is showing: what a test reads.
    pub fn reading_pane(&self) -> Option<gtk::Widget> {
        self.imp()
            .reading_pane
            .borrow()
            .as_ref()
            .map(|pane| pane.widget())
    }

    /// The cursor moved: while a message is open beside the list, show the
    /// cursor's row in its place, as `j`/`k` would (T232). Over the list the
    /// dialog takes the pointer, so only the keys move it, and they open
    /// what they land on themselves.
    fn follow_cursor(&self) {
        if !self.reading_beside() {
            return;
        }
        let Some(row) = self.cursor_row() else {
            return;
        };
        let showing = self.reading().and_then(|reading| reading.showing_row());
        if showing == Some(row.id()) {
            return;
        }
        // A draft is written, not read: passing over one opens no composer.
        // One on its way or stopped is read, so the pane shows it (T239).
        let draft = row.as_conversation().is_some_and(|row| {
            !postio_ui::focus_dialog::opens_to_read(row.summary.representative.send_state)
        });
        if draft || matches!(row, FocusRow::Digest(_)) {
            if let Some(reading) = self.reading() {
                reading.close();
            }
            return;
        }
        self.open_message();
    }

    /// `F8`: open messages beside the list, or over it again (T232). The
    /// window switches at once, an open message moving with it, and the
    /// choice is written to `config.toml` as `[focus] reading`, so it
    /// outlives the session; the watcher's echo of the write changes
    /// nothing.
    fn toggle_reading_pane(&self) {
        use postio_config::Reading;
        let imp = self.imp();
        let next = match imp.focus_config.borrow().reading {
            Reading::Dialog => Reading::Pane,
            Reading::Pane => Reading::Dialog,
        };
        imp.focus_config.borrow_mut().reading = next;
        self.place_reading();
        if let Some(menu) =
            self.lookup_action(crate::chrome::READING_PANE_ACTION.trim_start_matches("win."))
        {
            menu.change_state(&(next == Reading::Pane).to_variant());
        }
        let path = imp.config_path.borrow().clone();
        if let Some(path) = path {
            let original = std::fs::read_to_string(&path).unwrap_or_default();
            let written =
                postio_config::focus_edit::set_reading(&original, next).and_then(|edited| {
                    match edited {
                        Some(text) => postio_config::Config::write_text_to_path(&text, &path),
                        None => Ok(()),
                    }
                });
            if let Err(error) = written {
                tracing::warn!(%error, "Focus could not write where messages open");
            }
        }
        let beside = imp.placement.get() == postio_ui::focus_dialog::Placement::Pane;
        let said = postio_ui::focus_target::reading_placement(next == Reading::Pane, beside);
        imp.toast.show_notice(said);
        self.follow_toast();
        // Where the keyboard was, the message beside it or over it.
        if !beside && let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            reading.dialog().grab_focus();
        }
    }

    /// The open-email dialog, built the first time anything opens.
    fn reading_dialog(&self) -> Option<Rc<crate::open::OpenMessage>> {
        let client = self.imp().client.borrow().clone()?;
        let reading = self
            .imp()
            .reading
            .borrow_mut()
            .get_or_insert_with(|| {
                let reading = crate::open::OpenMessage::new(
                    client,
                    &self.keymap(),
                    &crate::open::allowlist_path(),
                    self.imp().runtime.borrow().clone(),
                );
                reading.connect_command(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |id| window.act(id)
                ));
                // Open for the dwell, it counts as read (T237): the host
                // marks it without an undo entry.
                reading.connect_read(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |message| window.post(Command::MarkReadOnDwell { message })
                ));
                self.imp().toast.set_over(Some(reading.toast_overlay()));
                reading.set_capture(self.imp().focus_config.borrow().vault.is_some());
                // The zoom `[reader]` says (T235).
                if let Some(zoom) = self.imp().zoom.get() {
                    reading.reader().set_zoom(zoom);
                }
                // A zoom a person chose is the next message's too (T242):
                // written to `[reader]` alone, which the watcher then reads
                // back as the zoom it already is.
                reading.reader().connect_zoom_changed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |percent| {
                        if let Some(path) = window.config_path()
                            && let Err(error) = postio_config::save_zoom(&path, percent)
                        {
                            tracing::warn!(%error, "could not save the zoom");
                        }
                    }
                ));
                // The notice only asks; leaving the list is logged by the
                // host, on this one deliberate click (T261).
                reading.reader().connect_unsubscribe_activated(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_| {
                        if let Some(message) = window.reading().and_then(|r| r.shown()) {
                            window.unsubscribe(message);
                        }
                    }
                ));
                // A chip asks for the chooser, at its part (T240).
                reading.connect_chip(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |part| window.offer_choices(Some(part))
                ));
                // Opening or closing it changes what the reading pane shows,
                // and closing it there gives the keyboard back to the list.
                reading.connect_changed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.reading_changed()
                ));
                if let Some(pane) = self.imp().reading_pane.borrow().as_ref()
                    && self.imp().placement.get() == postio_ui::focus_dialog::Placement::Pane
                {
                    reading.place(Some(pane.message_slot()));
                }
                reading
            })
            .clone();
        Some(reading)
    }

    /// The open message opened or closed: the pane shows what is open now,
    /// and a message closed beside the list leaves the keyboard in the list,
    /// on the row it was on (T232).
    fn reading_changed(&self) {
        let Some(reading) = self.reading() else {
            return;
        };
        // A message opened where the pane does not reach -- over a digest --
        // goes back to where messages open, once it closes.
        if !reading.is_open() {
            self.place_reading();
        }
        self.show_pane_page();
        if reading.in_pane() && !reading.is_open() {
            self.focus_list();
        }
    }

    /// `Enter`: the conversation under the cursor, over the list (screen 04).
    fn open_message(&self) {
        let (Some(pane), Some(row)) = (self.pane(), self.cursor_row()) else {
            return;
        };
        // A draft is written, not read: it opens in the composer (US11
        // scenario 3), whichever app left it -- unless it is on its way or
        // stopped, which opens to be read, with the verbs that settle it
        // (T239).
        if row.as_conversation().is_some_and(|row| {
            !postio_ui::focus_dialog::opens_to_read(row.summary.representative.send_state)
        }) {
            self.open_draft(row.id());
            return;
        }
        let Some(reading) = self.reading_dialog() else {
            return;
        };
        let position = crate::open::Position {
            index: pane.cursor().selected(),
            total: pane.feed().list().n_items(),
        };
        // The row already on screen is not read again: beside the list the
        // pane follows the cursor, and `j` both moves it and asks.
        if reading.showing_row() == Some(row.id()) && reading.in_pane() {
            return;
        }
        reading.show(self, &row, position);
        crate::a11y::teach_shortcuts(&reading.view());
        crate::motion::keep_to_budget(&reading.view());
    }

    /// Open URIs through `launch` rather than the desktop: what a test
    /// records instead of starting a browser.
    pub fn set_launcher(&self, launch: impl Fn(&str) + 'static) {
        self.imp().launcher.replace(Some(Rc::new(launch)));
    }

    /// Open `uri` outside Postio: a deliberate choice's only consequence.
    fn launch(&self, uri: &str) {
        let launch = self.imp().launcher.borrow().clone();
        match launch {
            Some(launch) => launch(uri),
            None => {
                // POSTIO-CONSENT: runs only when the person chooses a link or
                // a part in the open-with chooser (`o`, then a row), one at a
                // time; nothing opens on render, on arrival or from a setting.
                gtk::UriLauncher::new(uri).launch(Some(self), None::<&gio::Cancellable>, |_| {})
            }
        }
    }

    /// `o`: offer the open message's links and parts (US2 scenario 9), or
    /// a chip's part, which is the same chooser opened at that part (T240).
    fn offer_choices(&self, at: Option<postio_model::ids::AttachmentId>) {
        let Some(reading) = self.reading().filter(|reading| reading.is_open()) else {
            return;
        };
        let choices = reading.choices();
        if choices.is_empty() {
            self.imp()
                .toast
                .show_notice(postio_ui::focus_target::NOTHING_TO_OPEN);
            self.follow_toast();
            return;
        }
        let at = at.and_then(|wanted| {
            choices.iter().position(
                |choice| matches!(choice, crate::chooser::Choice::Part { id, .. } if *id == wanted),
            )
        });
        self.imp().choices.replace(choices.clone());
        let window = self.downgrade();
        crate::chooser::dialog(&choices, at, move |pick| {
            if let Some(window) = window.upgrade() {
                window.pick_choice(pick);
            }
        })
        .present(Some(self));
    }

    /// Ask the file-chooser portal where to save, through the seam a test
    /// answers for itself.
    pub fn set_file_picker(
        &self,
        pick: impl Fn(crate::chooser::SavePick, Box<dyn FnOnce(Option<std::path::PathBuf>)>) + 'static,
    ) {
        self.imp().file_picker.replace(Some(Rc::new(pick)));
    }

    fn pick_place(
        &self,
        pick: crate::chooser::SavePick,
        then: impl FnOnce(std::path::PathBuf) + 'static,
    ) {
        let seam = self.imp().file_picker.borrow().clone();
        // Dismissed is a person's answer, not a fault: nothing is written
        // and nothing is said.
        let answer = move |chosen: Option<std::path::PathBuf>| {
            if let Some(path) = chosen {
                then(path);
            }
        };
        if let Some(seam) = seam {
            return seam(pick, Box::new(answer));
        }
        // POSTIO-CONSENT: runs only when the person pressed Save or Save all
        // in the chooser; the portal's dialog is theirs to dismiss.
        let dialog = gtk::FileDialog::new();
        // The portal hands back a local path for every choice.
        let done = move |chosen: Result<gio::File, glib::Error>| {
            answer(chosen.ok().and_then(|file| file.path()));
        };
        match pick {
            crate::chooser::SavePick::File { suggested } => {
                dialog.set_title("Save attachment");
                dialog.set_initial_name(Some(&suggested));
                dialog.save(Some(self), None::<&gio::Cancellable>, done);
            }
            crate::chooser::SavePick::Folder => {
                dialog.set_title("Save attachments to");
                dialog.select_folder(Some(self), None::<&gio::Cancellable>, done);
            }
        }
    }

    /// What the open-with chooser offers, each as its words and its target
    /// or size; empty while it is not shown.
    pub fn choices_shown(&self) -> Vec<(String, String)> {
        let shown = self
            .visible_dialog()
            .is_some_and(|dialog| dialog.widget_name() == crate::chooser::DIALOG_NAME);
        if !shown {
            return Vec::new();
        }
        self.imp()
            .choices
            .borrow()
            .iter()
            .map(crate::chooser::Choice::said)
            .collect()
    }

    /// Choose the `index`th thing the chooser offers, as a click on it does.
    pub fn choose(&self, index: usize) {
        self.close_chooser();
        self.open_choice(index);
    }

    /// The row the chooser opened on: where a chip took it.
    pub fn choice_focused(&self) -> Option<String> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::chooser::DIALOG_NAME)
            .and_then(|dialog| crate::chooser::selected(&dialog))
    }

    fn close_chooser(&self) {
        if let Some(dialog) = self
            .visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::chooser::DIALOG_NAME)
        {
            dialog.close();
        }
    }

    fn pick_choice(&self, pick: crate::chooser::Pick) {
        use crate::chooser::Pick;
        match pick {
            Pick::Open(index) => self.open_choice(index),
            Pick::Save(index) => self.save_choice(index),
            Pick::SaveAll => self.save_all_parts(),
        }
    }

    /// Save the part the chooser offered at `index` to a file the portal
    /// names, written by the host (a part not here yet is fetched first).
    fn save_choice(&self, index: usize) {
        let choice = self.imp().choices.borrow().get(index).cloned();
        self.imp().choices.borrow_mut().clear();
        let Some(crate::chooser::Choice::Part { id, .. }) = choice else {
            return;
        };
        let (Some(reading), Some(client)) = (self.reading(), self.imp().client.borrow().clone())
        else {
            return;
        };
        let (Some(message), Some(node)) = (reading.shown(), reading.part_node(id)) else {
            return;
        };
        let suggested = postio_ui::reader::parts::save_name(&node);
        let window = self.downgrade();
        self.pick_place(crate::chooser::SavePick::File { suggested }, move |to| {
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the
                // host answers on its own runtime (ADR 0041).
                let saved = client.save_part(message, id, to).await;
                let Some(window) = window.upgrade() else {
                    return;
                };
                match saved {
                    Ok(path) => {
                        let name = path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        window.imp().toast.show_notice(&format!("Saved {name}"));
                    }
                    Err(error) => window.imp().toast.show_notice(&error.to_string()),
                }
                window.follow_toast();
            });
        });
    }

    /// Save every attachment into a folder the portal names, each under the
    /// name the sender gave it, no two over one another.
    fn save_all_parts(&self) {
        self.imp().choices.borrow_mut().clear();
        let (Some(reading), Some(client)) = (self.reading(), self.imp().client.borrow().clone())
        else {
            return;
        };
        let Some(message) = reading.shown() else {
            return;
        };
        let nodes = reading.part_nodes();
        if nodes.is_empty() {
            return;
        }
        let window = self.downgrade();
        self.pick_place(crate::chooser::SavePick::Folder, move |folder| {
            let names = postio_ui::reader::parts::save_names(&nodes);
            let targets: Vec<_> = nodes
                .iter()
                .zip(names)
                .filter_map(|(node, name)| Some((node.attachment?, folder.join(name))))
                .collect();
            let count = targets.len();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: as `save_choice`'s.
                let saved = client.save_parts(message, targets).await;
                let Some(window) = window.upgrade() else {
                    return;
                };
                let said = match saved {
                    Ok(failed) => postio_ui::reader::parts::save_all_failure(failed)
                        .unwrap_or_else(|| format!("Saved {count} attachments")),
                    Err(error) => error.to_string(),
                };
                window.imp().toast.show_notice(&said);
                window.follow_toast();
            });
        });
    }

    /// Open what the chooser offered at `index`: a link as it is, a part
    /// written out by the host first.
    fn open_choice(&self, index: usize) {
        let Some(choice) = self.imp().choices.borrow().get(index).cloned() else {
            return;
        };
        self.imp().choices.borrow_mut().clear();
        match choice {
            crate::chooser::Choice::Link { target, .. } => self.launch(&target),
            crate::chooser::Choice::Part { id, .. } => {
                let (Some(message), Some(client)) = (
                    self.reading().and_then(|reading| reading.shown()),
                    self.imp().client.borrow().clone(),
                ) else {
                    return;
                };
                let window = self.downgrade();
                glib::spawn_future_local(async move {
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive;
                    // the host answers on its own runtime (ADR 0041).
                    let written = client.open_part(message, id).await;
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    match written {
                        Ok(path) => window.launch(&gio::File::for_path(path).uri()),
                        Err(error) => {
                            window.imp().toast.show_notice(&error.to_string());
                            window.follow_toast();
                        }
                    }
                });
            }
        }
    }

    /// The raw source on screen, while it is shown.
    pub fn source_shown(&self) -> Option<String> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::source::DIALOG_NAME)
            .and_then(|dialog| crate::source::shown(&dialog))
    }

    /// `v`: the raw source of the message open, or else of the row under
    /// the cursor, read through the host and fetched only now if it is not
    /// here (US2 scenario 5).
    fn view_source(&self) {
        let message = self
            .reading()
            .filter(|reading| reading.is_open())
            .and_then(|reading| reading.shown())
            .or_else(|| self.cursor_row().map(|row| row.id()));
        let (Some(message), Some(client)) = (message, self.imp().client.borrow().clone()) else {
            return;
        };
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // answers on its own runtime (ADR 0041).
            let read = client.raw_source(message).await;
            let Some(window) = window.upgrade() else {
                return;
            };
            match read {
                Ok(raw) => crate::source::dialog(&raw, &window.keymap()).present(Some(&window)),
                Err(error) => {
                    window.imp().toast.show_notice(&error.to_string());
                    window.follow_toast();
                }
            }
        });
    }

    /// Where remote images a person allowed are fetched: the host's
    /// runtime, which the desktop app's fetcher runs on too.
    pub fn set_remote_runtime(&self, runtime: tokio::runtime::Handle) {
        self.imp().runtime.replace(Some(runtime));
    }

    /// The pinned saved searches, in `config.toml`'s order: each name and
    /// its query.
    pub fn set_saved_searches(&self, saved: Vec<(String, String)>) {
        if let Some(bar) = self.bar() {
            bar.set_saved(saved.clone());
        }
        self.imp().saved.replace(saved);
    }

    /// The folders popover, once it has been opened.
    pub fn places(&self) -> Option<Rc<crate::places::Places>> {
        self.imp().places.borrow().clone()
    }

    /// What the header strip names the list: the place it shows.
    pub fn place_name(&self) -> String {
        self.chrome()
            .map(|chrome| chrome.place())
            .unwrap_or_default()
    }

    /// `g o`, or a click on "Inbox ▾": the folders popover (screen 10).
    fn open_places(&self) {
        let (Some(chrome), Some(client)) = (self.chrome(), self.imp().client.borrow().clone())
        else {
            return;
        };
        let places = self
            .imp()
            .places
            .borrow_mut()
            .get_or_insert_with(|| {
                let places =
                    crate::places::Places::new(client, &self.keymap(), chrome.place_button());
                places.connect_go(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |destination, name| window.go_to(destination, &name)
                ));
                places.connect_command(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |command| window.act(command)
                ));
                // Chosen from or dismissed, the keyboard goes back to the
                // list, not to the button the popover hangs from.
                places.connect_closed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.focus_list_soon()
                ));
                places
            })
            .clone();
        let filtering = self.imp().focus_config.borrow().filtering;
        places.set_filtered_today(
            self.imp()
                .counts
                .get()
                .filter(|_| filtering)
                .map(|counts| counts.filtered_today),
        );
        places.open();
        crate::motion::keep_to_budget(self);
    }

    /// Show `destination` in the list, and name it in the header strip. A
    /// label is its search: Focus lists no label on its own.
    fn go_to(&self, destination: postio_ui::finder::Destination, name: &str) {
        use postio_ui::finder::Destination;
        let (Some(pane), Some(chrome)) = (self.pane(), self.chrome()) else {
            return;
        };
        match destination {
            Destination::Mailbox(mailbox) => {
                let inbox = self.is_focus_inbox(mailbox);
                self.imp().at_inbox.set(inbox);
                self.clear_selection();
                if inbox {
                    self.go_to_inbox();
                    return;
                }
                self.imp().has_action.set(false);
                self.imp()
                    .place
                    .replace(Some(postio_ui::focus_state::EmptyPlace::Folder(
                        name.to_owned(),
                    )));
                pane.feed().list().set_single_heading(None);
                pane.feed().open(ListScope::Mailbox(mailbox));
                chrome.set_place(name);
                self.show_counts();
                self.focus_list_soon();
            }
            Destination::Label(_) => {
                if let Some(bar) = self.bar() {
                    bar.open();
                    bar.set_text(&format!("label:\"{name}\""));
                }
            }
            Destination::Search(query) => {
                if let Some(bar) = self.bar() {
                    bar.open();
                    bar.set_text(&query);
                }
            }
            // A view over Drafts, listed as Snoozed is (T239).
            Destination::Outbox(account) => {
                self.imp().at_inbox.set(false);
                self.imp().has_action.set(false);
                self.imp()
                    .place
                    .replace(Some(postio_ui::focus_state::EmptyPlace::Outbox));
                self.clear_selection();
                pane.feed().list().set_single_heading(None);
                pane.feed().open(ListScope::Outbox(account));
                chrome.set_place(name);
                self.show_counts();
                self.focus_list_soon();
            }
        }
    }

    /// Back to Focus's own inbox: `g i`.
    fn go_to_inbox(&self) {
        let (Some(pane), Some(chrome)) = (self.pane(), self.chrome()) else {
            return;
        };
        self.imp().at_inbox.set(true);
        self.imp().has_action.set(false);
        self.imp().place.replace(None);
        self.clear_selection();
        pane.feed().list().set_single_heading(None);
        pane.feed().open(ListScope::Focus(FocusScope::Inbox));
        chrome.set_place("Inbox");
        self.show_counts();
        self.focus_list_soon();
    }

    /// Whether `mailbox` is an inbox: going there is going to Focus's.
    fn is_focus_inbox(&self, mailbox: postio_model::MailboxId) -> bool {
        self.pane()
            .is_some_and(|pane| pane.feed().is_inbox(mailbox))
    }

    /// The command bar, once the inbox is showing.
    pub fn bar(&self) -> Option<Rc<crate::bar::Bar>> {
        self.imp().bar.borrow().clone()
    }

    /// What a row of the bar asked for.
    fn bar_action(&self, action: crate::bar::BarAction) {
        match action {
            // A hit opens by itself. It was opened through the list's
            // cursor row first, so a fresh window -- no cursor row until a
            // key puts one there -- opened nothing at all.
            crate::bar::BarAction::Open { message, subject } => {
                if let Some(reading) = self.reading_dialog() {
                    reading.show_found(self, message, &subject);
                    crate::a11y::teach_shortcuts(&reading.view());
                    crate::motion::keep_to_budget(&reading.view());
                }
            }
            crate::bar::BarAction::Command(command) => self.act(command),
            crate::bar::BarAction::Go { destination, name } => self.go_to(destination, &name),
        }
    }

    /// The window became the active one, or stopped being it: the open
    /// message's read clock runs only while it is. Called from the window's
    /// own notification; public because a headless compositor grants no
    /// focus, so a test says what the notification would.
    pub fn focus_changed(&self, active: bool) {
        if let Some(reading) = self.reading() {
            reading.set_window_active(active);
        }
    }

    /// The open-email dialog, once a message has been opened.
    pub fn reading(&self) -> Option<Rc<crate::open::OpenMessage>> {
        self.imp().reading.borrow().clone()
    }

    /// `?`: the key map, over the window (screen 20).
    fn show_key_map(&self) {
        let dialog = crate::keymap_dialog::build(&self.keymap());
        dialog.set_widget_name(KEY_MAP);
        crate::motion::keep_to_budget(&dialog);
        dialog.present(Some(self));
    }

    /// Every command the command bar can list, by typing its name.
    pub fn command_bar_rows(&self) -> Vec<CommandId> {
        self.bar().map(|bar| bar.commands()).unwrap_or_default()
    }

    /// Every command a person can reach with the mouse somewhere in the
    /// window: the chrome and its menu, the bulk bar, the banner's button,
    /// the empty inbox's shortcuts, the toast's Undo, and the toast that
    /// offers to add an account when compose has none to write from
    /// (T172). Each is read from the table that builds it.
    pub fn controls(&self) -> Vec<CommandId> {
        let mut commands = Chrome::commands();
        commands.extend(Bulk::commands());
        commands.extend(
            [
                postio_ui::focus_state::BannerAction::Retry,
                postio_ui::focus_state::BannerAction::UpdatePassword,
            ]
            .map(banner_command),
        );
        let everything = postio_config::FocusConfig {
            filtering: true,
            ..postio_config::FocusConfig::default()
        };
        commands.extend(
            postio_ui::focus_state::empty_inbox(
                &everything,
                0,
                &self.keymap(),
                &postio_ui::clock::now(),
            )
            .shortcuts
            .into_iter()
            .map(|(_, _, command)| command),
        );
        commands.extend(crate::open::OpenMessage::controls());
        // The empty reading pane's shortcuts (T232).
        commands.extend(
            postio_ui::focus_state::empty_pane(&self.keymap())
                .shortcuts
                .into_iter()
                .map(|(_, _, command)| command),
        );
        commands.extend(crate::places::Places::controls());
        commands.extend(crate::row_menu::RowMenu::commands());
        commands.extend(crate::bar::Bar::controls());
        commands.extend(crate::filtered::FilteredView::controls());
        commands.extend(crate::digest::DigestWindow::controls());
        // The strip's counts are ways there: "186 filtered today g f" and
        // "4 digest rules g d".
        commands.extend([CommandId::GoToFiltered, CommandId::GoToDigestRules]);
        commands.push(CommandId::Undo);
        commands.push(CommandId::AddAccount);
        commands.extend(crate::settings::Settings::controls());
        commands.extend(crate::compose::Compose::controls());
        commands.extend(crate::capture::CaptureSheet::controls());
        // The answering actions a marked row draws, each a button.
        commands.extend([CommandId::AcceptInvite, CommandId::DeclineInvite]);
        commands.sort_by_key(|command| command.as_str());
        commands.dedup();
        commands
    }

    /// Open the row menu on the row at `position`, at `at` in the list's
    /// coordinates (T199). The cursor goes to the row, as a click's does.
    /// A row inside the selection gets a menu for the selection, and says
    /// so; a row outside it gets one for itself, and the selection is let
    /// go only when one of its verbs runs, so dismissing loses nothing. A
    /// digest's row has its own verbs and no menu.
    pub fn open_row_menu(&self, position: u32, at: gdk::Rectangle) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        if let Some(picker) = self.open_picker() {
            picker.close();
        }
        self.cursor_to(Some(position));
        let Some(row) = self.cursor_row() else {
            return;
        };
        let Some(conversation) = row.as_conversation() else {
            return;
        };
        let selection = imp.picked.selection();
        let inside = match &selection {
            Selection::These(picked) => picked.contains(&row.id()),
            Selection::Everything { except } => !except.contains(&row.id()),
        };
        let summary = inside
            .then(|| postio_ui::selection::summary(&selection, Some(pane.feed().total()), &[]))
            .flatten();
        imp.row_menu_alone.set(!inside);
        let menu = imp.row_menu.borrow().clone();
        let menu = menu.unwrap_or_else(|| {
            let menu = crate::row_menu::RowMenu::new(&self.keymap());
            menu.connect_command(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |id| window.row_menu_chose(id)
            ));
            imp.row_menu.replace(Some(Rc::clone(&menu)));
            menu
        });
        menu.open(
            pane.view(),
            &at,
            summary.as_deref(),
            crate::row_menu::Facts {
                unread: conversation.summary.unread_count > 0,
                flagged: conversation.summary.flagged,
                snoozed: matches!(
                    pane.feed().scope(),
                    Some(ListScope::Focus(FocusScope::Snoozed))
                ),
            },
        );
    }

    /// A verb chosen from the row menu: the one command its key runs, on
    /// the selection or, for a row outside it, on the row alone.
    fn row_menu_chose(&self, id: CommandId) {
        if self.imp().row_menu_alone.replace(false) {
            self.clear_selection();
        }
        self.act(id);
    }

    /// The row's right-click menu, once a right-click has built it.
    pub fn row_menu(&self) -> Option<Rc<crate::row_menu::RowMenu>> {
        self.imp().row_menu.borrow().clone()
    }

    /// The picker open at the row, if one is.
    pub fn open_picker(&self) -> Option<Rc<Picker>> {
        let imp = self.imp();
        [&imp.snooze, &imp.remind]
            .into_iter()
            .filter_map(|picker| {
                picker
                    .borrow()
                    .as_ref()
                    .map(|picker| Rc::clone(picker.picker()))
            })
            .chain(
                imp.labels
                    .borrow()
                    .as_ref()
                    .map(|labels| Rc::clone(labels.picker())),
            )
            .chain(
                imp.moves
                    .borrow()
                    .as_ref()
                    .map(|moves| Rc::clone(moves.picker())),
            )
            // The composer's remind picker, at its footer (US3).
            .chain(self.compose().and_then(|compose| compose.open_picker()))
            .find(|picker| picker.is_open())
    }

    /// Open the move picker at the cursor's row (US5 scenario 6): the
    /// move goes where every verb goes, and the selection with it.
    fn open_moves(&self) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let picker = imp.moves.borrow().clone();
        let picker = picker.unwrap_or_else(|| {
            let picker = crate::move_picker::MovePicker::new(client, &self.keymap());
            imp.moves.replace(Some(Rc::clone(&picker)));
            picker
        });
        let Some((anchor, rect)) = self.at_the_row() else {
            return;
        };
        let target = self.picker_target();
        picker.open(
            &anchor,
            Some(&rect),
            &target,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |command| window.send(command)
            ),
        );
    }

    /// Open the label picker at the cursor's row, over what a verb would
    /// aim at now (US5 scenario 5). Each change it makes is sent to those
    /// same conversations, and the selection goes when it closes.
    fn open_labels(&self) {
        let imp = self.imp();
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let Some(account) = imp.accounts.borrow().first().copied() else {
            return;
        };
        let aims = self.aims();
        if aims.is_empty() {
            return;
        }
        let picker = imp.labels.borrow().clone();
        let picker = picker.unwrap_or_else(|| {
            let picker = crate::label_picker::LabelPicker::new(client.clone(), &self.keymap());
            picker.picker().connect_closed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.clear_selection()
            ));
            imp.labels.replace(Some(Rc::clone(&picker)));
            picker
        });
        let threads = aims
            .iter()
            .filter_map(|aim| match aim {
                MessageTarget::Threads(threads) => Some(threads.clone()),
                _ => None,
            })
            .flatten()
            .collect();
        let Some((anchor, rect)) = self.at_the_row() else {
            return;
        };
        let target = self.picker_target();
        let aim = crate::label_picker::LabelAim {
            account,
            message: self.cursor_row().map(|row| row.id()),
            threads,
        };
        picker.open(&anchor, Some(&rect), &target, aim, move |command| {
            let (client, aims) = (client.clone(), aims.clone());
            glib::spawn_future_local(async move {
                for aim in aims {
                    // POSTIO-GLIB-SAFE: as `send`'s.
                    let sent = client.send(command.clone().with_target(aim)).await;
                    if let Err(error) = sent {
                        tracing::warn!(%error, "Focus could not send a command: {error}");
                    }
                }
            });
        });
    }

    /// Open the snooze or remind picker at the cursor's row, aimed at the
    /// selection when there is one (US5).
    fn open_when(&self, when: When) {
        let imp = self.imp();
        let slot = match when {
            When::Snooze => &imp.snooze,
            When::Remind => &imp.remind,
        };
        let picker = slot.borrow().clone();
        let picker = picker.unwrap_or_else(|| {
            let picker = WhenPicker::new(&self.keymap(), when);
            picker.connect_chosen(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |at| window.send(match when {
                    When::Snooze => Command::Snooze {
                        target: MessageTarget::Selection,
                        until: Some(at.to_utc()),
                    },
                    When::Remind => Command::RemindIfNoReply {
                        target: MessageTarget::Selection,
                        at: Some(at.to_utc()),
                    },
                })
            ));
            slot.replace(Some(Rc::clone(&picker)));
            picker
        });
        let Some((anchor, rect)) = self.at_the_row() else {
            return;
        };
        picker.open(
            &anchor,
            Some(&rect),
            &self.picker_target(),
            postio_ui::clock::now(),
        );
    }

    /// Where a picker hangs: from the list, under the cursor's row, at the
    /// subject column (screens 11-14).
    fn at_the_row(&self) -> Option<(gtk::ListView, gdk::Rectangle)> {
        let pane = self.pane()?;
        let view = pane.view().clone();
        let cursor = self.cursor_row()?;
        let row = pane
            .rows_on_screen()
            .into_iter()
            .find(|row| row.item().is_some_and(|item| item.id() == cursor.id()));
        let rect = match row.and_then(|row| {
            row.compute_point(&view, &gtk::graphene::Point::new(0.0, 0.0))
                .map(|top| (top, row.height()))
        }) {
            Some((top, height)) => gdk::Rectangle::new(
                crate::list::row::subject_x(view.width()) as i32,
                top.y() as i32,
                1,
                height,
            ),
            None => gdk::Rectangle::new(crate::list::row::subject_x(view.width()) as i32, 0, 1, 1),
        };
        Some((view, rect))
    }

    /// What a picker names as its target: the cursor's conversation, or
    /// how many are selected.
    fn picker_target(&self) -> String {
        postio_ui::focus_target::picker_target(&self.selection(), self.cursor_row().as_ref())
    }

    /// The key map, while it is open.
    pub fn key_map(&self) -> Option<adw::Dialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == KEY_MAP)
    }

    /// The compose dialog, while it is over the window (screens 05, 06).
    pub fn compose_dialog(&self) -> Option<adw::Dialog> {
        self.compose().and_then(|compose| compose.dialog())
    }

    /// The composer Focus writes in: the shared one, in a dialog.
    pub fn composer(&self) -> Option<postio_widgets::composer::Composer> {
        self.compose().map(|compose| compose.composer().clone())
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

    /// Say `sentence` in a toast: something that happened, with nothing
    /// to undo.
    pub fn say(&self, sentence: &str) {
        self.imp().toast.show_notice(sentence);
        self.follow_toast();
    }

    /// Keep track of the toast just shown until it goes.
    /// Say `sentence` in a toast whose Undo runs `on_undo`.
    pub(crate) fn show_removable(&self, sentence: &str, on_undo: impl Fn() + 'static) {
        self.imp().toast.show_removable(sentence, on_undo);
        self.follow_toast();
    }

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

    /// The overlay the toast on screen is drawn in: the dialog's while a
    /// message is open over the window, the window's otherwise.
    pub fn toast_host(&self) -> Option<adw::ToastOverlay> {
        self.imp().toast.host()
    }

    /// The toast on screen, if one is: its words, its button and how long
    /// it stays.
    pub fn toast(&self) -> Option<adw::Toast> {
        self.imp().on_screen.borrow().clone()
    }

    /// Take the toast on screen away, as its timeout would.
    pub fn dismiss_toast(&self) {
        let toast = self.imp().on_screen.borrow().clone();
        if let Some(toast) = toast {
            toast.dismiss();
        }
    }
}

impl FocusWindow {
    /// Where everything is, for a storyboard (specs/008-storyboards,
    /// contracts/observation.md § Focus).
    ///
    /// Every field is read off what is on screen -- the widget that really
    /// holds the keyboard, the cursor's row, the toast that is up -- and not
    /// off what a surface was told. It reads only: no store, no command, no
    /// focus change, and it builds no surface that is not already there.
    ///
    /// Unobserved, and left empty so a check on them reads "not applicable"
    /// and never a pass: `rows.first_visible` (the list is windowed over the
    /// store, so a row's index is not its place on screen),
    /// `reading.focused` (the open message shows one message, not a
    /// conversation of them) and `back_depth` (Back is a cascade, not a
    /// stack).
    pub fn observe(&self) -> postio_ui::observe::Observation {
        use postio_ui::observe::{
            Banner, Composer as ComposerState, Cursor, Keyboard, Notice, Observation, Overlay,
            OverlayState, Reading, Region, Rows, Scroll, Selection as SelectionCount, View,
            Window as WindowState,
        };
        use postio_widgets::storyboard::{deliver, reach};

        let toplevel: &gtk::Window = self.upcast_ref();
        let target = deliver::keyboard_target(toplevel);
        let focus = gtk::prelude::GtkWindowExt::focus(self);
        let within = |pane: &gtk::Widget| {
            focus
                .as_ref()
                .is_some_and(|widget| widget == pane || widget.is_ancestor(pane))
        };
        let dialog = deliver::presented_dialog(toplevel);
        let dialog_named = |name: &str| dialog.as_ref().is_some_and(|d| d.widget_name() == name);

        let bar = self.bar().filter(|bar| bar.is_open());
        let places = self.places().filter(|places| places.is_open());
        let picker = self.open_picker();
        let menu = self.row_menu().filter(|menu| menu.is_open());
        let key_map = self.key_map();
        let rule = self.rule_dialog().filter(|rule| rule.dialog().is_mapped());
        let reading = self.reading().filter(|reading| reading.is_open());
        let digest = self.digest();
        // The composer in its dialog or in the reading pane (T232).
        let compose_open = self.compose().is_some_and(|compose| compose.is_showing());
        let composition_detached = self
            .compose()
            .is_some_and(|compose| compose.composer().is_detached());
        let sign_in = self.add_account_dialog().is_some();
        let settings = self.settings_dialog().is_some();
        // The raw source and the open chooser are dialogs of their own over
        // the open message.
        let small_dialog = [crate::source::DIALOG_NAME, crate::chooser::DIALOG_NAME]
            .into_iter()
            .find(|name| dialog_named(name));
        // A message open beside the list, the keyboard on it (T232): the
        // window's own context says so, as it does for every key.
        let reading_here = self.visible_dialog().is_none() && self.reading_beside();

        let region = if menu.is_some() {
            Region::Menu
        } else if picker.is_some() || places.is_some() {
            Region::Picker
        } else if settings {
            Region::Settings
        } else if key_map.is_some()
            || rule.is_some()
            || small_dialog.is_some()
            || dialog_named(crate::capture::DIALOG_NAME)
            || sign_in
        {
            Region::Dialog
        } else if compose_open {
            Region::Composer
        } else if dialog_named(crate::open::DIALOG_NAME) || reading_here {
            Region::Reader
        } else if digest.is_some() {
            Region::Dialog
        } else if focus.is_none() {
            // Focus leaves the keyboard on the window itself when nothing
            // is focused (a closed bar hands focus back to nothing, by
            // design: `Bar::close`), and the window's own key controller
            // is where its list's keys are handled.
            Region::List
        } else if bar
            .as_ref()
            .is_some_and(|bar| within(&bar.widget().clone().upcast()))
        {
            Region::Search
        } else if self
            .pane()
            .is_some_and(|pane| within(pane.widget().upcast_ref()))
        {
            Region::List
        } else if dialog.is_none()
            && !self.is_typing()
            && self.key_context() == KeyContext::List
            && self.unavailable_reason().is_none()
        {
            // A control in the top bar or the foot strip holds focus, but
            // the window's controller takes every key first (capture
            // phase), and in the list's context they are the list's.
            Region::List
        } else {
            Region::Other
        };

        let field = match region {
            Region::Search | Region::Picker => Some("query".to_owned()),
            // The composer's field, named as contracts/observation.md names
            // them: the one the keyboard is in, in its dialog or the pane.
            Region::Composer => self
                .compose()
                .and_then(|compose| compose.composer().focused_field())
                .map(|field| {
                    use postio_widgets::composer::Field;
                    match field {
                        Field::To => "to",
                        Field::Cc => "cc",
                        Field::Bcc => "bcc",
                        Field::Subject => "subject",
                        Field::Body => "body",
                    }
                    .to_owned()
                }),
            _ => None,
        };

        let mut path = Vec::new();
        let mut node = Some(target.clone());
        while let Some(widget) = node {
            path.push(widget.type_().name().to_string());
            node = widget.parent();
        }
        path.reverse();

        let pane = self.pane();
        let at = pane
            .as_ref()
            .map(|pane| pane.cursor().selected())
            .filter(|at| *at != gtk::INVALID_LIST_POSITION);
        let row = self.cursor_row();
        let count = pane.as_ref().map(|pane| pane.feed().list().n_items());
        let selected = match self.selection() {
            Selection::These(ids) => ids.len() as u32,
            Selection::Everything { except } => {
                count.unwrap_or(0).saturating_sub(except.len() as u32)
            }
        };

        let overlay = if key_map.is_some() {
            OverlayState {
                kind: Overlay::Keymap,
                mode: None,
            }
        } else if rule.is_some() {
            OverlayState {
                kind: Overlay::Dialog,
                mode: Some("rule".to_owned()),
            }
        } else if settings {
            OverlayState {
                kind: Overlay::Dialog,
                mode: Some("settings".to_owned()),
            }
        } else if let Some(name) = small_dialog {
            OverlayState {
                kind: Overlay::Dialog,
                mode: Some(name.trim_start_matches("focus-").to_owned()),
            }
        } else if menu.is_some() {
            OverlayState {
                kind: Overlay::Menu,
                mode: None,
            }
        } else if picker.is_some() {
            OverlayState {
                kind: Overlay::Picker,
                mode: None,
            }
        } else if places.is_some() {
            OverlayState {
                kind: Overlay::Picker,
                mode: Some("places".to_owned()),
            }
        } else if bar.is_some() {
            OverlayState {
                kind: Overlay::Finder,
                mode: None,
            }
        } else {
            OverlayState {
                kind: Overlay::None,
                mode: None,
            }
        };

        let imp = self.imp();
        let notice = Notice {
            text: self.toast_showing(),
            tone: imp.toast.tone(),
            undo: imp.toast.offers_undo(),
        };

        let locked = self.unavailable_reason().is_some();
        let view = if locked {
            // The page a store that will not open leaves, a locked keyring
            // among the reasons: what the window shows instead of mail.
            View::Locked
        } else if settings {
            View::Settings
        } else if compose_open {
            View::Composer
        } else if sign_in {
            View::FirstRun
        } else if reading.is_some() {
            View::Reader
        } else if digest.is_some() {
            View::Digest
        } else if self.filtered().is_some() {
            View::Filtered
        } else if bar.is_some() {
            View::Search
        } else {
            View::List
        };

        let scroll = reading
            .as_ref()
            .filter(|_| view == View::Reader)
            .and_then(|reading| reading.reader().view().scroll_extent())
            .map(|(offset, max)| Scroll {
                offset: offset.max(0.0) as u32,
                max: max as u32,
            });

        let mut app = std::collections::BTreeMap::new();
        app.insert(
            "focus.sync".to_owned(),
            serde_json::Value::String(self.sync_said()),
        );
        app.insert(
            "focus.close_button".to_owned(),
            serde_json::Value::Bool(self.close_button_showing()),
        );
        let bulk = imp.bulk.borrow().as_ref().and_then(|bulk| bulk.summary());
        // What the empty page says, when it is the page showing in place of
        // the list: a place with nothing in it is not a blank pane.
        let empty = self
            .imp()
            .list_or_empty
            .borrow()
            .as_ref()
            .filter(|stack| stack.visible_child_name().as_deref() == Some(EMPTY))
            .and(imp.empty.borrow().as_ref())
            .map(|page| page.heading());
        app.insert("focus.empty.heading".to_owned(), serde_json::json!(empty));
        // What the header strip counts beside the place's name.
        if let Some(chrome) = self.chrome() {
            app.insert(
                "focus.strip.counts".to_owned(),
                serde_json::json!(chrome.counts_said()),
            );
            app.insert(
                "focus.strip.has_action_shown".to_owned(),
                serde_json::json!(chrome.has_action_shown()),
            );
        }
        app.insert(
            "focus.bulk".to_owned(),
            serde_json::json!({ "shown": bulk.is_some(), "summary": bulk }),
        );
        // How many of the cursor's conversation are still unread: a
        // message read by the dwell takes one off.
        if let Some(conversation) = row.as_ref().and_then(|row| row.as_conversation()) {
            app.insert(
                "focus.cursor.unread".to_owned(),
                serde_json::json!(conversation.summary.unread_count),
            );
        }
        if let Some(bar) = &bar {
            // What the bar lists, which the list behind it never shows: its
            // heading, how many messages it found and the row Return runs.
            // One key each, because a check names an app field by its whole
            // key (`app.focus.bar.messages`).
            let highlighted = bar.highlighted();
            let fields = [
                ("focus.bar.typed", serde_json::json!(bar.typed())),
                ("focus.bar.heading", serde_json::json!(bar.heading())),
                (
                    "focus.bar.messages",
                    serde_json::json!(bar.result_subjects().len()),
                ),
                (
                    "focus.bar.highlighted",
                    serde_json::json!(highlighted.as_ref().map(|(kind, _, _)| *kind)),
                ),
                (
                    "focus.bar.highlighted_id",
                    serde_json::json!(
                        highlighted
                            .as_ref()
                            .and_then(|(_, id, _)| id.map(|id| id.get().to_string()))
                    ),
                ),
                (
                    "focus.bar.highlighted_text",
                    serde_json::json!(highlighted.map(|(_, _, text)| text)),
                ),
            ];
            for (key, value) in fields {
                app.insert(key.to_owned(), value);
            }
        }
        if let Some(digest) = &digest {
            app.insert(
                "focus.digest_page".to_owned(),
                serde_json::Value::String(format!("{:?}", digest.showing()).to_lowercase()),
            );
        }

        Observation {
            window: if self.is_visible() {
                WindowState::Open
            } else {
                WindowState::Closed
            },
            view,
            scope: Some(self.place_name()).filter(|place| !place.is_empty()),
            keyboard: Keyboard {
                region,
                field,
                typing: self.is_typing(),
                // The window's own controller takes a key when nothing is
                // focused and nothing is over it, so that is reachable.
                reachable: reach::reachable(toplevel)
                    || (focus.is_none() && dialog.is_none() && self.is_visible()),
                widget: path.join("/"),
            },
            cursor: Cursor {
                index: at,
                id: row.as_ref().map(|row| row.id().get().to_string()),
                subject: row.as_ref().and_then(|row| match row {
                    crate::list::FocusRow::Digest(digest) => Some(digest.rule.clone()),
                    other => other
                        .as_conversation()
                        .and_then(|row| row.summary.subject.clone()),
                }),
            },
            rows: Rows {
                first_visible: None,
                count,
            },
            selection: SelectionCount { count: selected },
            overlay,
            notice,
            banner: Banner {
                title: self.banner_showing().map(|(title, _, _)| title),
            },
            reading: Reading {
                id: reading
                    .as_ref()
                    .filter(|_| view == View::Reader)
                    .and_then(|reading| reading.shown())
                    .map(|id| id.get().to_string()),
                focused: None,
                scroll,
            },
            composer: ComposerState {
                // A composition in a window of its own is still open, though
                // this window shows the list again.
                open: compose_open || composition_detached,
                detached: composition_detached,
            },
            back_depth: None,
            app,
        }
    }
}

/// The command a banner's button runs.
fn banner_command(action: postio_ui::focus_state::BannerAction) -> CommandId {
    match action {
        postio_ui::focus_state::BannerAction::Retry => CommandId::Refresh,
        postio_ui::focus_state::BannerAction::UpdatePassword => CommandId::UpdateCredential,
    }
}
