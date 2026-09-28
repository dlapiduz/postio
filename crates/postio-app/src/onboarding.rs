//! First run: the account form as the window's whole content.
//!
//! `postio_widgets::onboarding` draws canvas 3e and knows nothing about mail;
//! `postio_widgets::present::onboarding` joins it to the store's host, which
//! probes, proves and writes (ADR 0041, specs/007-postio-focus T165). What is
//! left here is the classic app's own: the form replacing an empty window
//! rather than floating over one, the sync-window step after the account is
//! saved, and bringing the window up over the new account once it is.
//!
//! # Nothing here blocks the UI
//!
//! The probe and the connection test are network work, done on the host's
//! runtime and answered over the client; the screen stays live throughout
//! and says which of the two it is waiting on.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use adw::prelude::*;
use postio_account::discovery::DiscoveryTransport;
use postio_core::CommandId;
use postio_core::bridge::EventStream;
use postio_core::state::SharedState;
use postio_gtk::onboarding::{Onboarding, Status};
use postio_gtk::window::Window;
use postio_model::Account;
#[cfg(test)]
use postio_storage::Store;
#[cfg(test)]
use postio_storage::repository::AccountRepository;
use postio_widgets::present::onboarding::Presenter;

use crate::Wiring;
pub(crate) use postio_session::onboarding::configured;
#[cfg(test)]
use postio_session::onboarding::persist;
use postio_session::onboarding::write_sync_window;
#[cfg(test)]
use postio_session::onboarding::{connection_settings, probe_options, status_for};

/// Whether this installation has an account yet.
///
/// A store that cannot be read counts as "no account": the screen is the only
/// way forward from there anyway, and refusing to show it would leave a
/// window with nothing in it and no way to fix that.
///
/// Only its test reads it: which screen a launch opens on is
/// `startup_route`'s question, answered at the composition root.
#[cfg(test)]
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

/// Put the first-run screen in the window and wire it to the store's host.
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
/// `transport` is where the host looks a new address's servers up, and
/// `opener` how a browser sign-in's consent link is opened: supplied rather
/// than constructed, so a test drives the same screen over a mock and a
/// fake browser (#282).
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
    // The probe and the writes are the store owner's (ADR 0041). This
    // screen is reached before any window is fed, so it connects its own
    // client, to a host over the same wiring that looks servers up through
    // `transport`.
    let frontend = crate::frontend::Frontend::in_process(&wiring.clone().with_discovery(transport));
    // Once the account is written, the same sequence `run()`'s `activate`
    // handler runs when an account is there from the start.
    let open = {
        let window = window.clone();
        let wiring = wiring.clone();
        move || {
            postio_session::blocking::now(crate::open_account(
                &window, &wiring, &state, &wired, &events, &notifier,
            ))
        }
    };
    install_for(window, &frontend, repairing, opener, open);
}

/// [`install`], over `frontend`'s client: `open` is what brings the window
/// up over the account once it is written.
pub fn install_for(
    window: &Window,
    frontend: &crate::frontend::Frontend,
    repairing: Option<Account>,
    opener: Arc<dyn postio_account::oauth::BrowserOpener>,
    open: impl Fn() + Clone + 'static,
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

    let presenter = Presenter::drive(&screen, &frontend.client, open_with(opener));

    // What finishes the screen for good: swap the window content back and
    // start the application over the new account, the exact sequence
    // `run()`'s `activate` handler runs when an account is there from the
    // start — installing the command handlers and draining the event
    // queues, not only feeding the panes. Without that a window fed here
    // would show mail and answer no key, which is the shape of bug
    // `postio-bl2` is named for.
    //
    // Held back from the save itself by the sync-window step (#876): the
    // account and its credential are already written by the time that step
    // shows, so `Status::Saved` is real, but the window this closure swaps
    // to should not appear until the user has chosen how far back to sync.
    // That is also why the host was asked to wait rather than start the
    // account's sync on save.
    let finish = {
        let window = window.clone();
        move || {
            window.set_content(previous.as_ref());
            open();
        }
    };
    screen.connect_start_sync(move |window| {
        if let Err(error) = write_sync_window(window) {
            tracing::warn!(%error, "could not save the chosen sync window");
        }
        finish();
    });
    presenter.connect_saved({
        let screen = screen.downgrade();
        move |_| {
            if let Some(screen) = screen.upgrade() {
                screen.set_status(Status::SyncWindow);
            }
        }
    });
}

/// How the classic app opens a browser sign-in's consent link: through
/// `opener`, the one the composition root was handed.
pub(crate) fn open_with(opener: Arc<dyn postio_account::oauth::BrowserOpener>) -> impl Fn(&str) {
    move |url| {
        // POSTIO-CONSENT: the consent link of a browser sign-in the person
        // began by pressing "Sign in with your browser", opened once, when
        // the host has bound the loopback listener waiting for its redirect.
        // Never on render and never retried on its own (ADR 0006 Q3).
        let opened = url
            .parse::<postio_account::oauth::Url>()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
            .and_then(|url| opener.open(&url));
        if let Err(error) = opened {
            tracing::warn!(%error, "could not open the sign-in link");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_account::discovery::{AccountSettings, DiscoveryOutcome};
    use postio_account::secret::{AccountKey, SecretStore};
    use postio_gtk::onboarding::Server;
    use postio_gtk::onboarding::Settings;
    use postio_gtk::onboarding::Submission;
    use postio_model::account::TransportSecurity;
    use postio_session::onboarding::explain;
    use std::time::Duration;

    use postio_account::discovery::{DiscoveryReport, ServerSettings, SettingsSource};

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
