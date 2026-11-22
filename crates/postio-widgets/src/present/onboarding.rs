//! The account form joined to the store's host through `postio-client` (ADR 0043). It asks the host
//! to save and wait
//! (`AfterSave::Wait`) and never starts the new account's sync, which is the app's to do from
//! [`Presenter::connect_saved`].
//! A consent link comes back unopened and is opened once, because the person pressed "Sign in with
//! your browser".

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;
use postio_client::Client;
use postio_client::protocol::{AfterSave, Stop};

use crate::onboarding::{BrowserSignIn, Onboarding, Status, Submission};

/// A probe opens connections to servers the person has not named yet, so the one before is stopped
/// when another starts,
/// when Connect is pressed, and when the form is left (#57, ADR 0012 Q3).
#[derive(Clone, Default)]
pub struct ProbeCancellation(Rc<RefCell<Option<Stop>>>);

impl ProbeCancellation {
    pub fn restart(&self) -> Stop {
        self.stop();
        let stop = Stop::new();
        *self.0.borrow_mut() = Some(stop.clone());
        stop
    }

    pub fn stop(&self) {
        if let Some(stop) = self.0.borrow_mut().take() {
            stop.stop();
        }
    }
}

type OpenLink = Box<dyn Fn(&str)>;

type Saved = Box<dyn Fn(&Submission)>;

/// Cheap to clone. The presenter holds the form weakly, so closing the form frees both.
#[derive(Clone)]
pub struct Presenter {
    inner: Rc<Inner>,
}

struct Inner {
    screen: glib::WeakRef<Onboarding>,
    client: Client,
    probe: ProbeCancellation,
    signing_in: RefCell<Option<String>>,
    /// A cancelled sign-in's ending is the person's, not a failure.
    cancelled: Cell<bool>,
    open_link: OpenLink,
    saved: RefCell<Vec<Saved>>,
}

impl Presenter {
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

    /// Never on a refused proof or a failed write.
    pub fn connect_saved(&self, handler: impl Fn(&Submission) + 'static) {
        self.inner.saved.borrow_mut().push(Box::new(handler));
    }

    /// A failed write is logged and `chosen` still runs: the default is what a fresh install has.
    /// Replaces what
    /// [`Presenter::connect_saved`] handlers would otherwise see first.
    pub fn ask_sync_window(&self, chosen: impl Fn(&Submission) + 'static) {
        let Some(screen) = self.screen() else {
            return;
        };
        let saved: Rc<RefCell<Option<Submission>>> = Rc::default();
        screen.connect_start_sync({
            let saved = saved.clone();
            move |window| {
                if let Err(error) = postio_ui::onboarding::write_sync_window(window) {
                    tracing::warn!(%error, "could not save the chosen sync window");
                }
                if let Some(submission) = saved.borrow().as_ref() {
                    chosen(submission);
                }
            }
        });
        self.connect_saved({
            let screen = screen.downgrade();
            move |submission| {
                saved.replace(Some(submission.clone()));
                if let Some(screen) = screen.upgrade() {
                    screen.set_status(Status::SyncWindow);
                }
            }
        });
    }

    pub fn stop(&self) {
        self.inner.probe.stop();
    }

    fn screen(&self) -> Option<Onboarding> {
        self.inner.screen.upgrade()
    }

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
            // A probe the person moved on from answers nobody: its status would land over whatever
            // the form says now.
            if stop.is_stopped() {
                return;
            }
            if let Some(screen) = presenter.screen() {
                screen.set_status(found.unwrap_or(Status::Manual { suggestion: None }));
            }
        });
    }

    fn submit(&self, submission: &Submission) {
        self.inner.probe.stop();
        if submission.oauth_client.is_some() {
            self.sign_in(submission.clone());
        } else {
            self.add(submission.clone());
        }
    }

    /// A refused proof writes nothing.
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

    fn sign_in(&self, submission: Submission) {
        let Some(screen) = self.screen() else {
            return;
        };
        self.inner.cancelled.set(false);
        self.inner
            .signing_in
            .replace(Some(submission.address.clone()));
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

pub fn open_in_browser(parent: &impl IsA<gtk::Widget>) -> impl Fn(&str) + 'static {
    let parent = parent.as_ref().downgrade();
    move |url| {
        // POSTIO-CONSENT: a browser sign-in's consent link, opened once,
        // because the person pressed "Sign in with your browser" on the
        // account form and the host has just bound the loopback listener
        // that waits for its redirect. Never on render, never retried on its
        // own. See ADR 0006 Q3 and CLAUDE.md, "Privacy".
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

/// Closed by every way out: `Esc`, the close button and its parent going away; closing stops the
/// probe it started. The
/// sheet is as tall as its form, and the form's body scrolls, never its header.
fn dialog(screen: &Onboarding, title: &str, presenter: &Presenter) -> adw::Dialog {
    let dialog = adw::Dialog::builder()
        .title(title)
        .content_width(560)
        .follows_content_size(true)
        .child(screen)
        .build();
    dialog.connect_closed({
        let presenter = presenter.clone();
        move |_| presenter.stop()
    });
    screen.connect_map(|screen| {
        let screen = screen.downgrade();
        glib::idle_add_local_once(move || {
            if let Some(screen) = screen.upgrade() {
                screen.focus_first_empty();
            }
        });
    });
    dialog
}

pub fn add_account(
    parent: &impl IsA<gtk::Widget>,
    client: &Client,
    open_link: impl Fn(&str) + 'static,
    on_saved: impl Fn(&Submission) + 'static,
) -> adw::Dialog {
    let screen = Onboarding::new();
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

/// The dialog closes at the history step too, since the account is already saved by then and must
/// still come up.
pub fn add_account_asking_history(
    parent: &impl IsA<gtk::Widget>,
    client: &Client,
    open_link: impl Fn(&str) + 'static,
    on_saved: impl Fn(&Submission) + 'static,
) -> adw::Dialog {
    let screen = Onboarding::new();
    screen.focus_name();
    let presenter = Presenter::drive(&screen, client, open_link);
    let dialog = dialog(&screen, "Add account", &presenter);
    let on_saved = Rc::new(on_saved);
    let waiting: Rc<RefCell<Option<Submission>>> = Rc::default();
    presenter.connect_saved({
        let waiting = waiting.clone();
        move |submission| {
            waiting.replace(Some(submission.clone()));
        }
    });
    presenter.ask_sync_window({
        let dialog = dialog.downgrade();
        let on_saved = on_saved.clone();
        let waiting = waiting.clone();
        move |submission| {
            waiting.take();
            if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
            on_saved(submission);
        }
    });
    dialog.connect_closed(move |_| {
        if let Some(submission) = waiting.take() {
            on_saved(&submission);
        }
    });
    dialog.present(Some(parent));
    dialog
}

/// `None` when no account is picked or the accounts could not be read.
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
        let cancellation = ProbeCancellation::default();
        cancellation.stop();
        cancellation.stop();

        let stop = cancellation.restart();
        cancellation.stop();
        cancellation.stop();
        assert!(stop.is_stopped());
    }
}
