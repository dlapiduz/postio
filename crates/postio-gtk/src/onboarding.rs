//! First run: one screen, an address and a password, then how far back to
//! sync.
//!
//! Canvas 3e. Type an address, Postio finds the servers, you confirm, then
//! choose a sync window (#876) — the canvas draws a local-store format
//! picker beside that step and `postio-hiy` records that decision as
//! dropped, so it is not here: every account is single SQLite/SQLCipher
//! store (ADR 0014), and there is no format to choose.
//!
//! # What this widget will not do
//!
//! It does not probe, it does not connect, and it does not write anything. It
//! cannot: `postio-gtk` may not link `io-imap` or `rusqlite`
//! (`scripts/checks/check-crate-boundaries.py`), and all three of those need one or
//! the other. So this is the form and the states, and
//! [`Onboarding::connect_probe`] / [`Onboarding::connect_submit`] are where
//! the composition root does the work — the same arrangement
//! [`crate::composer`] has with `postio-app`'s `compose.rs`.
//!
//! That is also why the settings this shows are [`Settings`] and not
//! `postio_account::discovery::AccountSettings`: a plain shape the view layer
//! owns, filled in by whoever ran the probe, exactly as [`crate::list::Row`]
//! stands in for a stored message.
//!
//! # When the probe runs
//!
//! When the address field is *committed* — Tab out of it, or press Enter —
//! and not on every keystroke. A probe is a series of requests to servers the
//! user has not named yet (their domain's autoconfig endpoint, then
//! Thunderbird's ISPDB), and CLAUDE.md's rule is that nothing leaves the
//! machine the user did not ask for. Finishing typing an address on a screen
//! whose whole purpose is finding its settings is asking; pausing mid-address
//! is not.

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
use postio_model::TransportSecurity;

/// Which of the three steps a status is in — `1 / 3`, the way the drawing
/// writes it.
///
/// Three rather than the two this screen used to count, and the split is
/// real rather than cosmetic: naming the account, proving you own it, and
/// saying how much of it to fetch are three different questions, and the
/// second one can fail and be retried without touching the other two.
/// Pure, so the mapping is tested without a display.
fn step_of(status: &Status) -> &'static str {
    match status {
        Status::Idle | Status::Probing | Status::Found(_) | Status::Manual { .. } => "1 / 3",
        Status::Connecting
        | Status::WaitingForBrowser
        | Status::Failed(_)
        | Status::Reauthenticate(_) => "2 / 3",
        Status::SyncWindow | Status::Saved => "3 / 3",
    }
}

/// What a browser sign-in is asking for, so the screen can say so.
///
/// **Postio never draws a provider's login form.** Consent happens in the
/// real browser, against the real domain, where the address bar is the thing
/// a person checks — an in-app web view is how credential phishing is
/// normally taught, and there is no way for a user to tell one from the
/// genuine article. So what this screen can offer instead is an honest
/// account of what is happening while the browser is open, which is what
/// this carries (ADR 0006 Q3, `Design/screens/23`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BrowserSignIn {
    /// Whose consent screen the browser was sent to — `Microsoft`, `Google`.
    pub provider: String,
    /// The scopes the request asks for, in the provider's own spelling.
    /// Rendered through [`plain_scope`], never raw, because a URL is not an
    /// answer to "what is this about to be allowed to do".
    pub scopes: Vec<String>,
    /// Where the browser will be sent back to — `http://127.0.0.1:41337/`.
    pub redirect_uri: String,
    /// The consent URL itself, for `Copy URL` and for opening it again when
    /// the browser swallowed the first one.
    pub authorize_url: String,
}

/// What a scope lets Postio do, in words rather than in a URL.
///
/// A person deciding whether to consent is owed a sentence, not
/// `https://outlook.office.com/IMAP.AccessAsUser.All`. Anything unrecognised
/// falls through verbatim rather than being dropped: an unfamiliar scope is
/// exactly the one worth showing, and silently hiding it would make this
/// list a worse lie than no list at all.
fn plain_scope(scope: &str) -> String {
    let folded = scope.to_ascii_lowercase();
    if folded.contains("imap") || folded == "https://mail.google.com/" {
        return "Read and change your mail".to_owned();
    }
    if folded.contains("smtp") || folded.contains("gmail.send") {
        return "Send mail as you".to_owned();
    }
    match folded.as_str() {
        "offline_access" => "Stay signed in without asking again".to_owned(),
        "openid" | "email" | "profile" => "Know which address you signed in as".to_owned(),
        _ => scope.to_owned(),
    }
}

/// One line of the scope list: a mark, and what it means in words.
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

/// One server, as the screen shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// Hostname.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Connection security. Carried losslessly from discovery (#534):
    /// flattening this to a bool once turned a provider's own
    /// plaintext-on-loopback answer into a TLS dial.
    pub security: TransportSecurity,
}

impl Server {
    /// `imap.fastmail.com:993 · TLS`, the way the canvas writes it.
    pub fn line(&self) -> String {
        let security = match self.security {
            TransportSecurity::Tls => "TLS",
            TransportSecurity::StartTls => "STARTTLS",
            TransportSecurity::None => "unencrypted",
        };
        format!("{}:{} · {security}", self.host, self.port)
    }
}

/// What Postio found, or what the user typed in instead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    /// Where mail is read from.
    pub imap: Server,
    /// Where mail is sent through.
    pub smtp: Server,
    /// The login name, which is not always the address — an iCloud custom
    /// domain logs in as the Apple ID.
    pub login: String,
    /// Whether this provider refuses ordinary account passwords.
    pub requires_app_password: bool,
    /// A sentence to show the user, from the provider table.
    pub note: Option<String>,
    /// Where to go and make an app-specific password.
    pub help_url: Option<String>,
    /// Where the settings came from, for the card's heading.
    pub source: String,
    /// Whether the provider prefers a browser sign-in (#534): the wizard
    /// shows the OAuth client fields and `Sign in with your browser`
    /// instead of the password entry. The app side holds the endpoints;
    /// this widget only needs to know which door to draw.
    pub oauth_sign_in: bool,
}

/// Everything the composition root needs to create the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submission {
    /// The address mail arrives at.
    pub address: String,
    /// What to show instead of the bare address — in the `From` header and
    /// the sidebar. Empty means unset, and the composition root falls back
    /// to the address exactly as it did before this field existed.
    pub name: String,
    /// The password, on its way to the keyring and nowhere else. Empty on
    /// an OAuth submission.
    pub password: String,
    /// The servers to use.
    pub settings: Settings,
    /// The OAuth client the user supplied, when the provider's door is the
    /// browser sign-in (#534). `Some` routes the submission through the
    /// authorization flow instead of a password test.
    pub oauth_client: Option<OAuthClientSubmission>,
}

/// The user's own OAuth client (ADR 0006 Q1, `own-client`): what the
/// sign-in flow presents to the provider. Postio ships no client of its
/// own until #195 clears review, so these come from the user's provider
/// console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OAuthClientSubmission {
    /// The client id, public by definition on a native app.
    pub client_id: String,
    /// The client secret, when the provider issued one — on its way to the
    /// keyring and nowhere else.
    pub client_secret: Option<String>,
}

/// Where the screen is in the one step it has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    /// Nothing typed yet.
    #[default]
    Idle,
    /// The probe is out.
    Probing,
    /// The probe answered with something authoritative.
    Found(Settings),
    /// The probe found nothing. The server fields open, because an empty form
    /// the user can fill in is the way forward and a shrug is not.
    Manual {
        /// An unverified guess to prefill with, if there was one.
        suggestion: Option<Settings>,
    },
    /// Testing the credentials against the real server.
    Connecting,
    /// The consent screen is open in the user's browser; Postio is waiting
    /// for the redirect. Cancellable — the screen shows its own Cancel and
    /// `Esc` means the same thing.
    WaitingForBrowser,
    /// It did not work, and this says why in words the user can act on.
    Failed(String),
    /// The account is configured; its password is not.
    ///
    /// Not a first run. The composition root reaches this when the store
    /// holds an account the keyring will not give up a password for — a
    /// credential write that failed, a keyring that was reset, an item
    /// somebody deleted. The address and the servers are already known, so
    /// the screen arrives filled in and asks for the one thing missing.
    ///
    /// It carries the servers rather than reading them back off the form
    /// because the form is empty until something fills it, and the thing
    /// that knows them is the account row.
    Reauthenticate(Settings),
    /// The account is saved; the last question before Postio starts talking
    /// to the server on its own is how far back the first sync reaches.
    SyncWindow,
    /// The account exists and the password is in the keyring.
    Saved,
}

impl Status {
    /// Whether the screen is waiting on something and should not be touched.
    pub fn is_busy(&self) -> bool {
        matches!(
            self,
            Status::Probing | Status::Connecting | Status::WaitingForBrowser
        )
    }

    /// The sentence under the form, when this state owes the user one.
    ///
    /// Pure, and public, so what the screen *says* can be checked without a
    /// display — the rendering needs one, the wording does not.
    pub fn message(&self) -> Option<&str> {
        match self {
            Status::Failed(reason) => Some(reason),
            Status::Reauthenticate(_) => Some(
                "Postio has no password for this account. Sign in again and it \
                 will go back into the keyring.",
            ),
            Status::WaitingForBrowser => Some(
                "Finish signing in in your browser. Postio is waiting for the \
                 redirect — cancel any time.",
            ),
            _ => None,
        }
    }
}

/// How far back the first sync reaches, chosen once per account on the
/// [`Status::SyncWindow`] step (#876).
///
/// Coarser than [`postio_config::sync::SyncConfig::initial_sync_messages`]
/// itself — a person thinks in a window of time, not a message count — so
/// each variant maps to a fixed count rather than to anything measured: no
/// per-account mailbox statistics exist at this point in onboarding
/// (discovery does not report message counts). `LastYear`'s count matches
/// that field's own default, so picking it changes nothing a fresh install
/// would not already do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncWindow {
    /// Roughly a month of ordinary mail.
    LastMonth,
    /// A year — [`SyncConfig`](postio_config::sync::SyncConfig)'s own
    /// default depth.
    #[default]
    LastYear,
    /// No cap: the highest count the field can hold.
    Everything,
}

impl SyncWindow {
    /// Every choice, in the order the picker offers them.
    pub const ALL: [SyncWindow; 3] = [
        SyncWindow::LastMonth,
        SyncWindow::LastYear,
        SyncWindow::Everything,
    ];

    /// What this writes to `SyncConfig::initial_sync_messages`.
    pub fn message_count(self) -> u32 {
        match self {
            SyncWindow::LastMonth => 500,
            SyncWindow::LastYear => 5_000,
            SyncWindow::Everything => u32::MAX,
        }
    }

    /// The picker's own label for this choice.
    pub fn label(self) -> &'static str {
        match self {
            SyncWindow::LastMonth => "Last 30 days",
            SyncWindow::LastYear => "Last year",
            SyncWindow::Everything => "Everything",
        }
    }

    /// A rough size/time readout under the picker.
    ///
    /// Built from a flat per-message estimate (75 KiB — ADR 0017 puts most
    /// of a message's bytes on the lazy attachment axis, so a synced-but-
    /// unopened message is mostly headers and text) and a flat fetch rate,
    /// for the same reason [`message_count`](Self::message_count) is a flat
    /// map rather than a measurement: nothing has synced yet to measure.
    pub fn estimate(self) -> String {
        const AVERAGE_MESSAGE_BYTES: u64 = 75 * 1024;
        const MESSAGES_PER_MINUTE: u64 = 120;
        if self == SyncWindow::Everything {
            return "Downloads everything the server has — size and time depend \
                     on the mailbox."
                .to_owned();
        }
        let count = u64::from(self.message_count());
        let megabytes = (count * AVERAGE_MESSAGE_BYTES) / (1024 * 1024);
        let minutes = count.div_ceil(MESSAGES_PER_MINUTE).max(1);
        format!(
            "About {megabytes} MB, {minutes} minute{} to sync",
            if minutes == 1 { "" } else { "s" }
        )
    }
}

/// How tall the form gets before it scrolls instead of growing.
///
/// Chosen so the whole plate — header, body and all — still fits inside the
/// shortest window Postio supports.
const BODY_MAX_HEIGHT: i32 = 520;

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
        /// The password field's whole row, so the OAuth mode can swap it out.
        pub(super) password_row: std::cell::OnceCell<gtk::Box>,
        /// The user's own OAuth client (#534): id, and the secret when the
        /// provider issued one.
        pub(super) oauth_client_id: gtk::Entry,
        pub(super) oauth_client_secret: gtk::PasswordEntry,
        pub(super) oauth_rows: std::cell::OnceCell<gtk::Box>,
        /// Cancels a sign-in waiting on the browser.
        pub(super) cancel_sign_in: gtk::Button,
        pub(super) on_cancel_sign_in: RefCell<Vec<Box<dyn Fn()>>>,
        /// The found-settings card and the three lines in it.
        pub(super) card: gtk::Box,
        pub(super) card_heading: gtk::Label,
        pub(super) card_icon: gtk::Image,
        pub(super) imap_line: gtk::Label,
        pub(super) smtp_line: gtk::Label,
        pub(super) auth_line: gtk::Label,
        /// The provider's own sentence — iCloud's app-password requirement.
        pub(super) note: gtk::Label,
        pub(super) help: gtk::LinkButton,
        /// The escape hatch, revealed by `Edit manually` or by a probe that
        /// found nothing.
        pub(super) manual: gtk::Box,
        pub(super) imap_host: gtk::Entry,
        pub(super) imap_port: gtk::Entry,
        pub(super) smtp_host: gtk::Entry,
        pub(super) smtp_port: gtk::Entry,
        pub(super) login: gtk::Entry,
        pub(super) connect: gtk::Button,
        /// The word on the Connect button. A handle of its own because
        /// `set_label` would replace the button's child, and the child is
        /// the word *and* its `Ret` key hint.
        pub(super) connect_label: gtk::Label,
        pub(super) edit: gtk::Button,
        /// Connect, Cancel sign-in, Edit manually and the tab hint — hidden
        /// as one row on [`Status::SyncWindow`], where none of them apply.
        pub(super) buttons: gtk::Box,
        /// "step 1 of 2" / "step 2 of 2" — [`Status::SyncWindow`] is the
        /// only status past the first, so this is exactly that check, not a
        /// counter.
        pub(super) step: gtk::Label,
        /// The sync-window picker, its estimate, and its own Start sync
        /// button — shown only on [`Status::SyncWindow`].
        /// The browser sign-in panel: what is happening, what is being
        /// asked for, and what is not. Shown only while the browser is out.
        pub(super) browser_box: gtk::Box,
        pub(super) browser_flow: gtk::Label,
        pub(super) browser_scopes: gtk::Box,
        pub(super) browser_copy: gtk::Button,
        pub(super) browser_reopen: gtk::Button,
        pub(super) browser_sign_in: RefCell<BrowserSignIn>,
        pub(super) sync_window_box: gtk::Box,
        /// The three windows, all on screen — see `build` for why this is
        /// not a dropdown. Built lazily for the reason
        /// this fills in during `build()` the way `password_row` does.
        pub(super) sync_window_picker: std::cell::OnceCell<crate::widgets::SegmentedControl>,
        pub(super) sync_estimate: gtk::Label,
        pub(super) start_sync: gtk::Button,
        pub(super) on_start_sync: RefCell<Vec<StartSyncHandler>>,
        pub(super) status_line: gtk::Label,
        pub(super) status: RefCell<Status>,
        /// The last settings the screen was shown, kept across `Connecting`,
        /// `Failed` and `Saved`. A failure that also wiped the card would
        /// take away the one thing the user needs to see to fix it: what it
        /// was trying to connect to.
        pub(super) shown: RefCell<Option<Settings>>,
        /// Set while the widget is writing its own fields, so filling the
        /// manual form from a probe does not read as the user editing it.
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
    /// The first-run screen.
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
    /// An empty first-run screen.
    pub fn new() -> Self {
        Self::default()
    }

    /// The name as typed, for the `From` header and the sidebar label.
    /// Empty means the user left it blank.
    pub fn name(&self) -> String {
        self.imp().name.text().trim().to_owned()
    }

    /// The address as typed.
    pub fn address(&self) -> String {
        self.imp().address.text().trim().to_owned()
    }

    /// Put an address in the field — restoring a half-finished first run, or
    /// rendering the screen for review.
    pub fn set_address(&self, address: &str) {
        self.imp().address.set_text(address);
    }

    /// Put a name in the field, as [`set_address`](Self::set_address) does.
    pub fn set_name(&self, name: &str) {
        self.imp().name.set_text(name);
    }

    /// The password as typed. Read once, on submit, and never stored here.
    pub fn password(&self) -> String {
        self.imp().password.text().to_string()
    }

    /// Where the screen is.
    pub fn status(&self) -> Status {
        self.imp().status.borrow().clone()
    }

    /// Move the screen on.
    /// Tells the screen what the browser has been sent to consent to.
    ///
    /// Called just before [`set_status`](Self::set_status) moves to
    /// [`Status::WaitingForBrowser`] — the facts come from the provider's
    /// own preset and from the loopback listener that has just bound, so
    /// there is nothing here the screen made up.
    pub fn set_browser_sign_in(&self, details: BrowserSignIn) {
        *self.imp().browser_sign_in.borrow_mut() = details;
        self.render_browser_sign_in();
    }

    /// Draws the scope list and the flow line from the current details.
    fn render_browser_sign_in(&self) {
        let imp = self.imp();
        let details = imp.browser_sign_in.borrow().clone();

        // Three facts, in the order they matter: who is being signed in to,
        // what is listening for the answer, and by which flow. The provider
        // line is what the drawing puts in the dialog's own title bar; it
        // sits here instead because this widget is hosted in dialogs and in
        // the first-run screen alike, and only one of those has a title bar
        // to put it in.
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
        // Said out loud, because the absence is the point: every scope in
        // the preset table is a mail scope, and a person consenting to a
        // Microsoft or Google sign-in has every reason to assume otherwise.
        imp.browser_scopes.append(&scope_line(
            "window-close-symbolic",
            "Not requested: contacts, calendar, files",
        ));

        let has_url = !details.authorize_url.is_empty();
        imp.browser_copy.set_visible(has_url);
        imp.browser_reopen.set_visible(has_url);
    }

    /// Opens the consent link again, for a browser that swallowed the
    /// first one — a default-browser handoff that lands on a locked screen,
    /// or a window that opened behind everything else.
    fn reopen_sign_in_link(&self) {
        let url = self.imp().browser_sign_in.borrow().authorize_url.clone();
        if url.is_empty() {
            return;
        }
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

    /// Puts the consent link on the clipboard, for finishing the sign-in in
    /// a browser other than the desktop's default one.
    fn copy_sign_in_link(&self) {
        let url = self.imp().browser_sign_in.borrow().authorize_url.clone();
        if url.is_empty() {
            return;
        }
        self.clipboard().set_text(&url);
    }

    pub fn set_status(&self, status: Status) {
        let imp = self.imp();
        // Filling the manual fields from a probe is the widget writing its
        // own form, not the user editing it.
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

    /// The settings the screen would submit: what the user typed into the
    /// manual fields, which the probe has already filled in when it found
    /// anything.
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
                // The manual form has no security selector; what the probe
                // found carries through, and hand-entered servers get TLS.
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

    /// Show or hide the server fields.
    pub fn show_manual(&self, show: bool) {
        self.imp().manual.set_visible(show);
        self.imp().edit.set_label(if show {
            "Hide server settings"
        } else {
            "Edit manually"
        });
    }

    /// Whether the server fields are open.
    pub fn manual_shown(&self) -> bool {
        self.imp().manual.is_visible()
    }

    /// Put the keyboard where the user starts.
    pub fn focus_address(&self) {
        self.imp().address.grab_focus();
    }

    /// Put the keyboard where a *repair* starts.
    ///
    /// [`Status::Reauthenticate`] arrives with the address already filled in
    /// and one field still empty. Landing the cursor in the address anyway
    /// would ask the user to find the field themselves, on a screen that
    /// looks finished.
    pub fn focus_password(&self) {
        self.imp().password.grab_focus();
    }

    /// Called when the address is committed and wants probing.
    pub fn connect_probe(&self, handler: impl Fn(&str) + 'static) {
        self.imp().on_probe.borrow_mut().push(Box::new(handler));
    }

    /// Called when `Connect` is pressed with something worth submitting.
    pub fn connect_submit(&self, handler: impl Fn(&Submission) + 'static) {
        self.imp().on_submit.borrow_mut().push(Box::new(handler));
    }

    /// Called when `Start sync` is pressed on the [`Status::SyncWindow`]
    /// step, with the window the picker was showing at the time.
    pub fn connect_start_sync(&self, handler: impl Fn(SyncWindow) + 'static) {
        self.imp()
            .on_start_sync
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// The picker's current choice.
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

    /// Fire `Start sync` for whatever the picker is showing.
    ///
    /// Public for the same reason [`Onboarding::probe`] is.
    pub fn start_sync(&self) {
        if !matches!(self.status(), Status::SyncWindow) {
            return;
        }
        let window = self.sync_window();
        for handler in self.imp().on_start_sync.borrow().iter() {
            handler(window);
        }
    }

    /// Run the probe for whatever is in the address field.
    ///
    /// Public so a test can drive it without synthesizing a focus change,
    /// which GTK4 gives no supported way to do.
    pub fn probe(&self) {
        let address = self.address();
        if !looks_like_an_address(&address) || self.status().is_busy() {
            return;
        }
        for handler in self.imp().on_probe.borrow().iter() {
            handler(&address);
        }
    }

    /// Whether the screen is asking for a browser sign-in rather than a
    /// password — decided by the settings on show (#534).
    fn oauth_mode(&self) -> bool {
        self.imp()
            .shown
            .borrow()
            .as_ref()
            .is_some_and(|settings| settings.oauth_sign_in)
    }

    /// What a click on Cancel — or `Esc` — does while the browser wait is
    /// up. The app side holds the flow's cancel token and answers through
    /// [`Onboarding::connect_cancel_sign_in`].
    pub fn cancel_sign_in(&self) {
        for handler in self.imp().on_cancel_sign_in.borrow().iter() {
            handler();
        }
    }

    /// Called when the user cancels a sign-in waiting on the browser.
    pub fn connect_cancel_sign_in(&self, handler: impl Fn() + 'static) {
        self.imp()
            .on_cancel_sign_in
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Submit, if there is anything to submit.
    ///
    /// Public for the same reason [`Onboarding::probe`] is.
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

    /// Whether `Connect` would do anything.
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

    /// Fills the OAuth client fields, for a test driving the screen.
    pub fn test_set_oauth_client(&self, client_id: &str, secret: &str) {
        self.imp().oauth_client_id.set_text(client_id);
        self.imp().oauth_client_secret.set_text(secret);
    }

    /// Sets the password field directly, without a key event.
    #[doc(hidden)]
    pub fn test_set_password(&self, password: &str) {
        self.imp().password.set_text(password);
    }

    /// Fires `activate` on each of the five manual server fields, exactly as
    /// a real Return keystroke would. Every field, not just one, so a fix
    /// that only wired up `login` cannot pass this — see `postio-68`.
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

    /// Fires `activate` on the name field, as Return would (#629).
    #[doc(hidden)]
    pub fn test_activate_name(&self) {
        self.imp().name.emit_activate();
    }

    /// Fires `activate` on the address field, as Return would.
    #[doc(hidden)]
    pub fn test_activate_address(&self) {
        self.imp().address.emit_activate();
    }

    /// Fires `activate` on the password field, as Return would.
    #[doc(hidden)]
    pub fn test_activate_password(&self) {
        self.imp().password.emit_activate();
    }

    /// Whether the address field is the one `grab_focus` last landed on —
    /// what a test uses to confirm Return in an earlier field moved on to
    /// it, since the field itself is private (#629).
    ///
    /// Walks up from the toplevel's own focus widget rather than asking
    /// `imp.address.is_focus()`: a `gtk::Entry` is a composite widget whose
    /// real keyboard focus lands on an internal `GtkText`, never on the
    /// `Entry` itself, so `is_focus` on the entry is always false. And not
    /// `has_focus`, which also asks whether the *toplevel* is active, which
    /// a headless test window never becomes.
    #[doc(hidden)]
    pub fn test_address_has_focus(&self) -> bool {
        self.root()
            .and_then(|root| root.downcast::<gtk::Window>().ok())
            .and_then(|window| gtk::prelude::RootExt::focus(&window))
            .is_some_and(|focus| {
                focus.is_ancestor(&self.imp().address)
                    || focus == self.imp().address.clone().upcast::<gtk::Widget>()
            })
    }

    /// Sets the sync-window picker's selection, as a click on the dropdown
    /// would — the same test-seam shape [`Onboarding::test_activate_name`]
    /// and its siblings use for a field GTK4 gives no supported way to
    /// synthesize a real interaction on.
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

    /// Whether the sync-window step's own section — the picker, its
    /// estimate and `Start sync` — is the one currently showing.
    #[doc(hidden)]
    pub fn test_sync_window_shown(&self) -> bool {
        self.imp().sync_window_box.is_visible()
    }

    /// The estimate line under the picker, exactly as shown.
    #[doc(hidden)]
    pub fn test_sync_estimate(&self) -> String {
        self.imp().sync_estimate.text().to_string()
    }

    // -- internals ---------------------------------------------------------

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

    /// Put the card, the buttons and the status line in step with the status.
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
                    // Before an address is typed there is no domain to name,
                    // and `Found settings for  — ...` with a hole in it reads
                    // as a bug rather than as a heading.
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
            // The heading names the server rather than saying "that did not
            // work" — the alert below already says that, and what the user
            // needs from the card is which host refused them.
            Status::Failed(_) => (
                true,
                match imp.shown.borrow().as_ref() {
                    Some(settings) => format!("Could not sign in to {}", settings.imap.host),
                    None => "That did not work".to_owned(),
                },
                "dialog-warning-symbolic",
            ),
            // Named for what it is, so a repair does not read as a fresh
            // first run: the account is there, and only the password is not.
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

        // The three mono lines come from the last settings the screen was
        // given, not from the current status, so they survive a failure.
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

        // Which credential door is open: the browser sign-in swaps the
        // password row for the OAuth client fields, and the primary button
        // says what it will actually do (#534).
        let oauth = self.oauth_mode();
        // While the browser is out there is nothing in here to type into,
        // and leaving the credential fields on screen invites somebody to
        // try — which is the exact confusion this step exists to avoid.
        let waiting = matches!(status, Status::WaitingForBrowser);
        if let Some(row) = imp.password_row.get() {
            row.set_visible(!oauth && !waiting);
        }
        if let Some(rows) = imp.oauth_rows.get() {
            rows.set_visible(oauth && !waiting);
        }
        imp.cancel_sign_in.set_visible(waiting);
        imp.browser_box.set_visible(waiting);

        // The provider's own sentence, and where to act on it. iCloud's is
        // the reason this exists: an Apple ID password simply will not work,
        // and a user who is not told that will type theirs and be told the
        // password is wrong.
        let note = settings.as_ref().and_then(|s| s.note.clone());
        imp.note.set_visible(note.is_some());
        imp.note.set_text(note.as_deref().unwrap_or_default());
        let help = settings.as_ref().and_then(|s| s.help_url.clone());
        imp.help.set_visible(help.is_some());
        if let Some(help) = &help {
            imp.help.set_uri(help);
            imp.help.set_label("Make an app-specific password");
        }

        // A probe that found nothing opens the form itself: that is the way
        // forward, and leaving the user to find the button would be a dead
        // end with a button behind it.
        if matches!(status, Status::Manual { .. }) && !self.manual_shown() {
            self.show_manual(true);
        }

        let busy = status.is_busy();
        imp.name.set_sensitive(!busy);
        imp.address.set_sensitive(!busy);
        imp.password.set_sensitive(!busy);
        imp.connect.set_sensitive(self.can_submit());
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

        // The sync-window step replaces Connect/Cancel/Edit with its own
        // picker and Start sync button — none of the credential buttons
        // apply once the account is already saved.
        let on_sync_window = matches!(status, Status::SyncWindow);
        imp.buttons.set_visible(!on_sync_window);
        imp.sync_window_box.set_visible(on_sync_window);
        imp.sync_estimate.set_text(&self.sync_window().estimate());
        imp.step.set_text(step_of(&status));

        self.update_property(&[gtk::accessible::Property::Label(&heading)]);
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-onboarding");
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Center);
        self.set_accessible_role(gtk::AccessibleRole::Group);

        let kicker = gtk::Label::new(Some("Add account"));
        kicker.add_css_class("postio-kicker");
        kicker.set_xalign(0.0);
        kicker.set_hexpand(true);
        kicker.set_accessible_role(gtk::AccessibleRole::Presentation);

        imp.step.set_text(step_of(&Status::Idle));
        imp.step.add_css_class("postio-onboarding-step");
        imp.step
            .set_accessible_role(gtk::AccessibleRole::Presentation);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-onboarding-header");
        header.append(&kicker);
        header.append(&imp.step);

        imp.name.set_placeholder_text(Some("Ada Lovelace"));
        imp.name.set_hexpand(true);
        // Nothing to probe or submit with only a name typed -- Return moves
        // on to the next field, the "Tab between fields" idiom the hint
        // under the form already promises (#629; #68 fixed the same dead-
        // Return gap on the two fields that existed then).
        imp.name.connect_activate(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.focus_address()
        ));

        imp.address.set_placeholder_text(Some("you@example.com"));
        imp.address.set_input_purpose(gtk::InputPurpose::Email);
        imp.address.set_hexpand(true);
        // Committing the field is what asks for a probe — see the module
        // docs on why this is not per keystroke.
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

        // `Esc` while the browser wait is up means what the button means.
        // A widget-local controller rather than a registry command: this
        // screen exists before any account does, outside the keymap's
        // contexts, and the binding is not rebindable on purpose.
        let escape = gtk::EventControllerKey::new();
        escape.connect_key_pressed(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape
                    && matches!(screen.status(), Status::WaitingForBrowser)
                {
                    screen.cancel_sign_in();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(escape);

        // -- the found-settings card ---------------------------------------

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

        // -- the escape hatch ----------------------------------------------

        imp.manual.set_orientation(gtk::Orientation::Vertical);
        imp.manual.set_spacing(8);
        imp.manual.set_visible(false);
        for (label, entry) in [
            ("IMAP server", &imp.imap_host),
            ("IMAP port", &imp.imap_port),
            ("SMTP server", &imp.smtp_host),
            ("SMTP port", &imp.smtp_port),
            ("Login name", &imp.login),
        ] {
            entry.set_hexpand(true);
            // Ret here has never done anything -- postio-68. Address and
            // password already commit the form this way; the five manual
            // fields had no handler on `activate` at all.
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
            imp.manual.append(&field(label, entry));
        }

        // -- the buttons ----------------------------------------------------

        imp.connect_label.set_text("Connect");
        let connect_hint = gtk::Label::new(Some("Ret"));
        connect_hint.add_css_class("postio-keyhint");
        connect_hint.set_accessible_role(gtk::AccessibleRole::Presentation);
        let connect_child = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        connect_child.append(&imp.connect_label);
        connect_child.append(&connect_hint);
        imp.connect.set_child(Some(&connect_child));
        imp.connect.add_css_class("suggested-action");
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
        imp.edit.add_css_class("flat");
        imp.edit.add_css_class("postio-ghost");
        imp.edit.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| {
                let shown = screen.manual_shown();
                screen.show_manual(!shown);
            }
        ));

        let hint = gtk::Label::new(Some("Tab between fields"));
        hint.add_css_class("postio-onboarding-hint");
        hint.set_hexpand(true);
        hint.set_xalign(1.0);
        hint.set_accessible_role(gtk::AccessibleRole::Presentation);

        imp.buttons.set_orientation(gtk::Orientation::Horizontal);
        imp.buttons.set_spacing(12);
        imp.buttons.append(&imp.connect);
        imp.buttons.append(&imp.cancel_sign_in);
        imp.buttons.append(&imp.edit);
        imp.buttons.append(&hint);

        // -- the browser sign-in panel (#1179, Design/screens/23) -----------
        //
        // Never a login form. What this shows is an account of what is
        // happening in the browser that is now in front of the user, and
        // what the request they are about to approve actually asks for.

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
        imp.browser_reopen
            .add_css_class("postio-settings-small-button");
        imp.browser_reopen.connect_clicked(glib::clone!(
            #[weak(rename_to = screen)]
            self,
            move |_| screen.reopen_sign_in_link()
        ));
        imp.browser_copy.set_label("Copy URL");
        imp.browser_copy
            .add_css_class("postio-settings-small-button");
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
            .append(&crate::widgets::kicker("SCOPES REQUESTED"));
        imp.browser_box.append(&imp.browser_scopes);
        imp.browser_box.append(&keyring);
        imp.browser_box.append(&browser_actions);
        self.render_browser_sign_in();

        // -- the sync-window step (#876) ------------------------------------

        // A closed set of three, all of them worth seeing at once — ADR
        // 0029 Q1, and the same control the settings window's Appearance
        // and Sync panes use for the same shape of choice. It was a
        // `DropDown`, which hid two of the three answers behind the one it
        // happened to be showing.
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

        imp.start_sync.set_label("Start sync");
        imp.start_sync.add_css_class("suggested-action");
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

        imp.status_line.add_css_class("postio-onboarding-failed");
        imp.status_line.set_xalign(0.0);
        imp.status_line.set_wrap(true);
        imp.status_line.set_visible(false);
        // A live region, so a screen reader hears the failure rather than
        // only a sighted user seeing it appear.
        imp.status_line
            .set_accessible_role(gtk::AccessibleRole::Alert);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        body.add_css_class("postio-onboarding-body");
        body.append(&field("Your name", &imp.name));
        body.append(&field("Email address", &imp.address));
        let password_row = field("Password", &imp.password);
        body.append(&password_row);
        let _ = imp.password_row.set(password_row);
        let oauth_rows = gtk::Box::new(gtk::Orientation::Vertical, 9);
        oauth_rows.append(&field("OAuth client ID", &imp.oauth_client_id));
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
        body.append(&imp.buttons);

        // The body scrolls; the header does not. With the server fields open
        // the form is taller than a small window, and a first-run screen that
        // cut off its own Connect button would be the worst possible place to
        // repeat `postio-qhz.4`.
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(BODY_MAX_HEIGHT);
        scroller.set_focusable(false);
        scroller.set_child(Some(&body));

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&scroller);
        self.set_child(Some(&column));

        self.render();
    }
}

/// A labelled field, the way the canvas draws one.
fn field(label: &str, entry: &impl IsA<gtk::Widget>) -> gtk::Box {
    let caption = gtk::Label::new(Some(label));
    caption.add_css_class("postio-onboarding-label");
    caption.set_xalign(0.0);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 5);
    column.append(&caption);
    column.append(entry);
    // The label names the field for a screen reader, rather than being read
    // as a stray line of text above it.
    entry
        .as_ref()
        .update_relation(&[gtk::accessible::Relation::LabelledBy(&[
            caption.upcast_ref()
        ])]);
    caption.set_accessible_role(gtk::AccessibleRole::Presentation);
    column
}

/// The domain of an address, for the card's heading.
pub fn domain_of(address: &str) -> String {
    address
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Whether there is enough of an address to be worth probing.
///
/// Deliberately loose: this decides whether to *ask*, and the probe itself
/// decides whether the address is real. Refusing to look up something a
/// server would have accepted is the worse mistake.
pub fn looks_like_an_address(address: &str) -> bool {
    let address = address.trim();
    match address.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_steps_are_the_three_questions_and_a_retry_stays_on_its_own() {
        assert_eq!(step_of(&Status::Idle), "1 / 3");
        assert_eq!(step_of(&Status::Probing), "1 / 3");
        assert_eq!(step_of(&Status::WaitingForBrowser), "2 / 3");
        assert_eq!(step_of(&Status::SyncWindow), "3 / 3");
        // A sign-in that failed has not sent anybody back to step one: the
        // address is still right, and only the credential is not.
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
        // The account is already configured, so a screen that only said
        // "add account" would be asking a question the user cannot answer:
        // what is wrong, and why now.
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
