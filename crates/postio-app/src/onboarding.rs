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
//! `postio-imap` does not stop `io-imap` reaching `postio-gtk` through it,
//! and `scripts/checks/check-crate-boundaries.py` fails. That is a real constraint
//! and it is not worked around here — it is simply the wrong place to have
//! looked. The composition root already depends on `postio-imap` with its
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
use postio_core::CommandId;
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::onboarding::{Onboarding, Server, Settings, Status, Submission};
use postio_gtk::window::Window;
use postio_imap::cancel::CancelToken;
use postio_imap::discovery::{
    AccountSettings, DiscoveryOutcome, DiscoveryReport, DiscoveryTransport, Encryption, Probe,
    ProbeOptions,
};
use postio_imap::imap::{ConnectionSettings, ImapSession, RustlsConnector};
use postio_imap::secret::{AccountKey, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity};
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

use crate::Wiring;

/// Whether this installation has an account yet.
///
/// A store that cannot be read counts as "no account": the screen is the only
/// way forward from there anyway, and refusing to show it would leave a
/// window with nothing in it and no way to fix that.
pub fn needed(database: &Database) -> bool {
    let Ok(connection) = database.connection() else {
        return true;
    };
    AccountRepository::new(&connection)
        .list_enabled()
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
pub fn install(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<CommandId>,
    events: Rc<RefCell<Option<EventStream>>>,
    notifier: crate::notifications::Notifier,
    repairing: Option<Account>,
    transport: Arc<dyn DiscoveryTransport>,
    opener: Arc<dyn postio_imap::oauth::BrowserOpener>,
) {
    let screen = Onboarding::new();
    let previous = window.content();
    window.set_content(Some(&screen));
    match &repairing {
        Some(account) => {
            screen.set_address(&account.address.address);
            screen.set_status(Status::Reauthenticate(configured(account)));
            if account.oauth.is_none() {
                screen.focus_password();
            }
        }
        None => screen.focus_address(),
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
            .map(|oauth| postio_imap::discovery::OAuthOffer {
                issuer: None,
                authorize: Some(oauth.authorize_url.clone()),
                token: Some(oauth.token_url.clone()),
                scopes: oauth.scopes.split_whitespace().map(str::to_owned).collect(),
            })
    })));
    // A repair over a JMAP account proves over JMAP again.
    let jmap: JmapOfferSlot =
        Rc::new(RefCell::new(repairing.as_ref().and_then(
            |account| match &account.backend {
                postio_model::account::Backend::Jmap { session_url } => {
                    Some(postio_imap::discovery::JmapOffer {
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

    screen.connect_submit({
        let screen = screen.clone();
        let wiring = wiring.clone();
        let cancellation = cancellation.clone();
        let on_saved = {
            let window = window.clone();
            let wiring = wiring.clone();
            let previous = previous.clone();
            move || {
                window.set_content(previous.as_ref());
                // The exact sequence `run()`'s `activate` handler runs when
                // an account is there from the start — installing the
                // command handlers and draining the event queues, not only
                // feeding the panes. Without that a window fed here would
                // show mail and answer no key, which is the shape of bug
                // `postio-bl2` is named for.
                crate::open_account(&window, &wiring, &state, &wired, &events, &notifier);
            }
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

/// How this application probes, as against how the crate probes by default.
///
/// The one difference is `guess_common_names`, which `postio-imap` ships off
/// because "an unverified guess presented as a *discovery* is worse than an
/// empty form" — and it is right about that. The composition root turns it on
/// because it controls what the guess is presented *as*: [`status_for`] can
/// only ever put it in [`Status::Manual`], the state whose heading says no
/// settings were published and whose form is open for editing. That is a
/// starting point, not a claim.
///
/// `postio-69`: without it, a domain publishing no autoconfig — every custom
/// domain, which is exactly the person least able to answer — got five empty
/// boxes.
fn probe_options() -> ProbeOptions {
    ProbeOptions {
        guess_common_names: true,
        ..ProbeOptions::default()
    }
}

/// What the screen should show for `report`.
///
/// Split out of [`probe`] so it can be driven without a network: the mapping
/// is where a discovery becomes a sentence, and it is the half that had the
/// bug.
fn status_for(report: &DiscoveryReport) -> Status {
    let found = report.settings().map(shown);
    match (&report.outcome, found) {
        (DiscoveryOutcome::Discovered(_), Some(settings)) => Status::Found(settings),
        // Everything else is manual entry, prefilled when there was anything
        // to prefill with. Never `Found`: see `probe_options`.
        (_, suggestion) => Status::Manual { suggestion },
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
pub(crate) type OAuthOfferSlot = Rc<RefCell<Option<postio_imap::discovery::OAuthOffer>>>;

/// The JMAP offer a preset row advertised (#545, ADR 0018 Q5) — parked at
/// probe time for the same reason as [`OAuthOfferSlot`]: endpoints are
/// protocol data the form fields deliberately do not carry. Present only
/// when the row's preference order puts `jmap` first.
pub(crate) type JmapOfferSlot = Rc<RefCell<Option<postio_imap::discovery::JmapOffer>>>;

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
    jmap: Option<postio_imap::discovery::JmapOffer>,
    on_saved: impl Fn() + 'static,
) {
    screen.set_status(Status::Connecting);

    let settings = connection_settings(&submission);
    // The password crosses to the runtime and back no further: it is moved
    // into the task, used for one login, and dropped there.
    let password = Password::new(submission.password.clone());
    let (sender, receiver) = async_channel::bounded(1);
    wiring.runtime.spawn(async move {
        // The proof tries backends in the row's preference order and the
        // first that works is the one stored (#545): a credential that
        // only speaks IMAP still lands on a provider advertising JMAP,
        // and one that speaks JMAP gets the native protocol.
        if let Some(offer) = &jmap
            && let Ok(url) = offer.session_url.parse()
        {
            let proof = postio_jmap::JmapBackend::new(url, password.expose());
            match postio_imap::backend::MailBackend::connect(&proof).await {
                Ok(_) => {
                    let backend = postio_model::account::Backend::Jmap {
                        session_url: offer.session_url.clone(),
                    };
                    let _ = sender.send(Ok(backend)).await;
                    return;
                }
                Err(error) => {
                    tracing::info!(
                        %error,
                        "the JMAP proof failed; trying the next backend"
                    );
                }
            }
        }
        let answer = match RustlsConnector::new() {
            Ok(connector) => ImapSession::open(&settings, &password, &connector)
                .await
                .map(|_| postio_model::account::Backend::Imap)
                .map_err(|error| explain(&error)),
            Err(error) => Err(format!(
                "Postio could not start a TLS connection on this machine: {error}"
            )),
        };
        let _ = sender.send(answer).await;
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
/// endpoints, run [`postio_imap::oauth::authorize`] through the system
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
    offer: postio_imap::discovery::OAuthOffer,
    cancel: CancelToken,
    opener: Arc<dyn postio_imap::oauth::BrowserOpener>,
    on_saved: impl Fn() + 'static,
) {
    let Some(client) = submission.oauth_client.clone() else {
        return;
    };
    screen.set_status(Status::WaitingForBrowser);

    let settings = connection_settings(&submission);
    let (sender, receiver) = async_channel::bounded(1);
    let flow_cancel = cancel.clone();
    let scopes = offer.scopes.clone();
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
                        persist_oauth(&database, secrets, &written, &endpoints, &scopes, tokens)
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

/// How a sign-in attempt ended without tokens.
enum SignInError {
    /// The user cancelled — the screen goes back, not to a failure.
    Cancelled,
    /// Everything else, in words the user can act on.
    Failed(String),
}

/// The runtime half of the sign-in: endpoints, the browser flow, and the
/// proof against the IMAP server, in that order.
async fn run_sign_in(
    settings: &ConnectionSettings,
    client: &postio_gtk::onboarding::OAuthClientSubmission,
    offer: &postio_imap::discovery::OAuthOffer,
    opener: &dyn postio_imap::oauth::BrowserOpener,
    cancel: &CancelToken,
) -> Result<
    (
        postio_imap::oauth::Endpoints,
        postio_imap::oauth::TokenResponse,
    ),
    SignInError,
> {
    use postio_imap::oauth;

    let cancelled = |error: &oauth::OAuthError| matches!(error, oauth::OAuthError::Cancelled);

    // Endpoints: the row's own, or resolved from its issuer (RFC 8414 —
    // ADR 0006 Q4 as amended by #152). Both are validated at preset load,
    // so a row reaching here without either is a bug worth the sentence.
    let endpoints = match (&offer.authorize, &offer.token) {
        (Some(authorize), Some(token)) => oauth::Endpoints {
            authorize: authorize.parse().map_err(|error| {
                SignInError::Failed(format!(
                    "The provider's sign-in address is invalid: {error}"
                ))
            })?,
            token: token.parse().map_err(|error| {
                SignInError::Failed(format!("The provider's token address is invalid: {error}"))
            })?,
        },
        _ => {
            let issuer = offer.issuer.as_deref().ok_or_else(|| {
                SignInError::Failed(
                    "This provider's settings name no OAuth endpoints — check the \
                     providers.toml row."
                        .to_owned(),
                )
            })?;
            let issuer = issuer.parse().map_err(|error| {
                SignInError::Failed(format!("The provider's issuer is invalid: {error}"))
            })?;
            oauth::exchange::resolve_endpoints(&issuer, cancel)
                .await
                .map_err(|error| {
                    if cancelled(&error) {
                        SignInError::Cancelled
                    } else {
                        SignInError::Failed(format!(
                            "Could not discover the provider's sign-in endpoints: {error}"
                        ))
                    }
                })?
        }
    };

    let tokens = oauth::authorize(
        oauth::AuthorizeRequest {
            client_id: client.client_id.clone(),
            client_secret: client.client_secret.clone(),
            authorize_endpoint: endpoints.authorize.clone(),
            token_endpoint: endpoints.token.clone(),
            scopes: offer.scopes.clone(),
        },
        opener,
        cancel,
    )
    .await
    .map_err(|error| {
        if cancelled(&error) {
            SignInError::Cancelled
        } else {
            SignInError::Failed(format!("The sign-in did not complete: {error}"))
        }
    })?;

    // The proof, before anything persists: the token opens a real session
    // against the account's own IMAP server, the same test the password
    // path runs. A consent screen that granted the wrong scopes fails
    // here, in front of the user, instead of at the first background sync.
    let mut verified = settings.clone();
    verified.auth = postio_model::account::AuthMethod::XOAuth2;
    let connector = RustlsConnector::new().map_err(|error| {
        SignInError::Failed(format!(
            "Postio could not start a TLS connection on this machine: {error}"
        ))
    })?;
    ImapSession::open(&verified, &tokens.access_token, &connector)
        .await
        .map(|_| ())
        .map_err(|error| SignInError::Failed(explain(&error)))?;

    Ok((endpoints, tokens))
}

/// The OAuth writes, in the same nothing-stranded order [`persist`] keeps:
/// secrets first, then the row, rolling the secrets back if the row write
/// fails.
async fn persist_oauth(
    database: &Database,
    secrets: Arc<dyn SecretStore>,
    submission: &Submission,
    endpoints: &postio_imap::oauth::Endpoints,
    scopes: &[String],
    tokens: postio_imap::oauth::TokenResponse,
) -> Result<(), String> {
    let Some(client) = submission.oauth_client.clone() else {
        return Err("The sign-in lost its client on the way to the store.".to_owned());
    };
    let key = AccountKey::new(submission.address.clone());

    let source = postio_imap::oauth::OwnClientTokenSource::new(
        secrets.clone(),
        endpoints.token.clone(),
        client.client_id.clone(),
        client.client_secret.clone(),
    );
    source.seed(&key, tokens).await.map_err(|error| {
        format!(
            "The sign-in worked but its token could not be stored in the \
             keyring: {error}. Is the keyring unlocked?"
        )
    })?;
    if let Some(secret) = &client.client_secret {
        source
            .store_client_secret(&key, &Password::new(secret.clone()))
            .await
            .map_err(|error| {
                format!("The OAuth client secret could not be stored in the keyring: {error}")
            })?;
    }

    if let Err(reason) = save_oauth(database, submission, &client, endpoints, scopes) {
        // Roll the secrets back the same way `persist` does: nothing reads
        // a credential no account row names, but leaving one is untidy.
        let _ = secrets
            .delete(&AccountKey::new(format!("{}#oauth-refresh", key.account())))
            .await;
        return Err(reason);
    }
    Ok(())
}

/// The row write for an OAuth sign-in: auth method, client, endpoints.
fn save_oauth(
    database: &Database,
    submission: &Submission,
    client: &postio_gtk::onboarding::OAuthClientSubmission,
    endpoints: &postio_imap::oauth::Endpoints,
    scopes: &[String],
) -> Result<(), String> {
    // A browser sign-in is an IMAP account today; the Gmail REST backend
    // is #546, gated on its preset row flipping after #195.
    save(database, submission, postio_model::account::Backend::Imap)?;
    let connection = database
        .connection()
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    let Some(mut account) = repository
        .list()
        .map_err(|error| format!("Postio could not read its local store: {error}"))?
        .into_iter()
        .find(|account| {
            account
                .address
                .address
                .eq_ignore_ascii_case(&submission.address)
        })
    else {
        return Err("The account row vanished while it was being written.".to_owned());
    };
    account.auth = AuthMethod::XOAuth2;
    account.oauth = Some(postio_model::account::OAuthConfig {
        client_id: client.client_id.clone(),
        token_url: endpoints.token.to_string(),
        authorize_url: endpoints.authorize.to_string(),
        scopes: scopes.join(" "),
    });
    repository
        .update(&mut account)
        .map_err(|error| format!("Postio could not record the sign-in: {error}"))
}

/// Both writes, in the order that cannot strand an account.
///
/// **The credential first, then the row.** 0.1.0 did it the other way round
/// and `postio-67` is what that cost: a keyring write that failed after the
/// row was committed left an account with no reachable password, which could
/// not sync, could not authenticate, and could not be repaired from inside
/// the application — onboarding is the only thing that writes a credential,
/// and `first_account().is_some()` meant onboarding never ran again.
///
/// The failure that order *does* leave behind — a secret with no account —
/// is rolled back here, and would be harmless even if the rollback failed:
/// nothing reads a credential no account row names.
///
/// **Must be polled on the engine runtime, not the GTK main context.** The
/// keyring is reached over D-Bus by a future bounded with
/// `tokio::time::timeout`, so awaiting it from `glib::spawn_future_local`
/// panics with "there is no reactor running" — `postio-66`, which shipped.
/// `feed.rs` states the rule: neither loop can drive the other, so runtime
/// work is spawned and answered over a channel.
async fn persist(
    database: &Database,
    secrets: &dyn SecretStore,
    submission: &Submission,
    backend: postio_model::account::Backend,
) -> Result<(), String> {
    let key = AccountKey::new(submission.address.clone());
    let password = Password::new(submission.password.clone());
    // Reported rather than swallowed: an account with no password in the
    // keyring cannot sync, and a silent failure would read as a Postio bug
    // rather than as a locked keyring.
    secrets.store(&key, &password).await.map_err(|error| {
        format!(
            "The password could not be stored in the keyring: {error}. \
             Is the keyring unlocked?"
        )
    })?;

    if let Err(reason) = save(database, submission, backend) {
        if let Err(error) = secrets.delete(&key).await {
            // Safe to log: no `SecretError` carries a password.
            tracing::warn!(%error, "the rolled-back credential could not be removed");
        }
        return Err(reason);
    }
    Ok(())
}

/// Write the account row, creating it or repairing the one already there.
///
/// Synchronous, because rusqlite is; called from [`persist`] on the engine
/// runtime, where one indexed insert is not worth a `spawn_blocking`.
///
/// # Why this can be a repair
///
/// Since `postio-67` the screen is reachable a second time: an account whose
/// credential the keyring will not give up is sent back here rather than
/// opened. That submit arrives over a row that already exists, and a second
/// row would leave `first_account` choosing between two accounts for the
/// same address. So an existing row is *updated* — and its identities are
/// left exactly as they are, because [`AccountRepository::update`] makes the
/// list it is handed authoritative and every saved draft points at one.
fn save(
    database: &Database,
    submission: &Submission,
    backend: postio_model::account::Backend,
) -> Result<(), String> {
    let connection = database
        .connection()
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    let existing = repository
        .list()
        .map_err(|error| format!("Postio could not read its local store: {error}"))?
        .into_iter()
        .find(|account| {
            account
                .address
                .address
                .eq_ignore_ascii_case(&submission.address)
        });

    match existing {
        Some(mut account) => {
            configure(&mut account, submission);
            account.backend = backend;
            repository
                .update(&mut account)
                .map_err(|error| format!("Postio could not update the account: {error}"))
        }
        None => {
            let email = EmailAddress::new(None::<String>, submission.address.clone());
            let mut account = Account::new(submission.address.clone(), email.clone());
            configure(&mut account, submission);
            account.backend = backend;
            let mut identity = Identity::new(AccountId::UNASSIGNED, email);
            identity.is_default = true;
            account.identities = vec![identity];
            repository
                .create(&mut account)
                .map(|_| ())
                .map_err(|error| format!("Postio could not write the account: {error}"))
        }
    }
}

/// Put the submitted servers on `account`, leaving its identities alone.
fn configure(account: &mut Account, submission: &Submission) {
    account.incoming.host = submission.settings.imap.host.clone();
    account.incoming.port = submission.settings.imap.port;
    account.incoming.security = submission.settings.imap.security;
    account.incoming.username = submission.settings.login.clone();
    account.outgoing.host = submission.settings.smtp.host.clone();
    account.outgoing.port = submission.settings.smtp.port;
    account.outgoing.security = submission.settings.smtp.security;
    account.outgoing.username = submission.settings.login.clone();
    // An OAuth submission's auth and client are written by `persist_oauth`,
    // which is the only caller holding the resolved endpoints; a password
    // submission resets both, so switching a repaired account from OAuth
    // back to a password leaves no stale client behind.
    if submission.oauth_client.is_none() {
        account.auth = AuthMethod::Password;
        account.oauth = None;
    }
    // A repair over an account somebody had disabled is still a repair: the
    // user just proved they want to sign in to it.
    account.enabled = true;
}

/// What the screen shows for an account the store already has.
///
/// The inverse of [`configure`]: a repair is asking for a password, not for
/// server settings, so the ones the account was signed in with last time are
/// what it offers. `source` names where they came from because the card
/// shows it, and "entered by hand" — what an empty form falls back to —
/// would be a lie the second time round.
pub(crate) fn configured(account: &Account) -> Settings {
    let server = |config: &postio_model::account::ServerConfig| Server {
        host: config.host.clone(),
        port: config.port,
        security: config.security,
    };
    Settings {
        imap: server(&account.incoming),
        smtp: server(&account.outgoing),
        login: account.incoming.username.clone(),
        requires_app_password: false,
        note: None,
        help_url: None,
        // A repair signs in the way the account did: an OAuth account's
        // repair is a fresh browser sign-in, not a password prompt for a
        // password that never existed (#534).
        oauth_sign_in: account.oauth.is_some()
            || matches!(
                account.auth,
                postio_model::account::AuthMethod::OAuth2
                    | postio_model::account::AuthMethod::XOAuth2
            ),
        source: "saved with this account".to_owned(),
    }
}

/// What the screen shows, from what the probe found.
fn shown(settings: &AccountSettings) -> Settings {
    let server = |server: &postio_imap::discovery::ServerSettings| Server {
        host: server.host.clone(),
        port: server.port,
        security: match server.encryption {
            Encryption::Tls => TransportSecurity::Tls,
            Encryption::StartTls => TransportSecurity::StartTls,
            Encryption::None => TransportSecurity::None,
        },
    };
    Settings {
        imap: server(&settings.imap),
        smtp: server(&settings.smtp),
        login: settings.login.clone(),
        requires_app_password: settings.requires_app_password,
        note: settings.note.clone(),
        help_url: settings.password_help_url.clone(),
        // The provider's preferred door (#534): a preset row that leads
        // with oauth2 opens the browser sign-in.
        oauth_sign_in: settings.oauth.is_some(),
        source: settings.source.label().to_owned(),
    }
}

/// The IMAP connection to test.
fn connection_settings(submission: &Submission) -> ConnectionSettings {
    ConnectionSettings::new(
        submission.settings.imap.host.clone(),
        submission.settings.imap.port,
        submission.settings.imap.security,
        submission.settings.login.clone(),
    )
}

/// Turn a backend error into something the user can act on.
///
/// The acceptance criterion is that a failure gives a *specific, actionable*
/// reason, and `BackendError` already distinguishes the cases that need
/// different actions. What it cannot know is the one that matters most here:
/// a provider that refuses ordinary account passwords will simply say the
/// credentials were rejected, and a user who has typed their Apple ID
/// password has no way to tell that from a typo.
///
/// No variant of `BackendError` carries a password, so these are safe to show
/// and safe to log.
fn explain(error: &postio_imap::backend::BackendError) -> String {
    use postio_imap::backend::BackendError as E;
    match error {
        E::Auth { .. } => "The server rejected that address and password.\n\n\
             If this is iCloud, Google or another provider with two-factor \
             authentication, your ordinary account password will not work \
             here — you need an app-specific password."
            .to_owned(),
        E::Tls { host, reason } => format!(
            "The secure connection to {host} could not be established: {reason}.\n\n\
             Postio will not fall back to an unencrypted connection. Check the \
             host name and port."
        ),
        E::TimedOut { after, .. } => format!(
            "The server did not answer within {}s. Check the host name and \
             port, and whether this machine can reach the internet.",
            after.as_secs_f32().round()
        ),
        E::Disconnected { reason, .. } => format!(
            "The connection was lost while signing in: {reason}. That usually \
             means the wrong port, or a server that is not IMAP."
        ),
        E::EmptyCapabilities { host } => format!(
            "{host} answered, but not like an IMAP server. Check the host name \
             and port."
        ),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use postio_imap::discovery::{DiscoveryReport, ServerSettings, SettingsSource};

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
                encryption: postio_imap::discovery::Encryption::Tls,
            },
            smtp: ServerSettings {
                host: "smtp.example.com".to_owned(),
                port: 465,
                encryption: postio_imap::discovery::Encryption::Tls,
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

    use postio_imap::secret::MemorySecretStore;

    /// The account the store holds, if it holds one.
    fn stored(database: &Database) -> Option<Account> {
        let connection = database.connection().expect("a connection");
        let accounts = AccountRepository::new(&connection)
            .list()
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
        let reason = explain(&postio_imap::backend::BackendError::Auth {
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
        let reason = explain(&postio_imap::backend::BackendError::TimedOut {
            context: "login".to_owned(),
            after: Duration::from_secs(30),
        });
        assert!(reason.contains("30s"), "{reason}");
    }

    #[test]
    fn tls_failure_says_postio_will_not_downgrade() {
        let reason = explain(&postio_imap::backend::BackendError::Tls {
            host: "imap.example.com".to_owned(),
            reason: "certificate expired".to_owned(),
        });
        assert!(reason.contains("imap.example.com"), "{reason}");
        assert!(reason.contains("will not fall back"), "{reason}");
    }

    #[test]
    fn a_fresh_store_needs_onboarding_and_one_with_an_account_does_not() {
        let database = postio_storage::test_support::memory();
        assert!(needed(&database), "nothing has been provisioned yet");

        let connection = database.connection().expect("a connection");
        let _account = postio_storage::test_support::account(&connection);
        drop(connection);
        assert!(
            !needed(&database),
            "an account exists, so the screen is done"
        );
    }

    #[tokio::test]
    async fn a_credential_that_cannot_be_stored_leaves_no_account_behind() {
        // `postio-67`: 0.1.0 wrote the row first. When the keyring write then
        // failed, the row stayed — and every launch after that opened an
        // account with no reachable password, in an application whose only
        // credential writer is the screen that never runs again.
        let database = postio_storage::test_support::memory();

        let outcome = persist(
            &database,
            &MemorySecretStore::locked(),
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await;

        assert!(outcome.is_err(), "a locked keyring has to fail the submit");
        assert!(
            stored(&database).is_none(),
            "the account row outlived the credential write that failed"
        );
    }

    #[tokio::test]
    async fn a_first_run_writes_both_the_row_and_the_credential() {
        let database = postio_storage::test_support::memory();
        let secrets = MemorySecretStore::new();

        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("both writes should land");

        let account = stored(&database).expect("an account row");
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

    #[tokio::test]
    async fn an_account_row_that_will_not_write_takes_its_credential_back() {
        // The other order's failure, and the reason the rollback is here: a
        // secret Postio kept for an account that does not exist is a secret
        // nobody asked it to keep.
        let database = postio_storage::test_support::memory();
        let connection = database.connection().expect("a connection");
        connection
            .execute("ALTER TABLE accounts RENAME TO accounts_elsewhere", [])
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

    #[tokio::test]
    async fn signing_in_again_repairs_the_account_rather_than_duplicating_it() {
        // What a repair run does. `startup_route` sends an account with no
        // credential back to this screen, so the second submit arrives over a
        // row that already exists — and a second row would leave
        // `first_account` picking between two.
        let database = postio_storage::test_support::memory();
        let secrets = MemorySecretStore::new();
        persist(
            &database,
            &secrets,
            &submission("old.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the first run should land");
        let first = stored(&database).expect("an account row");

        persist(
            &database,
            &secrets,
            &submission("new.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the repair should land");

        // `stored` fails the test outright on a second row.
        let repaired = stored(&database).expect("an account row");
        assert_eq!(repaired.id, first.id, "the repair replaced the account");
        assert_eq!(
            repaired.incoming.host, "new.example.com",
            "the repair did not take the corrected server"
        );
    }

    #[tokio::test]
    async fn a_repair_keeps_the_identity_the_drafts_point_at() {
        // `AccountRepository::update` makes the identity list authoritative,
        // so a repair that rebuilt the list from scratch would delete the
        // identity every saved draft refers to.
        let database = postio_storage::test_support::memory();
        let secrets = MemorySecretStore::new();
        persist(
            &database,
            &secrets,
            &submission("imap.example.com", TransportSecurity::Tls),
            postio_model::account::Backend::Imap,
        )
        .await
        .expect("the first run should land");
        let before = stored(&database).expect("an account row");
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

        let after = stored(&database).expect("an account row");
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
        // `postio-imap` on purpose (an unverified guess presented as a
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
}
