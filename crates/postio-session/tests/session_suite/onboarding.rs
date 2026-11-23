use std::time::Duration;

use postio_account::discovery::{
    AccountSettings, DiscoveryOutcome, DiscoveryReport, ServerSettings, SettingsSource,
};
use postio_account::secret::{AccountKey, MemorySecretStore, SecretStore};
use postio_model::Account;
use postio_model::account::TransportSecurity;
use postio_session::onboarding::{
    connection_settings, explain, persist, probe_options, status_for,
};
use postio_storage::Store;
use postio_storage::repository::AccountRepository;
use postio_ui::onboarding::{Server, Settings, Status, Submission};

fn nothing_published(suggestion: Option<AccountSettings>) -> DiscoveryReport {
    DiscoveryReport {
        email: "lena@example.com".to_owned(),
        domain: "example.com".to_owned(),
        outcome: DiscoveryOutcome::ManualEntry { suggestion },
        attempts: Vec::new(),
    }
}

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

fn resolved(source: SettingsSource, display_name: Option<&str>) -> AccountSettings {
    AccountSettings {
        display_name: display_name.map(str::to_owned),
        source,
        ..guessed()
    }
}

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
async fn a_credential_that_cannot_be_stored_leaves_no_account_behind() {
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

    let repaired = stored(&database).await.expect("an account row");
    assert_eq!(repaired.id, first.id, "the repair replaced the account");
    assert_eq!(
        repaired.incoming.host, "new.example.com",
        "the repair did not take the corrected server"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repair_keeps_the_identity_the_drafts_point_at() {
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
    let settings = resolved(SettingsSource::Builtin, Some("My Own Provider"));
    assert_eq!(found(settings).source, "My Own Provider");
}

#[test]
fn a_preset_row_with_no_display_name_falls_back_to_the_source_label() {
    let settings = resolved(SettingsSource::Builtin, None);
    assert_eq!(found(settings).source, SettingsSource::Builtin.label());
}

#[test]
fn an_autoconfig_documents_own_display_name_does_not_override_the_source_label() {
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
