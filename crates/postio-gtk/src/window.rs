//! Focus's one window: what it says while the store opens, the sentence when it cannot, and the
//! inbox once it has.
//! The window comes first and the store behind it: it says what it waits for only once the wait is
//! worth mentioning.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib::translate::IntoGlib;
use gtk::{gdk, glib};
use postio_client::Client;
use postio_core::state::{Selection, ViewScope};
use postio_core::{Command, CommandId, Keymap, MessageTarget, SharedState};
use postio_model::ids::{AccountId, MessageId};
use postio_model::{FocusScope, ListScope};
use postio_ui::keymap::{KeyContext, Outcome, Resolver};
use postio_ui::list_state::{OPENING_THRESHOLD, Waiting, describe_wait};
use postio_ui::selection::SelectionState;

use crate::bulk::Bulk;
use crate::chrome::Chrome;
use crate::list::{Feed, FocusRow, ListPane, RowObject};
use postio_focus::{Route, SurfaceKind};
use postio_widgets::list_model::WindowedModel;
use postio_widgets::widgets::pickers::{Picker, When, WhenPicker};

type Launcher = Rc<dyn Fn(&str)>;
type FilePicker = Rc<dyn Fn(crate::chooser::SavePick, Box<dyn FnOnce(Option<std::path::PathBuf>)>)>;

type Editor = Rc<dyn Fn(&std::path::Path)>;

const DEFAULT_GEOMETRY: postio_widgets::state::Geometry =
    postio_widgets::state::Geometry::new(1440, 900);
/// Put on the window while the list does not hold the keyboard: the cursor row then wears a neutral
/// mark, so the accent ring is on screen once, where the keyboard is.
const KEYBOARD_AWAY: &str = "focus-keyboard-away";
const BLANK: &str = "blank";
const OPENING: &str = "opening";
const UNAVAILABLE: &str = "unavailable";
const INBOX: &str = "inbox";
pub type Notifier = Rc<
    dyn Fn(
        postio_model::MailboxId,
        Vec<MessageId>,
        postio_ui::notify::Attention,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<postio_ui::notify::Notification>>>,
    >,
>;

type NotificationSink = Rc<dyn Fn(&postio_ui::notify::Notification)>;

const STOP_DIALOG: &str = "focus-stop-digesting";

const SWEEP_DIALOG: &str = "focus-sweep";

const RULES: &str = "rules";

const REMOVE_RULE_DIALOG: &str = "focus-remove-rule";

const FILTERED: &str = "filtered";
const CONTACTS_IO_DIALOG: &str = "focus-contacts-io";

const CONTACTS: &str = "contacts";
const RESULTS: &str = "results";
const MAIL: &str = "mail";
const LIST: &str = "list";
const KEY_MAP: &str = "focus-key-map";
const EMPTY: &str = "empty";

mod imp {
    use super::*;

    pub struct FocusWindow {
        pub pages: gtk::Stack,
        pub stage: gtk::Overlay,
        pub opening: adw::StatusPage,
        pub unavailable: adw::StatusPage,
        pub retry: gtk::Button,
        pub bar_before_mail: gtk::WindowHandle,
        pub inbox: gtk::Box,
        pub body: gtk::Stack,
        pub mail: gtk::Box,
        pub toast: postio_widgets::widgets::toast::Toast,
        pub on_screen: RefCell<Option<adw::Toast>>,
        pub pane: RefCell<Option<ListPane>>,
        pub chrome: RefCell<Option<Rc<Chrome>>>,
        pub client: RefCell<Option<Client>>,
        pub waiting: Cell<Option<Waiting>>,
        pub waiting_since: Cell<Option<std::time::Instant>>,
        pub on_retry: RefCell<Option<Rc<dyn Fn()>>>,
        pub refusal: RefCell<String>,
        pub keymap: RefCell<Keymap>,
        pub resolver: RefCell<Option<Resolver>>,
        /// Not the cursor, which is GTK's selection model on the list: a mirror of the controller's
        /// selection, written only when its `Selection` intent is applied (ADR 0045).
        pub picked: SelectionState,
        pub bulk: RefCell<Option<Rc<Bulk>>>,
        pub scope_footer: RefCell<Option<Rc<crate::scope_footer::ScopeFooter>>>,
        pub search_words: RefCell<String>,
        pub compose_message: Cell<Option<MessageId>>,
        pub replying_to: Cell<Option<u32>>,
        pub match_from_end: Cell<bool>,
        pub thread_menu: RefCell<Option<Rc<crate::thread_popovers::ThreadMenu>>>,
        pub outline: RefCell<Option<Rc<crate::thread_popovers::OutlinePopover>>>,
        pub state: RefCell<Option<SharedState>>,
        pub accounts: RefCell<Vec<AccountId>>,
        /// Set while the window itself puts the cursor on a list's first row: that opens nothing,
        /// even with the reading pane beside.
        pub placing_cursor: Cell<bool>,
        pub trackers: RefCell<postio_ui::status::Trackers>,
        /// A tracker says `Offline` for an account it has heard nothing of, which is not the same
        /// as the machine having no network.
        pub tracked: RefCell<Vec<AccountId>>,
        pub facts: RefCell<Vec<postio_ui::focus_state::AccountFacts>>,
        pub last_synced: Cell<Option<chrono::DateTime<chrono::Utc>>>,
        pub state_banner: RefCell<Option<Rc<crate::banner::StateBanner>>>,
        pub focus_config: RefCell<postio_config::FocusConfig>,
        pub list_or_empty: RefCell<Option<gtk::Stack>>,
        pub empty: RefCell<Option<Rc<crate::empty::EmptyInbox>>>,
        pub reading: RefCell<Option<Rc<crate::open::OpenMessage>>>,
        pub reading_pane: RefCell<Option<Rc<crate::reading_pane::ReadingPane>>>,
        pub split: RefCell<Option<gtk::Box>>,
        pub placement: Cell<postio_ui::focus_dialog::Placement>,
        pub launcher: RefCell<Option<super::Launcher>>,
        pub file_picker: RefCell<Option<super::FilePicker>>,
        pub choices: RefCell<Vec<crate::chooser::Choice>>,
        pub runtime: RefCell<Option<tokio::runtime::Handle>>,
        pub bar: RefCell<Option<Rc<crate::bar::Bar>>>,
        pub saved: RefCell<Vec<postio_ui::saved_search::SavedSearch>>,
        pub places: RefCell<Option<Rc<crate::places::Places>>>,
        /// So a press the window's controller and a dialog's both see is handled once
        /// (`keys_under_dialogs`).
        pub last_key: Cell<Option<(u32, u32)>>,
        /// Held so its changes keep being heard: libadwaita hands out a model of its own each time.
        pub dialogs: RefCell<Option<gtk::gio::ListModel>>,
        pub snooze: RefCell<Option<Rc<WhenPicker>>>,
        pub row_menu: RefCell<Option<Rc<crate::row_menu::RowMenu>>>,
        pub row_menu_alone: Cell<bool>,
        pub unanswered: RefCell<Vec<CommandId>>,
        pub remind: RefCell<Option<Rc<WhenPicker>>>,
        pub labels: RefCell<Option<Rc<crate::label_picker::LabelPicker>>>,
        pub notifier: RefCell<Option<super::Notifier>>,
        pub notification_sink: RefCell<Option<super::NotificationSink>>,
        pub config_path: RefCell<Option<std::path::PathBuf>>,
        pub timeline: RefCell<Option<postio_widgets::startup::Timeline>>,
        pub rules_view: RefCell<Option<Rc<crate::rules::RulesView>>>,
        pub rule_dialog: RefCell<Option<Rc<crate::rule_dialog::RuleDialog>>>,
        pub digest_window: RefCell<Option<Rc<crate::digest::DigestWindow>>>,
        pub filtered: RefCell<Option<Rc<crate::filtered::FilteredView>>>,
        pub contacts: RefCell<Option<Rc<crate::contacts::ContactsView>>>,
        pub results: RefCell<Option<Rc<crate::results::ResultsPane>>>,
        pub contacts_io: RefCell<Option<postio_ui::contacts_io::IoSheetView>>,
        pub contact_popover:
            RefCell<Option<(gtk::Widget, postio_ui::contact_popover::PopoverView)>>,
        pub pending_link: RefCell<Option<String>>,
        pub pending_mailto: RefCell<Option<postio_model::mailto::Mailto>>,
        pub moves: RefCell<Option<Rc<crate::move_picker::MovePicker>>>,
        pub driven_picker: RefCell<Option<Rc<crate::controller_picker::ControllerPicker>>>,
        pub compose: RefCell<Option<Rc<crate::compose::Compose>>>,
        pub warm: Cell<bool>,
        pub capture: RefCell<Option<Rc<crate::capture::CaptureSheet>>>,
        pub surfaces: RefCell<Vec<postio_focus::SurfaceKind>>,
        pub told_shown: Cell<Option<MessageId>>,
        pub adding_account: RefCell<Option<adw::Dialog>>,
        pub start_syncing: RefCell<Option<Rc<dyn Fn()>>>,
        pub settings: RefCell<Option<Rc<crate::settings::Settings>>>,
        pub settings_seams: RefCell<Option<crate::settings::Seams>>,
        pub editor: RefCell<Option<super::Editor>>,
        pub compose_config: RefCell<postio_config::ComposeConfig>,
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
                body: gtk::Stack::new(),
                mail: gtk::Box::new(gtk::Orientation::Vertical, 0),
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
                bulk: RefCell::default(),
                scope_footer: RefCell::default(),
                search_words: RefCell::default(),
                compose_message: Cell::new(None),
                replying_to: Cell::new(None),
                match_from_end: Cell::new(false),
                thread_menu: RefCell::default(),
                outline: RefCell::default(),
                state: RefCell::default(),
                accounts: RefCell::default(),
                placing_cursor: Cell::default(),
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
                last_key: Cell::new(None),
                dialogs: RefCell::default(),
                snooze: RefCell::default(),
                row_menu: RefCell::default(),
                row_menu_alone: Cell::new(false),
                unanswered: RefCell::default(),
                remind: RefCell::default(),
                labels: RefCell::default(),
                moves: RefCell::default(),
                driven_picker: RefCell::default(),
                pending_link: RefCell::default(),
                pending_mailto: RefCell::default(),
                compose: RefCell::default(),
                warm: Cell::default(),
                filtered: RefCell::default(),
                contacts: RefCell::default(),
                results: RefCell::default(),
                contacts_io: RefCell::default(),
                contact_popover: RefCell::default(),
                digest_window: RefCell::default(),
                rule_dialog: RefCell::default(),
                rules_view: RefCell::default(),
                config_path: RefCell::default(),
                timeline: RefCell::default(),
                notifier: RefCell::default(),
                notification_sink: RefCell::default(),
                capture: RefCell::default(),
                surfaces: RefCell::default(),
                told_shown: Cell::default(),
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
    pub struct FocusWindow(ObjectSubclass<imp::FocusWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget, gtk::Native,
            gtk::Root, gtk::ShortcutManager, gio::ActionGroup, gio::ActionMap;
}

impl FocusWindow {
    /// A missing or unreadable state file means 1440 by 900.
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

    fn follow_maximized(&self, button: &gtk::Button) {
        fn update(window: &FocusWindow, button: &gtk::Button) {
            let (icon, label) = if window.is_maximized() {
                ("window-restore-symbolic", "Restore window")
            } else {
                ("window-maximize-symbolic", "Maximize window")
            };
            button.set_icon_name(icon);
            button.set_tooltip_text(Some(label));
            button.update_property(&[gtk::accessible::Property::Label(label)]);
        }
        update(self, button);
        self.connect_maximized_notify(glib::clone!(
            #[weak]
            button,
            move |window| update(window, &button)
        ));
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

        // Nothing until the wait is worth saying: a window that flashes "Opening your mailbox" for
        // the 150 ms a healthy start takes is the flicker the budget forbids.
        imp.pages
            .add_named(&gtk::Box::new(gtk::Orientation::Vertical, 0), Some(BLANK));
        imp.pages.add_named(&imp.opening, Some(OPENING));
        imp.pages.add_named(&imp.unavailable, Some(UNAVAILABLE));
        imp.pages.add_named(&imp.inbox, Some(INBOX));
        imp.pages.set_visible_child_name(BLANK);

        // Before there is mail there is still a window to close: the same close button the inbox's
        // top bar ends with, running the same Quit.
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
        let maximize = crate::chrome::maximize_button();
        self.follow_maximized(&maximize);
        let controls = gtk::Box::new(
            gtk::Orientation::Horizontal,
            postio_widgets::widgets::space::S2,
        );
        controls.append(&maximize);
        controls.append(&close);
        bar.set_end_widget(Some(&controls));
        imp.bar_before_mail.set_child(Some(&bar));
        let before_mail = gtk::Box::new(gtk::Orientation::Vertical, 0);
        before_mail.append(&imp.bar_before_mail);
        imp.pages.set_vexpand(true);
        before_mail.append(&imp.pages);
        imp.stage.set_child(Some(&before_mail));
        imp.toast.overlay().set_child(Some(&imp.stage));
        self.set_content(Some(imp.toast.overlay()));

        self.drop_focus_that_leaves();
        self.keep_cursor_ring_where_the_keyboard_is();
        self.connect_is_active_notify(|window| window.focus_changed(window.is_active()));
        self.connect_close_request(|window| {
            window.save_geometry();
            glib::Propagation::Proceed
        });
        imp.pages.connect_visible_child_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.place_reading()
        ));

        // Capture, not bubble: a single-key binding has to be seen before the focused widget
        // consumes it, and whether it should is the resolver's decision.
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
        let undo = gio::SimpleAction::new("undo", None);
        undo.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.act(CommandId::Undo)
        ));
        self.add_action(&undo);
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

    /// GTK leaves the focus where it was when its widget is removed: the key then starts at a
    /// widget with no parents and never reaches the window, so `j`, `z` and `Esc` do nothing until
    /// a click. When the widget holding the keyboard leaves, the window takes it.
    fn drop_focus_that_leaves(&self) {
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

    /// GTK runs a key press from the focus up to the innermost dialog and stops there, so the
    /// window's controller never sees a key while a dialog is up.
    /// Each dialog gets a controller that asks the window, the composer's too: its host's
    /// `handle_key` takes nothing.
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
            .version(crate::app::version_line(
                env!("CARGO_PKG_VERSION"),
                crate::app::COMMIT,
            ))
            .license_type(gtk::License::MitX11)
            .build();
        about.present(Some(self));
    }

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
            pane.feed()
                .input(postio_focus::Input::Keymap(keymap.clone()));
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
        crate::a11y::teach_shortcuts(self);
        if let Some(open) = self.key_map() {
            open.force_close();
            self.show_key_map();
        }
    }

    pub fn keymap(&self) -> Keymap {
        self.imp().keymap.borrow().clone()
    }

    /// Quit is all there is to do before there is an inbox; the rest are the page's own, its
    /// button's Return among them.
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

    /// The whole keyboard path in one call: a test presses a key through it without synthesizing a
    /// GDK event, which GTK4 gives no supported way to do.
    pub fn handle_key(&self, key: gdk::Key, state: gdk::ModifierType) -> glib::Propagation {
        // A picker at the row has the keyboard: its keys are the picker context's. The row menu has
        // it while up: Escape closes it, a verb's key runs that verb, and the arrows and Enter walk
        // and press its items as GTK does.
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
        // A Keyboard row in Settings waiting for its key takes every key, Escape included, until it
        // has one.
        if self.settings_takes_every_key() {
            return glib::Propagation::Proceed;
        }
        let Some(chord) = postio_widgets::keys::chord(key, state) else {
            return glib::Propagation::Proceed;
        };
        // The Files tab's arrows walk its grid two ways, where elsewhere in the results they are
        // the list's `Down`, `Up` and `Right` to open.
        if state.is_empty()
            && self.top_surface().is_none()
            && self
                .results()
                .is_some_and(|results| results.grid_arrow(key))
        {
            return glib::Propagation::Stop;
        }
        if self.imp().resolver.borrow().is_none() {
            return self.key_before_mail(&chord);
        }
        let typing = self.is_typing();
        let outcome = {
            let context = self.key_context();
            let mut resolver = self.imp().resolver.borrow_mut();
            let Some(resolver) = resolver.as_mut() else {
                return glib::Propagation::Proceed;
            };
            resolver.press(&chord, context, typing, std::time::Instant::now())
        };
        let command = match &outcome {
            Outcome::Command(id) => id.parse::<CommandId>().ok(),
            _ => None,
        };
        // The surfaces over the list are the controller's table (`postio_focus::Route`): what it
        // says is its own, it runs; what it gives the toolkit, the widget under the keyboard takes.
        let top = self.top_surface();
        if top == Some(SurfaceKind::Composer)
            && let Some(compose) = self.compose()
        {
            return match outcome {
                Outcome::Command(id) => match id.parse::<CommandId>() {
                    Ok(
                        id @ (CommandId::ReplyToLatest
                        | CommandId::ShowNewer
                        | CommandId::ExpandQuoted
                        | CommandId::ForwardWhat1
                        | CommandId::ForwardWhat2
                        | CommandId::ForwardWhat3),
                    ) => {
                        self.run_in_controller(id);
                        glib::Propagation::Stop
                    }
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
        // The arrows and paging keys read the message, though the list's `Down` and `Up` are the
        // same chords.
        if top == Some(SurfaceKind::Message) && self.scroll_reading(key, state) {
            return glib::Propagation::Stop;
        }
        // Reading and writing a file are the window's whatever surface is on top: the controller
        // has no file to give.
        if let Some(command @ (CommandId::ContactsImport | CommandId::ContactsExport)) = command {
            self.act(command);
            return glib::Propagation::Stop;
        }
        // Filtered's rows are walked, selected and opened by the controller's Filtered surface,
        // which holds its own cursor: the inbox's list behind it is not what `x` or `j` mean here.
        if let Some(command) = command
            && self.filtered().is_some()
            && filtered_owns(command)
        {
            self.run_in_controller(command);
            return glib::Propagation::Stop;
        }
        if let Some(command) = command {
            match self.route(command) {
                Route::Controller => {
                    self.run_in_controller(command);
                    return glib::Propagation::Stop;
                }
                Route::Toolkit => return glib::Propagation::Proceed,
                Route::Surface(_) | Route::List => {}
                _ => {}
            }
        }
        match top {
            Some(SurfaceKind::Message) if self.visible_dialog().is_some() => {
                // What `reading_key` has no arm for goes to the window, so a command bound in the
                // open message is never dropped.
                let reached = self.reading_key(outcome.clone());
                if reached == glib::Propagation::Proceed
                    && let Some(command) = command
                    // `Return` on the message already open is the message's
                    // own (it scrolls), not the list's open.
                    && command != CommandId::OpenMessage
                {
                    self.act(command);
                    return glib::Propagation::Stop;
                }
                return reached;
            }
            // A message open beside the list has the open message's keys; the list keeps the rest,
            // `/` and `x` among them.
            Some(SurfaceKind::Message) => {
                if let Some(command) = command
                    && self.reading_key(outcome.clone()) == glib::Propagation::Proceed
                {
                    self.act(command);
                }
                return glib::Propagation::Stop;
            }
            Some(SurfaceKind::Digest) => return self.digest_key(outcome),
            Some(SurfaceKind::Capture) => return self.capture_key(outcome),
            Some(SurfaceKind::Settings) => return self.settings_key(outcome),
            // A dialog over the window has the keyboard: only the keys that close it are the
            // window's, the rest go on to the dialog rather than moving the list underneath.
            Some(SurfaceKind::KeyMap | SurfaceKind::Dialog) => {
                return glib::Propagation::Proceed;
            }
            _ => {}
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

    /// Only the top dialog is known, so a dialog under it is told again when it is the top.
    fn surfaces_now(&self) -> Vec<SurfaceKind> {
        let mut now = Vec::new();
        if self.filtered().is_some() {
            now.push(SurfaceKind::Filtered);
        }
        if self.bar().is_some_and(|bar| bar.is_open()) {
            now.push(SurfaceKind::Bar);
        }
        if self.reading_beside() {
            now.push(SurfaceKind::Message);
        }
        let over_message = self.reading().is_some_and(|reading| reading.is_open());
        if over_message {
            if !now.contains(&SurfaceKind::Message) {
                now.push(SurfaceKind::Message);
            }
            if self
                .imp()
                .thread_menu
                .borrow()
                .as_ref()
                .is_some_and(|menu| menu.is_open())
            {
                now.push(SurfaceKind::ThreadMenu);
            }
            if self
                .imp()
                .outline
                .borrow()
                .as_ref()
                .is_some_and(|outline| outline.is_open())
            {
                now.push(SurfaceKind::Outline);
            }
        }
        if self.compose().is_some_and(|compose| compose.is_showing()) {
            // On Linux the composer stacks over the open message, still open under it: told closed,
            // a hit's message would send the controller back to the search it came from.
            if !self.reading_beside() && self.reading().is_some_and(|reading| reading.is_open()) {
                now.push(SurfaceKind::Message);
            }
            now.push(SurfaceKind::Composer);
        } else if let Some(dialog) = self.visible_dialog() {
            let name = dialog.widget_name();
            now.push(if name == crate::open::DIALOG_NAME {
                SurfaceKind::Message
            } else if name == crate::digest::DIALOG_NAME {
                SurfaceKind::Digest
            } else if name == crate::capture::DIALOG_NAME {
                SurfaceKind::Capture
            } else if name == crate::settings::DIALOG_NAME {
                SurfaceKind::Settings
            } else if name == KEY_MAP {
                SurfaceKind::KeyMap
            } else {
                SurfaceKind::Dialog
            });
        }
        now
    }

    pub(crate) fn sync_surfaces(&self) {
        let Some(pane) = self.pane() else {
            return;
        };
        let now = self.surfaces_now();
        let before = self.imp().surfaces.replace(now.clone());
        for kind in before.iter().rev().filter(|kind| !now.contains(kind)) {
            pane.feed().input(postio_focus::Input::SurfaceClosed(*kind));
        }
        for kind in now.iter().filter(|kind| !before.contains(kind)) {
            pane.feed().input(postio_focus::Input::SurfaceOpened(*kind));
        }
        let shown = self
            .reading()
            .filter(|reading| reading.is_open())
            .and_then(|reading| reading.shown());
        if self.imp().told_shown.replace(shown) != shown {
            pane.feed().input(postio_focus::Input::Shown(shown));
        }
    }

    fn top_surface(&self) -> Option<SurfaceKind> {
        self.sync_surfaces();
        self.pane().and_then(|pane| pane.feed().top_surface())
    }

    fn route(&self, command: CommandId) -> Route {
        self.pane()
            .map_or(Route::List, |pane| pane.feed().route(command))
    }

    /// The open message's More menu and find are said first: Back closes them before it closes the
    /// message.
    fn run_in_controller(&self, command: CommandId) {
        let Some(pane) = self.pane() else {
            return;
        };
        if self.top_surface() == Some(SurfaceKind::Message)
            && let Some(reading) = self.reading()
        {
            pane.feed().input(postio_focus::Input::ReaderState {
                more_open: reading.more_open(),
                finding: reading.reader().finding(),
            });
        }
        pane.feed().command(command);
    }

    fn close_surface(&self, kind: SurfaceKind) {
        match kind {
            SurfaceKind::Message => {
                if let Some(reading) = self.reading() {
                    reading.close();
                }
            }
            SurfaceKind::KeyMap | SurfaceKind::Dialog => {
                if let Some(dialog) = self.visible_dialog() {
                    dialog.close();
                }
            }
            SurfaceKind::Settings => {
                if let Some(settings) = self.settings() {
                    settings.close();
                }
            }
            SurfaceKind::Capture => {
                if let Some(sheet) = self.capture() {
                    sheet.close();
                }
            }
            SurfaceKind::Bar => {
                if let Some(bar) = self.bar() {
                    bar.close();
                }
            }
            SurfaceKind::Picker => {
                if let Some(driven) = self.imp().driven_picker.borrow().as_ref() {
                    driven.close();
                }
            }
            SurfaceKind::Filtered => self.hide_filtered(),
            SurfaceKind::Contacts => self.leave_contacts(),
            SurfaceKind::ContactPopover => self.close_contact_popover(),
            SurfaceKind::ThreadMenu => {
                if let Some(menu) = self.imp().thread_menu.borrow().as_ref() {
                    menu.close();
                }
            }
            SurfaceKind::Outline => {
                if let Some(outline) = self.imp().outline.borrow().as_ref() {
                    outline.close();
                }
            }
            _ => {}
        }
    }

    /// Of that message and not the one the cursor is on.
    fn begin_answer(&self, kind: postio_focus::ComposerKind, message: Option<MessageId>) {
        use postio_focus::ComposerKind;
        let Some(compose) = self.compose() else {
            return;
        };
        self.imp().compose_message.set(message);
        // The controller put the composer on its stack itself: saying it opened again would begin a
        // second composition and forget the chooser and the bar it is about to draw.
        {
            let mut surfaces = self.imp().surfaces.borrow_mut();
            if !surfaces.contains(&SurfaceKind::Composer) {
                surfaces.push(SurfaceKind::Composer);
            }
        }
        compose.thread.clear();
        self.imp().replying_to.set(None);
        compose.dispatch(match kind {
            ComposerKind::ReplyAll => CommandId::ReplyAll,
            ComposerKind::Forward { .. } => CommandId::Forward,
            _ => CommandId::Reply,
        });
        self.note_replying_to(
            matches!(kind, ComposerKind::Reply | ComposerKind::ReplyAll)
                .then_some(message)
                .flatten(),
        );
    }

    fn note_replying_to(&self, message: Option<MessageId>) {
        let Some(message) = message else {
            return;
        };
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            use postio_model::listing::MailStore as _;
            // POSTIO-GLIB-SAFE: client calls are oneshot receives.
            let Ok(rows) = client.message_rows(vec![message]).await else {
                return;
            };
            let Some(thread) = rows.into_iter().next().and_then(|row| row.thread) else {
                return;
            };
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            let Ok(Some(view)) = client.thread(thread).await else {
                return;
            };
            let Some(at) = view.members.iter().position(|member| member.id == message) else {
                return;
            };
            if let Some(window) = weak.upgrade() {
                window.imp().replying_to.set(Some(at as u32 + 1));
            }
        });
    }

    fn show_newer(&self, ids: Option<&[MessageId]>) {
        let Some(compose) = self.compose() else {
            return;
        };
        let (Some(ids), Some(client)) = (ids, self.imp().client.borrow().clone()) else {
            compose.thread.show_newer(&[]);
            return;
        };
        let (ids, weak) = (ids.to_vec(), self.downgrade());
        glib::spawn_future_local(async move {
            use postio_model::listing::MailStore as _;
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            let Ok(rows) = client.message_rows(ids).await else {
                return;
            };
            let lines: Vec<(String, String)> = rows
                .iter()
                .map(|row| {
                    (
                        format!(
                            "{} \u{b7} {}",
                            row.from.as_ref().map_or("", |from| from.display()),
                            postio_ui::row::timestamp(row.received_at, postio_ui::clock::now())
                        ),
                        row.preview.clone().unwrap_or_default(),
                    )
                })
                .collect();
            if let Some(compose) = weak.upgrade().and_then(|window| window.compose()) {
                compose.thread.show_newer(&lines);
            }
        });
    }

    fn show_thread_menu(&self, view: &postio_ui::thread_outline::ThreadMenuView) {
        let Some(reading) = self.reading() else {
            return;
        };
        let menu = {
            let held = self.imp().thread_menu.borrow().clone();
            held.unwrap_or_else(|| {
                let menu = crate::thread_popovers::ThreadMenu::new(&reading.thread_button());
                menu.connect_pick(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |id| {
                        if let Some(pane) = window.pane() {
                            pane.feed().input(postio_focus::Input::ThreadMenuPick(id));
                        }
                    }
                ));
                menu.connect_closed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.thread_surface_closed(SurfaceKind::ThreadMenu)
                ));
                self.imp().thread_menu.replace(Some(Rc::clone(&menu)));
                menu
            })
        };
        menu.show(view);
        self.imp()
            .surfaces
            .borrow_mut()
            .push(SurfaceKind::ThreadMenu);
    }

    fn show_outline(&self, view: &postio_ui::thread_outline::OutlineView) {
        let (parent, rect): (gtk::Widget, Option<gdk::Rectangle>) =
            match self.reading().filter(|reading| reading.is_open()) {
                Some(reading) => (reading.thread_button().upcast(), None),
                None => match self.at_the_row() {
                    Some((anchor, rect)) => (anchor, Some(rect)),
                    None => return,
                },
            };
        let held = self.imp().outline.borrow().clone();
        let outline = match held {
            Some(held) if held.parent().as_ref() == Some(&parent) => held,
            stale => {
                if let Some(stale) = stale {
                    stale.discard();
                }
                let outline = crate::thread_popovers::OutlinePopover::new(&parent);
                outline.connect_input(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |input| {
                        if let Some(pane) = window.pane() {
                            pane.feed().input(postio_focus::Input::Outline(input));
                        }
                    }
                ));
                outline.connect_closed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.thread_surface_closed(SurfaceKind::Outline)
                ));
                self.imp().outline.replace(Some(Rc::clone(&outline)));
                outline
            }
        };
        outline.point_at(rect.as_ref());
        let was_open = outline.is_open();
        outline.show(view);
        if !was_open {
            self.imp().surfaces.borrow_mut().push(SurfaceKind::Outline);
        }
    }

    fn thread_surface_closed(&self, kind: SurfaceKind) {
        let known = {
            let mut surfaces = self.imp().surfaces.borrow_mut();
            let before = surfaces.len();
            surfaces.retain(|held| *held != kind);
            before != surfaces.len()
        };
        if known && let Some(pane) = self.pane() {
            pane.feed().input(postio_focus::Input::SurfaceClosed(kind));
        }
        // GTK hands the focus back to the button the popover hung from after this signal closes it:
        // the message takes it once the loop is idle, and again a beat later if the toolkit was
        // slower.
        if known && kind == SurfaceKind::Outline {
            for delay in [0u64, 60] {
                glib::timeout_add_local_once(
                    std::time::Duration::from_millis(delay),
                    glib::clone!(
                        #[weak(rename_to = window)]
                        self,
                        move || {
                            if let Some(reading) =
                                window.reading().filter(|reading| reading.is_open())
                            {
                                reading.focus_message();
                            }
                        }
                    ),
                );
            }
        }
    }

    fn show_reader_verb(&self, verb: postio_focus::ReaderVerb) {
        use postio_focus::ReaderVerb;
        let Some(reading) = self.reading() else {
            return;
        };
        match verb {
            ReaderVerb::CloseMore => reading.close_more(),
            ReaderVerb::CloseFind => reading.reader().close_find(),
            ReaderVerb::FindInMessage => reading.reader().find_in_message(),
            ReaderVerb::FindNext => reading.reader().find_step(true),
            ReaderVerb::FindPrevious => reading.reader().find_step(false),
            ReaderVerb::NextMatch | ReaderVerb::PrevMatch => {
                let forward = matches!(verb, ReaderVerb::NextMatch);
                if !reading.reader().view().find_walk(forward) {
                    self.imp().match_from_end.set(!forward);
                    if let Some(pane) = self.pane() {
                        pane.feed()
                            .input(postio_focus::Input::MatchEdge { forward });
                    }
                }
            }
            ReaderVerb::SwitchTreatment => {
                reading.reader().switch_treatment();
            }
            ReaderVerb::ViewSource => self.view_source(),
            ReaderVerb::ShowMore => reading.show_more(),
            _ => {}
        }
    }

    /// False too while a field is being typed in (the bar's, the dropdown's, a popover's), where
    /// one accent ring is the field's (#2081).
    fn list_holds_keyboard(&self) -> bool {
        self.visible_dialog().is_none()
            && !self
                .compose()
                .is_some_and(|compose| compose.is_showing() || compose.composer().is_detached())
            && !self.is_typing()
    }

    fn refresh_cursor_ring(&self) {
        if self.list_holds_keyboard() {
            self.remove_css_class(KEYBOARD_AWAY);
        } else {
            self.add_css_class(KEYBOARD_AWAY);
        }
    }

    /// The composer's showing settles a moment after the focus does, so the class is read again
    /// once the main loop is idle.
    fn keep_cursor_ring_where_the_keyboard_is(&self) {
        let settle = |window: &Self| {
            window.refresh_cursor_ring();
            glib::idle_add_local_once(glib::clone!(
                #[weak]
                window,
                move || window.refresh_cursor_ring()
            ));
        };
        self.connect_focus_widget_notify(move |window| settle(window));
        self.connect_visible_dialog_notify(move |window| settle(window));
    }

    fn is_typing(&self) -> bool {
        gtk::prelude::GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
            || self.composer_body_has_keyboard()
    }

    /// Settings' account list is the one context only this window can see.
    pub fn key_context(&self) -> KeyContext {
        self.sync_surfaces();
        let context = self
            .pane()
            .map_or(KeyContext::List, |pane| pane.feed().key_context());
        if context != KeyContext::Composer && self.settings_on_an_account() {
            return KeyContext::Accounts;
        }
        if context == KeyContext::Composer
            && self
                .compose()
                .is_some_and(|compose| compose.thread.chooser_has_focus())
        {
            return KeyContext::ForwardChooser;
        }
        context
    }

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

    pub fn capture(&self) -> Option<Rc<crate::capture::CaptureSheet>> {
        self.imp().capture.borrow().clone()
    }

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
            sheet.dialog().connect_closed(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_| window.sync_surfaces()
            ));
            imp.capture.replace(Some(Rc::clone(&sheet)));
            sheet
        });
        sheet.open(self, source, mode);
        self.sync_surfaces();
        crate::a11y::teach_shortcuts(sheet.dialog());
        crate::motion::keep_to_budget(sheet.dialog());
    }

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
            // Back, find, More, the treatment, the source and the thread's steps are the
            // controller's. A message the window showed itself
            // is no list row of the controller's, so the cursor's row is asked for after.
            Ok(
                id @ (CommandId::NextMessage
                | CommandId::PrevMessage
                | CommandId::FirstMessage
                | CommandId::LastMessage),
            ) => {
                self.cursor_command(id);
                if self
                    .pane()
                    .is_some_and(|pane| pane.feed().reading().is_none())
                {
                    self.open_message();
                }
            }

            Ok(
                id @ (CommandId::ZoomIn
                | CommandId::ZoomOut
                | CommandId::ZoomReset
                | CommandId::ShowImages
                | CommandId::AlwaysShowImages
                | CommandId::Unsubscribe),
            ) => self.act(id),
            Ok(CommandId::DismissMarker) => self.dismiss_marker(),
            Ok(CommandId::ToggleReadingPane) => self.toggle_reading_pane(),
            Ok(CommandId::OpenMessage) if self.offered_on_open_draft(CommandId::OpenMessage) => {
                self.act(CommandId::OpenMessage);
            }
            Ok(CommandId::OpenMessage) if reading.in_pane() => self.open_message(),
            Ok(CommandId::OpenAttachmentOrLink) => self.offer_choices(None),
            Ok(id @ (CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent)) => {
                self.act(id);
            }
            // The read clock stops: a message kept unread stays unread while it is open.
            Ok(CommandId::ToggleRead) => {
                reading.cancel_dwell();
                self.act(CommandId::ToggleRead);
            }
            Ok(
                id @ (CommandId::Reply
                | CommandId::ReplyAll
                | CommandId::Forward
                | CommandId::Compose
                | CommandId::AcceptInvite
                | CommandId::DeclineInvite),
            ) => self.act(id),
            // The message on screen going away is the controller's: it aims at the message alone
            // and steps on to the row that takes its place once the list lets it go.
            Ok(
                id @ (CommandId::AddLabel
                | CommandId::Move
                | CommandId::Snooze
                | CommandId::RemindIfNoReply
                | CommandId::CaptureTask
                | CommandId::CaptureNote
                | CommandId::Undo),
            ) => self.act(id),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }

    /// A `WebView`, which no type test says is being typed in (#602).
    pub(crate) fn composer_body_has_keyboard(&self) -> bool {
        self.compose().is_some_and(|compose| {
            compose.composer().focused_field() == Some(postio_widgets::composer::Field::Body)
        })
    }

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

    fn compose(&self) -> Option<Rc<crate::compose::Compose>> {
        self.imp().compose.borrow().clone()
    }

    fn mount_compose(&self, client: &Client, account: AccountId) {
        if self.imp().compose.borrow().is_some() {
            return;
        }
        let window = self.downgrade();
        let current: crate::compose::Current = Rc::new(move || {
            let window = window.upgrade()?;
            window
                .imp()
                .compose_message
                .get()
                .or_else(|| window.aimed_message())
        });
        let compose = crate::compose::Compose::new(self, client, account, current);
        compose.thread.connect_card(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |form| {
                if let Some(pane) = window.pane() {
                    pane.feed()
                        .input(postio_focus::Input::ForwardFormClicked(form));
                }
            }
        ));
        compose.thread.connect_action(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |at| window.run_in_controller(if at == 0 {
                CommandId::ShowNewer
            } else {
                CommandId::ReplyToLatest
            })
        ));
        compose.thread.connect_unfold(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.run_in_controller(CommandId::ExpandQuoted)
        ));
        if self.imp().warm.get() {
            compose.warm();
        }
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
            if let Some(pane) = window.pane() {
                pane.set_identities(
                    accounts
                        .iter()
                        .flat_map(|account| {
                            std::iter::once(account.address.clone())
                                .chain(account.identities.iter().map(|id| id.address.clone()))
                        })
                        .collect(),
                );
            }
            if let Some(pane) = window.pane() {
                pane.feed().input(postio_focus::Input::Accounts(
                    enabled.iter().map(|account| account.id).collect(),
                ));
            }
            if let Some(account) = enabled.first() {
                window.mount_compose(&client, account.id);
            }
            if let Some(reading) = window.reading() {
                reading.set_accounts(
                    enabled
                        .iter()
                        .map(|account| (account.id, account.address.address.clone()))
                        .collect(),
                );
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
            if enabled.is_empty() {
                window.open_add_account();
                return;
            }
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

    /// With no account at all this is the first run: once saved, the form asks how much history to
    /// sync (#876) before the connection comes up.
    /// A second call while the form is open reuses it rather than stacking a second wizard.
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

    /// Only the add-account form must not shut the window's close button out: it covers the window
    /// on a first run, where closing the app is the one other thing a person can want, and a modal
    /// dialog gives its scrim the click.
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
        // The app is closing, so the form goes first.
        if let Some(form) = imp.adding_account.take() {
            form.force_close();
        }
        self.act(CommandId::Quit);
        true
    }

    pub fn add_account_dialog(&self) -> Option<adw::Dialog> {
        self.imp().adding_account.borrow().clone()
    }

    fn offer_add_account_for_compose(&self) {
        let window = self.downgrade();
        self.imp().toast.show_prompt(
            "There's no account to write from yet.",
            "Add account",
            postio_ui::hints::key(&self.keymap(), CommandId::AddAccount),
            move || {
                if let Some(window) = window.upgrade() {
                    window.open_add_account();
                }
            },
        );
        self.follow_toast();
    }

    pub fn open_draft(&self, message: MessageId) {
        if let Some(compose) = self.compose() {
            compose.open_draft(message);
        }
    }

    fn expand_quoted_cap(&self) -> Option<String> {
        postio_ui::hints::key(&self.keymap(), CommandId::ExpandQuoted)
    }

    pub fn open_thread_draft(&self, draft: postio_model::DraftId) {
        if let Some(compose) = self.compose() {
            compose.open_thread_draft(draft);
        }
    }

    pub fn warm_composer(&self) {
        self.imp().warm.set(true);
        if let Some(compose) = self.compose() {
            compose.warm();
        }
    }

    pub fn act(&self, id: CommandId) {
        postio_widgets::jank::note_action(id);
        // A search's results are the controller's to drive, but not a message open over them: its
        // reply is the window's composer.
        if self.results().is_some()
            && self.top_surface().is_none()
            && let Some(pane) = self.pane()
            && pane.feed().answers(id)
        {
            return self.run_in_controller(id);
        }
        if postio_core::registry::get(id).contexts
            == postio_core::ContextSet::of(postio_core::Context::Results)
        {
            return self.run_in_controller(id);
        }
        if self.rules().is_some() && self.rules_act(id) {
            return;
        }
        if self.filtered().is_some() && filtered_owns(id) {
            return self.run_in_controller(id);
        }
        match id {
            CommandId::NextMessage
            | CommandId::PrevMessage
            | CommandId::FirstMessage
            | CommandId::LastMessage
            | CommandId::ToggleSelection
            | CommandId::ExtendSelectionDown
            | CommandId::ExtendSelectionUp
            | CommandId::SelectAll
            | CommandId::ExpandThread
            | CommandId::CollapseThread
            | CommandId::MuteThread
            | CommandId::ThreadOutline
            | CommandId::ThreadMenu
            | CommandId::SplitThread
            | CommandId::ToggleHasAction => self.cursor_command(id),
            CommandId::Back => {
                if let Some(places) = self.places().filter(|places| places.is_open()) {
                    places.close();
                } else if self.bar().is_some_and(|bar| bar.is_open()) || self.filtered().is_some() {
                    self.run_in_controller(CommandId::Back);
                } else {
                    self.cursor_command(CommandId::Back);
                }
            }
            CommandId::SavedSearch1
            | CommandId::SavedSearch2
            | CommandId::SavedSearch3
            | CommandId::SavedSearch4
            | CommandId::Search
            | CommandId::CommandPalette
            | CommandId::BackToWords
            | CommandId::SaveSearch
            | CommandId::ToggleResultOrder
            | CommandId::ShowAllResults
            | CommandId::ForgetRecent
            | CommandId::ExcludeSuggestion
            | CommandId::HistoryBack
            | CommandId::HistoryForward
            | CommandId::ResultsConversations
            | CommandId::ResultsFiles
            | CommandId::ResultsPeople
            | CommandId::GoToFolders
            | CommandId::GoToInbox
            | CommandId::GoToFiltered
            | CommandId::GoToDrafts
            | CommandId::GoToSent
            | CommandId::GoToArchive
            | CommandId::GoToSnoozed
            | CommandId::GoToFlagged
            | CommandId::GoToJunk
            | CommandId::GoToTrash
            | CommandId::GoToOutbox => self.run_in_controller(id),
            // Undo is the last action this window took, however long ago the toast went: the host
            // keeps the stack.
            // A toast with an Undo of its own (a send, queued) is the last thing said, so Undo
            // takes that back first (#1752).
            CommandId::Undo if self.imp().toast.activate_undo() => {}
            CommandId::Archive
            | CommandId::Delete
            | CommandId::ToggleRead
            | CommandId::Flag
            | CommandId::Unsnooze
            | CommandId::Undo
            | CommandId::Refresh => self.cursor_command(id),
            CommandId::Quit | CommandId::CloseWindow => self.close(),
            CommandId::UpdateCredential => self.update_credential(),
            CommandId::CheatSheet => self.show_key_map(),
            CommandId::Settings => self.toggle_settings(),
            CommandId::EditConfig => self.edit_config(),
            CommandId::ToggleAccountEnabled
            | CommandId::RemoveAccount
            | CommandId::RebuildAccountIndex
            | CommandId::SetDefaultAccount
            | CommandId::MapMailboxRole => self.account_verb(id),
            // The composer takes a waiting send off the queue before anything is edited.
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
            CommandId::NextScope => self.next_scope(),
            CommandId::ViewSource => self.view_source(),
            CommandId::OpenContacts
            | CommandId::ContactsSuggestions
            | CommandId::ContactPopover => {
                self.run_in_controller(id);
            }
            CommandId::ContactOpenCard
            | CommandId::ContactAdd
            | CommandId::ContactIgnore
            | CommandId::ContactArrivalPrev
            | CommandId::ContactArrivalNext
            | CommandId::ContactSetArrival
            | CommandId::ContactShowMail
            | CommandId::ContactCompose
            | CommandId::ContactsFilter
            | CommandId::ContactsTabPrevious
            | CommandId::ContactsTabNext
            | CommandId::ContactsToggleEveryone
            | CommandId::ContactJoin
            | CommandId::ContactAddAddress
            | CommandId::ContactDetachAddress
            | CommandId::ContactSetPreferred
            | CommandId::ContactNew
            | CommandId::ContactEdit
            | CommandId::ContactSave
            | CommandId::ContactDelete
            | CommandId::ContactRestore
            | CommandId::ContactsToggleDeleted
            | CommandId::SuggestionDismiss
            | CommandId::ContactGroupNew
            | CommandId::ContactGroupRename
            | CommandId::ContactGroupAdd
            | CommandId::ContactGroupRemove => self.run_in_controller(id),
            CommandId::ContactsImport => self.contacts_import(),
            CommandId::ContactsExport => self.contacts_export(),
            CommandId::SweepInbox => self.ask_sweep(),
            CommandId::ClearListMail => {
                if !self.imp().toast.take_offer() {
                    self.say(postio_ui::unsubscribe::NO_OFFER);
                }
            }
            CommandId::DismissMarker => self.dismiss_marker(),
            CommandId::MoreActions => {
                if let Some(reading) = self.reading() {
                    reading.show_more();
                }
            }
            CommandId::DigestRule => self.new_digest_rule(),
            CommandId::ToggleReadingPane => self.toggle_reading_pane(),
            CommandId::GoToDigestRules => self.show_rules(),
            CommandId::Snooze => self.open_when(When::Snooze),
            CommandId::RemindIfNoReply => self.open_when(When::Remind),
            CommandId::AddLabel => self.open_labels(),
            CommandId::Move => self.open_moves(),
            CommandId::AcceptInvite | CommandId::DeclineInvite => {
                if let Some(message) = self.aimed_message() {
                    self.answer(message, id);
                }
            }
            CommandId::CancelSend | CommandId::RetrySend | CommandId::MarkSent => {
                // One the open message offers moves the message out of the list it was opened from,
                // so it closes; the cursor stays, and the list's next row is not opened in its
                // place.
                let offered = self.offered_on_open_draft(id);
                self.settle_send(id);
                if offered && let Some(reading) = self.reading() {
                    reading.close();
                }
            }
            CommandId::CaptureTask => self.open_capture(crate::capture::Mode::Task),
            CommandId::CaptureNote => self.open_capture(crate::capture::Mode::Note),
            CommandId::Compose => match self.compose() {
                Some(compose) => compose.dispatch(id),
                None => self.offer_add_account_for_compose(),
            },
            CommandId::Reply | CommandId::ReplyAll
                if postio_ui::focus_target::refuses_reply(self.aimed_send_state()) =>
            {
                self.imp()
                    .toast
                    .show_notice(postio_ui::focus_target::NO_REPLY_TO_OUTGOING);
                self.follow_toast();
            }
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward
                if self
                    .reading()
                    .filter(|reading| reading.is_open())
                    .is_some_and(|reading| {
                        reading.foreign_draft()
                            && !postio_ui::focus_dialog::received_offers(id, true)
                    }) =>
            {
                self.imp()
                    .toast
                    .show_notice(postio_ui::focus_target::NO_REPLY_TO_OUTGOING);
                self.follow_toast();
            }
            CommandId::Reply
            | CommandId::ReplyAll
            | CommandId::Forward
            | CommandId::ReplyToLatest
            | CommandId::NextInConversation
            | CommandId::PrevInConversation
            | CommandId::FirstInConversation
            | CommandId::LatestInConversation
            | CommandId::FirstUnreadInConversation
            | CommandId::OutlineFirstUnread
            | CommandId::MoveThisMessage
            | CommandId::DeleteThisMessage
            | CommandId::ForwardWhat1
            | CommandId::ForwardWhat2
            | CommandId::ForwardWhat3
            | CommandId::ShowNewer
            | CommandId::ExpandQuoted => self.run_in_controller(id),
            CommandId::AddAccount => self.open_add_account(),
            CommandId::ZoomIn | CommandId::ZoomOut | CommandId::ZoomReset => {
                if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
                    match id {
                        CommandId::ZoomIn => reading.reader().zoom_in(),
                        CommandId::ZoomOut => reading.reader().zoom_out(),
                        _ => reading.reader().zoom_reset(),
                    }
                }
            }
            // `M` leaves the list on any message the shared rule allows: the key is not tied to the
            // band, which shows only for list mail.
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

    pub fn take_unanswered(&self) -> Vec<CommandId> {
        std::mem::take(&mut *self.imp().unanswered.borrow_mut())
    }

    /// GTK's focus on a row other than the cursor's would be a second ring, and on the top bar's
    /// Compose button Space or Return would press a control instead of acting on the row.
    pub fn focus_list(&self) {
        if let Some(results) = self.results() {
            results.focus();
            return;
        }
        if self.take_keyboard_to_the_empty_page() {
            return;
        }
        // Given to the view alone, GTK puts the keyboard on whichever row it last had: a row other
        // than the cursor's is a second place the keyboard seems to be.
        if let Some(pane) = self.pane() {
            gtk::prelude::GtkWindowExt::set_focus(self, Some(pane.view()));
            pane.focus_cursor();
        }
    }

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

    fn focus_cursor_soon(&self) {
        glib::idle_add_local_once(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || {
                if window.keyboard_is_free() {
                    window.focus_cursor();
                }
            }
        ));
    }

    pub fn focus_cursor(&self) {
        if let Some(results) = self.results() {
            results.focus();
        } else if !self.take_keyboard_to_the_empty_page()
            && let Some(pane) = self.pane()
        {
            pane.focus_cursor();
        }
    }

    fn empty_page_showing(&self) -> bool {
        self.imp()
            .list_or_empty
            .borrow()
            .as_ref()
            .is_some_and(|stack| stack.visible_child_name().as_deref() == Some(EMPTY))
    }

    fn take_keyboard_to_the_empty_page(&self) -> bool {
        self.empty_page_showing()
            && self
                .imp()
                .empty
                .borrow()
                .as_ref()
                .is_some_and(|page| page.take_the_keyboard())
    }

    fn keyboard_is_free(&self) -> bool {
        self.visible_dialog().is_none()
            && !self.is_typing()
            && self.bar().is_none_or(|bar| !bar.is_open())
            && self.places().is_none_or(|places| !places.is_open())
    }

    fn point_cursor(&self, position: u32) {
        if let Some(pane) = self.pane() {
            pane.feed().input(postio_focus::Input::Point(position));
        }
    }

    fn cursor_command(&self, id: CommandId) {
        self.sync_surfaces();
        if let Some(pane) = self.pane() {
            pane.feed().command(id);
        }
    }

    fn show_cursor(&self, position: u32, to_top: bool) {
        let Some(pane) = self.pane() else {
            return;
        };
        // A click on a row gives GTK's focus to that row, and the cursor
        // moving on would leave it behind with a ring of its own.
        let on_a_row = gtk::prelude::GtkWindowExt::focus(self).is_some_and(|focus| {
            focus != *pane.view().upcast_ref::<gtk::Widget>() && focus.is_ancestor(pane.view())
        });
        if position >= pane.feed().shown().n_items() {
            return;
        }
        pane.cursor().set_selected(position);
        if on_a_row {
            // The list view's own focus lands on its first row, so
            // the keyboard follows the cursor to its row instead.
            pane.focus_cursor();
        } else {
            pane.view()
                .scroll_to(position, gtk::ListScrollFlags::NONE, None);
        }
        if to_top {
            pane.to_top();
        }
    }

    fn show_intent(&self, intent: &postio_focus::Intent) {
        use postio_focus::Intent;
        match intent {
            Intent::Cursor { position, to_top } => self.show_cursor(*position, *to_top),
            Intent::Selection { selection, summary } => {
                self.show_selection(selection, summary.as_deref());
            }
            Intent::SingleHeading(heading) => {
                if let Some(pane) = self.pane() {
                    pane.feed().list().set_single_heading(heading.clone());
                }
            }
            Intent::ListToTop => {
                if let Some(pane) = self.pane() {
                    pane.to_top();
                }
            }
            Intent::Footer(footer) if self.filtered().is_some() => {
                if let Some(view) = self.filtered() {
                    view.show_scope(footer);
                }
            }
            // A window opened over the results: its footer is drawn in the
            // results' (#2193), since the list's is not on screen.
            Intent::Footer(footer) if self.results().is_some() => {
                if let Some(results) = self.results() {
                    results.show_scope(footer);
                }
            }
            Intent::Footer(footer) => {
                if let Some(bar) = self.imp().scope_footer.borrow().as_ref() {
                    bar.set(footer);
                }
            }
            Intent::ThreadBar(bar) => {
                if let Some(reading) = self.reading() {
                    reading.set_thread_bar(bar);
                }
            }
            Intent::Kicker(kicker) => {
                if let Some(reading) = self.reading() {
                    reading.set_kicker(kicker);
                }
            }
            Intent::OlderBanner(banner) => {
                if let Some(reading) = self.reading() {
                    reading.set_older_banner(banner.as_ref());
                }
            }
            Intent::ReplyTo(text) => {
                if let Some(reading) = self.reading() {
                    reading.set_reply_to(text.as_deref());
                }
            }
            Intent::NewReply(banner) => {
                if let Some(reading) = self.reading() {
                    reading.set_new_reply(banner.as_ref());
                }
            }
            Intent::ThreadMenu(view) => self.show_thread_menu(view),
            Intent::Outline(view) => self.show_outline(view),
            Intent::OutlineFocusFilter => {
                if let Some(outline) = self.imp().outline.borrow().as_ref() {
                    outline.focus_filter();
                }
            }
            Intent::Counts(_) => self.show_counts(),
            Intent::Empty(page) => self.show_empty(page.as_ref()),
            Intent::Toast { text, kind } => self.show_toast(text, kind),
            Intent::Reader(verb) => self.show_reader_verb(*verb),
            Intent::OpenMessage {
                message,
                index,
                total,
                host: postio_focus::Host::Own,
            } => self.show_message(*message, *index, *total),
            // `j` onto a digest's row with a message open leaves the message up; the digest's
            // window opens with Enter. The controller's own copy of the digest is dropped.
            Intent::OpenDigest { .. } => {
                if let Some(pane) = self.pane() {
                    pane.feed()
                        .input(postio_focus::Input::SurfaceClosed(SurfaceKind::Digest));
                }
            }
            Intent::Composer {
                kind: postio_focus::ComposerKind::Draft,
                message: Some(message),
            } => self.open_draft(*message),
            Intent::Composer {
                kind: postio_focus::ComposerKind::ThreadDraft { draft },
                message: None,
            } => self.open_thread_draft(*draft),
            Intent::Composer {
                kind:
                    kind @ (postio_focus::ComposerKind::Reply
                    | postio_focus::ComposerKind::ReplyAll
                    | postio_focus::ComposerKind::Forward { .. }),
                message,
            } => self.begin_answer(*kind, *message),
            Intent::ForwardChooser(view) => {
                if let Some(compose) = self.compose() {
                    compose.thread.show_chooser(view);
                }
            }
            Intent::OlderReply(bar) => {
                if let Some(compose) = self.compose() {
                    compose.thread.show_older(bar.as_deref());
                }
            }
            Intent::NewerMessages(ids) => self.show_newer(ids.as_deref()),
            Intent::QuotedExpanded(expanded) => {
                if let Some(compose) = self.compose() {
                    compose.thread.set_quoted_expanded(*expanded);
                    compose.sync_forwarded(self.expand_quoted_cap().as_deref());
                }
            }
            Intent::ComposerDraftWanted(edit) => {
                if let (Some(compose), Some(pane)) = (self.compose(), self.pane()) {
                    pane.feed().input(postio_focus::Input::ComposerDraft(
                        *edit,
                        Box::new(compose.composer().draft()),
                    ));
                }
            }
            Intent::ComposerDraft(draft) => {
                if let Some(compose) = self.compose() {
                    let in_reply_to = draft.in_reply_to;
                    compose.composer().resume(draft.as_ref().clone());
                    compose.sync_forwarded(self.expand_quoted_cap().as_deref());
                    self.note_replying_to(in_reply_to);
                }
            }
            Intent::CloseSurface(kind) => self.close_surface(*kind),
            Intent::OpenPicker(view) if view.kind == postio_focus::PickerKind::Move => {
                self.open_driven_picker(view);
            }
            Intent::PickerRows(view) if view.kind == postio_focus::PickerKind::Move => {
                if let Some(driven) = self.imp().driven_picker.borrow().as_ref() {
                    driven.show_rows(view);
                }
            }
            Intent::Contacts(view) => self.show_contacts(view),
            Intent::ContactsRows { .. } => {
                if let Some(view) = self.contacts() {
                    view.show_rows();
                }
            }
            Intent::ContactsCursor(index) => {
                if let Some(view) = self.contacts() {
                    view.show_cursor(*index);
                }
            }
            Intent::ContactsCard(card) => {
                if let Some(view) = self.contacts() {
                    view.show_card(card.as_deref().cloned());
                }
            }
            Intent::OpenBar { text, select, .. } => {
                if let Some(bar) = self.bar() {
                    bar.open_on(text, *select);
                }
            }
            Intent::BarLines(view) => {
                if let Some(bar) = self.bar() {
                    bar.show(view);
                }
            }
            Intent::Dropdown(view) => {
                if let Some(bar) = self.bar() {
                    bar.show_dropdown(view);
                }
            }
            Intent::ShowResults(view) => self.show_results(view),
            Intent::ResultsPage { first, count } => {
                if let Some(results) = self.results() {
                    results.rows_changed(*first, *count);
                }
            }
            Intent::ResultsCursor(position) => {
                if let Some(results) = self.results() {
                    results.show_cursor(*position);
                }
            }
            Intent::ResultSplice(splice) => {
                if let Some(results) = self.results() {
                    results.show_splice(splice.as_deref());
                }
            }
            Intent::ResultChildFocus(child) => {
                if let Some(results) = self.results() {
                    results.show_child_focus(*child);
                }
            }
            Intent::MatchBanner(banner) => {
                if let Some(reading) = self.reading() {
                    reading.set_match_banner(banner.as_ref());
                    // As words, as the index found them: "rate limit headers" matched "rate-limit
                    // headers" (#2193).
                    let words = self.imp().search_words.borrow().clone();
                    let from_end = self.imp().match_from_end.replace(false);
                    let query = if banner.is_some() {
                        words
                    } else {
                        String::new()
                    };
                    reading
                        .reader()
                        .view()
                        .set_find_words_from(&query, from_end);
                }
            }
            Intent::Query(query) => {
                self.imp().search_words.replace(query.words.clone());
                if let Some(results) = self.results() {
                    results.show_query(query);
                }
            }
            Intent::LeaveResults => self.leave_results(),
            Intent::Popover(view) => {
                if let Some(results) = self.results() {
                    results.show_popover(view.as_deref());
                }
            }
            Intent::QuickLook(view) => {
                if let Some(results) = self.results() {
                    let commands = postio_ui::search_view::quick_look_commands(&self.keymap());
                    results.show_quick_look(view.as_deref(), &commands);
                }
            }
            Intent::FileCopy(copy) => self.file_copy(copy.as_ref()),
            Intent::Relaxations(view) => {
                if let Some(results) = self.results() {
                    results.show_relaxations(view.as_deref());
                }
            }
            Intent::Place { name } => self.show_place(name),
            Intent::OpenPlaces => self.open_places(),
            Intent::PlacesChanged => {
                if let Some(bar) = self.bar() {
                    bar.places_read();
                }
                if let Some(places) = self.places() {
                    places.redraw();
                }
                self.show_state();
            }
            Intent::ShowFiltered => self.show_filtered(),
            Intent::Filtered(frame) => {
                if let Some(view) = self.filtered() {
                    view.show(frame);
                }
            }
            Intent::FilteredFocus(position) => {
                if let Some(view) = self.filtered() {
                    view.show_focus(*position);
                }
            }
            Intent::FilteredSplice { anchor, children } => {
                if let Some(view) = self.filtered() {
                    view.show_splice(*anchor, children);
                }
            }
            Intent::FilteredUnsplice { anchor } => {
                if let Some(view) = self.filtered() {
                    view.show_unsplice(*anchor);
                }
            }
            Intent::FilteredSelection { selected, bar } => {
                if let Some(view) = self.filtered() {
                    view.show_selection(selected, bar.as_ref());
                }
            }
            Intent::SaveSearch(save) => self.save_search(save),
            Intent::SavePopover(view) => {
                if let Some(results) = self.results() {
                    results.show_save(view.as_deref());
                }
            }
            Intent::Run(id) => self.act(*id),
            Intent::ContactPopover(view) => self.show_contact_popover(view),
            Intent::ContactsSettings(view) => {
                if let Some(settings) = self.settings() {
                    settings.panel().show_suggestions(view);
                }
            }
            Intent::ContactEdit(edit) => {
                if let Some(view) = self.contacts() {
                    view.show_edit(edit.as_deref().cloned());
                }
            }
            Intent::ContactMerge(sheet) => {
                if let Some(view) = self.contacts() {
                    view.show_merge(sheet.as_deref().cloned());
                }
            }
            Intent::ContactsFocusFilter => {
                if let Some(view) = self.contacts() {
                    view.focus_filter();
                }
            }
            Intent::WriteTo(address) => {
                self.open_mailto(postio_model::mailto::Mailto {
                    to: vec![postio_model::EmailAddress::new(None::<String>, address)],
                    ..Default::default()
                });
            }
            _ => {}
        }
    }

    fn show_toast(&self, text: &str, kind: &postio_focus::ToastKind) {
        use postio_focus::ToastKind;
        let toast = &self.imp().toast;
        match kind {
            ToastKind::Completed {
                undoable,
                seconds: Some(seconds),
            } => {
                self.teach_undo_key();
                toast.show_action_completed_for(text, *undoable, *seconds);
            }
            ToastKind::Completed {
                undoable,
                seconds: None,
            } => {
                self.teach_undo_key();
                toast.show_action_completed(text, *undoable);
            }
            ToastKind::Undone => toast.show_undo_performed(text),
            ToastKind::Notice => toast.show_notice(text),
            ToastKind::Offer { .. } => return,
        }
        self.follow_toast();
    }

    fn pick(&self, row: &FocusRow, pick: crate::list::row::Pick) {
        use crate::list::row::Pick;
        let Some(pane) = self.pane() else {
            return;
        };
        if let Some(target) = pane.feed().shown().position_of(row.id()) {
            pane.feed().pick(target, pick == Pick::Range);
        }
    }

    pub fn clear_selection(&self) {
        if let Some(pane) = self.pane() {
            pane.feed().clear_selection();
        }
    }

    pub fn selection(&self) -> Selection {
        self.pane().map_or_else(
            || Selection::These(Vec::new()),
            |pane| pane.feed().selection(),
        )
    }

    pub fn cursor_row(&self) -> Option<FocusRow> {
        let pane = self.pane()?;
        let at = pane.cursor().selected();
        if at == gtk::INVALID_LIST_POSITION {
            return None;
        }
        pane.feed()
            .shown()
            .item(at)
            .and_downcast::<RowObject>()
            .and_then(|row| row.item())
    }

    pub fn cursor_child(&self) -> Option<postio_ui::thread_list::ChildRowView> {
        let pane = self.pane()?;
        let at = pane.cursor().selected();
        if at == gtk::INVALID_LIST_POSITION {
            return None;
        }
        pane.feed().shown().child_at(at)
    }

    fn aims(&self) -> Vec<MessageTarget> {
        let Some(pane) = self.pane() else {
            return Vec::new();
        };
        let aimed = pane.feed().aim();
        let Some(everything) = aimed.everything else {
            return aimed.aims;
        };
        if let Some(state) = self.imp().state.borrow().as_ref() {
            let (sink, _) = postio_core::bridge::event_channel();
            state.update(&sink, |app| {
                let mut events = app.open_view(ViewScope::Focus {
                    accounts: everything.accounts.clone(),
                });
                events.extend(app.select_all());
                events
            });
            state.update(&sink, |app| {
                let mut events = Vec::new();
                for message in &everything.except {
                    events.extend(app.toggle_selection(*message));
                }
                events
            });
        }
        vec![MessageTarget::Selection]
    }

    fn post(&self, command: Command) {
        if let Some(pane) = self.pane() {
            pane.feed().input(postio_focus::Input::Post(command));
        }
    }

    fn send(&self, command: Command) {
        if let Some(pane) = self.pane() {
            pane.feed().input(postio_focus::Input::Verb(command));
        }
    }

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

    pub fn show_unavailable(&self, reason: &str, retry: impl Fn() + 'static) {
        self.show_refusal(
            postio_ui::focus_state::CANT_OPEN_MAIL,
            reason,
            postio_ui::focus_state::TRY_AGAIN,
            Rc::new(retry),
        );
    }

    /// The store can never be read here (a schema no migration carries forward, or a key the
    /// keyring does not hold), so trying again would meet the same file: say what a fresh store
    /// keeps and what stays behind before it is chosen.
    pub fn show_start_over(
        &self,
        because: postio_session::Unreadable,
        start_over: impl Fn() + 'static,
    ) {
        use postio_ui::focus_state as words;
        let (title, reason) = match because {
            postio_session::Unreadable::AnotherBuild => {
                (words::STORE_FROM_ANOTHER_VERSION, words::START_OVER)
            }
            postio_session::Unreadable::AnotherKey => (
                words::STORE_UNDER_ANOTHER_KEY,
                words::START_OVER_UNDER_ANOTHER_KEY,
            ),
        };
        self.show_refusal(
            title,
            reason,
            words::START_A_FRESH_STORE,
            Rc::new(start_over),
        );
    }

    pub fn show_starting_over(&self) {
        let imp = self.imp();
        postio_widgets::widgets::keyhint::dress(
            &imp.retry,
            postio_ui::focus_state::STARTING_A_FRESH_STORE,
            None,
        );
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
        // The button holds the keyboard (below), so Return answers it.
        postio_widgets::widgets::keyhint::dress(&imp.retry, action, Some("Return"));
        imp.retry.set_sensitive(true);
        imp.on_retry.replace(Some(run));
        // The inbox's own top bar took the close button when it opened: the one before mail comes
        // back, so the page saying why there is no mail can always be closed.
        if imp.bar_before_mail.parent().is_none()
            && let Some(before_mail) = imp.stage.child().and_downcast::<gtk::Box>()
        {
            before_mail.prepend(&imp.bar_before_mail);
        }
        imp.pages.set_visible_child_name(UNAVAILABLE);
        imp.retry.grab_focus();
    }

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

    pub fn unavailable_reason(&self) -> Option<String> {
        let imp = self.imp();
        (imp.pages.visible_child_name().as_deref() == Some(UNAVAILABLE))
            .then(|| imp.refusal.borrow().clone())
    }

    pub fn retry(&self) {
        let retry = self.imp().on_retry.borrow().clone();
        if let Some(retry) = retry {
            retry();
        }
    }

    pub fn show_inbox(&self, client: Client, state: SharedState, keymap: Keymap) {
        self.imp().state.replace(Some(state));
        let imp = self.imp();
        // Out of the tree, not hidden, so the window has one close button.
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
        let titlebar = gtk::WindowHandle::new();
        titlebar.set_child(Some(chrome.top_bar()));
        self.follow_maximized(chrome.maximize());
        imp.inbox.append(&titlebar);
        imp.mail.set_vexpand(true);
        imp.body.set_vexpand(true);
        imp.body.set_transition_type(gtk::StackTransitionType::None);
        imp.body.add_named(&imp.mail, Some(MAIL));
        imp.inbox.append(&imp.body);
        imp.mail.append(chrome.strip());
        imp.chrome.replace(Some(Rc::clone(&chrome)));
        let state_banner = crate::banner::StateBanner::new();
        state_banner.connect_action(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |action| window.act(banner_command(action))
        ));
        imp.mail.append(state_banner.widget());
        imp.state_banner.replace(Some(state_banner));
        self.show_state();
        self.set_keymap(keymap);
        let feed = Feed::new(client.clone());
        feed.connect_filled(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.list_landed()
        ));
        feed.connect_intent(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |intent| window.show_intent(intent)
        ));
        feed.input(postio_focus::Input::Keymap(self.keymap()));
        feed.input(postio_focus::Input::Config(
            imp.focus_config.borrow().clone(),
        ));
        feed.input(postio_focus::Input::SavedSearches(
            imp.saved.borrow().clone(),
        ));
        let pane = ListPane::new(feed.clone(), self.keymap(), imp.picked.clone());
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
                        .and_then(|pane| pane.feed().shown().position_of(row.id()))
                    {
                        window.point_cursor(position);
                    }
                    window.act(command);
                }
            }
        ));
        pane.connect_row_pick(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |row, pick| window.pick(row, pick)
        ));
        pane.connect_row_chevron(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |row, expanded| {
                let Some(position) = window
                    .pane()
                    .and_then(|pane| pane.feed().shown().position_of(row.id()))
                else {
                    return;
                };
                window.point_cursor(position);
                window.cursor_command(if expanded {
                    CommandId::CollapseThread
                } else {
                    CommandId::ExpandThread
                });
            }
        ));
        pane.view().connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, position| {
                window.point_cursor(position);
                window.act(CommandId::OpenMessage);
            }
        ));
        // While a message is open beside the list, the pane shows the cursor's row wherever the
        // cursor goes: a click is the pointer's `j`/`k`.
        pane.cursor().connect_selected_notify(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |cursor| {
                // The list view moves the cursor on its own (a click, a row archived from under it)
                // and the controller walks on from wherever it is.
                if let Some(pane) = window.pane() {
                    let at = cursor.selected();
                    pane.feed()
                        .cursor_is((at != gtk::INVALID_LIST_POSITION).then_some(at));
                }
                window.follow_cursor();
            }
        ));
        self.connect_drag_out(&pane);
        pane.connect_row_menu(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |position, at| window.open_row_menu(position, at)
        ));
        let empty = crate::empty::EmptyInbox::new();
        empty.hold_the_keyboard();
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
        let bar = crate::bar::Bar::new(&self.keymap());
        bar.connect_input({
            let feed = feed.clone();
            let window = self.downgrade();
            move |input| match input {
                crate::bar::BarInput::Typed(text) => {
                    feed.input(postio_focus::Input::Typed { text })
                }
                crate::bar::BarInput::Run(token) => feed.input(postio_focus::Input::BarRun(token)),
                crate::bar::BarInput::Highlighted(token) => {
                    feed.input(postio_focus::Input::SearchHighlighted(token))
                }
                crate::bar::BarInput::Command(id) => {
                    if let Some(window) = window.upgrade() {
                        window.run_in_controller(id);
                    }
                }
            }
        });
        bar.connect_tab({
            let feed = feed.clone();
            move || feed.bar_tab()
        });
        bar.set_home(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.focus_list()
        ));
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            bar.set_field(chrome.center().clone().upcast());
        }
        imp.stage.add_overlay(bar.widget());
        list_or_empty.set_vexpand(true);
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
        imp.mail.append(&split);
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
        // At the window's foot, under the list and the pane alike: its verbs want the window's
        // width, not the list's.
        imp.mail.append(bulk.widget());
        imp.bulk.replace(Some(Rc::clone(&bulk)));
        let scope_footer = Rc::new(crate::scope_footer::ScopeFooter::new());
        imp.mail.append(scope_footer.widget());
        imp.scope_footer.replace(Some(scope_footer));
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
        self.focus_list();

        // Exactly one reader of the client's events, on the main loop: clones of the receiver would
        // compete for them.
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

    fn show_selection(&self, selection: &Selection, summary: Option<&str>) {
        let imp = self.imp();
        imp.picked.set(selection.clone());
        if let Some(bulk) = imp.bulk.borrow().as_ref() {
            bulk.set_summary(summary);
        }
        if let Some(pane) = imp.pane.borrow().as_ref() {
            pane.redraw_rows();
        }
    }

    fn list_landed(&self) {
        let Some(pane) = self.pane() else {
            return;
        };
        // Placing the cursor is not reading: it opens nothing, even with the reading pane beside,
        // unless an undo puts it on the row it brought back.
        if pane.feed().restoring() {
            pane.feed().landed();
        } else {
            self.placing(|| pane.feed().landed());
        }
        if pane.feed().take_restored() {
            self.reopen_restored();
        }
        self.show_counts();
    }

    fn placing(&self, place: impl FnOnce()) {
        let imp = self.imp();
        let before = imp.placing_cursor.replace(true);
        place();
        imp.placing_cursor.set(before);
    }

    fn answer(&self, message: MessageId, id: CommandId) {
        let message = Some(message);
        self.post(match id {
            CommandId::DeclineInvite => Command::DeclineInvite { message },
            _ => Command::AcceptInvite { message },
        });
    }

    /// With a message open over the list, an undo takes it back to the conversation it brought
    /// back, so the dialog and the list never point at two.
    fn reopen_restored(&self) {
        if self.reading().is_some_and(|reading| reading.is_open()) {
            self.open_message();
        }
    }

    fn aimed_message(&self) -> Option<MessageId> {
        let open = self
            .reading()
            .filter(|reading| reading.is_open())
            .map(|reading| reading.shown());
        if open.is_none()
            && self.cursor_row().is_none()
            && self.cursor_child().is_some()
            && let Some(pane) = self.pane()
            && let Some(MessageTarget::Messages(messages)) = pane
                .feed()
                .aim_for(CommandId::Reply)
                .aims
                .into_iter()
                .next()
            && let [message] = messages.as_slice()
        {
            return Some(*message);
        }
        postio_ui::focus_target::aimed_message(open, self.cursor_row().as_ref())
    }

    fn aimed_send_state(&self) -> Option<postio_model::DraftState> {
        if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            return reading.send_state();
        }
        self.cursor_row()?
            .as_conversation()
            .and_then(|row| row.summary.representative.send_state)
    }

    fn offered_on_open_draft(&self, id: CommandId) -> bool {
        self.reading()
            .filter(|reading| reading.is_open())
            .and_then(|reading| postio_ui::focus_dialog::send_verbs(reading.send_state()))
            .is_some_and(|verbs| verbs.contains(&id))
    }

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

    fn show_counts(&self) {
        let imp = self.imp();
        let Some(chrome) = imp.chrome.borrow().clone() else {
            return;
        };
        let feed = self.pane().map(|pane| pane.feed().clone());
        chrome.set_digest_rules(imp.focus_config.borrow().digests.len());
        // The host's counts are the inbox's: beside "Flagged" they described a place the person had
        // left.
        if !self.at_inbox() {
            let listed = feed
                .as_ref()
                .filter(|feed| feed.has_landed())
                .map(|feed| feed.total());
            chrome.set_place_counts(listed);
            chrome.set_filtered_today(None);
            return;
        }
        let counts = feed.as_ref().and_then(Feed::counts);
        if let Some(counts) = counts {
            chrome.set_counts(counts.conversations, counts.unread);
            let filtered = imp
                .focus_config
                .borrow()
                .filtering
                .then_some(counts.filtered_today);
            chrome.set_filtered_today(filtered.filter(|count| *count > 0));
            if let Some(places) = imp.places.borrow().as_ref() {
                places.redraw();
            }
            if let Some(settings) = imp.settings.borrow().as_ref() {
                settings.panel().set_filtered_today(filtered);
            }
        }
        let has_action = counts.map(|counts| counts.has_action);
        let label = postio_ui::focus_row::has_action_label(has_action);
        let on = feed.as_ref().is_some_and(Feed::has_action);
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
            && let Some(feed) = feed
            && feed.list().single_heading().as_deref() != Some(label.as_str())
        {
            feed.list().set_single_heading(Some(label));
        }
    }

    fn show_empty(&self, page: Option<&postio_ui::focus_state::EmptyInbox>) {
        let imp = self.imp();
        let Some(stack) = imp.list_or_empty.borrow().clone() else {
            return;
        };
        // Whether the keyboard was at home: then it goes with the switch to the page that comes in,
        // never where GTK's own hand-over would put it (#2195).
        let was_home =
            gtk::prelude::GtkWindowExt::focus(self).is_none_or(|focus| focus.is_ancestor(&stack));
        let before = stack.visible_child_name();
        match (page, imp.empty.borrow().as_ref()) {
            (Some(page), Some(widget)) => {
                widget.show(page);
                stack.set_visible_child_name(EMPTY);
            }
            _ => stack.set_visible_child_name(LIST),
        }
        if was_home && before != stack.visible_child_name() && self.keyboard_is_free() {
            self.focus_list();
        }
        let beside = self.imp().placement.get() == postio_ui::focus_dialog::Placement::Pane;
        if beside != (stack.visible_child_name().as_deref() == Some(LIST)) {
            self.place_reading();
        }
    }

    pub(crate) fn filtered_today(&self) -> Option<u32> {
        let imp = self.imp();
        let filtering = imp.focus_config.borrow().filtering;
        self.pane()
            .and_then(|pane| pane.feed().counts())
            .filter(|_| filtering)
            .map(|counts| counts.filtered_today)
    }

    pub fn set_focus_config(&self, focus: postio_config::FocusConfig) {
        let capture = focus.vault.is_some();
        let beside = focus.reading == postio_config::Reading::Pane;
        let for_controller = focus.clone();
        if let Some(settings) = self.imp().settings.borrow().as_ref() {
            settings
                .panel()
                .set_contact_rules(focus.contacts().to_vec());
        }
        self.imp().focus_config.replace(focus);
        if let Some(menu) =
            self.lookup_action(crate::chrome::READING_PANE_ACTION.trim_start_matches("win."))
        {
            menu.change_state(&beside.to_variant());
        }
        self.show_capture(capture);
        if let Some(pane) = self.pane() {
            pane.feed()
                .input(postio_focus::Input::Config(for_controller));
        }
        self.show_counts();
        self.place_reading();
    }

    fn show_capture(&self, capture: bool) {
        if let Some(pane) = self.pane() {
            pane.set_capture(capture);
        }
        if let Some(reading) = self.reading() {
            reading.set_capture(capture);
        }
    }

    pub fn chrome(&self) -> Option<Rc<Chrome>> {
        self.imp().chrome.borrow().clone()
    }

    fn hear(&self, event: &postio_core::Event) {
        use postio_core::Event;
        match event {
            Event::NewMail {
                mailbox, messages, ..
            } => self.announce(*mailbox, messages.clone()),
            Event::BodyLoaded {
                message, sections, ..
            }
            | Event::BodyFailed {
                message, sections, ..
            } => {
                if let Some(reading) = self.reading() {
                    reading.body_landed(*message);
                }
                if let Some(compose) = self.compose() {
                    compose.composer().fetch_settled(*message, sections);
                }
            }
            Event::BodyReachable { message, .. } => {
                if let Some(reading) = self.reading() {
                    reading.body_landed(*message);
                }
            }
            _ => {}
        }
        self.hear_sync(event);
        if let Some(settings) = self.imp().settings.borrow().as_ref() {
            settings.hear(event);
        }
        if let Some(pane) = self.imp().pane.borrow().as_ref() {
            if matches!(event, Event::UndoPerformed { .. }) {
                pane.feed().input(postio_focus::Input::AtTop(
                    pane.widget().vadjustment().value() <= 0.0,
                ));
            }
            pane.feed().handle(event);
        }
    }

    pub fn set_notification_sink(&self, sink: impl Fn(&postio_ui::notify::Notification) + 'static) {
        self.imp().notification_sink.replace(Some(Rc::new(sink)));
    }

    pub fn set_notifier(&self, notifier: Notifier) {
        self.imp().notifier.replace(Some(notifier));
    }

    pub fn set_start_syncing(&self, start: Rc<dyn Fn()>) {
        self.imp().start_syncing.replace(Some(start));
    }

    fn announce(&self, mailbox: postio_model::MailboxId, messages: Vec<MessageId>) {
        let Some(notifier) = self.imp().notifier.borrow().clone() else {
            return;
        };
        let imp = self.imp();
        let looking = imp.pages.visible_child_name().as_deref() == Some(INBOX)
            && self.at_inbox()
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

    fn dismiss_marker(&self) {
        if let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            let Some(message) = reading.shown() else {
                return;
            };
            reading.clear_marker();
            // The controller aims it at the message it shows; one the window showed itself (a hit,
            // a digest's email) it does not know.
            if self.pane().and_then(|pane| pane.feed().reading()) == Some(message) {
                self.cursor_command(CommandId::DismissMarker);
            } else {
                self.post(Command::DismissMarker {
                    target: MessageTarget::Messages(vec![message]),
                    dismissed: true,
                });
            }
            return;
        }
        self.cursor_command(CommandId::DismissMarker);
    }

    fn digest_at_cursor(&self) -> Option<crate::list::Digest> {
        match self.cursor_row()? {
            FocusRow::Digest(digest) => Some(digest),
            _ => None,
        }
    }

    fn archive_digest(&self, delivery: postio_model::DeliveryId) {
        if let Some(pane) = self.pane() {
            pane.feed()
                .input(postio_focus::Input::ArchiveDigest(delivery));
        }
    }

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

    pub fn digest(&self) -> Option<Rc<crate::digest::DigestWindow>> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::digest::DIALOG_NAME)?;
        self.imp().digest_window.borrow().clone()
    }

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
                    self.place_reading();
                    reading.show_found(self, message, &subject, None);
                }
            }
        }
    }

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
            Ok(CommandId::Archive) => self.digest_action(crate::digest::DigestAction::ArchiveAll),
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

    pub fn stop_digesting_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == STOP_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

    /// One toast: when the list left mail behind it says so and offers the one action `[focus]
    /// after_unsubscribe` names; otherwise it only confirms (#1952). It never blocks.
    fn unsubscribe(&self, message: MessageId) {
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let name = match client.unsubscribe(message).await {
                    Ok(name) => name,
                    Err(error) => {
                        window.imp().toast.show_notice(&error.to_string());
                        window.follow_toast();
                        return;
                    }
                };
                // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
                let found = client.list_mail(message).await.ok();
                window.confirm_unsubscribed(&name, found);
            }
        ));
    }

    fn confirm_unsubscribed(
        &self,
        name: &str,
        found: Option<postio_client::protocol::ListMailFound>,
    ) {
        use postio_ui::unsubscribe::{SweepScope, sweep_notice};
        let action = self.imp().focus_config.borrow().after_unsubscribe;
        let notice = found.as_ref().and_then(|found| {
            sweep_notice(
                &SweepScope::ListId(name.to_owned()),
                found.ids.len(),
                found.more,
                action,
            )
        });
        let (Some(notice), Some(found)) = (notice, found) else {
            self.imp()
                .toast
                .show_notice(&postio_ui::focus_target::unsubscribed(name));
            self.follow_toast();
            return;
        };
        let ids = found.ids;
        let taken = notice.clone();
        self.imp().toast.show_offer(
            &notice.text,
            &notice.button,
            postio_ui::hints::key(&self.keymap(), CommandId::ClearListMail),
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move || window.post(taken.command(ids.clone()))
            ),
        );
        self.follow_toast();
    }

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

    /// Only when the user has brought a model with `like_this` on; whether it connects is a further
    /// question left to `Client::digest_like_this`.
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

    pub fn set_timeline(&self, timeline: postio_widgets::startup::Timeline) {
        self.imp().timeline.replace(Some(timeline));
    }

    pub fn timeline(&self) -> Option<postio_widgets::startup::Timeline> {
        self.imp().timeline.borrow().clone()
    }

    pub fn set_config_path(&self, path: Option<std::path::PathBuf>) {
        self.imp().config_path.replace(path);
    }

    fn save_search(&self, save: &postio_focus::SaveSearch) {
        let path = self.imp().config_path.borrow().clone();
        let saved = postio_focus::save_search(path.as_deref(), save);
        if let Ok(saved) = &saved {
            self.imp().saved.replace(saved.searches.clone());
        }
        if let Some(pane) = self.pane() {
            pane.feed().input(postio_focus::Input::SearchSaved(saved));
        }
    }

    pub fn rules(&self) -> Option<Rc<crate::rules::RulesView>> {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() != Some(RULES) {
            return None;
        }
        imp.rules_view.borrow().clone()
    }

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
        // `RulesView::show` teaches its own caps: it redraws rows once `read_holds`'s fetch lands,
        // and a call only here would miss those.
        view.open(imp.focus_config.borrow().digests.clone());
    }

    /// What only the inbox answers is refused there, so a key does not act on mail nobody can see.
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
            | CommandId::CloseWindow
            | CommandId::Search
            | CommandId::CommandPalette => return false,
            _ => {}
        }
        true
    }

    fn rules_action(&self, action: crate::rules::RulesAction) {
        use crate::rules::RulesAction;
        match action {
            RulesAction::Back => self.leave_filtered(),
            RulesAction::Edit(name) => self.edit_digest_rule(&name),
            RulesAction::Remove { name, holds } => self.ask_remove_rule(&name, holds),
        }
    }

    /// No undo takes that mail back into the rule, so this is the one place the list asks first.
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

    pub fn remove_rule_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == REMOVE_RULE_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

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

    pub fn rule_dialog(&self) -> Option<Rc<crate::rule_dialog::RuleDialog>> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::rule_dialog::DIALOG_NAME)?;
        self.imp().rule_dialog.borrow().clone()
    }

    pub fn sweep_confirmation(&self) -> Option<adw::AlertDialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == SWEEP_DIALOG)
            .and_then(|dialog| dialog.downcast().ok())
    }

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

    pub fn filtered(&self) -> Option<Rc<crate::filtered::FilteredView>> {
        let imp = self.imp();
        if imp.pages.visible_child_name().as_deref() != Some(FILTERED) {
            return None;
        }
        imp.filtered.borrow().clone()
    }

    fn show_filtered(&self) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        let view = imp.filtered.borrow().clone();
        // A view still here from the last visit is shown empty: its rows were the controller's
        // then, not now.
        if let Some(view) = &view {
            view.clear();
        }
        let view = view.unwrap_or_else(|| {
            let feed = pane.feed().clone();
            let view = crate::filtered::FilteredView::new(
                &self.keymap(),
                move |input| feed.input(input),
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |command| window.run_in_controller(command)
                ),
            );
            view.connect_action(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |action| window.filtered_action(action)
            ));
            imp.pages.add_named(view.widget(), Some(FILTERED));
            imp.filtered.replace(Some(Rc::clone(&view)));
            view
        });
        imp.pages.set_visible_child_name(FILTERED);
        view.focus_list();
    }

    pub fn contacts(&self) -> Option<Rc<crate::contacts::ContactsView>> {
        let imp = self.imp();
        if imp.body.visible_child_name().as_deref() != Some(CONTACTS) {
            return None;
        }
        imp.contacts.borrow().clone()
    }

    pub fn results(&self) -> Option<Rc<crate::results::ResultsPane>> {
        let imp = self.imp();
        if imp.body.visible_child_name().as_deref() != Some(RESULTS) {
            return None;
        }
        imp.results.borrow().clone()
    }

    fn show_results(&self, frame: &postio_focus::ResultsView) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        let view = imp.results.borrow().clone();
        let view = view.unwrap_or_else(|| {
            let view = crate::results::ResultsPane::new(
                pane.feed().clone(),
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |command| window.press_command(command)
                ),
            );
            imp.body.add_named(view.widget(), Some(RESULTS));
            imp.results.replace(Some(Rc::clone(&view)));
            view
        });
        view.show(frame);
        if imp.body.visible_child_name().as_deref() != Some(RESULTS) {
            imp.body.set_visible_child_name(RESULTS);
            if let Some(chrome) = imp.chrome.borrow().as_ref() {
                chrome.show_query(Some(view.query_field().upcast_ref()));
            }
            view.focus();
        }
    }

    fn leave_results(&self) {
        let imp = self.imp();
        if imp.body.visible_child_name().as_deref() == Some(RESULTS) {
            imp.body.set_visible_child_name(MAIL);
            self.focus_list();
        }
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            chrome.show_query(None);
        }
        // The page is kept for the next search, and the controller says "found nothing" of the
        // results it holds: once they are left it holds none.
        if let Some(results) = imp.results.borrow().as_ref() {
            results.show_relaxations(None);
        }
    }

    fn show_contacts(&self, frame: &postio_focus::ContactsView) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        let view = imp.contacts.borrow().clone();
        let view = view.unwrap_or_else(|| {
            let view = crate::contacts::ContactsView::new(
                pane.feed().clone(),
                glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |command| window.press_command(command)
                ),
            );
            imp.body.add_named(view.widget(), Some(CONTACTS));
            imp.contacts.replace(Some(Rc::clone(&view)));
            view
        });
        view.show_frame(frame);
        if imp.body.visible_child_name().as_deref() != Some(CONTACTS) {
            imp.body.set_visible_child_name(CONTACTS);
            view.focus_list();
        }
    }

    /// Nothing is merged: the look-alikes wait in the review.
    fn contacts_import(&self) {
        let weak = self.downgrade();
        // POSTIO-CONSENT: a file is read only because somebody chose it in
        // the file chooser; it is parsed on this computer and nothing is
        // uploaded.
        self.pick_place(crate::chooser::SavePick::Cards, move |path| {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let Some(client) = window.imp().client.borrow().clone() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                window.show_toast(
                    "Postio could not read that file.",
                    &postio_focus::ToastKind::Notice,
                );
                return;
            };
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: one client call, a oneshot receive.
                match client.import_contacts(bytes).await {
                    Ok(summary) => {
                        if let Some(pane) = window.pane() {
                            pane.feed().input(postio_focus::Input::ContactsImported {
                                made: summary.made.clone(),
                                touched: summary.touched.clone(),
                            });
                        }
                        let sheet =
                            postio_ui::contacts_io::import_sheet(&summary, &window.keymap());
                        window.show_contacts_io(sheet);
                    }
                    Err(error) => {
                        window.show_toast(&error.to_string(), &postio_focus::ToastKind::Notice)
                    }
                }
            });
        });
    }

    fn contacts_export(&self) {
        let words = postio_ui::contacts_io::panel_words();
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let weak = self.downgrade();
        glib::spawn_future_local(async move {
            let Some(window) = weak.upgrade() else {
                return;
            };
            // POSTIO-GLIB-SAFE: one client call, a oneshot receive.
            let (text, count) = match client.export_contacts().await {
                Ok(done) => done,
                Err(error) => {
                    window.show_toast(&error.to_string(), &postio_focus::ToastKind::Notice);
                    return;
                }
            };
            let keymap = window.keymap();
            let again = weak.clone();
            // POSTIO-CONSENT: a file is written only to the name and place
            // somebody chose in the file chooser.
            window.pick_place(
                crate::chooser::SavePick::CardsFile {
                    suggested: words.export_name.clone(),
                },
                move |path| {
                    let Some(window) = again.upgrade() else {
                        return;
                    };
                    match std::fs::write(&path, text.as_bytes()) {
                        Ok(()) => window
                            .show_contacts_io(postio_ui::contacts_io::export_sheet(count, &keymap)),
                        Err(_) => window.show_toast(
                            "Postio could not write that file.",
                            &postio_focus::ToastKind::Notice,
                        ),
                    }
                },
            );
        });
    }

    fn show_contacts_io(&self, sheet: postio_ui::contacts_io::IoSheetView) {
        let dialog = adw::AlertDialog::new(Some(&sheet.title), Some(&sheet.lines.join("\n\n")));
        dialog.set_widget_name(CONTACTS_IO_DIALOG);
        dialog.add_response("done", &sheet.done.label);
        dialog.set_close_response("done");
        dialog.set_default_response(Some("done"));
        if let Some(review) = &sheet.review {
            dialog.add_response("review", &review.label);
            dialog.set_response_appearance("review", adw::ResponseAppearance::Suggested);
            dialog.set_default_response(Some("review"));
        }
        self.imp().contacts_io.replace(Some(sheet));
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |_, response| {
                    window.imp().contacts_io.take();
                    if response == "review" {
                        window.press_command(CommandId::ContactsSuggestions);
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    pub fn close_dialogs_for_test(&self) {
        if let Some(dialog) = self.visible_dialog() {
            dialog.close();
        }
        self.imp().contacts_io.take();
    }

    pub fn contacts_io_sheet(&self) -> Option<postio_ui::contacts_io::IoSheetView> {
        self.imp().contacts_io.borrow().clone()
    }

    pub fn contact_popover(&self) -> Option<postio_ui::contact_popover::PopoverView> {
        self.imp()
            .contact_popover
            .borrow()
            .as_ref()
            .map(|(_, view)| view.clone())
    }

    pub fn contact_popover_texts(&self) -> Vec<String> {
        self.imp()
            .contact_popover
            .borrow()
            .as_ref()
            .map(|(widget, _)| crate::contacts::texts_of(widget))
            .unwrap_or_default()
    }

    fn show_contact_popover(&self, view: &postio_ui::contact_popover::PopoverView) {
        use crate::contacts::popover;
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        let (start, top) = match imp.contact_popover.take() {
            Some((old, _)) => {
                let at = (old.margin_start(), old.margin_top());
                imp.stage.remove_overlay(&old);
                at
            }
            None => self.popover_anchor(&pane),
        };
        let weak = self.downgrade();
        let hooks = popover::PopoverHooks {
            command: Rc::new({
                let weak = weak.clone();
                move |command| {
                    if let Some(window) = weak.upgrade() {
                        window.press_command(command);
                    }
                }
            }),
            arrival: Rc::new(move |arrival| {
                if let Some(window) = weak.upgrade()
                    && let Some(pane) = window.pane()
                {
                    pane.feed()
                        .input(postio_focus::Input::ContactArrival(arrival));
                }
            }),
        };
        let card = popover::build(view, &hooks);
        card.set_margin_start(start);
        card.set_margin_top(top);
        imp.stage.add_overlay(&card);
        imp.contact_popover
            .replace(Some((card.upcast(), view.clone())));
    }

    fn popover_anchor(&self, pane: &ListPane) -> (i32, i32) {
        let stage = &self.imp().stage;
        let (width, height) = (stage.width(), stage.height());
        let centre = ((width - 400) / 2, 90);
        let Some(cursor) = self.cursor_row() else {
            return centre;
        };
        let Some(row) = pane
            .rows_on_screen()
            .into_iter()
            .find(|row| row.item().is_some_and(|item| item.id() == cursor.id()))
        else {
            return centre;
        };
        let Some(at) = row.compute_point(stage, &gtk::graphene::Point::new(0.0, 0.0)) else {
            return centre;
        };
        let below = at.y() as i32 + row.height();
        let tall = 300;
        let top = if below + tall > height {
            (at.y() as i32 - tall).max(8)
        } else {
            below
        };
        (((at.x() as i32) + 56).clamp(8, (width - 408).max(8)), top)
    }

    fn close_contact_popover(&self) {
        let imp = self.imp();
        if let Some((card, _)) = imp.contact_popover.take() {
            imp.stage.remove_overlay(&card);
        }
    }

    pub fn press_command(&self, command: CommandId) {
        match self.route(command) {
            Route::Controller => self.run_in_controller(command),
            _ => self.act(command),
        }
    }

    fn leave_contacts(&self) {
        self.imp().body.set_visible_child_name(MAIL);
        self.focus_list();
    }

    fn leave_filtered(&self) {
        self.imp().pages.set_visible_child_name(INBOX);
        self.placing(|| self.cursor_command(CommandId::FirstMessage));
        self.focus_list();
    }

    fn hide_filtered(&self) {
        self.imp().pages.set_visible_child_name(INBOX);
        self.focus_list();
    }

    fn filtered_action(&self, action: crate::filtered::FilteredAction) {
        match action {
            crate::filtered::FilteredAction::Sweep => self.ask_sweep(),
        }
    }

    fn hear_sync(&self, event: &postio_core::Event) {
        use postio_core::Event;
        let imp = self.imp();
        // The first word about an account is news even when it changes nothing in its tracker: a
        // tracker starts at `Offline`, so a first report of `Offline` would never be drawn.
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

    fn note_account(&self, account: AccountId) -> bool {
        let mut tracked = self.imp().tracked.borrow_mut();
        let new = !tracked.contains(&account);
        if new {
            tracked.push(account);
        }
        new
    }

    fn show_state(&self) {
        let imp = self.imp();
        let statuses = imp.trackers.borrow().statuses(&imp.tracked.borrow());
        let banner = postio_ui::focus_state::banner(&statuses, &imp.facts.borrow());
        if let Some(slot) = imp.state_banner.borrow().as_ref() {
            let key = banner
                .as_ref()
                .and_then(postio_ui::focus_state::Banner::action)
                .and_then(|(_, action)| {
                    postio_ui::hints::key(&self.keymap(), banner_command(action))
                });
            slot.show(banner.as_ref(), key);
        }
        let folders = self
            .pane()
            .map(|pane| pane.feed().folder_names())
            .unwrap_or_default();
        let label =
            postio_ui::focus_state::sync_label_here(&statuses, imp.last_synced.get(), &folders);
        if let Some(chrome) = imp.chrome.borrow().as_ref() {
            chrome.set_sync(&label.text, label.icon, label.detail.as_deref());
        }
    }

    pub fn banner_showing(&self) -> Option<(String, Option<String>, Option<f64>)> {
        self.imp()
            .state_banner
            .borrow()
            .as_ref()
            .and_then(|slot| slot.showing())
    }

    pub fn sync_said(&self) -> String {
        self.chrome()
            .map(|chrome| chrome.sync_said())
            .unwrap_or_default()
    }

    pub fn sync_detail(&self) -> Option<String> {
        self.chrome().and_then(|chrome| chrome.sync_detail())
    }

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

    /// A link that arrives before the store is open waits for it.
    pub fn open_link(&self, uri: &str) {
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
                    reading.show_found(&window, message, &row.subject.unwrap_or_default(), None);
                }
            }
        ));
    }

    /// A link that arrives before the composer is mounted (a cold launch from a browser) waits for
    /// it.
    fn open_mailto(&self, mailto: postio_model::mailto::Mailto) {
        match self.compose() {
            Some(compose) => compose.open_mailto(mailto),
            None => {
                self.imp().pending_mailto.replace(Some(mailto));
            }
        }
    }

    /// A layout of its own, so the widths come from the allocation and never from a size request
    /// the window would then refuse to shrink below;
    /// a crossing of the narrowest width a pane fits moves an open message between pane and dialog.
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
        // The pane is the inbox list's: Filtered, the digest rules and a digest over the window
        // have no list beside it, and an empty inbox has nothing to open.
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

    fn reading_beside(&self) -> bool {
        self.reading()
            .is_some_and(|reading| reading.in_pane() && reading.is_open())
    }

    pub fn reading_pane(&self) -> Option<gtk::Widget> {
        self.imp()
            .reading_pane
            .borrow()
            .as_ref()
            .map(|pane| pane.widget())
    }

    /// Over the list the dialog takes the pointer, so only the keys move the cursor, and they open
    /// what they land on themselves.
    fn follow_cursor(&self) {
        if self.imp().placing_cursor.get() || !self.reading_beside() {
            return;
        }
        self.open_message();
    }

    /// The choice is written to `config.toml` as `[focus] reading`; the watcher's echo of the write
    /// changes nothing.
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
        if !beside && let Some(reading) = self.reading().filter(|reading| reading.is_open()) {
            reading.dialog().grab_focus();
        }
    }

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
                reading.connect_bar_room(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |room| {
                        if let Some(pane) = window.pane() {
                            pane.feed().input(postio_focus::Input::BarRoom(room));
                        }
                    }
                ));
                reading.connect_thread_chip(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |click| {
                        let Some(pane) = window.pane() else {
                            return;
                        };
                        pane.feed().input(match click {
                            crate::thread_bar::ChipClick::Message(index) => {
                                postio_focus::Input::ThreadBarChipClicked(index)
                            }
                            crate::thread_bar::ChipClick::More(from) => {
                                postio_focus::Input::ThreadBarMoreClicked { from }
                            }
                        });
                    }
                ));
                reading.connect_thread_menu(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move || window.act(CommandId::ThreadMenu)
                ));
                reading.connect_read(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |message| window.post(Command::MarkReadOnDwell { message })
                ));
                self.imp().toast.set_over(Some(reading.toast_overlay()));
                reading.dialog().connect_closed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_| window.focus_cursor_soon()
                ));
                reading.set_capture(self.imp().focus_config.borrow().vault.is_some());
                reading.set_accounts(
                    self.imp()
                        .facts
                        .borrow()
                        .iter()
                        .map(|facts| (facts.id, facts.address.clone()))
                        .collect(),
                );
                if let Some(zoom) = self.imp().zoom.get() {
                    reading.reader().set_zoom(zoom);
                }
                // A zoom a person chose is the next message's too: written to `[reader]` alone,
                // which the watcher reads back as the zoom it already is.
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
                // The notice only asks; leaving the list is logged by the host, on this one
                // deliberate click.
                reading.reader().connect_unsubscribe_activated(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |_| {
                        if let Some(message) = window.reading().and_then(|r| r.shown()) {
                            window.unsubscribe(message);
                        }
                    }
                ));
                reading.connect_chip(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    move |part| window.offer_choices(Some(part))
                ));
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

    fn reading_changed(&self) {
        // Told at once, so a message closed by its own button is off the controller's stack before
        // any key is resolved.
        self.sync_surfaces();
        let Some(reading) = self.reading() else {
            return;
        };
        if !reading.is_open() {
            // What was said while the message was up (a send queued, an archive done) goes on being
            // said, with its Undo, over the list.
            if self.imp().toast.rehome() {
                self.follow_toast();
            }
            self.place_reading();
        }
        // The offer ends with the moment it was made, rather than staying reachable by key unseen.
        if reading.is_open() && self.imp().toast.drop_offer() {
            self.imp().on_screen.take();
        }
        self.show_pane_page();
        if reading.in_pane() && !reading.is_open() {
            self.focus_list();
        }
        if let Some(results) = self.results() {
            results.set_window_open(reading.is_open());
        }
    }

    /// The controller decides what opens (a draft is written, not read). With a message already up,
    /// it follows the cursor instead.
    fn open_message(&self) {
        self.sync_surfaces();
        let Some(pane) = self.pane() else {
            return;
        };
        if pane.feed().top_surface() == Some(SurfaceKind::Message) {
            pane.feed().input(postio_focus::Input::Follow);
        } else {
            pane.feed().command(CommandId::OpenMessage);
        }
    }

    fn show_message(&self, message: MessageId, index: u32, total: u32) {
        if let Some(view) = self.filtered()
            && let Some(reading) = self.reading_dialog()
        {
            let subject = view.focused_subject().unwrap_or_default();
            reading.show_found(self, message, &subject, None);
            crate::a11y::teach_shortcuts(&reading.view());
            crate::motion::keep_to_budget(&reading.view());
            return;
        }
        if let Some(reading) = self.reading_dialog()
            && reading.is_open()
            && self
                .pane()
                .is_some_and(|pane| pane.feed().walking(message, reading.row()))
        {
            reading.show_in_place(message);
            return;
        }
        let row = self.pane().and_then(|pane| {
            let at = pane.feed().list().position_of(message)?;
            pane.feed()
                .list()
                .item(at)
                .and_downcast::<RowObject>()
                .and_then(|row| row.item())
        });
        if self
            .pane()
            .is_some_and(|pane| pane.feed().among_hits(message))
            && let Some(reading) = self.reading_dialog()
        {
            let subject = row
                .as_ref()
                .and_then(|row| row.as_conversation())
                .and_then(|conversation| conversation.summary.subject.clone())
                .unwrap_or_default();
            let hit = crate::open::Position { index, total };
            reading.show_found(self, message, &subject, Some(hit));
            crate::a11y::teach_shortcuts(&reading.view());
            crate::motion::keep_to_budget(&reading.view());
            return;
        }
        // A message of a thread that is not its newest has no list row: it opens by its id, titled
        // by the cursor's row, or the open thread's row on a child.
        if row.is_none()
            && let Some(pane) = self.pane()
            && let Some(reading) = self.reading_dialog()
        {
            let on_row = self.cursor_row().or_else(|| {
                let open = pane.feed().shown().open_thread()?;
                pane.feed()
                    .list()
                    .item(open.anchor)
                    .and_downcast::<RowObject>()
                    .and_then(|row| row.item())
            });
            let subject = on_row
                .and_then(|row| {
                    row.as_conversation()
                        .and_then(|conversation| conversation.summary.subject.clone())
                })
                .unwrap_or_default();
            let _ = (index, total);
            reading.show_found(self, message, &subject, None);
            crate::a11y::teach_shortcuts(&reading.view());
            crate::motion::keep_to_budget(&reading.view());
            return;
        }
        let (Some(row), Some(reading)) = (row, self.reading_dialog()) else {
            if let Some(pane) = self.pane() {
                pane.feed()
                    .input(postio_focus::Input::SurfaceClosed(SurfaceKind::Message));
            }
            return;
        };
        let position = crate::open::Position { index, total };
        // Beside the list the pane follows the cursor, and `j` both moves it and asks.
        if reading.showing_row() == Some(row.id()) && reading.in_pane() {
            return;
        }
        reading.show(self, &row, position);
        crate::a11y::teach_shortcuts(&reading.view());
        crate::motion::keep_to_budget(&reading.view());
    }

    pub fn set_launcher(&self, launch: impl Fn(&str) + 'static) {
        self.imp().launcher.replace(Some(Rc::new(launch)));
    }

    /// Linux has no system Quick Look, so Space opens the copy in the desktop's app. The copy stays
    /// until the controller takes it away, since the app may still be reading it; Save writes it
    /// elsewhere and lets the copy go either way.
    fn file_copy(&self, copy: Option<&postio_focus::FileCopy>) {
        let Some(copy) = copy else {
            return;
        };
        match copy.purpose {
            postio_focus::FilePurpose::Preview => {
                let uri = gio::File::for_path(&copy.path).uri();
                self.launch(&uri);
            }
            postio_focus::FilePurpose::Save => {
                let from = copy.path.clone();
                let suggested = postio_ui::reader::parts::clean_file_name(&copy.name)
                    .unwrap_or_else(|| "file".to_owned());
                let weak = self.downgrade();
                self.pick_place_or_not(
                    crate::chooser::SavePick::File { suggested },
                    move |chosen| {
                        let Some(window) = weak.upgrade() else {
                            return;
                        };
                        if let Some(to) = chosen
                            && let Err(error) = std::fs::copy(&from, &to)
                        {
                            tracing::warn!(kind = ?error.kind(), "a search's file could not be saved");
                            window.show_toast(
                                "Postio could not save that file.",
                                &postio_focus::ToastKind::Notice,
                            );
                        }
                        if let Some(pane) = window.pane() {
                            pane.feed().input(postio_focus::Input::FileCopyDone);
                        }
                    },
                );
            }
        }
    }

    fn launch(&self, uri: &str) {
        let launch = self.imp().launcher.borrow().clone();
        match launch {
            Some(launch) => launch(uri),
            None => {
                // POSTIO-CONSENT: runs only when the person chooses a link or
                // a part in the open-with chooser (`A`, then a row), one at a
                // time; nothing opens on render, on arrival or from a setting.
                gtk::UriLauncher::new(uri).launch(Some(self), None::<&gio::Cancellable>, |_| {})
            }
        }
    }

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
        // Dismissed is a person's answer, not a fault: nothing is written and nothing is said.
        self.pick_place_or_not(pick, move |chosen| {
            if let Some(path) = chosen {
                then(path);
            }
        });
    }

    fn pick_place_or_not(
        &self,
        pick: crate::chooser::SavePick,
        answer: impl FnOnce(Option<std::path::PathBuf>) + 'static,
    ) {
        let seam = self.imp().file_picker.borrow().clone();
        if let Some(seam) = seam {
            return seam(pick, Box::new(answer));
        }
        // POSTIO-CONSENT: runs only when the person pressed Save or Save all
        // in the chooser; the portal's dialog is theirs to dismiss.
        let dialog = gtk::FileDialog::new();
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
            crate::chooser::SavePick::Cards => {
                dialog.set_title(&postio_ui::contacts_io::panel_words().import_title);
                dialog.open(Some(self), None::<&gio::Cancellable>, done);
            }
            crate::chooser::SavePick::CardsFile { suggested } => {
                dialog.set_title(&postio_ui::contacts_io::panel_words().export_title);
                dialog.set_initial_name(Some(&suggested));
                dialog.save(Some(self), None::<&gio::Cancellable>, done);
            }
        }
    }

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

    pub fn choose(&self, index: usize) {
        self.close_chooser();
        self.open_choice(index);
    }

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
                        window
                            .imp()
                            .toast
                            .show_notice(&postio_ui::reader::parts::saved(&name));
                    }
                    Err(error) => window.imp().toast.show_notice(&error.to_string()),
                }
                window.follow_toast();
            });
        });
    }

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
                    Ok(failed) => postio_ui::reader::parts::saved_all(count, failed),
                    Err(error) => error.to_string(),
                };
                window.imp().toast.show_notice(&said);
                window.follow_toast();
            });
        });
    }

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

    pub fn source_shown(&self) -> Option<String> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == crate::source::DIALOG_NAME)
            .and_then(|dialog| crate::source::shown(&dialog))
    }

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

    pub fn set_remote_runtime(&self, runtime: tokio::runtime::Handle) {
        self.imp().runtime.replace(Some(runtime));
    }

    pub fn set_saved_searches(&self, saved: Vec<postio_ui::saved_search::SavedSearch>) {
        if let Some(pane) = self.pane() {
            pane.feed()
                .input(postio_focus::Input::SavedSearches(saved.clone()));
        }
        self.imp().saved.replace(saved);
    }

    pub fn places(&self) -> Option<Rc<crate::places::Places>> {
        self.imp().places.borrow().clone()
    }

    pub fn place_name(&self) -> String {
        self.chrome()
            .map(|chrome| chrome.place())
            .unwrap_or_default()
    }

    fn open_places(&self) {
        let (Some(chrome), Some(pane)) = (self.chrome(), self.pane()) else {
            return;
        };
        let places = self
            .imp()
            .places
            .borrow_mut()
            .get_or_insert_with(|| {
                let places = crate::places::Places::new(&self.keymap(), chrome.place_button());
                places.connect_source({
                    let feed = pane.feed().clone();
                    move |filter| feed.places(filter)
                });
                places.connect_go({
                    let feed = pane.feed().clone();
                    move |token| feed.input(postio_focus::Input::OpenPlace(token))
                });
                // Chosen from or dismissed, the keyboard goes back to the list, not to the button
                // the popover hangs from.
                places.connect_closed(glib::clone!(
                    #[weak(rename_to = window)]
                    self,
                    #[strong(rename_to = feed)]
                    pane.feed(),
                    move || {
                        feed.input(postio_focus::Input::PlacesShown(false));
                        window.focus_list_soon();
                    }
                ));
                places
            })
            .clone();
        places.open();
        pane.feed().input(postio_focus::Input::PlacesShown(true));
        crate::motion::keep_to_budget(self);
    }

    fn open_mailbox(&self, mailbox: postio_model::MailboxId, name: &str) {
        let (Some(pane), Some(chrome)) = (self.pane(), self.chrome()) else {
            return;
        };
        pane.feed().leave_list();
        pane.feed().note_place(name);
        pane.feed().open(ListScope::Mailbox(mailbox));
        chrome.set_place(name);
        self.show_counts();
        self.focus_list_soon();
    }

    fn next_scope(&self) {
        use postio_model::listing::MailStore as _;
        use postio_ui::places::{Scope, next_scope};
        let Some(client) = self.imp().client.borrow().clone() else {
            return;
        };
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            let mut inboxes = Vec::new();
            // POSTIO-GLIB-SAFE: client calls are oneshot receives.
            for account in client.accounts().await.unwrap_or_default() {
                if !account.enabled {
                    continue;
                }
                // POSTIO-GLIB-SAFE: as above.
                let mailboxes = client.mailboxes(account.id).await.unwrap_or_default();
                if let Some(inbox) = mailboxes
                    .into_iter()
                    .find(|mailbox| mailbox.role == postio_model::MailboxRole::Inbox)
                {
                    let name = format!("Inbox \u{b7} {}", account.address.address);
                    inboxes.push((inbox.id, name));
                }
            }
            let Some(window) = window.upgrade() else {
                return;
            };
            let Some(pane) = window.pane() else {
                return;
            };
            let ids: Vec<_> = inboxes.iter().map(|(id, _)| *id).collect();
            let here = match pane.feed().scope() {
                _ if window.at_inbox() => Some(Scope::Focus),
                Some(ListScope::Mailbox(mailbox)) => Some(Scope::Inbox(mailbox)),
                _ => None,
            };
            match next_scope(&ids, here) {
                Scope::Focus if here == Some(Scope::Focus) => {}
                Scope::Focus => window.go_to_inbox(),
                Scope::Inbox(mailbox) => {
                    let name = inboxes
                        .iter()
                        .find(|(id, _)| *id == mailbox)
                        .map_or("Inbox", |(_, name)| name.as_str());
                    window.open_mailbox(mailbox, name);
                }
            }
        });
    }

    fn go_to_inbox(&self) {
        let (Some(pane), Some(chrome)) = (self.pane(), self.chrome()) else {
            return;
        };
        pane.feed().leave_list();
        pane.feed().open(ListScope::Focus(FocusScope::Inbox));
        chrome.set_place("Inbox");
        self.show_counts();
        self.focus_list_soon();
    }

    fn at_inbox(&self) -> bool {
        self.pane().is_none_or(|pane| {
            matches!(
                pane.feed().scope(),
                None | Some(ListScope::Focus(FocusScope::Inbox | FocusScope::HasAction))
            )
        })
    }

    fn show_place(&self, name: &str) {
        if let Some(chrome) = self.chrome() {
            chrome.set_place(name);
        }
        self.show_counts();
        self.focus_list_soon();
    }

    pub fn bar(&self) -> Option<Rc<crate::bar::Bar>> {
        self.imp().bar.borrow().clone()
    }

    /// Public because a headless compositor grants no focus, so a test says what the notification
    /// would.
    pub fn focus_changed(&self, active: bool) {
        if let Some(reading) = self.reading() {
            reading.set_window_active(active);
        }
    }

    pub fn reading(&self) -> Option<Rc<crate::open::OpenMessage>> {
        self.imp().reading.borrow().clone()
    }

    fn show_key_map(&self) {
        let dialog = crate::keymap_dialog::build(&self.keymap());
        dialog.set_widget_name(KEY_MAP);
        crate::motion::keep_to_budget(&dialog);
        dialog.present(Some(self));
    }

    pub fn command_bar_rows(&self) -> Vec<CommandId> {
        self.bar().map(|bar| bar.commands()).unwrap_or_default()
    }

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
        commands.extend(
            postio_ui::focus_state::empty_pane(&self.keymap())
                .shortcuts
                .into_iter()
                .map(|(_, _, command)| command),
        );
        commands.extend(crate::places::Places::controls());
        commands.extend(crate::row_menu::RowMenu::commands());
        commands.extend(crate::thread_popovers::ThreadMenu::controls());
        commands.extend([CommandId::ExpandThread, CommandId::CollapseThread]);
        commands.extend(crate::bar::Bar::controls());
        commands.extend(crate::filtered::FilteredView::controls());
        commands.extend(crate::contacts::ContactsView::controls());
        commands.extend(crate::digest::DigestWindow::controls());
        commands.extend([CommandId::GoToFiltered, CommandId::GoToDigestRules]);
        commands.push(CommandId::Undo);
        commands.push(CommandId::AddAccount);
        commands.push(CommandId::ClearListMail);
        commands.extend(crate::settings::Settings::controls());
        commands.extend(crate::compose::Compose::controls());
        commands.extend(crate::capture::CaptureSheet::controls());
        commands.extend([CommandId::AcceptInvite, CommandId::DeclineInvite]);
        commands.sort_by_key(|command| command.as_str());
        commands.dedup();
        commands
    }

    /// A row inside the selection gets a menu for the selection; a row outside gets one for itself,
    /// and the selection is let go only when one of its verbs runs, so dismissing loses nothing.
    pub fn open_row_menu(&self, position: u32, at: gdk::Rectangle) {
        let imp = self.imp();
        let Some(pane) = self.pane() else {
            return;
        };
        if let Some(picker) = self.open_picker() {
            picker.close();
        }
        self.point_cursor(position);
        let Some(row) = self.cursor_row() else {
            return;
        };
        let Some(conversation) = row.as_conversation() else {
            return;
        };
        let selection = self.selection();
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

    fn row_menu_chose(&self, id: CommandId) {
        if self.imp().row_menu_alone.replace(false) {
            self.clear_selection();
        }
        self.act(id);
    }

    pub fn row_menu(&self) -> Option<Rc<crate::row_menu::RowMenu>> {
        self.imp().row_menu.borrow().clone()
    }

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
            .chain(
                imp.driven_picker
                    .borrow()
                    .as_ref()
                    .map(|driven| Rc::clone(driven.picker())),
            )
            .chain(self.compose().and_then(|compose| compose.open_picker()))
            .find(|picker| picker.is_open())
    }

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

    fn open_driven_picker(&self, view: &postio_focus::PickerView) {
        let imp = self.imp();
        let picker = imp.driven_picker.borrow().clone();
        let picker = picker.unwrap_or_else(|| {
            let picker = crate::controller_picker::ControllerPicker::new(&self.keymap());
            picker.connect_input(glib::clone!(
                #[weak(rename_to = window)]
                self,
                move |input| {
                    if let Some(pane) = window.pane() {
                        pane.feed().input(input);
                    }
                }
            ));
            imp.driven_picker.replace(Some(Rc::clone(&picker)));
            picker
        });
        let Some((anchor, rect)) = self.at_the_row() else {
            return;
        };
        picker.open(view, &anchor, Some(&rect));
    }

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

    fn at_the_row(&self) -> Option<(gtk::Widget, gdk::Rectangle)> {
        // With the message open over the list, the list is behind a modal dialog and cannot hold
        // the keyboard: a picker hangs from the dialog, under its action row.
        if let Some(reading) = self.reading()
            && reading.is_open()
            && let Some(hung) = reading.picker_anchor()
        {
            return Some(hung);
        }
        let pane = self.pane()?;
        let view = pane.view().clone();
        let row = match self.cursor_row() {
            Some(cursor) => pane
                .rows_on_screen()
                .into_iter()
                .find(|row| row.item().is_some_and(|item| item.id() == cursor.id())),
            None => {
                let child = self.cursor_child()?;
                pane.rows_on_screen()
                    .into_iter()
                    .find(|row| row.child().as_ref() == Some(&child))
            }
        };
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
        Some((view.upcast(), rect))
    }

    fn picker_target(&self) -> String {
        postio_ui::focus_target::picker_target(&self.selection(), self.cursor_row().as_ref())
    }

    pub fn key_map(&self) -> Option<adw::Dialog> {
        self.visible_dialog()
            .filter(|dialog| dialog.widget_name() == KEY_MAP)
    }

    pub fn compose_dialog(&self) -> Option<adw::Dialog> {
        self.compose().and_then(|compose| compose.dialog())
    }

    pub fn composer(&self) -> Option<postio_widgets::composer::Composer> {
        self.compose().map(|compose| compose.composer().clone())
    }

    pub fn pane(&self) -> Option<ListPane> {
        self.imp().pane.borrow().clone()
    }

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

    pub fn say(&self, sentence: &str) {
        self.imp().toast.show_notice(sentence);
        self.follow_toast();
    }

    pub(crate) fn teach_undo_key(&self) {
        let key = postio_ui::hints::key(&self.keymap(), CommandId::Undo);
        self.imp().toast.set_undo_key(key.clone());
        if let Some(settings) = self.settings() {
            settings.set_undo_key(key);
        }
    }

    pub(crate) fn show_removable(&self, sentence: &str, on_undo: impl Fn() + 'static) {
        self.teach_undo_key();
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

    pub fn toast_showing(&self) -> Option<String> {
        self.imp()
            .on_screen
            .borrow()
            .as_ref()
            .and_then(|toast| toast.title().map(|title| title.to_string()))
    }

    pub fn toast_host(&self) -> Option<adw::ToastOverlay> {
        self.imp().toast.host()
    }

    pub fn toast(&self) -> Option<adw::Toast> {
        self.imp().on_screen.borrow().clone()
    }

    pub fn dismiss_toast(&self) {
        let toast = self.imp().on_screen.borrow().clone();
        if let Some(toast) = toast {
            toast.dismiss();
        }
    }
}

impl FocusWindow {
    /// Reads only: no store, no command, no focus change, and builds no surface not already there.
    /// Every field comes off what is on screen, not what a surface was told.
    /// `rows.first_visible`, `reading.focused` and `back_depth` are unobserved and left empty, so a
    /// check reads "not applicable".
    pub fn observe(&self) -> postio_ui::observe::Observation {
        use postio_ui::observe::{
            Banner, Composer as ComposerState, Cursor, Keyboard, Notice, Observation, Overlay,
            OverlayState, Reading, Region, Rows, Scroll, Selection as SelectionCount, View,
            Window as WindowState,
        };
        use postio_widgets::storyboard::{deliver, reach};

        let toplevel: &gtk::Window = self.upcast_ref();
        // A composition in a window of its own holds the keyboard there: the widget a key reaches
        // is that window's focus.
        let detached = self
            .compose()
            .and_then(|compose| compose.composer().detached_window())
            .filter(|host| host.is_visible());
        let target = match detached.as_ref() {
            Some(host) => deliver::keyboard_target(host.upcast_ref()),
            None => deliver::keyboard_target(toplevel),
        };
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
        let compose_open = self.compose().is_some_and(|compose| compose.is_showing());
        let composition_detached = self
            .compose()
            .is_some_and(|compose| compose.composer().is_detached());
        let sign_in = self.add_account_dialog().is_some();
        let settings = self.settings_dialog().is_some();
        let small_dialog = [crate::source::DIALOG_NAME, crate::chooser::DIALOG_NAME]
            .into_iter()
            .find(|name| dialog_named(name));
        let reading_here = self.visible_dialog().is_none() && self.reading_beside();
        // The merge sheet over the Contacts view takes every key until answered.
        let merging = self.contacts().is_some_and(|view| view.merge_open());

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
            || merging
        {
            Region::Dialog
        } else if compose_open || composition_detached {
            Region::Composer
        } else if dialog_named(crate::open::DIALOG_NAME) || reading_here {
            Region::Reader
        } else if digest.is_some() {
            Region::Dialog
        } else if self.results().is_some() && self.key_context() == KeyContext::Results {
            Region::Results
        } else if focus.is_none() {
            // Focus leaves the keyboard on the window itself when nothing is focused (a closed bar
            // hands focus back to nothing: `Bar::close`).
            Region::List
        } else if focus.as_ref().is_some_and(|widget| {
            std::iter::successors(Some(widget.clone()), |widget| widget.parent())
                .any(|widget| widget.type_().name() == "AdwToastWidget")
        }) {
            Region::Other
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
            // A control in the top bar or foot strip holds focus, but the window's controller takes
            // every key first (capture phase).
            Region::List
        } else {
            Region::Other
        };

        let field = match region {
            Region::Search | Region::Picker => Some("query".to_owned()),
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
        let count = pane.as_ref().map(|pane| pane.feed().shown().n_items());
        let row_count = pane.as_ref().map(|pane| pane.feed().list().n_items());
        let selected = match self.selection() {
            _ if let Some(view) = self.filtered() => view.selected().len() as u32,
            Selection::These(ids) => ids.len() as u32,
            Selection::Everything { except } => {
                row_count.unwrap_or(0).saturating_sub(except.len() as u32)
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
        } else if merging {
            OverlayState {
                kind: Overlay::Dialog,
                mode: Some("merge".to_owned()),
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
        // A toast whose overlay is not on screen is not a notice anyone can read.
        let notice = if imp.toast.host().is_some_and(|host| host.is_mapped()) {
            Notice {
                text: self.toast_showing(),
                tone: imp.toast.tone(),
                undo: imp.toast.offers_undo(),
            }
        } else {
            Notice {
                text: None,
                tone: None,
                undo: false,
            }
        };

        let locked = self.unavailable_reason().is_some();
        let view = if locked {
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
        // The cursor's ring is drawn from the cursor, so only these say whether the keyboard and
        // what a person sees are the same row.
        if let Some(widget) = focus.as_ref()
            && let Some(pane) = self.pane()
            && widget.is_ancestor(pane.view())
        {
            let row = widget
                .first_child()
                .and_downcast::<crate::list::row::RowWidget>()
                .or_else(|| {
                    widget
                        .ancestor(crate::list::row::RowWidget::static_type())
                        .and_downcast::<crate::list::row::RowWidget>()
                })
                .and_then(|row| row.item());
            if let Some(row) = row {
                app.insert(
                    "focus.keyboard.row".to_owned(),
                    serde_json::json!(row.id().get().to_string()),
                );
            }
        }
        app.insert(
            "focus.keyboard.on_control".to_owned(),
            serde_json::json!(focus.as_ref().is_some_and(|widget| {
                widget.is::<gtk::Button>()
                    || widget.ancestor(gtk::Button::static_type()).is_some()
                    || widget.is::<gtk::CheckButton>()
                    || widget.is::<gtk::Switch>()
            })),
        );
        app.insert(
            "focus.cursor.ring".to_owned(),
            serde_json::json!(if self.has_css_class(KEYBOARD_AWAY) {
                "quiet"
            } else {
                "accent"
            }),
        );
        if let Some(reading) = reading.as_ref()
            && view == View::Reader
        {
            app.insert(
                "focus.keyboard.in_message".to_owned(),
                serde_json::json!(reading.message_has(&target)),
            );
        }
        if let Some(reading) = reading.as_ref()
            && view == View::Reader
        {
            app.insert(
                "focus.reading.position".to_owned(),
                serde_json::json!(reading.subtitle()),
            );
        }
        if let Some(reading) = reading.as_ref()
            && view == View::Reader
        {
            let body = if reading.reader().absent().is_some() {
                "absent".to_owned()
            } else {
                reading.body_text().trim().to_owned()
            };
            app.insert("focus.reading.body".to_owned(), serde_json::json!(body));
        }
        if let Some(reading) = reading.as_ref()
            && let Some(item) = reading.more_item_with_focus()
        {
            app.insert("focus.keyboard.item".to_owned(), serde_json::json!(item));
        }
        if let Some(digest) = digest.as_ref() {
            app.insert(
                "focus.digest.reference".to_owned(),
                serde_json::json!(digest.focused_reference()),
            );
        }
        let empty = self
            .imp()
            .list_or_empty
            .borrow()
            .as_ref()
            .filter(|stack| stack.visible_child_name().as_deref() == Some(EMPTY))
            .and(imp.empty.borrow().as_ref())
            .map(|page| page.heading());
        app.insert("focus.empty.heading".to_owned(), serde_json::json!(empty));
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
        if let Some(compose) = self.compose()
            && let composer = compose.composer()
            && composer.is_open()
        {
            app.insert(
                "focus.composer.holds_keyboard".to_owned(),
                serde_json::json!(detached.as_ref().is_none_or(|host| {
                    gtk::prelude::GtkWindowExt::focus(host)
                        .is_some_and(|focus| focus.is_ancestor(composer))
                })),
            );
            app.insert(
                "focus.composer.quoted".to_owned(),
                serde_json::json!(composer.draft().body.text.is_some()),
            );
            app.insert(
                "focus.composer.marks".to_owned(),
                serde_json::json!(composer.marks_lit().join(" ")),
            );
            // Which view the body is in (spec 014 US7), what the Markdown
            // view holds without its quote lines, and the status line the
            // composer speaks on.
            let source = composer.source_text();
            app.insert(
                "focus.composer.view".to_owned(),
                serde_json::json!(if source.is_some() { "markdown" } else { "rich" }),
            );
            app.insert(
                "focus.composer.source".to_owned(),
                serde_json::json!(source.map(|text| postio_ui::editor::source::markdown_of(&text))),
            );
            app.insert(
                "focus.composer.status".to_owned(),
                serde_json::json!(composer.status()),
            );
            app.insert(
                "focus.composer.not_attached".to_owned(),
                serde_json::json!(composer.not_attached()),
            );
        }
        app.insert(
            "focus.bulk".to_owned(),
            serde_json::json!({ "shown": bulk.is_some(), "summary": bulk }),
        );
        if let Some(conversation) = row.as_ref().and_then(|row| row.as_conversation()) {
            app.insert(
                "focus.cursor.unread".to_owned(),
                serde_json::json!(conversation.summary.unread_count),
            );
        }
        if let Some(reading) = reading.as_ref()
            && view == View::Reader
        {
            let (chip, chips) = reading.thread_bar_state();
            app.insert("focus.reading.chip".to_owned(), serde_json::json!(chip));
            app.insert("focus.reading.chips".to_owned(), serde_json::json!(chips));
            app.insert(
                "focus.reading.kicker".to_owned(),
                serde_json::json!(reading.thread_chip_said()),
            );
            app.insert(
                "focus.reading.match_banner".to_owned(),
                serde_json::json!(reading.match_banner_said()),
            );
            app.insert(
                "focus.reading.matched".to_owned(),
                serde_json::json!(
                    reading
                        .matched_chips()
                        .iter()
                        .map(|chip| (chip + 1).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
            app.insert(
                "focus.reading.new_reply".to_owned(),
                serde_json::json!(reading.new_reply_up()),
            );
            app.insert(
                "focus.reading.banner".to_owned(),
                serde_json::json!(reading.banner_said().is_some()),
            );
            app.insert(
                "focus.reading.reply_to".to_owned(),
                serde_json::json!(reading.reply_to_said()),
            );
            app.insert(
                "focus.reading.find_matches".to_owned(),
                serde_json::json!(reading.reader().view().find_state().1),
            );
        }
        if let Some(results) = self.results() {
            app.insert(
                "focus.results.children".to_owned(),
                serde_json::json!(results.child_count()),
            );
            app.insert(
                "focus.results.child".to_owned(),
                serde_json::json!(results.child_focus()),
            );
            app.insert(
                "focus.results.tab".to_owned(),
                serde_json::json!(results.tab_said()),
            );
            app.insert(
                "focus.results.count_line".to_owned(),
                serde_json::json!(results.count_line()),
            );
            app.insert(
                "focus.results.reason".to_owned(),
                serde_json::json!(results.top_reason()),
            );
        }
        if let Some(compose) = self.compose()
            && compose.is_showing()
        {
            let parts = &compose.thread;
            let draft = compose.composer().draft();
            app.insert(
                "focus.compose.older.open".to_owned(),
                serde_json::json!(parts.older_title().is_some()),
            );
            if let Some(title) = parts.older_title() {
                app.insert(
                    "focus.compose.older.title".to_owned(),
                    serde_json::json!(title),
                );
            }
            app.insert(
                "focus.compose.older.actions".to_owned(),
                serde_json::json!(parts.older_actions()),
            );
            app.insert(
                "focus.compose.newer.count".to_owned(),
                serde_json::json!(parts.newer_count()),
            );
            app.insert(
                "focus.compose.chooser.open".to_owned(),
                serde_json::json!(parts.chooser_open()),
            );
            app.insert(
                "focus.compose.chooser.focused".to_owned(),
                serde_json::json!(parts.chooser_has_focus()),
            );
            if parts.chooser_open() {
                app.insert(
                    "focus.compose.chooser.selected".to_owned(),
                    serde_json::json!(parts.chooser_selected()),
                );
                app.insert(
                    "focus.compose.chooser.cards".to_owned(),
                    serde_json::json!(parts.chooser_cards()),
                );
            }
            app.insert(
                "focus.compose.quoted_expanded".to_owned(),
                serde_json::json!(parts.quoted_expanded()),
            );
            app.insert(
                "focus.compose.replying_to".to_owned(),
                serde_json::json!(self.imp().replying_to.get()),
            );
            app.insert(
                "focus.compose.subject".to_owned(),
                serde_json::json!(draft.subject),
            );
            app.insert(
                "focus.compose.typed".to_owned(),
                serde_json::json!(draft.body.text.as_deref().map(typed_part)),
            );
            let eml = draft
                .attachments
                .iter()
                .filter(|file| {
                    file.filename
                        .as_deref()
                        .is_some_and(|name| name.ends_with(".eml"))
                })
                .count();
            app.insert(
                "focus.compose.forwarded.rows".to_owned(),
                serde_json::json!(parts.forwarded_rows()),
            );
            app.insert(
                "focus.compose.forwarded.eml".to_owned(),
                serde_json::json!(eml),
            );
            app.insert(
                "focus.compose.forwarded.messages".to_owned(),
                serde_json::json!(forwarded_count(&draft.subject)),
            );
        }
        let (menu, outline) = (
            self.imp().thread_menu.borrow().clone(),
            self.imp().outline.borrow().clone(),
        );
        if let Some(menu) = menu {
            app.insert(
                "focus.menu.open".to_owned(),
                serde_json::json!(menu.is_open()),
            );
            if menu.is_open() {
                app.insert(
                    "focus.menu.groups".to_owned(),
                    serde_json::json!(menu.groups_said()),
                );
                app.insert(
                    "focus.menu.keycaps".to_owned(),
                    serde_json::json!(menu.keycaps_said()),
                );
            }
        }
        if let Some(outline) = outline {
            app.insert(
                "focus.outline.open".to_owned(),
                serde_json::json!(outline.is_open()),
            );
            if outline.is_open() {
                app.insert(
                    "focus.outline.rows".to_owned(),
                    serde_json::json!(outline.row_count()),
                );
                app.insert(
                    "focus.outline.cursor".to_owned(),
                    serde_json::json!(outline.cursor()),
                );
                app.insert(
                    "focus.outline.filtering".to_owned(),
                    serde_json::json!(outline.filtering()),
                );
            }
        }
        let footer = match (self.filtered(), self.results()) {
            (Some(view), _) => Some(view.scope_footer().clone()),
            (None, Some(results)) => results.window_footer(),
            (None, None) => self.imp().scope_footer.borrow().clone(),
        };
        if let Some(footer) = footer.filter(|footer| footer.widget().is_visible()) {
            app.insert(
                "focus.footer.title".to_owned(),
                serde_json::json!(footer.title()),
            );
            app.insert(
                "focus.footer.labels".to_owned(),
                serde_json::json!(footer.labels()),
            );
            app.insert(
                "focus.footer.note".to_owned(),
                serde_json::json!(footer.note()),
            );
        }
        let child = self.cursor_child();
        if child.is_some() || row.is_some() {
            app.insert(
                "focus.cursor.kind".to_owned(),
                serde_json::json!(if child.is_some() { "child" } else { "row" }),
            );
        }
        if let Some(conversation) = row.as_ref().and_then(|row| row.as_conversation()) {
            let badge = conversation
                .summary
                .muted
                .map(postio_ui::thread_outline::muted_badge);
            app.insert("focus.row.muted".to_owned(), serde_json::json!(badge));
            if conversation.summary.message_count > 1 {
                let me = pane
                    .as_ref()
                    .map(|pane| pane.identities())
                    .unwrap_or_default();
                let said = postio_ui::focus_row::ThreadRowView::new(
                    &conversation.summary,
                    &conversation.labels,
                    &me,
                    false,
                    postio_ui::clock::now(),
                );
                app.insert(
                    "focus.row.participants".to_owned(),
                    serde_json::json!(said.participants),
                );
                app.insert(
                    "focus.row.line".to_owned(),
                    serde_json::json!(said.latest_line),
                );
            }
        }
        if let Some(child) = &child {
            app.insert("focus.cursor.tag".to_owned(), serde_json::json!(child.tag));
            app.insert(
                "focus.cursor.line".to_owned(),
                serde_json::json!(child.line),
            );
        }
        if let Some(bar) = &bar {
            // One key each, because a check names an app field by its whole key.
            let highlighted = bar.highlighted();
            let fields = [
                ("focus.bar.typed", serde_json::json!(bar.typed())),
                ("focus.bar.hints", serde_json::json!(bar.hints())),
                (
                    "focus.bar.messages",
                    serde_json::json!(bar.result_subjects().len()),
                ),
                (
                    "focus.bar.highlighted",
                    serde_json::json!(highlighted.as_ref().map(|(kind, _)| *kind)),
                ),
                (
                    "focus.bar.highlighted_text",
                    serde_json::json!(highlighted.map(|(_, text)| text)),
                ),
            ];
            for (key, value) in fields {
                app.insert(key.to_owned(), value);
            }
        }
        if let Some(settings) = self.settings().filter(|settings| settings.is_open()) {
            let panel = settings.panel();
            app.insert(
                "focus.settings.section".to_owned(),
                serde_json::json!(panel.current_section().label()),
            );
            app.insert(
                "focus.settings.listed".to_owned(),
                serde_json::json!(
                    panel
                        .listed_sections()
                        .into_iter()
                        .map(|section| section.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
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
                // With the composition in a window of its own, the keyboard is that window's.
                // `is_typing` stays the main window's.
                typing: match detached.as_ref() {
                    Some(_) => {
                        target.is::<gtk::Text>()
                            || target.is::<gtk::TextView>()
                            || self.composer_body_has_keyboard()
                    }
                    None => self.is_typing(),
                },
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
                open: compose_open || composition_detached,
                detached: composition_detached,
            },
            back_depth: None,
            app,
        }
    }
}

/// The sweep is the window's (it asks first, in a dialog); everything else is the app's wherever it
/// is.
fn filtered_owns(id: CommandId) -> bool {
    postio_ui::filtered::TAB_COMMANDS.contains(&id)
        || matches!(
            id,
            CommandId::NextMessage
                | CommandId::PrevMessage
                | CommandId::OpenMessage
                | CommandId::ToggleSelection
                | CommandId::ExtendSelectionDown
                | CommandId::ExtendSelectionUp
                | CommandId::SelectAll
                | CommandId::ExpandThread
                | CommandId::CollapseThread
                | CommandId::RestoreFiltered
                | CommandId::Delete
                | CommandId::DeleteThisMessage
                | CommandId::Flag
                | CommandId::ToggleRead
                | CommandId::Back
                | CommandId::GoToInbox
        )
}

fn banner_command(action: postio_ui::focus_state::BannerAction) -> CommandId {
    match action {
        postio_ui::focus_state::BannerAction::Retry => CommandId::Refresh,
        postio_ui::focus_state::BannerAction::UpdatePassword => CommandId::UpdateCredential,
    }
}

fn typed_part(body: &str) -> String {
    match body.find("\n\nOn ") {
        Some(at) => body[..at].to_owned(),
        None => body.trim_end().to_owned(),
    }
}

fn forwarded_count(subject: &str) -> Option<u32> {
    let open = subject.rfind('(')?;
    subject[open + 1..]
        .split_whitespace()
        .next()
        .and_then(|count| count.parse().ok())
}
