//! First run: one screen, an address and a password.
//!
//! Canvas 3e. Type an address, Postio finds the servers, you confirm. Step 1
//! of 1 — the canvas draws a local-store picker beside it and `postio-hiy`
//! records that decision as dropped, so it is not here.
//!
//! # What this widget will not do
//!
//! It does not probe, it does not connect, and it does not write anything. It
//! cannot: `postio-gtk` may not link `io-imap` or `rusqlite`
//! (`scripts/check-crate-boundaries.py`), and all three of those need one or
//! the other. So this is the form and the states, and
//! [`Onboarding::connect_probe`] / [`Onboarding::connect_submit`] are where
//! the composition root does the work — the same arrangement
//! [`crate::composer`] has with `postio-app`'s `compose.rs`.
//!
//! That is also why the settings this shows are [`Settings`] and not
//! `postio_imap::discovery::AccountSettings`: a plain shape the view layer
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

/// One server, as the screen shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    /// Hostname.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Whether the connection is TLS from the first byte.
    pub tls: bool,
}

impl Server {
    /// `imap.fastmail.com:993 · TLS`, the way the canvas writes it.
    pub fn line(&self) -> String {
        let security = if self.tls { "TLS" } else { "STARTTLS" };
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
}

/// Everything the composition root needs to create the account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submission {
    /// The address mail arrives at.
    pub address: String,
    /// The password, on its way to the keyring and nowhere else.
    pub password: String,
    /// The servers to use.
    pub settings: Settings,
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
    /// It did not work, and this says why in words the user can act on.
    Failed(String),
    /// The account exists and the password is in the keyring.
    Saved,
}

impl Status {
    /// Whether the screen is waiting on something and should not be touched.
    pub fn is_busy(&self) -> bool {
        matches!(self, Status::Probing | Status::Connecting)
    }
}

/// How tall the form gets before it scrolls instead of growing.
///
/// Chosen so the whole plate — header, body and all — still fits inside the
/// shortest window Postio supports.
const BODY_MAX_HEIGHT: i32 = 520;

type SubmitHandler = Box<dyn Fn(&Submission)>;
type ProbeHandler = Box<dyn Fn(&str)>;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Onboarding {
        pub(super) address: gtk::Entry,
        pub(super) password: gtk::PasswordEntry,
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

    /// The address as typed.
    pub fn address(&self) -> String {
        self.imp().address.text().trim().to_owned()
    }

    /// Put an address in the field — restoring a half-finished first run, or
    /// rendering the screen for review.
    pub fn set_address(&self, address: &str) {
        self.imp().address.set_text(address);
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
    pub fn set_status(&self, status: Status) {
        let imp = self.imp();
        // Filling the manual fields from a probe is the widget writing its
        // own form, not the user editing it.
        if let Status::Found(settings)
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
            Status::Found(settings) => Some(settings.clone()),
            Status::Manual {
                suggestion: Some(settings),
            } => Some(settings.clone()),
            _ => None,
        };
        Settings {
            imap: Server {
                host: imp.imap_host.text().trim().to_owned(),
                port: port(&imp.imap_port, 993),
                tls: true,
            },
            smtp: Server {
                host: imp.smtp_host.text().trim().to_owned(),
                port: port(&imp.smtp_port, 465),
                tls: true,
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

    /// Called when the address is committed and wants probing.
    pub fn connect_probe(&self, handler: impl Fn(&str) + 'static) {
        self.imp().on_probe.borrow_mut().push(Box::new(handler));
    }

    /// Called when `Connect` is pressed with something worth submitting.
    pub fn connect_submit(&self, handler: impl Fn(&Submission) + 'static) {
        self.imp().on_submit.borrow_mut().push(Box::new(handler));
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

    /// Submit, if there is anything to submit.
    ///
    /// Public for the same reason [`Onboarding::probe`] is.
    pub fn submit(&self) {
        if !self.can_submit() {
            return;
        }
        let submission = Submission {
            address: self.address(),
            password: self.password(),
            settings: self.settings(),
        };
        for handler in self.imp().on_submit.borrow().iter() {
            handler(&submission);
        }
    }

    /// Whether `Connect` would do anything.
    pub fn can_submit(&self) -> bool {
        let settings = self.settings();
        looks_like_an_address(&self.address())
            && !self.password().is_empty()
            && !settings.imap.host.is_empty()
            && !settings.smtp.host.is_empty()
            && !self.status().is_busy()
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
            Status::Saved => (true, "Account added".to_owned(), "object-select-symbolic"),
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
            imp.auth_line
                .set_text("Auth   plain · secret in the system keyring");
        }

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
        imp.address.set_sensitive(!busy);
        imp.password.set_sensitive(!busy);
        imp.connect.set_sensitive(self.can_submit());
        imp.connect_label
            .set_text(if matches!(status, Status::Connecting) {
                "Connecting…"
            } else {
                "Connect"
            });

        imp.status_line
            .set_visible(matches!(status, Status::Failed(_)));
        if let Status::Failed(reason) = &status {
            imp.status_line.set_text(reason);
        }
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

        let step = gtk::Label::new(Some("step 1 of 1"));
        step.add_css_class("postio-onboarding-step");
        step.set_accessible_role(gtk::AccessibleRole::Presentation);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-onboarding-header");
        header.append(&kicker);
        header.append(&step);

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

        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        buttons.append(&imp.connect);
        buttons.append(&imp.edit);
        buttons.append(&hint);

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
        body.append(&field("Email address", &imp.address));
        body.append(&field("Password", &imp.password));
        body.append(&imp.card);
        body.append(&imp.manual);
        body.append(&imp.status_line);
        body.append(&buttons);

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
            tls: true,
        };
        assert_eq!(tls.line(), "imap.fastmail.com:993 · TLS");

        let starttls = Server {
            host: "mail.example.com".to_owned(),
            port: 143,
            tls: false,
        };
        assert_eq!(starttls.line(), "mail.example.com:143 · STARTTLS");
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
            Status::Saved,
        ] {
            assert!(!settled.is_busy(), "{settled:?}");
        }
    }
}
