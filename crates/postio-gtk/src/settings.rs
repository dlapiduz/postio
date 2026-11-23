//! Settings in Focus: the shared `SettingsPanel` in a dialog over the list, in the message dialog's
//! frame.
//! What is Focus's is the frame and the `Outside` the presenters ask for what is not the host's to
//! answer.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_model::Account;
use postio_model::ids::AccountId;
use postio_ui::focus_dialog;
use postio_widgets::present::settings::accounts::Accounts;
use postio_widgets::present::settings::{Later, Outside, Reached, Reindexing};
use postio_widgets::settings::SettingsPanel;

pub const DIALOG_NAME: &str = "focus-settings";

#[derive(Clone)]
pub struct Seams {
    pub runtime: tokio::runtime::Handle,
    pub secrets: Arc<dyn postio_account::secret::SecretStore>,
    pub attachments_eager: bool,
    pub agent_activity: AgentActivity,
}

pub type AgentActivity = Arc<
    dyn Fn() -> std::pin::Pin<
            Box<dyn std::future::Future<Output = postio_ui::agent_access::Activity> + Send>,
        > + Send
        + Sync,
>;

/// Built on the first open and kept, so a section, a scroll position and a half-typed search
/// survive closing it.
pub struct Settings {
    dialog: adw::Dialog,
    panel: SettingsPanel,
    /// One over the window would be under the dialog's scrim, where a removal's Undo cannot be
    /// reached.
    toast: postio_widgets::widgets::toast::Toast,
    accounts: RefCell<Option<Accounts>>,
    open: Cell<bool>,
    following: Cell<bool>,
}

impl Settings {
    pub fn new(client: Client, outside: Rc<dyn Outside>) -> Rc<Settings> {
        let panel = SettingsPanel::new();
        // The message dialog is at most 820 wide, so a pane is at most 606: too narrow for two
        // columns side by side.
        panel.set_narrow(true);
        panel.set_vexpand(true);

        let search = panel.search_field();
        search.set_valign(gtk::Align::Center);
        search.add_css_class("focus-settings-search");
        search.set_width_chars(16);
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
                    // Hidden while closed: the presenters read what costs only while it shows.
                    settings.panel.set_visible(false);
                }
            }
        });

        postio_widgets::present::settings::egress::install(&settings.panel, client.clone());
        postio_widgets::present::settings::backfill::install(&settings.panel, client.clone());
        glib::spawn_future_local({
            let settings = Rc::downgrade(&settings);
            async move {
                let Some(panel) = settings.upgrade().map(|settings| settings.panel.clone()) else {
                    return;
                };
                // POSTIO-GLIB-SAFE: client calls, oneshot receives.
                postio_widgets::present::settings::privacy::install(&panel, client.clone()).await;
                let installing = Accounts::install(&panel, client, outside, Reindexing::default());
                // POSTIO-GLIB-SAFE: client calls, oneshot receives.
                let accounts = installing.await;
                if let Some(settings) = settings.upgrade() {
                    settings.accounts.replace(Some(accounts));
                }
            }
        });
        settings
    }

    pub fn panel(&self) -> &SettingsPanel {
        &self.panel
    }

    pub fn dialog(&self) -> &adw::Dialog {
        &self.dialog
    }

    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    pub fn present(self: &Rc<Self>, window: &gtk::Window) {
        if self.open.replace(true) {
            return;
        }
        self.follow(window);
        // Read fresh on every open, so what a reader last wrote to the remote-image allow list
        // shows without watching the file.
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

    pub fn close(&self) {
        if self.open.get() {
            self.dialog.close();
        }
    }

    pub fn undo(&self) -> bool {
        self.toast.activate_undo()
    }

    pub fn set_undo_key(&self, key: Option<String>) {
        self.toast.set_undo_key(key);
    }

    pub fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        self.toast.show_removable(description, undo);
    }

    pub fn hear(&self, event: &postio_core::Event) {
        if let Some(accounts) = self.accounts.borrow().as_ref() {
            accounts.hear(event);
        }
    }

    pub fn refresh_accounts(&self) {
        let accounts = self.accounts.borrow().clone();
        if let Some(accounts) = accounts {
            glib::spawn_future_local(async move {
                // POSTIO-GLIB-SAFE: client calls, oneshot receives.
                accounts.refresh().await
            });
        }
    }
}

pub struct FocusOutside {
    pub window: glib::WeakRef<crate::window::FocusWindow>,
    pub seams: Option<Seams>,
    pub client: Client,
}

impl Outside for FocusOutside {
    fn test_connection(&self, account: Account) -> Later<Reached> {
        let Some(seams) = self.seams.clone() else {
            let refused = || Err("this window opened no store to test from".to_owned());
            return Box::pin(std::future::ready(Reached {
                incoming: refused(),
                outgoing: refused(),
                trouble: None,
            }));
        };
        let (sender, receiver) = async_channel::bounded(1);
        seams.runtime.spawn(async move {
            let found = postio_session::reachability::test_over_tls(&account, &seams.secrets).await;
            let trouble = found.trouble();
            let (incoming, outgoing) = found.into_results();
            let _ = sender
                .send(Reached {
                    incoming,
                    outgoing,
                    trouble,
                })
                .await;
        });
        Box::pin(async move {
            receiver.recv().await.unwrap_or_else(|_| {
                let died = || Err("the test did not finish".to_owned());
                Reached {
                    incoming: died(),
                    outgoing: died(),
                    trouble: None,
                }
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
            let updating = postio_widgets::present::settings::credential::update(
                &window, &client, account, open_link, saved,
            );
            // POSTIO-GLIB-SAFE: reading the account is a client call, a
            // oneshot receive; the host answers on its own runtime.
            updating.await;
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

const REFERENCE_WINDOW: (i32, i32) = (1440, 900);

impl Settings {
    pub fn controls() -> Vec<postio_core::CommandId> {
        use postio_core::CommandId::*;
        vec![
            EditConfig,
            ToggleAccountEnabled,
            RemoveAccount,
            RebuildAccountIndex,
            SetDefaultAccount,
            MapMailboxRole,
        ]
    }

    pub fn set_keymap(&self, keymap: &postio_core::Keymap, problems: &[String]) {
        self.panel.set_keymap(keymap);
        self.panel.set_keymap_problems(problems);
    }
}

impl crate::window::FocusWindow {
    pub fn settings(&self) -> Option<Rc<Settings>> {
        self.imp_settings().borrow().clone()
    }

    pub fn settings_dialog(&self) -> Option<adw::Dialog> {
        self.settings()
            .filter(|settings| settings.is_open())
            .map(|settings| settings.dialog().clone())
    }

    pub fn set_settings_seams(&self, seams: Seams) {
        self.imp().settings_seams.replace(Some(seams));
    }

    pub fn set_editor(&self, open: impl Fn(&std::path::Path) + 'static) {
        self.imp().editor.replace(Some(Rc::new(open)));
    }

    pub(crate) fn toggle_settings(&self) {
        match self.settings().filter(|settings| settings.is_open()) {
            Some(settings) => settings.close(),
            None => self.open_settings(),
        }
    }

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
            let for_take_back = client.clone();
            let runtime = self.imp().runtime.borrow().clone();
            let settings = Settings::new(client, outside);
            // A person's pin or marker is theirs, not the buffer's: the host takes it back, and the
            // window hears of it as of any change.
            settings.panel().connect_contact_take_back(move |undo| {
                let client = for_take_back.clone();
                let (entry, kind) = match undo {
                    postio_ui::filtering::Undo::Never(entry) => (entry.clone(), None),
                    postio_ui::filtering::Undo::Marker { sender, kind } => {
                        (sender.clone(), Some(kind.clone()))
                    }
                };
                let Some(runtime) = runtime.clone() else {
                    return;
                };
                runtime.spawn(async move {
                    if let Err(error) = client.take_back_filter(&entry, kind.as_deref()).await {
                        tracing::warn!(%error, "could not take a filter entry back");
                    }
                });
            });
            settings.panel().connect_command({
                let window = self.downgrade();
                move |id| {
                    let Some(window) = window.upgrade() else {
                        return;
                    };
                    // A command that goes somewhere in the list goes there, not behind the dialog.
                    if id == postio_core::CommandId::GoToFiltered
                        && let Some(settings) = window.settings()
                    {
                        settings.close();
                    }
                    window.act(id);
                }
            });
            settings.panel().set_agent_entry(
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| {
                        postio_session::agent_access::entry_point(
                            &exe,
                            std::env::var("FLATPAK_ID").ok().as_deref(),
                            std::path::Path::exists,
                        )
                    })
                    .as_deref(),
            );
            if let Some(seams) = self.imp().settings_seams.borrow().clone() {
                let panel = settings.panel().downgrade();
                settings.panel().connect_agents_opened(move || {
                    let (sender, receiver) = async_channel::bounded(1);
                    let reading = (seams.agent_activity)();
                    seams.runtime.spawn(async move {
                        let _ = sender.send(reading.await).await;
                    });
                    let panel = panel.clone();
                    glib::spawn_future_local(async move {
                        // POSTIO-GLIB-SAFE: a channel receive; the log is read
                        // on the host's runtime.
                        if let Ok(activity) = receiver.recv().await
                            && let Some(panel) = panel.upgrade()
                        {
                            panel.set_agent_activity(activity);
                        }
                    });
                });
            }
            settings.panel().connect_contacts(
                {
                    let window = self.downgrade();
                    move || {
                        if let Some(window) = window.upgrade()
                            && let Some(pane) = window.pane()
                        {
                            pane.feed()
                                .input(postio_focus::Input::ContactsSettingsOpened);
                        }
                    }
                },
                {
                    let window = self.downgrade();
                    move |index| {
                        if let Some(window) = window.upgrade()
                            && let Some(pane) = window.pane()
                        {
                            pane.feed()
                                .input(postio_focus::Input::ContactsSuggestion(index));
                        }
                    }
                },
            );
            if let Some(path) = self.config_path() {
                settings.panel().load(&path);
            }
            let keymap = self.keymap();
            let (_, problems) = crate::keys::resolver(&keymap);
            settings.set_keymap(&keymap, &problems);
            // Otherwise the controller hears of it only at the next key (#2218).
            settings.dialog().connect_closed({
                let window = self.downgrade();
                move |_| {
                    if let Some(window) = window.upgrade() {
                        window.sync_surfaces();
                    }
                }
            });
            self.imp_settings().replace(Some(Rc::clone(&settings)));
            settings
        });
        settings.panel().set_filtered_today(self.filtered_today());
        settings
            .panel()
            .set_contact_rules(self.imp().focus_config.borrow().contacts().to_vec());
        settings.present(self.upcast_ref());
        self.sync_surfaces();
    }

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

    /// Escape and `mod+comma` close Settings (the controller's); everything else is its controls':
    /// typing in a field types.
    pub(crate) fn settings_key(&self, outcome: postio_ui::keymap::Outcome) -> glib::Propagation {
        use postio_core::CommandId;
        let postio_ui::keymap::Outcome::Command(id) = outcome else {
            return glib::Propagation::Proceed;
        };
        match id.parse::<CommandId>() {
            Ok(CommandId::EditConfig) => {
                self.edit_config();
                glib::Propagation::Stop
            }
            // Resolved only while the keyboard is on an account row (`key_context`).
            Ok(
                verb @ (CommandId::ToggleAccountEnabled
                | CommandId::RemoveAccount
                | CommandId::RebuildAccountIndex
                | CommandId::SetDefaultAccount
                | CommandId::MapMailboxRole),
            ) => {
                self.account_verb(verb);
                glib::Propagation::Stop
            }
            Ok(CommandId::Undo) => {
                let undone = self.settings().is_some_and(|settings| settings.undo());
                if undone {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            _ => glib::Propagation::Proceed,
        }
    }

    /// With no row focused it does nothing: the first account is never a guess at whose mail to
    /// remove.
    pub(crate) fn account_verb(&self, verb: postio_core::CommandId) {
        use postio_core::CommandId;
        use postio_widgets::settings::AccountAction;
        let Some(settings) = self.settings().filter(|settings| settings.is_open()) else {
            self.open_settings();
            return;
        };
        let panel = settings.panel();
        let Some(account) = panel.focused_account() else {
            return;
        };
        match verb {
            CommandId::ToggleAccountEnabled => {
                panel.toggle_account_enabled(account);
            }
            CommandId::RemoveAccount => {
                panel.request_account_action(account, AccountAction::Remove)
            }
            CommandId::RebuildAccountIndex => {
                panel.request_account_action(account, AccountAction::RebuildIndex)
            }
            CommandId::SetDefaultAccount => {
                panel.request_account_action(account, AccountAction::SetDefault)
            }
            CommandId::MapMailboxRole => panel.open_account_detail(account),
            _ => {}
        }
    }

    pub(crate) fn settings_on_an_account(&self) -> bool {
        self.settings().is_some_and(|settings| {
            settings.is_open() && settings.panel().focused_account().is_some()
        })
    }

    pub(crate) fn settings_takes_every_key(&self) -> bool {
        self.settings()
            .is_some_and(|settings| settings.is_open() && settings.panel().is_capturing())
    }

    /// What Settings' "Revert file" goes back to.
    pub(crate) fn settings_note_known_good(&self, text: &str) {
        if let Some(settings) = self.settings() {
            settings.panel().note_known_good(text);
        }
    }

    pub fn set_compose_config(&self, compose: postio_config::ComposeConfig) {
        if let Some(composer) = self.composer() {
            place_signatures(&composer, &compose);
        }
        self.imp().compose_config.replace(compose);
    }

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

    pub(crate) fn config_path(&self) -> Option<std::path::PathBuf> {
        self.imp().config_path.borrow().clone()
    }

    /// A removed account's, which the global undo stack never holds.
    pub(crate) fn offer_undo(&self, description: &str, undo: Box<dyn Fn()>) {
        self.teach_undo_key();
        match self.settings().filter(|settings| settings.is_open()) {
            Some(settings) => settings.offer_undo(description, undo),
            None => self.imp().toast.show_removable(description, undo),
        }
    }

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
