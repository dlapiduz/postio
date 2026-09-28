//! The account form, joined to the store's host: the presenter both desktop
//! apps drive their account dialogs with (ADR 0043; specs/007-postio-focus
//! T165).
//!
//! [`crate::onboarding::Onboarding`] draws canvas 3e and knows nothing about
//! mail. This is the other half. The probe, the connection test, the
//! browser sign-in and the two writes are the host's (ADR 0041): this asks
//! for each through `postio-client` and shows the answer, so the form
//! behaves the same over the classic app's host and over Focus's.
//!
//! # What it does not do
//!
//! Start the new account's sync. It asks the host to save and wait
//! ([`AfterSave::Wait`]): the classic first run starts the sync once the
//! person has chosen how far back to go, the add-account dialog brings the
//! account into a window that is already running, and a credential update is
//! over an account whose sync is running already. Each is its app's to do,
//! from [`Presenter::connect_saved`].
//!
//! Nor does it open the browser on its own. A browser sign-in's consent link
//! comes back from the host unopened, and the app hands the presenter how it
//! opens one; the presenter opens it once, because the person pressed "Sign
//! in with your browser".
//!
//! # Nothing here blocks the main loop
//!
//! Every client call is a oneshot receive: the host answers on its own
//! runtime, and the form says what it is waiting for meanwhile.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::{AfterSave, Stop};

use crate::onboarding::{BrowserSignIn, Onboarding, Status, Submission};

/// The stop for the probe in flight, if there is one.
///
/// A probe opens connections to servers the person has not named yet, so the
/// one before is stopped when another starts, when Connect is pressed, and
/// when the form is left (#57, ADR 0012 Q3). The form already refuses to
/// start a probe while one is busy, but that answers what the form says,
/// not whether a socket is open: two reasons to be right is the right
/// number for a failure nobody would see.
#[derive(Clone, Default)]
pub struct ProbeCancellation(Rc<RefCell<Option<Stop>>>);

impl ProbeCancellation {
    /// Stops whatever probe is in flight and hands back a stop for the new
    /// one.
    pub fn restart(&self) -> Stop {
        self.stop();
        let stop = Stop::new();
        *self.0.borrow_mut() = Some(stop.clone());
        stop
    }

    /// Stops whatever probe is in flight, if any. Idempotent.
    pub fn stop(&self) {
        if let Some(stop) = self.0.borrow_mut().take() {
            stop.stop();
        }
    }
}

/// How an app opens a browser sign-in's consent link.
type OpenLink = Box<dyn Fn(&str)>;

/// What runs once an account is saved.
type Saved = Box<dyn Fn(&Submission)>;

/// An account form, driven through a client.
///
/// Cheap to clone. The form holds the presenter through the handlers it is
/// given, and the presenter holds the form weakly, so closing the form
/// frees both.
#[derive(Clone)]
pub struct Presenter {
    inner: Rc<Inner>,
}

struct Inner {
    screen: glib::WeakRef<Onboarding>,
    client: Client,
    probe: ProbeCancellation,
    /// The address of the browser sign-in under way, if one is.
    signing_in: RefCell<Option<String>>,
    /// Whether the person cancelled the sign-in under way: its ending is
    /// then theirs, not a failure.
    cancelled: Cell<bool>,
    open_link: OpenLink,
    saved: RefCell<Vec<Saved>>,
}

impl Presenter {
    /// Drive `screen` through `client`: its probe, its Connect, its
    /// browser sign-in and its Cancel. `open_link` opens a consent link for
    /// the person who asked for the sign-in.
    pub fn drive(screen: &Onboarding, client: &Client, open_link: impl Fn(&str) + 'static) -> Self {
        let presenter = Presenter {
            inner: Rc::new(Inner {
                screen: screen.downgrade(),
                client: client.clone(),
                probe: ProbeCancellation::default(),
                signing_in: RefCell::new(None),
                cancelled: Cell::new(false),
                open_link: Box::new(open_link),
                saved: RefCell::new(Vec::new()),
            }),
        };
        screen.connect_probe({
            let presenter = presenter.clone();
            move |address| presenter.probe(address)
        });
        screen.connect_submit({
            let presenter = presenter.clone();
            move |submission| presenter.submit(submission)
        });
        screen.connect_cancel_sign_in({
            let presenter = presenter.clone();
            move || presenter.cancel_sign_in()
        });
        presenter
    }

    /// Run `handler` once an account is written: its credential and its
    /// row, both. Never on a refused proof or a failed write.
    pub fn connect_saved(&self, handler: impl Fn(&Submission) + 'static) {
        self.inner.saved.borrow_mut().push(Box::new(handler));
    }

    /// The form was left: stop the probe it started.
    pub fn stop(&self) {
        self.inner.probe.stop();
    }

    fn screen(&self) -> Option<Onboarding> {
        self.inner.screen.upgrade()
    }

    /// Look up `address`'s servers and show what was found.
    fn probe(&self, address: &str) {
        let Some(screen) = self.screen() else {
            return;
        };
        screen.set_status(Status::Probing);
        let stop = self.inner.probe.restart();
        let client = self.inner.client.clone();
        let address = address.to_owned();
        let presenter = self.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // probes on its own runtime.
            let found = client.discover_until(address, stop.clone()).await;
            // A probe the person moved on from answers nobody: its status
            // would land over whatever the form says now.
            if stop.is_stopped() {
                return;
            }
            if let Some(screen) = presenter.screen() {
                screen.set_status(found.unwrap_or(Status::Manual { suggestion: None }));
            }
        });
    }

    /// Connect: prove the credential and save the account, or begin the
    /// browser sign-in.
    fn submit(&self, submission: &Submission) {
        // Pressing Connect settles the question the probe was asking.
        self.inner.probe.stop();
        if submission.oauth_client.is_some() {
            self.sign_in(submission.clone());
        } else {
            self.add(submission.clone());
        }
    }

    /// The password path: the host proves the credential, then writes it
    /// and the row, in `postio_session::onboarding::persist`'s order. A
    /// refused proof writes nothing.
    fn add(&self, submission: Submission) {
        let Some(screen) = self.screen() else {
            return;
        };
        screen.set_status(Status::Connecting);
        let client = self.inner.client.clone();
        let presenter = self.clone();
        glib::spawn_future_local(async move {
            let saving = client.add_account_then(submission.clone(), AfterSave::Wait);
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // proves and writes on its own runtime.
            let saved = saving.await;
            presenter.settle(
                &submission,
                saved.map_err(|error| error.message().to_owned()),
            );
        });
    }

    /// The browser sign-in (#534, ADR 0006 Q3): the host binds its loopback
    /// listener and answers with the consent link, which is opened here once;
    /// then it waits for the redirect, proves the token and saves.
    fn sign_in(&self, submission: Submission) {
        let Some(screen) = self.screen() else {
            return;
        };
        self.inner.cancelled.set(false);
        self.inner
            .signing_in
            .replace(Some(submission.address.clone()));
        // Who is being signed in to is known now; what the browser is sent
        // to approve arrives with the link.
        screen.set_browser_sign_in(BrowserSignIn {
            provider: submission.settings.source.clone(),
            ..BrowserSignIn::default()
        });
        screen.set_status(Status::WaitingForBrowser);
        let client = self.inner.client.clone();
        let presenter = self.clone();
        glib::spawn_future_local(async move {
            let beginning = client.begin_oauth_then(submission.clone(), AfterSave::Wait);
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host
            // runs the sign-in on its own runtime.
            let consent = beginning.await;
            let ended = match consent {
                Ok(consent) => {
                    if let Some(screen) = presenter.screen() {
                        screen.set_browser_sign_in(consent.clone());
                    }
                    if !presenter.inner.cancelled.get() {
                        (presenter.inner.open_link)(&consent.authorize_url);
                    }
                    // POSTIO-GLIB-SAFE: a client call is a oneshot receive;
                    // the host waits for the redirect on its own runtime.
                    client.finish_oauth(submission.address.clone()).await
                }
                Err(error) => Err(error),
            };
            presenter.inner.signing_in.take();
            if ended.is_err() && presenter.inner.cancelled.get() {
                // The person's own Esc: back to where they were, quietly.
                if let Some(screen) = presenter.screen() {
                    screen.set_status(Status::Found(submission.settings.clone()));
                }
                return;
            }
            presenter.settle(
                &submission,
                ended.map_err(|error| error.message().to_owned()),
            );
        });
    }

    /// Cancel, or `Esc`, while the browser is out.
    fn cancel_sign_in(&self) {
        let Some(address) = self.inner.signing_in.borrow().clone() else {
            return;
        };
        self.inner.cancelled.set(true);
        let client = self.inner.client.clone();
        glib::spawn_future_local(async move {
            // POSTIO-GLIB-SAFE: a client call is a oneshot receive.
            if let Err(error) = client.cancel_oauth(address).await {
                tracing::warn!(%error, "could not cancel the sign-in");
            }
        });
    }

    /// Show how a save ended, and tell whoever waits on a saved account.
    fn settle(&self, submission: &Submission, saved: Result<(), String>) {
        let Some(screen) = self.screen() else {
            return;
        };
        match saved {
            Ok(()) => {
                screen.set_status(Status::Saved);
                for handler in self.inner.saved.borrow().iter() {
                    handler(submission);
                }
            }
            Err(reason) => screen.set_status(Status::Failed(reason)),
        }
    }
}

/// Opens a consent link in the desktop's browser, over `parent`.
///
/// What an app hands [`Presenter::drive`] when it has no opener of its own.
pub fn open_in_browser(parent: &impl IsA<gtk::Widget>) -> impl Fn(&str) + 'static {
    let parent = parent.as_ref().downgrade();
    move |url| {
        // POSTIO-CONSENT: a browser sign-in's consent link, opened once,
        // because the person pressed "Sign in with your browser" on the
        // account form and the host has just bound the loopback listener
        // that waits for its redirect. Never on render, never retried on its
        // own. See ADR 0006 Q3 and CLAUDE.md, "Privacy is a feature".
        let window = parent
            .upgrade()
            .and_then(|parent| parent.root())
            .and_downcast::<gtk::Window>();
        gtk::UriLauncher::new(url).launch(window.as_ref(), gtk::gio::Cancellable::NONE, |result| {
            if let Err(error) = result {
                tracing::warn!(%error, "could not open the sign-in link");
            }
        });
    }
}

/// A form in a dialog, closed by every way out: `Esc`, the close button,
/// and its parent going away. Closing it stops the probe it started.
fn dialog(screen: &Onboarding, title: &str, presenter: &Presenter) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(420)
        .content_height(420)
        .child(screen)
        .build();
    dialog.connect_closed({
        let presenter = presenter.clone();
        move |_| presenter.stop()
    });
    dialog
}

/// The add-account dialog over `parent` (#64, ADR 0012 Q1): a blank form,
/// floating over the window rather than replacing it.
///
/// Once the account is written the dialog closes itself and `on_saved`
/// hears the form's submission: bringing the account into the window is the
/// app's.
pub fn add_account(
    parent: &impl IsA<gtk::Widget>,
    client: &Client,
    open_link: impl Fn(&str) + 'static,
    on_saved: impl Fn(&Submission) + 'static,
) -> adw::Dialog {
    let screen = Onboarding::new();
    // A fresh form starts at its first field, which is the name.
    screen.focus_name();
    let presenter = Presenter::drive(&screen, client, open_link);
    let dialog = dialog(&screen, "Add account", &presenter);
    presenter.connect_saved({
        let dialog = dialog.downgrade();
        move |submission| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            on_saved(submission);
        }
    });
    dialog.present(Some(parent));
    dialog
}

/// The credential dialog over `parent` (#464; spec 007 US6 scenario 3): the
/// form for the first account `which` picks, arriving filled in from its row
/// with the cursor in the password.
///
/// `None` when no account is picked, or the accounts could not be read.
/// Once the new credential is written the dialog closes itself and
/// `on_saved` runs.
pub async fn update_credential(
    parent: &impl IsA<gtk::Widget>,
    client: &Client,
    which: impl Fn(&postio_model::Account) -> bool,
    open_link: impl Fn(&str) + 'static,
    on_saved: impl Fn() + 'static,
) -> Option<adw::Dialog> {
    // POSTIO-GLIB-SAFE: a client call is a oneshot receive; the host answers
    // on its own runtime.
    let accounts = client.accounts().await.ok()?;
    let account = accounts.into_iter().find(|account| which(account))?;

    let screen = Onboarding::new();
    screen.set_address(&account.address.address);
    screen.set_status(Status::Reauthenticate(postio_ui::onboarding::configured(
        &account,
    )));
    screen.focus_password();
    let presenter = Presenter::drive(&screen, client, open_link);
    let dialog = dialog(&screen, "Update credential", &presenter);
    presenter.connect_saved({
        let dialog = dialog.downgrade();
        move |_| {
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            on_saved();
        }
    });
    dialog.present(Some(parent));
    Some(dialog)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Cancelling the probe that is in flight (#57) ---------------------

    #[test]
    fn a_probe_gets_a_live_stop() {
        let cancellation = ProbeCancellation::default();
        let stop = cancellation.restart();
        assert!(
            !stop.is_stopped(),
            "the probe was handed a stop that was already pulled"
        );
    }

    #[test]
    fn starting_a_probe_stops_the_one_before_it() {
        let cancellation = ProbeCancellation::default();
        let first = cancellation.restart();
        let second = cancellation.restart();

        assert!(first.is_stopped(), "the earlier probe kept its socket");
        assert!(!second.is_stopped(), "the new probe starts live");
    }

    #[test]
    fn leaving_the_form_stops_the_probe() {
        let cancellation = ProbeCancellation::default();
        let stop = cancellation.restart();

        cancellation.stop();

        assert!(
            stop.is_stopped(),
            "pressing Connect left a discovery request open for an answer \
             nobody will read"
        );
    }

    #[test]
    fn stopping_twice_is_harmless() {
        // Connect can be pressed without a probe ever having run: typed
        // address, straight to the password field.
        let cancellation = ProbeCancellation::default();
        cancellation.stop();
        cancellation.stop();

        let stop = cancellation.restart();
        cancellation.stop();
        cancellation.stop();
        assert!(stop.is_stopped());
    }
}
