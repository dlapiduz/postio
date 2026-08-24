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
//! and `scripts/check-crate-boundaries.py` fails. That is a real constraint
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
    AccountSettings, DiscoveryOutcome, Encryption, PimalayaTransport, Probe,
};
use postio_imap::imap::{ConnectionSettings, ImapSession, RustlsConnector};
use postio_imap::secret::{AccountKey, KeyringSecretStore, Password, SecretStore};
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
/// `state`, `wired` and `streams` are the same three pieces `run()`'s
/// `activate` handler already holds once an account is there from the
/// start — passed through so a screen that just created one can finish the
/// exact same sequence: install every command handler and drain the two
/// event queues, not merely feed the panes. Skipping that would leave a
/// window with mail in it and no key that does anything, the same shape of
/// bug `postio-bl2` is named for.
pub fn install(
    window: &Window,
    wiring: &Wiring,
    state: SharedState,
    wired: Vec<CommandId>,
    streams: Rc<RefCell<Vec<Option<EventStream>>>>,
    notifier: crate::notifications::Notifier,
) {
    let screen = Onboarding::new();
    let previous = window.content();
    window.set_content(Some(&screen));
    screen.focus_address();

    screen.connect_probe({
        let screen = screen.clone();
        let runtime = wiring.runtime.clone();
        move |address| probe(&screen, &runtime, address)
    });

    screen.connect_submit({
        let screen = screen.clone();
        let window = window.clone();
        let wiring = wiring.clone();
        let previous = previous.clone();
        let state = state.clone();
        let wired = wired.clone();
        let streams = Rc::clone(&streams);
        let notifier = notifier.clone();
        move |submission| {
            submit(
                &screen,
                &window,
                &wiring,
                previous.as_ref(),
                submission.clone(),
                state.clone(),
                wired.clone(),
                Rc::clone(&streams),
                notifier.clone(),
            )
        }
    });
}

/// Run the autoconfig probe for `address` and show what it found.
fn probe(screen: &Onboarding, runtime: &tokio::runtime::Handle, address: &str) {
    screen.set_status(Status::Probing);

    let (sender, receiver) = async_channel::bounded(1);
    let email = address.to_owned();
    runtime.spawn(async move {
        let probe = Probe::new(Arc::new(PimalayaTransport::new()));
        let answer = probe.run(&email, &CancelToken::new()).await;
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
                    let found = report.settings().map(shown);
                    screen.set_status(match (&report.outcome, found) {
                        (DiscoveryOutcome::Discovered(_), Some(settings)) => {
                            Status::Found(settings)
                        }
                        (_, suggestion) => Status::Manual { suggestion },
                    });
                }
                Err(error) => {
                    tracing::info!(%error, "autoconfig found nothing");
                    screen.set_status(Status::Manual { suggestion: None });
                }
            }
        }
    });
}

/// Test the credentials, then write the account and the password.
#[allow(clippy::too_many_arguments)]
fn submit(
    screen: &Onboarding,
    window: &Window,
    wiring: &Wiring,
    previous: Option<&gtk::Widget>,
    submission: Submission,
    state: SharedState,
    wired: Vec<CommandId>,
    streams: Rc<RefCell<Vec<Option<EventStream>>>>,
    notifier: crate::notifications::Notifier,
) {
    screen.set_status(Status::Connecting);

    let settings = connection_settings(&submission);
    // The password crosses to the runtime and back no further: it is moved
    // into the task, used for one login, and dropped there.
    let password = Password::new(submission.password.clone());
    let (sender, receiver) = async_channel::bounded(1);
    wiring.runtime.spawn(async move {
        let answer = match RustlsConnector::new() {
            Ok(connector) => ImapSession::open(&settings, &password, &connector)
                .await
                .map(|_| ())
                .map_err(|error| explain(&error)),
            Err(error) => Err(format!(
                "Postio could not start a TLS connection on this machine: {error}"
            )),
        };
        let _ = sender.send(answer).await;
    });

    glib::spawn_future_local({
        let screen = screen.clone();
        let window = window.clone();
        let wiring = wiring.clone();
        let previous = previous.cloned();
        async move {
            let answer = match receiver.recv().await {
                Ok(answer) => answer,
                Err(_) => Err("Postio's runtime stopped before the server answered.".to_owned()),
            };
            if let Err(reason) = answer {
                screen.set_status(Status::Failed(reason));
                return;
            }

            // Only now, with the credentials known good. Writing first would
            // leave a broken account behind every failed attempt.
            if let Err(reason) = save(&wiring.database, &submission) {
                screen.set_status(Status::Failed(reason));
                return;
            }

            // The credential goes over D-Bus from the runtime, and the answer
            // comes back over a channel — the same crossing the connection
            // test above makes, and for the same reason.
            let (sender, receiver) = async_channel::bounded(1);
            let address = submission.address.clone();
            let password = Password::new(submission.password.clone());
            wiring.runtime.spawn(async move {
                let _ = sender.send(store_credential(address, password).await).await;
            });
            let stored = receiver.recv().await.unwrap_or_else(|_| {
                Err("Postio's runtime stopped before the keyring answered.".to_owned())
            });
            if let Err(reason) = stored {
                screen.set_status(Status::Failed(reason));
                return;
            }

            screen.set_status(Status::Saved);
            window.set_content(previous.as_ref());
            // The exact sequence `run()`'s `activate` handler runs when an
            // account is there from the start — installing the command
            // handlers and draining the event queues, not only feeding the
            // panes. Without that a window fed here would show mail and
            // answer no key, which is the shape of bug `postio-bl2` is
            // named for.
            crate::open_account(&window, &wiring, &state, &wired, &streams, &notifier);
        }
    });
}

/// The first of the two writes: the account row.
///
/// Synchronous, and deliberately separate from the credential. The keyring
/// is reached over D-Bus by a future that needs a tokio runtime to be polled
/// at all, and this runs on the GTK main context where there is none — see
/// [`store_credential`].
fn save(database: &Database, submission: &Submission) -> Result<(), String> {
    let address = submission.address.clone();
    let email = EmailAddress::new(None::<String>, address.clone());
    let mut account = Account::new(address.clone(), email.clone());
    account.incoming.host = submission.settings.imap.host.clone();
    account.incoming.port = submission.settings.imap.port;
    account.incoming.security = security(submission.settings.imap.tls);
    account.incoming.username = submission.settings.login.clone();
    account.outgoing.host = submission.settings.smtp.host.clone();
    account.outgoing.port = submission.settings.smtp.port;
    account.outgoing.security = security(submission.settings.smtp.tls);
    account.outgoing.username = submission.settings.login.clone();
    account.auth = AuthMethod::Password;
    let mut identity = Identity::new(AccountId::UNASSIGNED, email);
    identity.is_default = true;
    account.identities = vec![identity];

    let connection = database
        .connection()
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    AccountRepository::new(&connection)
        .create(&mut account)
        .map_err(|error| format!("Postio could not write the account: {error}"))?;

    Ok(())
}

/// The second write: the credential, into the keyring.
///
/// **Must be polled on the engine runtime, not the GTK main context.** It
/// reaches the Secret Service over D-Bus and bounds the round trip with
/// `tokio::time::timeout`, so awaiting it from `glib::spawn_future_local`
/// panics with "there is no reactor running" — which is what 0.1.0 did, on
/// the main thread, immediately after a successful login. `feed.rs` explains
/// the rule this broke: neither loop can drive the other, so runtime work is
/// spawned and answered over a channel.
///
/// The failure is reported rather than swallowed: an account with no password
/// in the keyring cannot sync, and a silent failure would look like a Postio
/// bug rather than a locked keyring.
async fn store_credential(address: String, password: Password) -> Result<(), String> {
    KeyringSecretStore::default()
        .store(&AccountKey::new(address), &password)
        .await
        .map_err(|error| {
            format!(
                "The account was added but the password could not be stored: {error}. \
                 Is the keyring unlocked?"
            )
        })
}

/// What the screen shows, from what the probe found.
fn shown(settings: &AccountSettings) -> Settings {
    let server = |server: &postio_imap::discovery::ServerSettings| Server {
        host: server.host.clone(),
        port: server.port,
        tls: server.encryption == Encryption::Tls,
    };
    Settings {
        imap: server(&settings.imap),
        smtp: server(&settings.smtp),
        login: settings.login.clone(),
        requires_app_password: settings.requires_app_password,
        note: settings.note.clone(),
        help_url: settings.password_help_url.clone(),
        source: settings.source.label().to_owned(),
    }
}

/// The IMAP connection to test.
fn connection_settings(submission: &Submission) -> ConnectionSettings {
    ConnectionSettings::new(
        submission.settings.imap.host.clone(),
        submission.settings.imap.port,
        security(submission.settings.imap.tls),
        submission.settings.login.clone(),
    )
}

fn security(tls: bool) -> TransportSecurity {
    if tls {
        TransportSecurity::Tls
    } else {
        TransportSecurity::StartTls
    }
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

    fn submission(host: &str, tls: bool) -> Submission {
        Submission {
            address: "lena@example.com".to_owned(),
            password: "hunter2".to_owned(),
            settings: Settings {
                imap: Server {
                    host: host.to_owned(),
                    port: 993,
                    tls,
                },
                smtp: Server {
                    host: "smtp.example.com".to_owned(),
                    port: 465,
                    tls: true,
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
        let mut wanted = submission("imap.mail.me.com", true);
        wanted.settings.login = "lena@example.net".to_owned();

        let settings = connection_settings(&wanted);
        assert_eq!(settings.username, "lena@example.net");
        assert_eq!(settings.host, "imap.mail.me.com");
        assert_eq!(settings.port, 993);
        assert_eq!(settings.security, TransportSecurity::Tls);
    }

    #[test]
    fn a_server_without_implicit_tls_is_tested_over_starttls() {
        let settings = connection_settings(&submission("mail.example.com", false));
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
}
