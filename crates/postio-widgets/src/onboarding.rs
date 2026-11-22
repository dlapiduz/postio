//! First run: an address and a password, then how far back to sync. It does not probe, connect or
//! write: `postio-widgets`
//! may not link `io-imap` or the store, so [`Onboarding::connect_probe`] and
//! [`Onboarding::connect_submit`] are the seams.
//! The probe runs when the address is committed, not per keystroke: it contacts servers the user
//! has not named yet.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
pub use postio_ui::onboarding::{
    BrowserSignIn, OAuthClientSubmission, Server, Settings, Status, Submission, SyncWindow,
    domain_of, looks_like_an_address,
};
use postio_ui::onboarding::{plain_scope, step_of};

fn scope_line(icon: &str, text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mark = gtk::Image::from_icon_name(icon);
    mark.add_css_class("postio-onboarding-scope-mark");
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.add_css_class("postio-onboarding-scope");
    row.append(&mark);
    row.append(&label);
    row
}

const PORT_CHARS: i32 = 6;

type SubmitHandler = Box<dyn Fn(&Submission)>;
type ProbeHandler = Box<dyn Fn(&str)>;
type StartSyncHandler = Box<dyn Fn(SyncWindow)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Onboarding {
        pub(super) name: gtk::Entry,
        pub(super) address: gtk::Entry,
        pub(super) password: gtk::PasswordEntry,
        pub(super) password_row: std::cell::OnceCell<gtk::Box>,
        pub(super) oauth_client_id: gtk::Entry,
        pub(super) oauth_client_secret: gtk::PasswordEntry,
        pub(super) oauth_rows: std::cell::OnceCell<gtk::Box>,
        pub(super) cancel_sign_in: gtk::Button,
        pub(super) on_cancel_sign_in: RefCell<Vec<Box<dyn Fn()>>>,
        pub(super) card: gtk::Box,
        pub(super) card_heading: gtk::Label,
        pub(super) card_icon: gtk::Image,
        pub(super) imap_line: gtk::Label,
        pub(super) smtp_line: gtk::Label,
        pub(super) auth_line: gtk::Label,
        pub(super) note: gtk::Label,
        pub(super) help: gtk::LinkButton,
        pub(super) manual: gtk::Box,
        pub(super) imap_host: gtk::Entry,
        pub(super) imap_port: gtk::Entry,
        pub(super) smtp_host: gtk::Entry,
        pub(super) smtp_port: gtk::Entry,
        pub(super) login: gtk::Entry,
        pub(super) connect: gtk::Button,
        /// A handle of its own because `set_label` would replace the button's child, which is the
        /// word and its `Ret` key hint.
        pub(super) connect_label: gtk::Label,
        pub(super) edit: gtk::Button,
        pub(super) buttons: gtk::Box,
        pub(super) step: gtk::Label,
        /// The name and address fields as one group, so the last step can put the credential form
        /// away.
        pub(super) identity_rows: gtk::Box,
        pub(super) browser_box: gtk::Box,
        pub(super) browser_flow: gtk::Label,
        pub(super) browser_scopes: gtk::Box,
        pub(super) browser_copy: gtk::Button,
        pub(super) browser_reopen: gtk::Button,
        pub(super) browser_sign_in: RefCell<BrowserSignIn>,
        pub(super) sync_window_box: gtk::Box,
        pub(super) sync_window_picker: std::cell::OnceCell<crate::widgets::SegmentedControl>,
        pub(super) sync_estimate: gtk::Label,
        pub(super) start_sync: gtk::Button,
        pub(super) on_start_sync: RefCell<Vec<StartSyncHandler>>,
        pub(super) status_line: gtk::Label,
        pub(super) submit_hint: gtk::Label,
        pub(super) status: RefCell<Status>,
        /// Kept across `Connecting`, `Failed` and `Saved`: a failure that wiped the card would hide
        /// what it was connecting to.
        pub(super) shown: RefCell<Option<Settings>>,
        /// Set while the widget writes its own fields, so filling the form from a probe does not
        /// read as the user editing.
        pub(super) echoing: Cell<bool>,
        pub(super) on_probe: RefCell<Vec<ProbeHandler>>,
        pub(super) on_submit: RefCell<Vec<SubmitHandler>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Onboarding {
        const NAME: &'static str = "PostioOnboarding";
        type Type = super::Onboarding;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Onboarding {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Onboarding {}
    impl BinImpl for Onboarding {}
}

glib::wrapper! {
    pub struct Onboarding(ObjectSubclass<imp::Onboarding>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Onboarding {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Onboarding {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn name(&self) -> String {
        self.imp().name.text().trim().to_owned()
    }

    pub fn address(&self) -> String {
        self.imp().address.text().trim().to_owned()
    }

    pub fn set_address(&self, address: &str) {
        self.imp().address.set_text(address);
    }

    pub fn set_name(&self, name: &str) {
        self.imp().name.set_text(name);
    }

    /// Read once, on submit, and never stored here.
    pub fn password(&self) -> String {
        self.imp().password.text().to_string()
    }

    pub fn status(&self) -> Status {
        self.imp().status.borrow().clone()
    }

    pub fn set_browser_sign_in(&self, details: BrowserSignIn) {
        *self.imp().browser_sign_in.borrow_mut() = details;
        self.render_browser_sign_in();
    }

    fn render_browser_sign_in(&self) {
        let imp = self.imp();
        let details = imp.browser_sign_in.borrow().clone();

        // The provider line sits here, not in a title bar: this widget is hosted in dialogs and in
        // the first-run screen, and
        // only one of those has a title bar.
        let mut lines = Vec::new();
        if !details.provider.is_empty() {
            lines.push(format!("signing in to {}", details.provider));
        }
        if !details.redirect_uri.is_empty() {
            lines.push(format!("listening {}", details.redirect_uri));
        }
        lines.push("authorization_code + PKCE".to_owned());
        imp.browser_flow.set_text(&lines.join("\n"));

        while let Some(child) = imp.browser_scopes.first_child() {
            imp.browser_scopes.remove(&child);
        }
        for scope in &details.scopes {
            imp.browser_scopes
                .append(&scope_line("object-select-symbolic", &plain_scope(scope)));
        }
        // Said out loud because the absence is the point: every scope in the preset table is a mail
        // scope, and a consenting
        // user may assume otherwise.
        imp.browser_scopes.append(&scope_line(
            "window-close-symbolic",
            "Not requested: contacts, calendar, files",
        ));

        let has_url = !details.authorize_url.is_empty();
        imp.browser_copy.set_visible(has_url);
        imp.browser_reopen.set_visible(has_url);
    }

    fn reopen_sign_in_link(&self) {
        let url = self.imp().browser_sign_in.borrow().authorize_url.clone();
        if url.is_empty() {
            return;
        }
        // POSTIO-CONSENT: opened only from the `Open link again` button on
        // the browser sign-in step, which is only on screen because the
        // user already asked for a browser sign-in and the browser is
        // already out. One press, one open — never on render, never
        // retried on its own, and the URL is the very one `authorize`
        // handed the opener a moment ago rather than a second one built
        // here. See ADR 0006 Q3 and CLAUDE.md, "Privacy".
        gtk::UriLauncher::new(&url).launch(
            self.root().and_downcast_ref::<gtk::Window>(),
            gtk::gio::Cancellable::NONE,
            |result| {
                if let Err(error) = result {
                    tracing::warn!(%error, "could not reopen the sign-in link");
                }
            },
        );
    }

    fn copy_sign_in_link(&self) {
        let url = self.imp().browser_sign_in.borrow().authorize_url.clone();
        if url.is_empty() {
            return;
        }
        self.clipboard().set_text(&url);
    }

    pub fn set_status(&self, status: Status) {
        let imp = self.imp();
        if let Status::Found(settings)
        | Status::Reauthenticate(settings)
        | Status::Manual {
            suggestion: Some(settings),
        } = &status
        {
            self.fill_manual(settings);
            *imp.shown.borrow_mut() = Some(settings.clone());
        }
        if matches!(status, Status::Idle) {
            imp.shown.borrow_mut().take();
        }
        *imp.status.borrow_mut() = status;
        self.render();
    }

    pub fn settings(&self) -> Settings {
        let imp = self.imp();
        let port =
            |entry: &gtk::Entry, fallback: u16| entry.text().trim().parse().unwrap_or(fallback);
        let found = match &*imp.status.borrow() {
            Status::Found(settings) | Status::Reauthenticate(settings) => Some(settings.clone()),
            Status::Manual {
                suggestion: Some(settings),
            } => Some(settings.clone()),
            _ => None,
        };
        Settings {
            imap: Server {
                host: imp.imap_host.text().trim().to_owned(),
                port: port(&imp.imap_port, 993),
                // The manual form has no security selector: a probe's value carries through,
                // hand-entered servers get TLS.
                security: found
                    .as_ref()
                    .map(|settings| settings.imap.security)
                    .unwrap_or_default(),
            },
            smtp: Server {
                host: imp.smtp_host.text().trim().to_owned(),
                port: port(&imp.smtp_port, 465),
                security: found
                    .as_ref()
                    .map(|settings| settings.smtp.security)
                    .unwrap_or_default(),
            },
            login: match imp.login.text().trim() {
                "" => self.address(),
                login => login.to_owned(),
            },
            requires_app_password: found
                .as_ref()
                .is_some_and(|settings| settings.requires_app_password),
            note: found.as_ref().and_then(|settings| settings.note.clone()),
            help_url: found
                .as_ref()
                .and_then(|settings| settings.help_url.clone()),
            oauth_sign_in: found
                .as_ref()
                .is_some_and(|settings| settings.oauth_sign_in),
            source: found
                .map(|settings| settings.source)
                .unwrap_or_else(|| "entered by hand".to_owned()),
        }
    }

    pub fn show_manual(&self, show: bool) {
        self.imp().manual.set_visible(show);
        self.imp().edit.set_label(if show {
            "Hide server settings"
        } else {
            "Edit manually"
        });
    }

    pub fn manual_shown(&self) -> bool {
        self.imp().manual.is_visible()
    }

    /// The name is the first field; the address was until #603 added the name above it.
    pub fn focus_name(&self) {
        self.imp().name.grab_focus();
    }

    /// A field grabs the keyboard only while mapped, and the dialog gives its first focusable
    /// widget (the close X) the
    /// keyboard as it opens. Nothing moves when every field shown is filled.
    pub fn focus_first_empty(&self) {
        let imp = self.imp();
        let fields: [&gtk::Widget; 10] = [
            imp.name.upcast_ref(),
            imp.address.upcast_ref(),
            imp.password.upcast_ref(),
            imp.oauth_client_id.upcast_ref(),
            imp.oauth_client_secret.upcast_ref(),
            imp.imap_host.upcast_ref(),
            imp.imap_port.upcast_ref(),
            imp.smtp_host.upcast_ref(),
            imp.smtp_port.upcast_ref(),
            imp.login.upcast_ref(),
        ];
        let empty = fields.into_iter().find(|field| {
            field.is_mapped()
                && field.is_sensitive()
                && field
                    .dynamic_cast_ref::<gtk::Editable>()
                    .is_some_and(|editable| editable.text().is_empty())
        });
        if let Some(field) = empty {
            field.grab_focus();
        } else if imp.cancel_sign_in.is_mapped() && imp.cancel_sign_in.is_sensitive() {
            imp.cancel_sign_in.grab_focus();
        }
    }

    pub fn focus_address(&self) {
        self.imp().address.grab_focus();
    }

    /// [`Status::Reauthenticate`] arrives with the address filled in and one field still empty.
    pub fn focus_password(&self) {
        self.imp().password.grab_focus();
    }

    pub fn connect_probe(&self, handler: impl Fn(&str) + 'static) {
        self.imp().on_probe.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_submit(&self, handler: impl Fn(&Submission) + 'static) {
        self.imp().on_submit.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_start_sync(&self, handler: impl Fn(SyncWindow) + 'static) {
        self.imp()
            .on_start_sync
            .borrow_mut()
            .push(Box::new(handler));
    }

    pub fn sync_window(&self) -> SyncWindow {
        SyncWindow::ALL
            .get(
                self.imp()
                    .sync_window_picker
                    .get()
                    .and_then(|picker| picker.selected())
                    .unwrap_or(0),
            )
            .copied()
            .unwrap_or_default()
    }

    pub fn start_sync(&self) {
        if !matches!(self.status(), Status::SyncWindow) {
            return;
        }
        let window = self.sync_window();
        for handler in self.imp().on_start_sync.borrow().iter() {
            handler(window);
        }
    }

    /// Public so a test can drive it: GTK4 has no supported way to synthesize a focus change.
    pub fn probe(&self) {
        let address = self.address();
        if !looks_like_an_address(&address) || self.status().is_busy() {
            return;
        }
        for handler in self.imp().on_probe.borrow().iter() {
            handler(&address);
        }
    }

    fn oauth_mode(&self) -> bool {
        self.imp()
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|settings| settings.oauth_sign_in)
    }

    pub fn cancel_sign_in(&self) {
        for handler in self.imp().on_cancel_sign_in.borrow().iter() {
            handler();
        }
    }

    pub fn connect_cancel_sign_in(&self, handler: impl Fn() + 'static) {
        self.imp()
            .on_cancel_sign_in
            .borrow_mut()
            .push(Box::new(handler));
    }

    pub fn submit(&self) {
        if !self.can_submit() {
            return;
        }
        let oauth_client = self.oauth_mode().then(|| {
            let secret = self.imp().oauth_client_secret.text().to_string();
            OAuthClientSubmission {
                client_id: self.imp().oauth_client_id.text().trim().to_string(),
                client_secret: (!secret.is_empty()).then_some(secret),
            }
        });
        let submission = Submission {
            address: self.address(),
            name: self.name(),
            password: self.password(),
            settings: self.settings(),
            oauth_client,
        };
        for handler in self.imp().on_submit.borrow().iter() {
            handler(&submission);
        }
    }

    pub fn can_submit(&self) -> bool {
        let settings = self.settings();
        let credential = if self.oauth_mode() {
            !self.imp().oauth_client_id.text().trim().is_empty()
        } else {
            !self.password().is_empty()
        };
        looks_like_an_address(&self.address())
            && credential
            && !settings.imap.host.is_empty()
            && !settings.smtp.host.is_empty()
            && !self.status().is_busy()
    }

    #[doc(hidden)]
    pub fn test_submit_hint(&self) -> Option<String> {
        let hint = &self.imp().submit_hint;
        hint.is_visible().then(|| hint.text().to_string())
    }

    #[doc(hidden)]
    pub fn test_set_oauth_client_id(&self, id: &str) {
        self.imp().oauth_client_id.set_text(id);
    }

    #[doc(hidden)]
    pub fn test_set_password(&self, password: &str) {
        self.imp().password.set_text(password);
    }

    #[doc(hidden)]
    pub fn test_activate_manual_fields(&self) {
        let imp = self.imp();
        for entry in [
            &imp.imap_host,
            &imp.imap_port,
            &imp.smtp_host,
            &imp.smtp_port,
            &imp.login,
        ] {
            entry.emit_activate();
        }
    }

    #[doc(hidden)]
    pub fn test_activate_name(&self) {
        self.imp().name.emit_activate();
    }

    #[doc(hidden)]
    pub fn test_activate_address(&self) {
        self.imp().address.emit_activate();
    }

    #[doc(hidden)]
    pub fn test_activate_password(&self) {
        self.imp().password.emit_activate();
    }

    /// Walks up from the toplevel's focus widget: a `gtk::Entry`'s real focus is an internal
    /// `GtkText`, so `is_focus` on the
    /// entry is always false, and `has_focus` needs an active toplevel, which a headless window
    /// never is.
    #[doc(hidden)]
    pub fn test_name_has_focus(&self) -> bool {
        self.root()
            .and_downcast::<gtk::Window>()
            .and_then(|window| gtk::prelude::RootExt::focus(&window))
            .is_some_and(|focus| {
                focus.is_ancestor(&self.imp().name)
                    || focus == self.imp().name.clone().upcast::<gtk::Widget>()
            })
    }

    pub fn test_address_has_focus(&self) -> bool {
        self.root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .and_then(|window| gtk::prelude::RootExt::focus(&window))
            .is_some_and(|focus| {
                focus.is_ancestor(&self.imp().address)
                    || focus == self.imp().address.clone().upcast::<gtk::Widget>()
            })
    }

    #[doc(hidden)]
    pub fn test_select_sync_window(&self, window: SyncWindow) {
        if let (Some(picker), Some(index)) = (
            self.imp().sync_window_picker.get(),
            SyncWindow::ALL
                .iter()
                .position(|candidate| *candidate == window),
        ) {
            picker.test_press(index);
        }
    }

    #[doc(hidden)]
    pub fn test_sync_window_shown(&self) -> bool {
        self.imp().sync_window_box.is_visible()
    }

    #[doc(hidden)]
    pub fn test_sync_estimate(&self) -> String {
        self.imp().sync_estimate.text().to_string()
    }

    fn fill_manual(&self, settings: &Settings) {
        let imp = self.imp();
        imp.echoing.set(true);
        imp.imap_host.set_text(&settings.imap.host);
        imp.imap_port.set_text(&settings.imap.port.to_string());
        imp.smtp_host.set_text(&settings.smtp.host);
        imp.smtp_port.set_text(&settings.smtp.port.to_string());
        imp.login.set_text(&settings.login);
        imp.echoing.set(false);
    }

    fn render(&self) {
        let imp = self.imp();
        let status = imp.status.borrow().clone();

        let (card, heading, icon) = match &status {
            Status::Idle => (false, String::new(), ""),
            Status::Probing => (
                true,
                match domain_of(&self.address()).as_str() {
                    "" => "Looking for settings…".to_owned(),
                    domain => format!("Looking for settings for {domain}…"),
                },
                "content-loading-symbolic",
            ),
            Status::Found(settings) => (
                true,
                match domain_of(&self.address()).as_str() {
                    // Before an address is typed there is no domain to name.
                    "" => format!("Found settings — {}", settings.source),
                    domain => format!("Found settings for {domain} — {}", settings.source),
                },
                "object-select-symbolic",
            ),
            Status::Manual { .. } => (
                true,
                match domain_of(&self.address()).as_str() {
                    "" => "No settings found. Fill them in below.".to_owned(),
                    domain => format!("No published settings for {domain}. Fill them in below."),
                },
                "dialog-information-symbolic",
            ),
            Status::Connecting => (
                true,
                match imp.shown.borrow().as_ref() {
                    Some(settings) => format!("Signing in to {}…", settings.imap.host),
                    None => "Testing the connection…".to_owned(),
                },
                "content-loading-symbolic",
            ),
            Status::Failed(_) => (
                true,
                match imp.shown.borrow().as_ref() {
                    Some(settings) => format!("Could not sign in to {}", settings.imap.host),
                    None => "That did not work".to_owned(),
                },
                "dialog-warning-symbolic",
            ),
            Status::Reauthenticate(_) => (
                true,
                match domain_of(&self.address()).as_str() {
                    "" => "Sign in again".to_owned(),
                    domain => format!("Sign in again to {domain}"),
                },
                "dialog-password-symbolic",
            ),
            Status::WaitingForBrowser => (
                true,
                "Waiting for your browser…".to_owned(),
                "content-loading-symbolic",
            ),
            Status::SyncWindow | Status::Saved => {
                (true, "Account added".to_owned(), "object-select-symbolic")
            }
        };
        imp.card.set_visible(card);
        imp.card_heading.set_text(&heading);
        imp.card_icon.set_icon_name(Some(icon));

        // From the last settings given, not the current status, so they survive a failure.
        let settings = imp.shown.borrow().clone();
        let has_lines = settings.is_some();
        for line in [&imp.imap_line, &imp.smtp_line, &imp.auth_line] {
            line.set_visible(has_lines);
        }
        if let Some(settings) = &settings {
            imp.imap_line
                .set_text(&format!("IMAP   {}", settings.imap.line()));
            imp.smtp_line
                .set_text(&format!("SMTP   {}", settings.smtp.line()));
            imp.auth_line.set_text(if settings.oauth_sign_in {
                "Auth   OAuth 2 · tokens in the system keyring"
            } else {
                "Auth   plain · secret in the system keyring"
            });
        }

        let oauth = self.oauth_mode();
        let waiting = matches!(status, Status::WaitingForBrowser);
        let settled = matches!(status, Status::SyncWindow | Status::Saved);
        let asking = !waiting && !settled;
        imp.identity_rows.set_visible(asking);
        if let Some(row) = imp.password_row.get() {
            row.set_visible(!oauth && asking);
        }
        if let Some(rows) = imp.oauth_rows.get() {
            rows.set_visible(oauth && asking);
        }
        imp.manual.set_visible(imp.manual.is_visible() && asking);
        imp.cancel_sign_in.set_visible(waiting);
        imp.browser_box.set_visible(waiting);
        if waiting {
            let screen = self.downgrade();
            glib::idle_add_local_once(move || {
                if let Some(screen) = screen.upgrade() {
                    screen.focus_first_empty();
                }
            });
        }

        // iCloud's is why this exists: an Apple ID password will not work, and a user not told that
        // types theirs and is told it
        // is wrong.
        let note = settings.as_ref().and_then(|s| s.note.clone());
        imp.note.set_visible(note.is_some());
        imp.note.set_text(note.as_deref().unwrap_or_default());
        let help = settings.as_ref().and_then(|s| s.help_url.clone());
        imp.help.set_visible(help.is_some());
        if let Some(help) = &help {
            imp.help.set_uri(help);
            imp.help.set_label("Make an app-specific password");
        }

        if matches!(status, Status::Manual { .. }) && !self.manual_shown() {
            self.show_manual(true);
        }

        let busy = status.is_busy();
        imp.name.set_sensitive(!busy);
        imp.address.set_sensitive(!busy);
        imp.password.set_sensitive(!busy);
        imp.connect.set_sensitive(self.can_submit());
        let servers_known = settings
            .as_ref()
            .is_some_and(|s| !s.imap.host.is_empty() && !s.smtp.host.is_empty());
        let credential_given = if oauth {
            !imp.oauth_client_id.text().trim().is_empty()
        } else {
            !self.password().is_empty()
        };
        let blocker = postio_ui::onboarding::submit_blocker(
            &self.address(),
            oauth,
            credential_given,
            servers_known,
        )
        .filter(|_| asking && !busy);
        imp.submit_hint.set_visible(blocker.is_some());
        imp.submit_hint.set_text(blocker.unwrap_or_default());
        imp.connect_label.set_text(match (&status, oauth) {
            (Status::Connecting, _) => "Connecting…",
            (Status::WaitingForBrowser, _) => "Waiting…",
            (_, true) => "Sign in with your browser",
            (_, false) => "Connect",
        });

        imp.status_line.set_visible(status.message().is_some());
        if let Some(message) = status.message() {
            imp.status_line.set_text(message);
        }

        let on_sync_window = matches!(status, Status::SyncWindow);
        imp.buttons.set_visible(!on_sync_window);
        let arriving = on_sync_window && !imp.sync_window_box.is_visible();
        imp.sync_window_box.set_visible(on_sync_window);
        if arriving {
            let start = imp.start_sync.clone();
            glib::idle_add_local_once(move || {
                start.grab_focus();
            });
        }
        imp.sync_estimate.set_text(&self.sync_window().estimate());
        imp.step.set_text(step_of(&status));

        self.update_property(&[gtk::accessible::Property::Label(&heading)]);
    }

    fn build(&self) {
        let imp = self.imp();
        crate::widgets::plate::dress(self, "postio-onboarding", "Add account");

        imp.step.set_text(step_of(&Status::Idle));
        imp.step.add_css_class("postio-onboarding-step");
        imp.step
            .set_accessible_role(gtk::AccessibleRole::Presentation);

        let header = crate::widgets::plate::header("postio-onboarding", "Add account");
        header
            .first_child()
            .inspect(|title| title.set_hexpand(true));
        header.append(&imp.step);
        let close = crate::widgets::close_button();
        close.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| {
                if let Some(dialog) = screen.ancestor(adw::Dialog::static_type()) {
                    dialog
                        .downcast_ref::<adw::Dialog>()
                        .map(|dialog| dialog.close());
                }
            }
        ));
        header.append(&close);

        imp.name.set_placeholder_text(Some("Ada Lovelace"));
        imp.name.set_hexpand(true);
        // Nothing to probe or submit with only a name: Return moves on to the next field (#629).
        imp.name.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.focus_address()
        ));

        imp.address.set_placeholder_text(Some("you@example.com"));
        imp.address.set_input_purpose(gtk::InputPurpose::Email);
        imp.address.set_hexpand(true);
        imp.address.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.probe()
        ));
        let leaving = gtk::EventControllerFocus::new();
        leaving.connect_leave(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.probe()
        ));
        imp.address.add_controller(leaving);
        imp.address.connect_changed(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.render()
        ));

        imp.password.set_show_peek_icon(true);
        imp.password.set_hexpand(true);
        imp.password.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.submit()
        ));
        imp.password.connect_changed(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.render()
        ));

        imp.oauth_client_id
            .set_placeholder_text(Some("something.apps.example.com"));
        imp.oauth_client_id.set_hexpand(true);
        imp.oauth_client_id.connect_changed(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.render()
        ));
        imp.oauth_client_id.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.submit()
        ));
        imp.oauth_client_secret.set_show_peek_icon(true);
        imp.oauth_client_secret.set_hexpand(true);
        imp.oauth_client_secret.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.submit()
        ));

        imp.cancel_sign_in.set_label("Cancel sign-in");
        imp.cancel_sign_in.set_visible(false);
        imp.cancel_sign_in.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.cancel_sign_in()
        ));

        // The plate's own Escape rather than a registry command: this screen exists before any
        // account, outside the keymap's
        // contexts, and is not rebindable.
        let screen = self.downgrade();
        crate::widgets::plate::connect_escape(self, move || {
            let Some(screen) = screen.upgrade() else {
                return false;
            };
            let waiting = matches!(screen.status(), Status::WaitingForBrowser);
            if waiting {
                screen.cancel_sign_in();
            }
            waiting
        });

        imp.card_icon.set_pixel_size(16);
        imp.card_icon.add_css_class("postio-onboarding-card-icon");
        imp.card_heading
            .add_css_class("postio-onboarding-card-heading");
        imp.card_heading.set_xalign(0.0);
        imp.card_heading.set_wrap(true);
        imp.card_heading.set_hexpand(true);

        let card_head = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        card_head.append(&imp.card_icon);
        card_head.append(&imp.card_heading);

        for line in [&imp.imap_line, &imp.smtp_line, &imp.auth_line] {
            line.add_css_class("postio-onboarding-line");
            line.set_xalign(0.0);
            line.set_ellipsize(pango::EllipsizeMode::Middle);
        }

        imp.note.add_css_class("postio-onboarding-note");
        imp.note.set_xalign(0.0);
        imp.note.set_wrap(true);
        imp.note.set_visible(false);

        imp.help.add_css_class("postio-onboarding-help");
        imp.help.set_halign(gtk::Align::Start);
        imp.help.set_visible(false);

        imp.card.set_orientation(gtk::Orientation::Vertical);
        imp.card.set_spacing(5);
        imp.card.add_css_class("postio-onboarding-card");
        imp.card.set_visible(false);
        imp.card.append(&card_head);
        imp.card.append(&imp.imap_line);
        imp.card.append(&imp.smtp_line);
        imp.card.append(&imp.auth_line);
        imp.card.append(&imp.note);
        imp.card.append(&imp.help);

        imp.manual.set_orientation(gtk::Orientation::Vertical);
        imp.manual.set_spacing(8);
        imp.manual.set_visible(false);
        // A server and its port share a row, so the form with these open, a card and a refusal
        // still fits a dialog in a window
        // 800 pixels tall.
        let rows: [&[(&str, &gtk::Entry, bool)]; 3] = [
            &[
                ("IMAP server", &imp.imap_host, false),
                ("IMAP port", &imp.imap_port, true),
            ],
            &[
                ("SMTP server", &imp.smtp_host, false),
                ("SMTP port", &imp.smtp_port, true),
            ],
            &[("Login name", &imp.login, false)],
        ];
        for row in rows {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S3);
            for &(label, entry, is_port) in row {
                entry.set_hexpand(!is_port);
                if is_port {
                    entry.set_width_chars(PORT_CHARS);
                    entry.set_max_width_chars(PORT_CHARS);
                }
                entry.connect_activate(glib::clone!(
                    #[weak(rename_to = screen)]
                    self,
                    move |_| screen.submit()
                ));
                entry.connect_changed(glib::clone!(
                    #[weak(rename_to = screen)]
                    self,
                    move |_| {
                        if !screen.imp().echoing.get() {
                            screen.render();
                        }
                    }
                ));
                let field = field(label, entry);
                field.set_hexpand(!is_port);
                line.append(&field);
            }
            imp.manual.append(&line);
        }

        imp.connect_label.set_text("Connect");
        let connect_hint = crate::widgets::keyhint::cap(
            &postio_ui::hints::fixed(
                "Return",
                "connect",
                "Return in any field submits the form; it is the entry's own \
                 activation, not a command in the registry",
            )
            .key,
        );
        let connect_child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        connect_child.append(&imp.connect_label);
        connect_child.append(&connect_hint);
        imp.connect.set_child(Some(&connect_child));
        crate::widgets::button::style(
            &imp.connect,
            crate::widgets::button::Kind::Primary,
            crate::widgets::button::Size::Regular,
        );
        imp.connect.set_sensitive(false);
        imp.connect
            .update_property(&[gtk::accessible::Property::Label(
                "Test the connection and add the account",
            )]);
        imp.connect.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.submit()
        ));

        imp.edit.set_label("Edit manually");
        crate::widgets::button::style(
            &imp.edit,
            crate::widgets::button::Kind::Ghost,
            crate::widgets::button::Size::Regular,
        );
        imp.edit.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| {
                let shown = screen.manual_shown();
                screen.show_manual(!shown);
            }
        ));

        let tab = crate::widgets::KeyLine::new("postio-onboarding-hint");
        tab.set([&postio_ui::hints::fixed(
            "Tab",
            "between fields",
            "moving between a form's fields is the toolkit's focus order",
        )]);
        let hint = tab.widget().clone();
        hint.set_hexpand(true);
        hint.set_xalign(1.0);

        imp.buttons.set_orientation(gtk::Orientation::Horizontal);
        imp.buttons.set_spacing(12);
        imp.buttons.append(&imp.connect);
        imp.buttons.append(&imp.cancel_sign_in);
        imp.buttons.append(&imp.edit);
        imp.buttons.append(&hint);

        // Never a login form: this shows what is happening in the browser and what the request
        // actually asks for.

        let explanation = gtk::Label::new(Some(
            "Consent happens in your real browser. Nothing is typed into this \
             app — it listens on a loopback port for the redirect.",
        ));
        explanation.set_xalign(0.0);
        explanation.set_wrap(true);
        explanation.add_css_class("postio-onboarding-note");

        imp.browser_flow.set_xalign(0.0);
        imp.browser_flow.set_wrap(false);
        imp.browser_flow
            .add_css_class("postio-onboarding-browser-flow");

        imp.browser_scopes
            .set_orientation(gtk::Orientation::Vertical);
        imp.browser_scopes.set_spacing(5);

        let keyring = gtk::Label::new(Some("refresh token → system keyring, never config.toml"));
        keyring.set_xalign(0.0);
        keyring.set_wrap(true);
        keyring.add_css_class("postio-onboarding-browser-flow");

        imp.browser_reopen.set_label("Open link again");
        crate::widgets::button::style(
            &imp.browser_reopen,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
        imp.browser_reopen.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.reopen_sign_in_link()
        ));
        imp.browser_copy.set_label("Copy URL");
        crate::widgets::button::style(
            &imp.browser_copy,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
        imp.browser_copy.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.copy_sign_in_link()
        ));
        let browser_actions = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        browser_actions.append(&imp.browser_reopen);
        browser_actions.append(&imp.browser_copy);

        imp.browser_box.set_orientation(gtk::Orientation::Vertical);
        imp.browser_box.set_spacing(10);
        imp.browser_box.set_visible(false);
        imp.browser_box.append(&explanation);
        imp.browser_box.append(&imp.browser_flow);
        imp.browser_box
            .append(&crate::widgets::kicker("Scopes requested"));
        imp.browser_box.append(&imp.browser_scopes);
        imp.browser_box.append(&keyring);
        imp.browser_box.append(&browser_actions);
        self.render_browser_sign_in();

        // A closed set of three, all worth seeing at once (ADR 0029 Q1): the `DropDown` it replaced
        // hid two of the three answers.
        let labels: Vec<&str> = SyncWindow::ALL
            .iter()
            .map(|window| window.label())
            .collect();
        let picker = crate::widgets::SegmentedControl::new("How far back to sync", &labels);
        picker.set_selected(
            SyncWindow::ALL
                .iter()
                .position(|window| *window == SyncWindow::default())
                .unwrap_or(0),
        );
        picker.connect_selected(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.render()
        ));
        let picker_widget = picker.widget().clone();
        let _ = imp.sync_window_picker.set(picker);

        imp.sync_estimate.add_css_class("postio-onboarding-note");
        imp.sync_estimate.set_xalign(0.0);
        imp.sync_estimate.set_wrap(true);

        crate::widgets::keyhint::dress(
            &imp.start_sync,
            "Start sync",
            Some(
                &postio_ui::hints::fixed(
                    "Return",
                    "start sync",
                    "the step's one button holds the keyboard, so Return is its own \
                     activation, not a command in the registry",
                )
                .key,
            ),
        );
        crate::widgets::button::style(
            &imp.start_sync,
            crate::widgets::button::Kind::Primary,
            crate::widgets::button::Size::Regular,
        );
        imp.start_sync.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.start_sync()
        ));

        imp.sync_window_box
            .set_orientation(gtk::Orientation::Vertical);
        imp.sync_window_box.set_spacing(9);
        imp.sync_window_box.set_visible(false);
        imp.sync_window_box
            .append(&field("How far back to sync", &picker_widget));
        imp.sync_window_box.append(&imp.sync_estimate);
        imp.sync_window_box.append(&imp.start_sync);

        crate::widgets::callout(&imp.status_line, "postio-onboarding-failed");
        imp.status_line.set_visible(false);
        // A live region, so a screen reader hears the failure.
        imp.status_line
            .set_accessible_role(gtk::AccessibleRole::Alert);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        body.add_css_class("postio-onboarding-body");
        imp.identity_rows
            .set_orientation(gtk::Orientation::Vertical);
        imp.identity_rows.set_spacing(9);
        imp.identity_rows.append(&field("Your name", &imp.name));
        imp.identity_rows
            .append(&field("Email address", &imp.address));
        body.append(&imp.identity_rows);
        let password_row = field("Password", &imp.password);
        body.append(&password_row);
        let _ = imp.password_row.set(password_row);
        let oauth_rows = gtk::Box::new(gtk::Orientation::Vertical, 9);
        oauth_rows.append(&field("OAuth client ID (required)", &imp.oauth_client_id));
        oauth_rows.append(&field(
            "Client secret (only if your provider issued one)",
            &imp.oauth_client_secret,
        ));
        let oauth_note = gtk::Label::new(Some(
            "From your own developer console at the provider — Postio ships \
             no built-in client yet, so the sign-in runs as yours.",
        ));
        oauth_note.add_css_class("postio-onboarding-note");
        oauth_note.set_xalign(0.0);
        oauth_note.set_wrap(true);
        oauth_rows.append(&oauth_note);
        oauth_rows.set_visible(false);
        body.append(&oauth_rows);
        let _ = imp.oauth_rows.set(oauth_rows);
        body.append(&imp.card);
        body.append(&imp.manual);
        body.append(&imp.browser_box);
        body.append(&imp.sync_window_box);
        body.append(&imp.status_line);
        imp.submit_hint.add_css_class("postio-onboarding-note");
        imp.submit_hint.set_xalign(0.0);
        imp.submit_hint.set_wrap(true);
        imp.submit_hint.set_visible(false);
        body.append(&imp.submit_hint);
        body.append(&imp.buttons);

        // The body scrolls, the header does not: with the server fields open the form is taller
        // than a small window, and Connect must not be cut off.
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_vexpand(true);
        scroller.set_focusable(false);
        scroller.set_child(Some(&body));

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&scroller);
        self.set_child(Some(&column));

        self.render();
    }
}

fn field(label: &str, entry: &impl IsA<gtk::Widget>) -> gtk::Box {
    crate::widgets::field(label, entry, "postio-onboarding-field")
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::TransportSecurity;

    #[test]
    fn the_three_steps_are_the_three_questions_and_a_retry_stays_on_its_own() {
        assert_eq!(step_of(&Status::Idle), "1 / 3");
        assert_eq!(step_of(&Status::Probing), "1 / 3");
        assert_eq!(step_of(&Status::WaitingForBrowser), "2 / 3");
        assert_eq!(step_of(&Status::SyncWindow), "3 / 3");
        assert_eq!(
            step_of(&Status::Failed("AADSTS65005 · consent_required".into())),
            "2 / 3"
        );
    }

    #[test]
    fn a_domain_comes_off_the_address_folded() {
        assert_eq!(domain_of("Lena@Example.COM"), "example.com");
        assert_eq!(domain_of("lena@example.com "), "example.com");
        assert_eq!(domain_of("not-an-address"), "");
    }

    #[test]
    fn an_address_is_worth_probing_once_it_has_a_domain() {
        assert!(looks_like_an_address("lena@example.com"));
        assert!(looks_like_an_address("lena@mail.example.co.uk"));
    }

    #[test]
    fn a_half_typed_address_is_not() {
        for half in [
            "",
            "lena",
            "lena@",
            "@example.com",
            "lena@example",
            "lena@.com",
        ] {
            assert!(!looks_like_an_address(half), "{half:?}");
        }
    }

    #[test]
    fn a_server_reads_the_way_the_canvas_writes_it() {
        let tls = Server {
            host: "imap.fastmail.com".to_owned(),
            port: 993,
            security: TransportSecurity::Tls,
        };
        assert_eq!(tls.line(), "imap.fastmail.com:993 · TLS");

        let starttls = Server {
            host: "mail.example.com".to_owned(),
            port: 143,
            security: TransportSecurity::StartTls,
        };
        assert_eq!(starttls.line(), "mail.example.com:143 · STARTTLS");

        let plain = Server {
            host: "127.0.0.1".to_owned(),
            port: 10143,
            security: TransportSecurity::None,
        };
        assert_eq!(plain.line(), "127.0.0.1:10143 · unencrypted");
    }

    #[test]
    fn a_repair_says_which_of_the_two_things_is_missing() {
        let repair = Status::Reauthenticate(Settings::default());
        let message = repair.message().expect("a repair owes the user a sentence");
        assert!(
            message.to_lowercase().contains("password"),
            "the one missing thing has to be named: {message}"
        );
        assert!(
            message.to_lowercase().contains("keyring"),
            "and where it goes, so the user knows what signing in again does: {message}"
        );
    }

    #[test]
    fn the_settled_states_that_owe_no_sentence_say_nothing() {
        for quiet in [
            Status::Idle,
            Status::Probing,
            Status::Found(Settings::default()),
            Status::Manual { suggestion: None },
            Status::Connecting,
            Status::Saved,
        ] {
            assert_eq!(quiet.message(), None, "{quiet:?}");
        }
        assert_eq!(Status::Failed("nope".to_owned()).message(), Some("nope"));
    }

    #[test]
    fn only_the_waiting_states_are_busy() {
        assert!(Status::Probing.is_busy());
        assert!(Status::Connecting.is_busy());
        for settled in [
            Status::Idle,
            Status::Found(Settings::default()),
            Status::Manual { suggestion: None },
            Status::Failed("no".to_owned()),
            Status::Reauthenticate(Settings::default()),
            Status::Saved,
        ] {
            assert!(!settled.is_busy(), "{settled:?}");
        }
    }
}
