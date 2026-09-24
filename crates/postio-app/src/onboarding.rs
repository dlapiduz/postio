//! First run: probe, test, and write the account.
//!
//! `postio_gtk::onboarding` draws canvas 3e and knows nothing about mail. This
//! is the other half — the probe, the connection test and the two writes —
//! and it lives here for the reason everything in this crate lives here: the
//! view layer may not link `io-imap` or `rusqlite`, and all three need one or
//! the other.
//!
//! # Why not `postio-core`
//!
//! Because it cannot be done there, and the bead's own notes record the dead
//! end in detail: Cargo resolves one feature set per package across the
//! workspace, so a `default-features = false` edge from `postio-core` to
//! `postio-account` does not stop `io-imap` reaching `postio-gtk` through it,
//! and `scripts/checks/check-crate-boundaries.py` fails. That is a real constraint
//! and it is not worked around here — it is simply the wrong place to have
//! looked. The composition root already depends on `postio-account` with its
//! default features, is guarded against nothing, and is where `compose.rs`
//! and `feed.rs` already join the two halves.
//!
//! # The two writes
//!
//! An account row, and a credential in the keyring — the same pair
//! `examples/provision.rs` makes, which is what has been standing in for this
//! screen. The password goes to the Secret Service and nowhere else: not to
//! the store, not to `config.toml`, not to a log, and not into any error this
//! module produces.
//!
//! # Nothing here blocks the UI
//!
//! The probe and the connection test are both network work, and both are
//! spawned on the runtime and answered over a channel the main context
//! awaits — the same crossing `feed.rs` makes for a page read. The screen
//! stays live throughout and says which of the two it is waiting on.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use gtk::glib;
use postio_account::cancel::CancelToken;
use postio_account::discovery::{DiscoveryTransport, Probe};
use postio_core::CommandId;
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::onboarding::{BrowserSignIn, Onboarding, Status, Submission};
use postio_gtk::window::Window;
use postio_model::Account;
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

use crate::Wiring;
pub(crate) use postio_session::onboarding::configured;
use postio_session::onboarding::{
    SignInError, connection_settings, persist, persist_oauth, probe_options, prove, provider_name,
    run_sign_in, status_for, write_sync_window,
};

/// Whether this installation has an account yet.
///
/// A store that cannot be read counts as "no account": the screen is the only
/// way forward from there anyway, and refusing to show it would leave a
/// window with nothing in it and no way to fix that.
pub async fn needed(database: &Store) -> bool {
    let Ok(connection) = database.connect().await else {
        return true;
    };
    AccountRepository::new(&connection)
        .list_enabled()
        .await
        .map(|accounts| accounts.is_empty())
        .unwrap_or(true)
}

/// Put the first-run screen in the window and wire it to the network.
///
/// The screen becomes the window's whole content — there is nothing behind it
/// to go back to, so an overlay over an empty three-pane shell would be
/// pretending otherwise. `set_content` rather than anything in
/// `postio_gtk::window`, so the frontend needs no first-run concept at all.
///
/// On success the original content goes back and the application starts as it
/// would have if the account had been there all along.
///
/// `state`, `wired` and `events` are the same three pieces `run()`'s
/// `activate` handler already holds once an account is there from the
/// start — passed through so a screen that just created one can finish the
/// exact same sequence: install every command handler and drain the two
/// event queues, not merely feed the panes. Skipping that would leave a
/// window with mail in it and no key that does anything, the same shape of
/// bug `postio-bl2` is named for.
///
/// # `repairing`
///
/// `Some` when this is not a first run: [`crate::startup_route`] found an
/// account the keyring will not give up a password for, and sent it back
/// here rather than into a window that cannot sync. The screen arrives
/// knowing the address and the servers, says which one thing is missing,
/// and puts the cursor in the field for it. Before `postio-67` that state
/// had nowhere to go at all: onboarding only ran when the store held no
/// account, so an account with a broken credential was permanent.
#[allow(clippy::too_many_arguments)]
pub async fn install(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<CommandId>,
    events: Rc<RefCell<Option<EventStream>>>,
    notifier: crate::notifications::Notifier,
    repairing: Option<Account>,
    transport: Arc<dyn DiscoveryTransport>,
    opener: Arc<dyn postio_account::oauth::BrowserOpener>,
) {
    let screen = Onboarding::new();
    let previous = window.content();
    // Under the window's chrome, not instead of it.
    //
    // `set_content` replaces everything, and everything is where this window
    // keeps its header bar — so the wizard had no title bar and no close
    // button, and a first run could only be left by killing the process
    // (there is no `quit` command either). Giving the *screen* a header
    // instead put a title bar inside the wizard's own content, which draws as
    // part of the wizard rather than as the window's and looks wrong.
    //
    // So the screen goes under chrome of its own, whose top bar is the
    // window's title bar for as long as the wizard is up —
    // `widgets::under_window_chrome` says why it is flat and title-less.
    window.set_content(Some(&postio_gtk::widgets::under_window_chrome(&screen)));
    match &repairing {
        Some(account) => {
            screen.set_address(&account.address.address);
            screen.set_status(Status::Reauthenticate(configured(account)));
            if account.oauth.is_none() {
                screen.focus_password();
            }
        }
        // A fresh form starts at its first field, which is the name.
        None => screen.focus_name(),
    }

    // One per screen, shared by the two closures below: the probe replaces
    // the token in it, `Connect` clears it.
    let cancellation = ProbeCancellation::default();

    // The provider's OAuth offer, parked by the probe for the submit to
    // sign in with — and pre-filled on a repair, where the account row
    // already recorded the resolved endpoints (#534).
    let offer: OAuthOfferSlot = Rc::new(RefCell::new(repairing.as_ref().and_then(|account| {
        account
            .oauth
            .as_ref()
            .map(|oauth| postio_account::discovery::OAuthOffer {
                issuer: None,
                authorize: Some(oauth.authorize_url.clone()),
                token: Some(oauth.token_url.clone()),
                scopes: oauth.scopes.split_whitespace().map(str::to_owned).collect(),
                refresh_token_lifetime_days: oauth.refresh_token_lifetime_days,
            })
    })));
    // A repair over a JMAP account proves over JMAP again.
    let jmap: JmapOfferSlot =
        Rc::new(RefCell::new(repairing.as_ref().and_then(
            |account| match &account.backend {
                postio_model::account::Backend::Jmap { session_url } => {
                    Some(postio_account::discovery::JmapOffer {
                        session_url: session_url.clone(),
                    })
                }
                // A Gmail-REST repair re-proves through OAuth like any
                // other Gmail account; there is no JMAP offer to park.
                postio_model::account::Backend::Imap | postio_model::account::Backend::Gmail => {
                    None
                }
            },
        )));

    // The browser wait's own cancel token, wired to the screen's Cancel
    // button and Esc. Separate from the probe's: cancelling a sign-in must
    // not kill a probe and vice versa.
    let sign_in_cancel: Rc<RefCell<Option<CancelToken>>> = Rc::new(RefCell::new(None));
    screen.connect_cancel_sign_in({
        let sign_in_cancel = sign_in_cancel.clone();
        move || {
            if let Some(cancel) = sign_in_cancel.borrow().as_ref() {
                cancel.cancel();
            }
        }
    });

    screen.connect_probe({
        let screen = screen.clone();
        let runtime = wiring.runtime.clone();
        let cancellation = cancellation.clone();
        let offer = offer.clone();
        let jmap = jmap.clone();
        move |address| {
            probe_with_offer(
                &screen,
                &runtime,
                address,
                &cancellation,
                Arc::clone(&transport),
                offer.clone(),
                jmap.clone(),
            )
        }
    });

    // What finishes the screen for good: swap the window content back and
    // start the application over the new account, the exact sequence
    // `run()`'s `activate` handler runs when an account is there from the
    // start — installing the command handlers and draining the event
    // queues, not only feeding the panes. Without that a window fed here
    // would show mail and answer no key, which is the shape of bug
    // `postio-bl2` is named for.
    //
    // Held back from `submit`/`submit_oauth`'s own `on_saved` (below) by
    // the sync-window step (#876): the account and its credential are
    // already written by the time that step shows, so `Status::Saved` is
    // real, but the window this closure swaps to should not appear until
    // the user has chosen how far back to sync.
    let finish = {
        let window = window.clone();
        let wiring = wiring.clone();
        let previous = previous.clone();
        move || {
            postio_session::blocking::now(async {
                window.set_content(previous.as_ref());
                crate::open_account(&window, &wiring, &state, &wired, &events, &notifier).await;
            })
        }
    };
    screen.connect_start_sync({
        let finish = finish.clone();
        move |window| {
            if let Err(error) = write_sync_window(window) {
                tracing::warn!(%error, "could not save the chosen sync window");
            }
            finish();
        }
    });

    screen.connect_submit({
        let screen = screen.clone();
        let wiring = wiring.clone();
        let cancellation = cancellation.clone();
        // `submit`/`submit_oauth` show the sync-window step and stop —
        // `finish` runs from `connect_start_sync` above once the user picks
        // one and presses `Start sync`, not from here.
        let on_saved = {
            let screen = screen.clone();
            move || screen.set_status(Status::SyncWindow)
        };
        let offer = offer.clone();
        let jmap = jmap.clone();
        let sign_in_cancel = sign_in_cancel.clone();
        let opener = opener.clone();
        move |submission| {
            // Pressing Connect settles the question the probe was asking, and
            // the screen is on its way out either way. Leaving a discovery
            // request open past that point is a socket held for an answer
            // nobody will read.
            cancellation.stop();
            if submission.oauth_client.is_some() {
                let Some(offer) = offer.borrow().clone() else {
                    screen.set_status(Status::Failed(
                        "This provider's OAuth settings were not found — probe \
                         the address again."
                            .to_owned(),
                    ));
                    return;
                };
                let cancel = CancelToken::new();
                *sign_in_cancel.borrow_mut() = Some(cancel.clone());
                submit_oauth(
                    &screen,
                    &wiring,
                    submission.clone(),
                    offer,
                    cancel,
                    opener.clone(),
                    on_saved.clone(),
                );
            } else {
                submit(
                    &screen,
                    &wiring,
                    submission.clone(),
                    jmap.borrow().clone(),
                    on_saved.clone(),
                )
            }
        }
    });
}

/// The cancel token for the probe currently in flight, if there is one.
///
/// #57 gave the transport a token it can actually act on — a cancelled probe
/// now fails its socket at the next read rather than running on detached.
/// This is the other half: something has to *do* the cancelling, and before
/// this the composition root handed `Probe::run` a
/// `CancelToken::new()` it then dropped on the floor, so no probe in the
/// shipping application was ever cancellable at all.
///
/// `Rc<RefCell<..>>` rather than a plain field: the probe closure and the
/// submit closure both need it, and both are `'static` closures owned by the
/// screen.
#[derive(Clone, Default)]
pub(crate) struct ProbeCancellation(Rc<RefCell<Option<CancelToken>>>);

impl ProbeCancellation {
    /// Stops whatever probe is in flight and hands back a token for the new
    /// one.
    ///
    /// The view layer already refuses to start a second probe while
    /// `Status::is_busy`, so the cancel here is usually a no-op — but that
    /// guard lives in another crate and answers a question about *what the
    /// screen says*, which is not the same question as whether a socket is
    /// open. Two independent reasons to be correct is the right number for
    /// something whose failure is invisible.
    pub(crate) fn restart(&self) -> CancelToken {
        self.stop();
        let token = CancelToken::new();
        *self.0.borrow_mut() = Some(token.clone());
        token
    }

    /// Stops whatever probe is in flight, if any. Idempotent.
    pub(crate) fn stop(&self) {
        if let Some(token) = self.0.borrow_mut().take() {
            token.cancel();
        }
    }
}

/// Run the autoconfig probe for `address` and show what it found.
///
/// `transport` is supplied rather than constructed. It used to be built right
/// here, inside the spawned task, which meant the only way to reach this
/// function was to dial the network — and no test in the default suite may.
/// The mapping had already been split out into [`status_for`] so *it* could be
/// tested; everything around it, which is where the wiring lives, stayed
/// uncovered (#282).
pub(crate) fn probe(
    screen: &Onboarding,
    runtime: &tokio::runtime::Handle,
    address: &str,
    cancellation: &ProbeCancellation,
    transport: Arc<dyn DiscoveryTransport>,
    jmap: JmapOfferSlot,
) {
    probe_with_offer(
        screen,
        runtime,
        address,
        cancellation,
        transport,
        OAuthOfferSlot::default(),
        jmap,
    )
}

/// The OAuth offer the last successful probe carried, shared between the
/// probe that writes it and the submit that reads it (#534). The screen's
/// form fields cannot carry it — endpoints and scopes are protocol data
/// the widget deliberately does not know.
pub(crate) type OAuthOfferSlot = Rc<RefCell<Option<postio_account::discovery::OAuthOffer>>>;

/// The JMAP offer a preset row advertised (#545, ADR 0018 Q5) — parked at
/// probe time for the same reason as [`OAuthOfferSlot`]: endpoints are
/// protocol data the form fields deliberately do not carry. Present only
/// when the row's preference order puts `jmap` first.
pub(crate) type JmapOfferSlot = Rc<RefCell<Option<postio_account::discovery::JmapOffer>>>;

/// [`probe`], also parking the discovered OAuth offer in `offer` for the
/// submit handler to sign in with.
pub(crate) fn probe_with_offer(
    screen: &Onboarding,
    runtime: &tokio::runtime::Handle,
    address: &str,
    cancellation: &ProbeCancellation,
    transport: Arc<dyn DiscoveryTransport>,
    offer: OAuthOfferSlot,
    jmap: JmapOfferSlot,
) {
    screen.set_status(Status::Probing);

    let (sender, receiver) = async_channel::bounded(1);
    let email = address.to_owned();
    let cancel = cancellation.restart();
    runtime.spawn(async move {
        let probe = Probe::with_options(transport, probe_options());
        let answer = probe.run(&email, &cancel).await;
        let _ = sender.send(answer).await;
    });

    glib::spawn_future_local({
        let screen = screen.clone();
        async move {
            let Ok(answer) = receiver.recv().await else {
                // The runtime went away. Rare, and the form still works.
                screen.set_status(Status::Manual { suggestion: None });
                return;
            };
            match answer {
                Ok(report) => {
                    *offer.borrow_mut() = report
                        .settings()
                        .and_then(|settings| settings.oauth.clone());
                    *jmap.borrow_mut() = report.settings().and_then(|settings| {
                        (settings.backends.first().map(String::as_str) == Some("jmap"))
                            .then(|| settings.jmap.clone())
                            .flatten()
                    });
                    screen.set_status(status_for(&report));
                }
                Err(error) => {
                    tracing::info!(%error, "autoconfig found nothing");
                    *offer.borrow_mut() = None;
                    *jmap.borrow_mut() = None;
                    screen.set_status(Status::Manual { suggestion: None });
                }
            }
        }
    });
}

/// Test the credentials, then write the account and the password, then run
/// `on_saved` -- what happens next differs by host (#464): the first-run and
/// startup-repair screen replaces itself with the running application
/// ([`install`]'s own `on_saved`, built from the same five pieces this
/// function used to take directly); a credential-update dialog over an
/// already-running app
/// ([`crate::settings_credential::install`]) only has to close.
///
/// `on_saved` runs once, only after the credential and the account row are
/// both written -- never on a failed probe or a failed connection test.
pub(crate) fn submit(
    screen: &Onboarding,
    wiring: &Wiring,
    submission: Submission,
    jmap: Option<postio_account::discovery::JmapOffer>,
    on_saved: impl Fn() + 'static,
) {
    screen.set_status(Status::Connecting);

    let (sender, receiver) = async_channel::bounded(1);
    let proving = submission.clone();
    wiring.runtime.spawn(async move {
        let _ = sender.send(prove(&proving, jmap.as_ref()).await).await;
    });

    glib::spawn_future_local({
        let screen = screen.clone();
        let wiring = wiring.clone();
        async move {
            let answer = match receiver.recv().await {
                Ok(answer) => answer,
                Err(_) => Err("Postio's runtime stopped before the server answered.".to_owned()),
            };
            let backend = match answer {
                Ok(backend) => backend,
                Err(reason) => {
                    screen.set_status(Status::Failed(reason));
                    return;
                }
            };

            // Only now, with the credentials known good. Writing either half
            // first would leave a broken account behind every failed attempt.
            //
            // Both writes go over to the runtime together and answer over a
            // channel — the same crossing the connection test above makes,
            // and for the same reason: the keyring is a tokio future and this
            // is the glib main context. See [`persist`] for the order they
            // happen in and why it is that way round.
            let (sender, receiver) = async_channel::bounded(1);
            let database = wiring.database.clone();
            let secrets = wiring.secrets.clone();
            let written = submission.clone();
            wiring.runtime.spawn(async move {
                let _ = sender
                    .send(persist(&database, secrets.as_ref(), &written, backend).await)
                    .await;
            });
            let stored = receiver.recv().await.unwrap_or_else(|_| {
                Err("Postio's runtime stopped before the account was saved.".to_owned())
            });
            if let Err(reason) = stored {
                screen.set_status(Status::Failed(reason));
                return;
            }

            screen.set_status(Status::Saved);
            on_saved();
        }
    });
}

/// The browser sign-in, end to end (#534, ADR 0006 Q3): resolve the
/// endpoints, run [`postio_account::oauth::authorize`] through the system
/// browser, prove the token against the IMAP server, and only then
/// persist — the same nothing-stranded order the password path keeps.
///
/// Cancellable at every stage: `cancel` is the flow's own token, wired to
/// the screen's Cancel button and `Esc`. A cancelled attempt returns the
/// screen to the settings it was showing, because the user changed their
/// mind — that is not a failure and must not read as one.
pub(crate) fn submit_oauth(
    screen: &Onboarding,
    wiring: &Wiring,
    submission: Submission,
    offer: postio_account::discovery::OAuthOffer,
    cancel: CancelToken,
    opener: Arc<dyn postio_account::oauth::BrowserOpener>,
    on_saved: impl Fn() + 'static,
) {
    let Some(client) = submission.oauth_client.clone() else {
        return;
    };
    // What the browser is about to be asked to approve, so the screen can
    // say so rather than showing a spinner (#1179, `Design/screens/23`).
    // The scopes are known here, from the provider's own preset row; the
    // consent URL and the loopback port are not known until `authorize`
    // binds its listener and opens the browser, so they arrive through
    // `AnnouncingOpener` below, at the moment they become true.
    let (announce, announced) = async_channel::bounded(1);
    let opener: Arc<dyn postio_account::oauth::BrowserOpener> = Arc::new(AnnouncingOpener {
        inner: opener,
        announce,
    });
    screen.set_browser_sign_in(BrowserSignIn {
        provider: provider_name(&submission.settings),
        scopes: offer.scopes.clone(),
        ..BrowserSignIn::default()
    });
    screen.set_status(Status::WaitingForBrowser);

    glib::spawn_future_local({
        let screen = screen.clone();
        let scopes = offer.scopes.clone();
        let provider = provider_name(&submission.settings);
        async move {
            let Ok(url) = announced.recv().await else {
                return;
            };
            // The redirect the listener actually bound, read back off the
            // request rather than guessed: `authorize` chose an ephemeral
            // port, and this is the only place its number appears.
            let redirect_uri = url
                .query_pairs()
                .find(|(key, _)| key == "redirect_uri")
                .map(|(_, value)| value.into_owned())
                .unwrap_or_default();
            screen.set_browser_sign_in(BrowserSignIn {
                provider,
                scopes,
                redirect_uri,
                authorize_url: url.to_string(),
            });
        }
    });

    let settings = connection_settings(&submission);
    let (sender, receiver) = async_channel::bounded(1);
    let flow_cancel = cancel.clone();
    let scopes = offer.scopes.clone();
    let refresh_lifetime = offer.refresh_token_lifetime_days;
    wiring.runtime.spawn(async move {
        let answer = run_sign_in(&settings, &client, &offer, opener.as_ref(), &flow_cancel).await;
        let _ = sender.send(answer).await;
    });

    glib::spawn_future_local({
        let screen = screen.clone();
        let wiring = wiring.clone();
        async move {
            let answer = match receiver.recv().await {
                Ok(answer) => answer,
                Err(_) => Err(SignInError::Failed(
                    "Postio's runtime stopped before the sign-in finished.".to_owned(),
                )),
            };
            let (endpoints, tokens) = match answer {
                Ok(done) => done,
                Err(SignInError::Cancelled) => {
                    // The user's own Esc. Back to where they were, quietly.
                    screen.set_status(Status::Found(submission.settings.clone()));
                    return;
                }
                Err(SignInError::Failed(reason)) => {
                    screen.set_status(Status::Failed(reason));
                    return;
                }
            };

            let (sender, receiver) = async_channel::bounded(1);
            let database = wiring.database.clone();
            let secrets = wiring.secrets.clone();
            let written = submission.clone();
            let scopes = scopes.clone();
            wiring.runtime.spawn(async move {
                let _ = sender
                    .send(
                        persist_oauth(
                            &database,
                            secrets,
                            &written,
                            &endpoints,
                            &scopes,
                            refresh_lifetime,
                            tokens,
                        )
                        .await,
                    )
                    .await;
            });
            let stored = receiver.recv().await.unwrap_or_else(|_| {
                Err("Postio's runtime stopped before the account was saved.".to_owned())
            });
            if let Err(reason) = stored {
                screen.set_status(Status::Failed(reason));
                return;
            }

            screen.set_status(Status::Saved);
            on_saved();
        }
    });
}

/// Forwards to the real opener and announces the URL on its way past.
///
/// The consent URL and the loopback port it carries are built inside
/// [`postio_account::oauth::authorize`] and never returned — the only moment
/// they are visible to anything outside that function is the call to
/// [`postio_account::oauth::BrowserOpener::open`]. So the screen learns what
/// it is showing from here rather than from a second construction of the
/// same URL, which would be a copy that could drift from the one the browser
/// actually got.
///
/// `try_send` on a bounded(1) channel, never `send`: `open` is called on the
/// runtime and must not be made to wait on a GTK task that may never run —
/// a screen that has already been closed leaves nobody receiving, and
/// blocking the sign-in on that would be a hang rather than a missing line.
struct AnnouncingOpener {
    inner: Arc<dyn postio_account::oauth::BrowserOpener>,
    announce: async_channel::Sender<postio_account::oauth::Url>,
}

impl postio_account::oauth::BrowserOpener for AnnouncingOpener {
    fn open(&self, url: &postio_account::oauth::Url) -> std::io::Result<()> {
        let _ = self.announce.try_send(url.clone());
        self.inner.open(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_account::discovery::{AccountSettings, DiscoveryOutcome};
    use postio_account::secret::{AccountKey, SecretStore};
    use postio_gtk::onboarding::Server;
    use postio_gtk::onboarding::Settings;
    use postio_model::account::TransportSecurity;
    use postio_session::onboarding::explain;
    use std::time::Duration;

    use postio_account::discovery::{DiscoveryReport, ServerSettings, SettingsSource};

    // -- Cancelling the probe that is in flight (#57) ---------------------
    //
    // The transport can now act on a cancelled token, and the composition
    // root used to hand it a `CancelToken::new()` it immediately forgot --
    // so no probe in the shipping application was cancellable, whatever the
    // layers underneath could do. These cover the bookkeeping that changed;
    // the two call sites using it are one line each.

    #[test]
    fn a_probe_gets_a_live_token() {
        let cancellation = ProbeCancellation::default();
        let token = cancellation.restart();
        assert!(
            !token.is_cancelled(),
            "the probe was handed a token that was already spent"
        );
    }

    #[test]
    fn starting_a_probe_stops_the_one_before_it() {
        let cancellation = ProbeCancellation::default();
        let first = cancellation.restart();
        let second = cancellation.restart();

        assert!(first.is_cancelled(), "the earlier probe kept its socket");
        assert!(!second.is_cancelled(), "the new probe starts live");
    }

    #[test]
    fn leaving_the_screen_stops_the_probe() {
        let cancellation = ProbeCancellation::default();
        let token = cancellation.restart();

        cancellation.stop();

        assert!(
            token.is_cancelled(),
            "pressing Connect left a discovery request open for an answer \
             nobody will read"
        );
    }

    #[test]
    fn stopping_twice_is_harmless() {
        // `Connect` can be pressed without a probe ever having run -- typed
        // address, straight to the password field.
        let cancellation = ProbeCancellation::default();
        cancellation.stop();
        cancellation.stop();

        let token = cancellation.restart();
        cancellation.stop();
        cancellation.stop();
        assert!(token.is_cancelled());
    }

    /// A report from a domain that publishes nothing, with the guess on.
    fn nothing_published(suggestion: Option<AccountSettings>) -> DiscoveryReport {
        DiscoveryReport {
            email: "lena@example.com".to_owned(),
            domain: "example.com".to_owned(),
            outcome: DiscoveryOutcome::ManualEntry { suggestion },
            attempts: Vec::new(),
        }
    }

    /// What `guess_common_names` produces for `example.com`.
    fn guessed() -> AccountSettings {
        AccountSettings {
            imap: ServerSettings {
                host: "imap.example.com".to_owned(),
                port: 993,
                encryption: postio_account::discovery::Encryption::Tls,
            },
            smtp: ServerSettings {
                host: "smtp.example.com".to_owned(),
                port: 465,
                encryption: postio_account::discovery::Encryption::Tls,
            },
            email: "lena@example.com".to_owned(),
            login: "lena@example.com".to_owned(),
            display_name: None,
            source: SettingsSource::Guess,
            requires_app_password: false,
            note: None,
            password_help_url: None,
            oauth: None,
            jmap: None,
            backends: vec!["imap".to_owned()],
        }
    }

    /// [`guessed`], but resolved by `source` and carrying `display_name` --
    /// the shape a preset row or an autoconfig/ISPDB document actually
    /// produces (#1115).
    fn resolved(source: SettingsSource, display_name: Option<&str>) -> AccountSettings {
        AccountSettings {
            display_name: display_name.map(str::to_owned),
            source,
            ..guessed()
        }
    }

    use postio_account::secret::MemorySecretStore;

    /// The account the store holds, if it holds one.
    async fn stored(database: &Store) -> Option<Account> {
        let connection = database.connect().await.expect("a connection");
        let accounts = AccountRepository::new(&connection)
            .list()
            .await
            .expect("the accounts should read");
        assert!(
            accounts.len() < 2,
            "onboarding wrote {} rows",
            accounts.len()
        );
        accounts.into_iter().next()
    }

    fn submission(host: &str, security: TransportSecurity) -> Submission {
        Submission {
            address: "lena@example.com".to_owned(),
            name: String::new(),
            password: "hunter2".to_owned(),
            oauth_client: None,
            settings: Settings {
                imap: Server {
                    host: host.to_owned(),
                    port: 993,
                    security,
                },
                smtp: Server {
                    host: "smtp.example.com".to_owned(),
                    port: 465,
                    security: TransportSecurity::Tls,
                },
                login: "lena@example.com".to_owned(),
                ..Settings::default()
            },
        }
    }

    #[test]
    fn the_connection_test_uses_the_login_name_not_the_address() {
        // An iCloud custom domain logs in as the Apple ID, which is the case
        // `examples/provision.rs` needs POSTIO_USERNAME for.
        let mut wanted = submission("imap.mail.me.com", TransportSecurity::Tls);
        wanted.settings.login = "lena@example.net".to_owned();

        let settings = connection_settings(&wanted);
        assert_eq!(settings.username, "lena@example.net");
        assert_eq!(settings.host, "imap.mail.me.com");
        assert_eq!(settings.port, 993);
        assert_eq!(settings.security, TransportSecurity::Tls);
    }

    #[test]
    fn a_server_without_implicit_tls_is_tested_over_starttls() {
        let settings =
            connection_settings(&submission("mail.example.com", TransportSecurity::StartTls));
        assert_eq!(settings.security, TransportSecurity::StartTls);
    }

    #[test]
    fn a_rejected_password_says_what_to_do_about_it() {
        let reason = explain(&postio_account::backend::BackendError::Auth {
            account: "lena@example.com".to_owned(),
            reason: "AUTHENTICATIONFAILED".to_owned(),
        });

        assert!(
            reason.contains("app-specific password"),
            "the commonest cause of this has to be named: {reason}"
        );
        assert!(
            !reason.contains("hunter2"),
            "no failure may ever carry the password: {reason}"
        );
    }

    #[test]
    fn a_timeout_names_the_budget_it_blew() {
        let reason = explain(&postio_account::backend::BackendError::TimedOut {
            context: "login".to_owned(),
            after: Duration::from_secs(30),
        });
        assert!(reason.contains("30s"), "{reason}");
    }

    #[test]
    fn tls_failure_says_postio_will_not_downgrade() {
        let reason = explain(&postio_account::backend::BackendError::Tls {
            host: "imap.example.com".to_owned(),
            reason: "certificate expired".to_owned(),
        });
        assert!(reason.contains("imap.example.com"), "{reason}");
        assert!(reason.contains("will not fall back"), "{reason}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_fresh_store_needs_onboarding_and_one_with_an_account_does_not() {
        let database = postio_storage::test_support::memory().await;
        assert!(needed(&database).await, "nothing has been provisioned yet");

        let connection = database.connect().await.expect("a connection");
        let _account = postio_storage::test_support::account(&connection).await;
        drop(connection);
        assert!(
            !needed(&database).await,
            "an account exists, so the screen is done"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_credential_that_cannot_be_stored_leaves_no_account_behind() {
        // `postio-67`: 0.1.0 wrote the row first. When the keyring write then
        // failed, the row stayed — and every launch after that opened an
        // account with no reachable password, in an application whose only
        // credential writer is the screen that never runs again.
        let database = postio_storage::test_support::memory().await;

        let outcome = persist(
            &database,
            &MemorySecretStore::locked(),
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await;

        assert!(outcome.is_err(), "a locked keyring has to fail the submit");
        assert!(
            stored(&database).await.is_none(),
            "the account row outlived the credential write that failed"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_first_run_writes_both_the_row_and_the_credential() {
        let database = postio_storage::test_support::memory().await;
        let secrets = MemorySecretStore::new();

        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("both writes should land");

        let account = stored(&database).await.expect("an account row");
        assert_eq!(account.address.address, "lena@example.com");
        assert_eq!(account.incoming.host, "imap.example.com");
        assert_eq!(
            secrets
                .retrieve(&AccountKey::new("lena@example.com"))
                .await
                .expect("a credential")
                .expose(),
            "hunter2"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_name_at_onboarding_becomes_the_from_name_and_the_sidebar_label() {
        let database = postio_storage::test_support::memory().await;
        let secrets = MemorySecretStore::new();
        let mut named = submission("imap.example.com", TransportSecurity::Tls);
        named.name = "Lena Lovelace".to_owned();

        persist(
            &database,
            &secrets,
            &named,
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("both writes should land");

        let account = stored(&database).await.expect("an account row");
        assert_eq!(account.display_name, "Lena Lovelace");
        assert_eq!(account.address.name.as_deref(), Some("Lena Lovelace"));
        assert_eq!(
            account.identities[0].display_name, "Lena Lovelace",
            "the From header reads this, per EmailAddress::display"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_blank_name_leaves_the_address_as_the_label_exactly_as_before() {
        let database = postio_storage::test_support::memory().await;
        let secrets = MemorySecretStore::new();

        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("both writes should land");

        let account = stored(&database).await.expect("an account row");
        assert_eq!(account.display_name, "lena@example.com");
        assert_eq!(account.address.name, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_account_row_that_will_not_write_takes_its_credential_back() {
        // The other order's failure, and the reason the rollback is here: a
        // secret Postio kept for an account that does not exist is a secret
        // nobody asked it to keep.
        let database = postio_storage::test_support::memory().await;
        let connection = database.connect().await.expect("a connection");
        connection
            .execute("ALTER TABLE accounts RENAME TO accounts_elsewhere", ())
            .await
            .expect("the table should move out of the way");
        drop(connection);
        let secrets = MemorySecretStore::new();

        let outcome = persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await;

        assert!(outcome.is_err(), "there is no table to write the row into");
        assert!(
            secrets.is_empty(),
            "the credential stayed behind for an account that was never created"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn signing_in_again_repairs_the_account_rather_than_duplicating_it() {
        // What a repair run does. `startup_route` sends an account with no
        // credential back to this screen, so the second submit arrives over a
        // row that already exists — and a second row would leave
        // `first_account` picking between two.
        let database = postio_storage::test_support::memory().await;
        let secrets = MemorySecretStore::new();
        persist(
            &database,
            &secrets,
            &submission("old.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the first run should land");
        let first = stored(&database).await.expect("an account row");

        persist(
            &database,
            &secrets,
            &submission("new.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the repair should land");

        // `stored` fails the test outright on a second row.
        let repaired = stored(&database).await.expect("an account row");
        assert_eq!(repaired.id, first.id, "the repair replaced the account");
        assert_eq!(
            repaired.incoming.host, "new.example.com",
            "the repair did not take the corrected server"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_repair_keeps_the_identity_the_drafts_point_at() {
        // `AccountRepository::update` makes the identity list authoritative,
        // so a repair that rebuilt the list from scratch would delete the
        // identity every saved draft refers to.
        let database = postio_storage::test_support::memory().await;
        let secrets = MemorySecretStore::new();
        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the first run should land");
        let before = stored(&database).await.expect("an account row");
        let identity = before
            .identities
            .first()
            .expect("a first run gives the account its default identity")
            .id;

        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the repair should land");

        let after = stored(&database).await.expect("an account row");
        assert_eq!(
            after.identities.first().map(|i| i.id),
            Some(identity),
            "the repair rewrote the identity, orphaning anything pointing at it"
        );
    }

    #[test]
    fn the_probe_asks_for_a_guess_when_nothing_is_published() {
        // `postio-69`: the screen handed the user five empty boxes for the
        // one domain shape least able to fill them in — a custom domain that
        // publishes no autoconfig. The guess is off by default in
        // `postio-account` on purpose (an unverified guess presented as a
        // *discovery* is worse than nothing); the composition root turns it
        // on because `Status::Manual` presents it as a starting point to
        // edit, which is a different claim.
        assert!(
            probe_options().guess_common_names,
            "with the guess off there is nothing to prefill the manual form with"
        );
    }

    #[test]
    fn a_guess_reaches_the_form_as_a_prefill_rather_than_being_dropped() {
        let status = status_for(&nothing_published(Some(guessed())));

        let Status::Manual {
            suggestion: Some(settings),
        } = status
        else {
            panic!("the guess did not reach the form: {status:?}");
        };
        assert_eq!(settings.imap.host, "imap.example.com");
        assert_eq!(settings.imap.port, 993);
        assert_eq!(settings.smtp.host, "smtp.example.com");
        assert_eq!(settings.smtp.port, 465);
        assert_eq!(settings.login, "lena@example.com");
    }

    #[test]
    fn a_guess_is_never_shown_as_a_discovery() {
        // The whole reason the guess is safe to turn on. `Status::Found`
        // says Postio looked this up; `Status::Manual` says "here is a
        // starting point, check it". A guess must only ever be the second.
        assert!(matches!(
            status_for(&nothing_published(Some(guessed()))),
            Status::Manual { .. }
        ));
    }

    #[test]
    fn a_domain_that_publishes_nothing_and_cannot_be_guessed_still_opens_the_form() {
        assert!(matches!(
            status_for(&nothing_published(None)),
            Status::Manual { suggestion: None }
        ));
    }

    /// A discovered account, from `settings` -- the `Status::Found` half of
    /// [`status_for`], which is the only path [`shown`] is reached through.
    fn found(settings: AccountSettings) -> Settings {
        let report = DiscoveryReport {
            email: "lena@example.com".to_owned(),
            domain: "example.com".to_owned(),
            outcome: DiscoveryOutcome::Discovered(settings),
            attempts: Vec::new(),
        };
        match status_for(&report) {
            Status::Found(settings) => settings,
            other => panic!("expected Status::Found, got {other:?}"),
        }
    }

    #[test]
    fn a_preset_row_names_itself_rather_than_the_mechanism_that_found_it() {
        // #1115: providers.toml's own row, not `SettingsSource::label()`'s
        // generic "known provider" -- a fixture row, not a shipped
        // provider, so this test does not name a real vendor.
        let settings = resolved(SettingsSource::Builtin, Some("My Own Provider"));
        assert_eq!(found(settings).source, "My Own Provider");
    }

    #[test]
    fn a_preset_row_with_no_display_name_falls_back_to_the_source_label() {
        // Nothing to name it with -- `settings_for` always sets one, but
        // the fallback exists for exactly the case that guarantee slips.
        let settings = resolved(SettingsSource::Builtin, None);
        assert_eq!(found(settings).source, SettingsSource::Builtin.label());
    }

    #[test]
    fn an_autoconfig_documents_own_display_name_does_not_override_the_source_label() {
        // The trap this fix has to avoid: autoconfig and ISPDB documents can
        // carry their own `<displayName>` (`discovery::mod.rs`'s shared
        // XML-shaped builder), so `display_name.is_some()` alone cannot be
        // the test -- #877 decided a scraped document still names its
        // *mechanism*, only a preset row Postio ships by hand names itself.
        for source in [
            SettingsSource::WellKnown,
            SettingsSource::Autoconfig,
            SettingsSource::Ispdb,
            SettingsSource::Srv,
            SettingsSource::Mx,
            SettingsSource::Guess,
        ] {
            let settings = resolved(source, Some("A Document's Own Name"));
            assert_eq!(
                found(settings).source,
                source.label(),
                "{source:?} must still show its own label, not the document's display name"
            );
        }
    }
}
