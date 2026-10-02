//! Settings in Focus (T234; `specs/007-postio-focus/screens.md`, "Settings"):
//! the shared settings panel in a dialog over the list, in the message
//! dialog's frame.
//!
//! The panel is `postio_widgets::settings::SettingsPanel`, the classic app's
//! window, and its presenters are `postio_widgets::present::settings`, over
//! the same client the window reads through. What is Focus's here is the
//! frame -- the dialog, its header and its size -- and the [`Outside`] the
//! presenters ask for what is not the host's to answer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_core::Frontend;
use postio_model::Account;
use postio_model::ids::AccountId;
use postio_ui::focus_dialog;
use postio_widgets::present::settings::accounts::Accounts;
use postio_widgets::present::settings::{Later, Outside, Reached, Reindexing};
use postio_widgets::settings::SettingsPanel;

/// The dialog's widget name, which the window's keyboard reads to know
/// Settings has it.
pub const DIALOG_NAME: &str = "focus-settings";

/// What Settings needs from the process that opened the store: where the
/// connection test and the token-expiry line run, the keyring they read, and
/// whether attachments are fetched eagerly. Set by `startup::adopt_at`.
#[derive(Clone)]
pub struct Seams {
    /// The host's runtime, which a connection test is polled on.
    pub runtime: tokio::runtime::Handle,
    /// The keyring a connection test and the token-expiry line read.
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    /// `[sync] attachment_fetch = "eager"`, which an account row's weight
    /// mentions.
    pub attachments_eager: bool,
}

/// The settings dialog: built on the first open and kept, so a section, a
/// scroll position and a half-typed search survive closing it.
pub struct Settings {
    dialog: adw::Dialog,
    panel: SettingsPanel,
    /// The dialog's own toasts: one over the window would be under the
    /// dialog's scrim, where a removal's Undo cannot be reached.
    toast: postio_widgets::widgets::toast::Toast,
    accounts: RefCell<Option<Accounts>>,
    open: Cell<bool>,
    /// Whether the dialog is following its window's size yet.
    following: Cell<bool>,
}

impl Settings {
    /// The dialog over `client`, with what `outside` answers for its
    /// presenters.
    pub fn new(client: Client, outside: Rc<dyn Outside>) -> Rc<Settings> {
        let panel = SettingsPanel::new();
        panel.set_frontend(Frontend::Focus);
        // The message dialog is at most 820 wide, so a pane is at most 606:
        // too narrow for two columns side by side (screens.md, "Narrow").
        panel.set_narrow(true);
        panel.set_vexpand(true);

        // The header: the message dialog's (screens.md, "Settings"). The
        // find-a-setting field where the open message has its steps, the
        // title centred, and the one X at the right end (T192).
        let search = panel.search_field();
        search.set_valign(gtk::Align::Center);
        search.add_css_class("focus-settings-search");
        let title = gtk::Label::new(Some("Settings"));
        title.add_css_class("focus-open-title");
        title.set_accessible_role(gtk::AccessibleRole::Heading);
        let close = postio_widgets::widgets::close_button();
        close.add_css_class("focus-open-close");
        let header = gtk::CenterBox::new();
        header.add_css_class("focus-open-header");
        header.add_css_class("focus-settings-header");
        header.set_start_widget(Some(&search));
        header.set_center_widget(Some(&title));
        header.set_end_widget(Some(&close));

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("focus-settings");
        content.append(&header);
        content.append(&panel);

        let toast = postio_widgets::widgets::toast::Toast::new();
        toast.overlay().set_child(Some(&content));
        let dialog = adw::Dialog::builder()
            .title("Settings")
            .content_width(focus_dialog::dialog_width(REFERENCE_WINDOW.0))
            .content_height(focus_dialog::dialog_height(REFERENCE_WINDOW.1))
            .child(toast.overlay())
            .build();
        dialog.set_widget_name(DIALOG_NAME);

        let settings = Rc::new(Settings {
            dialog,
            panel,
            toast,
            accounts: RefCell::default(),
            open: Cell::new(false),
            following: Cell::new(false),
        });
        close.connect_clicked({
            let settings = Rc::downgrade(&settings);
            move |_| {
                if let Some(settings) = settings.upgrade() {
                    settings.close();
                }
            }
        });
        // The panel's own Escape, from wherever its keyboard is.
        settings.panel.connect_dismissed({
            let settings = Rc::downgrade(&settings);
            move || {
                if let Some(settings) = settings.upgrade() {
                    settings.close();
                }
            }
        });
        settings.dialog.connect_closed({
            let settings = Rc::downgrade(&settings);
            move |_| {
                if let Some(settings) = settings.upgrade() {
                    settings.open.set(false);
                    // Hidden while closed, as the classic app hides it: the
                    // presenters read what costs only while it shows.
                    settings.panel.set_visible(false);
                }
            }
        });

        // The presenters, over the window's client (T233).
        postio_widgets::present::settings::egress::install(&settings.panel, client.clone());
        postio_widgets::present::settings::backfill::install(&settings.panel, client.clone());
        glib::spawn_future_local({
            let settings = Rc::downgrade(&settings);
            async move {
                let Some(panel) = settings.upgrade().map(|settings| settings.panel.clone()) else {
                    return;
                };
                postio_widgets::present::settings::privacy::install(&panel, client.clone()).await;
                let accounts =
                    Accounts::install(&panel, client, outside, Reindexing::default()).await;
                if let Some(settings) = settings.upgrade() {
                    settings.accounts.replace(Some(accounts));
                }
            }
        });
        settings
    }

    /// The panel inside the dialog.
    pub fn panel(&self) -> &SettingsPanel {
        &self.panel
    }

    /// The dialog.
    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    /// Whether the dialog is over the window.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Present the dialog over `window`, sized as the message dialog is
    /// for it, and following it as it is resized.
    pub fn present(self: &Rc<Self>, window: &gtk::Window) {
        if self.open.replace(true) {
            return;
        }
        self.follow(window);
        // Read fresh on every open: whatever a reader last wrote to the
        // remote-image allow list shows here without watching the file.
        let path = crate::open::allowlist_path();
        self.panel.set_remote_image_allowlist(
            postio_ui::allowlist::RemoteImageAllowList::load_from(&path),
            path,
        );
        self.panel.set_visible(true);
        crate::motion::keep_to_budget(&self.dialog);
        self.dialog.present(Some(window));
        self.panel.grab_focus();
    }

    /// The message dialog's size for a window `width` by `height`.
    fn fit(&self, width: i32, height: i32) {
        let (width, height) = if width > 0 && height > 0 {
            (width, height)
        } else {
            REFERENCE_WINDOW
        };
        let wide = focus_dialog::dialog_width(width);
        if self.dialog.content_width() != wide {
            self.dialog.set_content_width(wide);
        }

        let tall = focus_dialog::dialog_height(height);
        if self.dialog.content_height() != tall {
            self.dialog.set_content_height(tall);
        }
    }

    /// Fit the dialog to `window` now, and again whenever it is resized: its
    /// surface's `layout` says when, as the open message follows it.
    fn follow(self: &Rc<Self>, window: &gtk::Window) {
        let (width, height) = match (window.width(), window.height()) {
            (width, height) if width > 0 && height > 0 => (width, height),
            _ => window.default_size(),
        };
        self.fit(width, height);
        if self.following.get() {
            return;
        }
        let Some(surface) = window.surface() else {
            return;
        };
        self.following.set(true);
        let settings = Rc::downgrade(self);
        let window = window.downgrade();
        surface.connect_layout(move |_, _, _| {
            if let (Some(settings), Some(window)) = (settings.upgrade(), window.upgrade()) {
                settings.fit(window.width(), window.height());
            }
        });
    }

    /// Close the dialog, if it is open.
    pub fn close(&self) {
        if self.open.get() {
            self.dialog.close();
        }
    }

    /// `description` on the dialog's toast, with an Undo that runs `undo`.
    pub fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        self.toast.show_removable(description, undo);
    }

    /// What the store just said, as far as the account rows care.
    pub fn hear(&self, event: &postio_core::Event) {
        if let Some(accounts) = self.accounts.borrow().as_ref() {
            accounts.hear(event);
        }
    }

    /// Read the account rows again: an account was added.
    pub fn refresh_accounts(&self) {
        let accounts = self.accounts.borrow().clone();
        if let Some(accounts) = accounts {
            glib::spawn_future_local(async move { accounts.refresh().await });
        }
    }
}

/// What Focus answers for its settings: the connection test and the
/// token-expiry line from this process, a removal's undo on the window's
/// toast, and a role mapping's command through the window.
pub struct FocusOutside {
    /// The window the dialog is over.
    pub window: glib::WeakRef<crate::window::FocusWindow>,
    /// What the store's opener left for Settings, if it has opened one.
    pub seams: Option<Seams>,
    /// The client the window reads through.
    pub client: Client,
}

impl Outside for FocusOutside {
    fn test_connection(&self, account: Account) -> Later<Reached> {
        let Some(seams) = self.seams.clone() else {
            return Box::pin(std::future::ready((
                Err("this window opened no store to test from".to_owned()),
                Err("this window opened no store to test from".to_owned()),
            )));
        };
        let (sender, receiver) = async_channel::bounded(1);
        seams.runtime.spawn(async move {
            let found = postio_session::reachability::test_over_tls(&account, &seams.secrets).await;
            let _ = sender.send(found.into_results()).await;
        });
        Box::pin(async move {
            receiver.recv().await.unwrap_or_else(|_| {
                let died = || Err("the test did not finish".to_owned());
                (died(), died())
            })
        })
    }

    fn token_expiries(
        &self,
        accounts: Vec<(AccountId, String)>,
    ) -> Later<Vec<(AccountId, Option<std::time::SystemTime>)>> {
        let Some(seams) = self.seams.clone() else {
            return Box::pin(std::future::ready(Vec::new()));
        };
        let (sender, receiver) = async_channel::bounded(1);
        seams.runtime.spawn(async move {
            let expiries =
                postio_session::reachability::token_expiries(seams.secrets.as_ref(), accounts)
                    .await;
            let _ = sender.send(expiries).await;
        });
        Box::pin(async move { receiver.recv().await.unwrap_or_default() })
    }

    fn update_credential(&self, account: AccountId, saved: Box<dyn Fn()>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let client = self.client.clone();
        glib::spawn_future_local(async move {
            let open_link = postio_widgets::present::onboarding::open_in_browser(&window);
            // POSTIO-GLIB-SAFE: reading the account is a client call, a
            // oneshot receive; the host answers on its own runtime.
            postio_widgets::present::settings::credential::update(
                &window, &client, account, open_link, saved,
            )
            .await;
        });
    }

    fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        if let Some(window) = self.window.upgrade() {
            window.offer_undo(description, undo);
        }
    }

    fn run(&self, command: postio_core::Command) {
        if let Some(window) = self.window.upgrade() {
            window.post_command(command);
        }
    }

    fn attachments_eager(&self) -> bool {
        self.seams
            .as_ref()
            .is_some_and(|seams| seams.attachments_eager)
    }
}

/// The reference window the dialog is sized for before it knows its own:
/// the references' 1440x900.
const REFERENCE_WINDOW: (i32, i32) = (1440, 900);

impl Settings {
    /// The commands Settings has a control for: its foot strip's "Open in
    /// $EDITOR".
    pub fn controls() -> Vec<postio_core::CommandId> {
        vec![postio_core::CommandId::EditConfig]
    }

    /// Take `keymap` as the keys in force: the foot strip's cap, the
    /// Keyboard section's rows, and what could not be bound.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap, problems: &[String]) {
        self.panel.set_keymap(keymap);
        self.panel.set_keymap_problems(problems);
    }
}

impl crate::window::FocusWindow {
    /// Settings, once it has been opened.
    pub fn settings(&self) -> Option<Rc<Settings>> {
        self.imp_settings().borrow().clone()
    }

    /// The settings dialog, while it is over the window.
    pub fn settings_dialog(&self) -> Option<adw::Dialog> {
        self.settings()
            .filter(|settings| settings.is_open())
            .map(|settings| settings.dialog().clone())
    }

    /// What Settings asks of the process that opened the store.
    pub fn set_settings_seams(&self, seams: Seams) {
        self.imp().settings_seams.replace(Some(seams));
    }

    /// Open `config.toml` with `open` rather than the person's editor: what a
    /// test records `mod+e` with (T235).
    pub fn set_editor(&self, open: impl Fn(&std::path::Path) + 'static) {
        self.imp().editor.replace(Some(Rc::new(open)));
    }

    /// `mod+comma` and the main menu's Settings: open it, or close it if it
    /// is open.
    pub(crate) fn toggle_settings(&self) {
        match self.settings().filter(|settings| settings.is_open()) {
            Some(settings) => settings.close(),
            None => self.open_settings(),
        }
    }

    /// Settings over the list, built on its first open.
    fn open_settings(&self) {
        let Some(client) = self.client() else {
            return;
        };
        let settings = self.settings().unwrap_or_else(|| {
            let outside = Rc::new(FocusOutside {
                window: self.downgrade(),
                seams: self.imp().settings_seams.borrow().clone(),
                client: client.clone(),
            });
            let settings = Settings::new(client, outside);
            settings.panel().connect_command({
                let window = self.downgrade();
                move |id| {
                    if let Some(window) = window.upgrade() {
                        window.act(id);
                    }
                }
            });
            if let Some(path) = self.config_path() {
                settings.panel().load(&path);
            }
            let keymap = self.keymap();
            let (_, problems) = crate::keys::resolver(&keymap);
            settings.set_keymap(&keymap, &problems);
            self.imp_settings().replace(Some(Rc::clone(&settings)));
            settings
        });
        settings.present(self.upcast_ref());
    }

    /// `mod+e`, from anywhere in Focus: `config.toml` in `$VISUAL` or
    /// `$EDITOR` (T235). With no file to open, the toast says so.
    pub(crate) fn edit_config(&self) {
        let Some(path) = self.config_path() else {
            self.say("There is no config.toml to edit");
            return;
        };
        let open = self.imp().editor.borrow().clone();
        match open {
            Some(open) => open(&path),
            None => postio_widgets::editor::open(&path),
        }
    }

    /// A key while Settings is up: Escape and `mod+comma` close it, `mod+e`
    /// opens the file, and everything else is its controls' -- typing in a
    /// field types.
    pub(crate) fn settings_key(&self, outcome: postio_ui::keymap::Outcome) -> glib::Propagation {
        use postio_core::CommandId;
        let postio_ui::keymap::Outcome::Command(id) = outcome else {
            return glib::Propagation::Proceed;
        };
        match id.parse::<CommandId>() {
            Ok(CommandId::Back | CommandId::Settings) => {
                if let Some(settings) = self.settings() {
                    settings.close();
                }
                glib::Propagation::Stop
            }
            Ok(CommandId::EditConfig) => {
                self.edit_config();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// Whether Settings' Keyboard section is waiting for a key to bind.
    pub(crate) fn settings_takes_every_key(&self) -> bool {
        self.settings()
            .is_some_and(|settings| settings.is_open() && settings.panel().is_capturing())
    }

    /// The file was saved and loads without error: what Settings' "Revert
    /// file" goes back to (T235).
    pub(crate) fn settings_note_known_good(&self, text: &str) {
        if let Some(settings) = self.settings() {
            settings.panel().note_known_good(text);
        }
    }

    /// Take `[compose]` as the file says now: the next reply or forward
    /// places its signature by it (T235).
    pub fn set_compose_config(&self, compose: postio_config::ComposeConfig) {
        if let Some(composer) = self.composer() {
            place_signatures(&composer, &compose);
        }
        self.imp().compose_config.replace(compose);
    }

    /// Take `[reader]` as the file says now: the open message reads at its
    /// zoom, and so does the next (T235).
    pub fn set_reader_config(&self, reader: &postio_config::ReaderConfig) {
        self.imp().zoom.set(Some(reader.zoom));
        if let Some(reading) = self.reading() {
            reading.reader().set_zoom(reader.zoom);
        }
    }

    fn imp_settings(&self) -> &RefCell<Option<Rc<Settings>>> {
        &self.imp().settings
    }

    fn client(&self) -> Option<Client> {
        self.imp().client.borrow().clone()
    }

    fn config_path(&self) -> Option<std::path::PathBuf> {
        self.imp().config_path.borrow().clone()
    }

    /// `description` on the toast, with an Undo that runs `undo`: a removed
    /// account's, which the global undo stack never holds.
    pub(crate) fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        match self.settings().filter(|settings| settings.is_open()) {
            Some(settings) => settings.offer_undo(description, undo),
            None => self.imp().toast.show_removable(description, undo),
        }
    }

    /// Hand `command` to the host, as the window's own verbs are.
    pub(crate) fn post_command(&self, command: postio_core::Command) {
        if let Some(client) = self.client() {
            let window = self.downgrade();
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: sending a command is a channel send; the
                // host answers on its own runtime.
                if let Err(error) = client.send(command).await {
                    tracing::warn!(%error, "Focus could not hand a settings command to the host");
                    if let Some(window) = window.upgrade() {
                        window.say("Focus could not change that");
                    }
                }
            });
        }
    }
}

/// Tell `composer` where a signature goes on a reply and on a forward, as
/// `compose` says.
pub(crate) fn place_signatures(
    composer: &postio_widgets::composer::Composer,
    compose: &postio_config::ComposeConfig,
) {
    let placement = |setting| match setting {
        postio_config::SignaturePlacement::AboveQuote => postio_body::Placement::AboveQuote,
        postio_config::SignaturePlacement::BelowQuote => postio_body::Placement::BelowQuote,
    };
    composer.set_signature_placement(
        placement(compose.signature_on_reply),
        placement(compose.signature_on_forward),
    );
}
