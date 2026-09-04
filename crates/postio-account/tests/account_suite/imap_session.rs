//! Opening an IMAP session: TLS, authentication, and the capability re-read.
//!
//! `io-imap` is sans-I/O, so the whole handshake runs here against a recorded
//! transcript with no socket involved. The live-server tests at the bottom are
//! `#[ignore]`d and need `POSTIO_TEST_*` in the environment.

use postio_account::backend::{BackendError, Capability};
use postio_account::imap::{
    ConnectionSettings, IMAPS_PORT, ImapScript, ImapSession, RustlsConnector, ScriptedConnector,
};
use postio_account::secret::Password;
use postio_model::TransportSecurity;

fn password() -> Password {
    Password::new("app-specific-password")
}

fn settings() -> ConnectionSettings {
    ConnectionSettings::new(
        "imap.example.com",
        IMAPS_PORT,
        TransportSecurity::Tls,
        "someone@example.com",
    )
}

// ---------------------------------------------------------------------------
// The post-auth capability re-read
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_capability_set_comes_from_after_authentication() {
    // This is the whole reason this bead exists. iCloud's banner advertises
    // IMAP4rev1 and the auth mechanisms and nothing else; CONDSTORE, QRESYNC,
    // IDLE and UIDPLUS appear only once logged in. Trust the banner and the
    // client silently loses incremental sync forever.
    let connector = ScriptedConnector::extensions_hidden_until_login();

    let session = ImapSession::open(&settings(), &password(), &connector)
        .await
        .expect("the scripted iCloud handshake");

    let capabilities = session.capabilities();
    assert!(capabilities.contains(Capability::CondStore));
    assert!(capabilities.contains(Capability::QResync));
    assert!(capabilities.contains(Capability::Idle));
    assert!(capabilities.contains(Capability::UidPlus));
    assert!(capabilities.contains(Capability::Move));
    assert!(capabilities.supports_incremental_sync());

    // …and the re-read really happened: a CAPABILITY command went out after
    // the AUTHENTICATE.
    let commands = connector.log().commands();
    let authenticated = commands
        .iter()
        .position(|command| command.contains("AUTHENTICATE"))
        .expect("no AUTHENTICATE was sent");
    assert!(
        commands
            .iter()
            .skip(authenticated + 1)
            .any(|command| command.contains("CAPABILITY")),
        "no CAPABILITY was issued after authentication: {commands:?}"
    );
}

#[tokio::test]
async fn the_banner_alone_would_not_have_been_enough() {
    // Guards the test above against becoming vacuous: if the transcript ever
    // starts advertising the extensions up front, the assertion proves nothing.
    let banner_only =
        ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] iCloud ready")
            .on("AUTHENTICATE", "{tag} OK [CAPABILITY IMAP4rev1] done");
    let connector = ScriptedConnector::new(banner_only);

    let session = ImapSession::open(&settings(), &password(), &connector)
        .await
        .unwrap();

    assert!(!session.capabilities().contains(Capability::QResync));
    assert!(!session.capabilities().supports_incremental_sync());
}

#[tokio::test]
async fn the_session_reports_the_endpoint_and_the_account_it_authenticated_as() {
    let session = ImapSession::open(
        &settings(),
        &password(),
        &ScriptedConnector::extensions_hidden_until_login(),
    )
    .await
    .unwrap();

    assert_eq!(session.endpoint(), "imap.example.com:993");
    assert_eq!(session.account(), "someone@example.com");
    assert!(session.is_encrypted());
}

// ---------------------------------------------------------------------------
// TLS
// ---------------------------------------------------------------------------

#[tokio::test]
async fn implicit_tls_is_the_only_connection_attempted() {
    let connector = ScriptedConnector::extensions_hidden_until_login();

    ImapSession::open(&settings(), &password(), &connector)
        .await
        .unwrap();

    let log = connector.log();
    assert_eq!(log.tls, [("imap.example.com".to_owned(), 993)]);
    assert!(log.tcp.is_empty(), "a plaintext socket was opened as well");
}

#[tokio::test]
async fn a_tls_failure_is_surfaced_and_never_retried_in_the_clear() {
    let connector = ScriptedConnector::extensions_hidden_until_login()
        .failing_tls("certificate is not trusted");

    let error = ImapSession::open(&settings(), &password(), &connector)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::Tls { .. }));
    assert!(error.to_string().contains("certificate is not trusted"));
    assert!(!error.is_transient());

    let log = connector.log();
    assert_eq!(log.tls.len(), 1);
    assert!(
        log.tcp.is_empty(),
        "the client fell back to plaintext after a TLS failure"
    );
    assert!(
        log.written.is_empty(),
        "credentials were written to a connection that failed to secure"
    );
}

#[tokio::test]
async fn cleartext_to_a_remote_host_never_opens_a_socket_at_all() {
    let settings = ConnectionSettings::new(
        "imap.example.com",
        143,
        TransportSecurity::None,
        "someone@example.com",
    );
    let connector = ScriptedConnector::extensions_hidden_until_login();

    let error = ImapSession::open(&settings, &password(), &connector)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::Tls { .. }));
    let log = connector.log();
    assert!(log.tcp.is_empty() && log.tls.is_empty());
}

#[tokio::test]
async fn starttls_upgrades_the_connection_before_authenticating() {
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 STARTTLS] example ready")
        .on("STARTTLS", "{tag} OK begin TLS negotiation now")
        .on("AUTHENTICATE", "{tag} OK AUTHENTICATE completed")
        .on(
            "CAPABILITY",
            "* CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN ENABLE CONDSTORE QRESYNC IDLE UIDPLUS\n\
             {tag} OK CAPABILITY completed",
        );
    let connector = ScriptedConnector::new(script);
    let settings = ConnectionSettings::new(
        "imap.example.com",
        143,
        TransportSecurity::StartTls,
        "someone@example.com",
    );

    let session = ImapSession::open(&settings, &password(), &connector)
        .await
        .expect("the scripted STARTTLS handshake");

    let log = connector.log();
    assert_eq!(log.tcp, [("imap.example.com".to_owned(), 143)]);
    assert_eq!(log.upgrades, ["imap.example.com"]);
    assert!(session.is_encrypted());
    assert!(session.capabilities().contains(Capability::QResync));

    // The password went out after the upgrade, not before it.
    let commands = log.commands();
    let upgraded = commands
        .iter()
        .position(|command| command.contains("STARTTLS"))
        .expect("no STARTTLS was sent");
    let authenticated = commands
        .iter()
        .position(|command| command.contains("AUTHENTICATE"))
        .expect("no AUTHENTICATE was sent");
    assert!(upgraded < authenticated);
}

#[tokio::test]
async fn a_failed_starttls_upgrade_does_not_authenticate_anyway() {
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 STARTTLS] example ready")
        .on("STARTTLS", "{tag} OK begin TLS negotiation now");
    let connector = ScriptedConnector::new(script).failing_tls("handshake failed");
    let settings = ConnectionSettings::new(
        "imap.example.com",
        143,
        TransportSecurity::StartTls,
        "someone@example.com",
    );

    let error = ImapSession::open(&settings, &password(), &connector)
        .await
        .unwrap_err();

    assert!(matches!(error, BackendError::Tls { .. }));
    assert!(
        !connector
            .log()
            .commands()
            .iter()
            .any(|command| command.contains("AUTHENTICATE")),
        "credentials were sent over a connection that failed to upgrade"
    );
}

// ---------------------------------------------------------------------------
// Authentication
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_rejected_password_is_an_authentication_failure_not_a_retry() {
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] iCloud ready").on(
        "AUTHENTICATE",
        "{tag} NO [AUTHENTICATIONFAILED] Authentication failed",
    );

    let error = ImapSession::open(&settings(), &password(), &ScriptedConnector::new(script))
        .await
        .unwrap_err();

    assert!(error.is_authentication_failure());
    assert!(
        !error.is_transient(),
        "a rejected app-specific password must not be retried"
    );
    assert!(error.to_string().contains("someone@example.com"));
}

#[tokio::test]
async fn a_connection_that_dies_mid_handshake_is_transient() {
    // The transcript stops after the greeting, so the next read sees EOF.
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] iCloud ready");
    let connector = ScriptedConnector::new(script.on("AUTHENTICATE", ""));

    let error = ImapSession::open(&settings(), &password(), &connector)
        .await
        .unwrap_err();

    assert!(!error.is_authentication_failure());
}

// ---------------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_password_never_reaches_a_log_line_or_a_debug_rendering() {
    let secret = "hunter2-app-specific";
    let connector = ScriptedConnector::extensions_hidden_until_login();

    let session = ImapSession::open(&settings(), &Password::new(secret), &connector)
        .await
        .unwrap();

    let rendered = format!("{session:?}");
    assert!(!rendered.contains(secret));
    assert!(rendered.contains("imap.example.com:993"));

    // It is on the wire, base64-encoded, because that is what SASL PLAIN is —
    // but never in the clear, and never in anything we would print.
    assert!(!String::from_utf8_lossy(&connector.log().written).contains(secret));
}

#[tokio::test]
async fn an_authentication_error_does_not_quote_the_password_back() {
    let secret = "hunter2-app-specific";
    let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
        .on("AUTHENTICATE", "{tag} NO Authentication failed");

    let error = ImapSession::open(
        &settings(),
        &Password::new(secret),
        &ScriptedConnector::new(script),
    )
    .await
    .unwrap_err();

    assert!(!error.to_string().contains(secret));
}

// ---------------------------------------------------------------------------
// Live server. Ignored by default; needs a real account.
// ---------------------------------------------------------------------------

/// Reads the live-test credentials, or skips.
///
/// `POSTIO_TEST_IMAP_USER` and `POSTIO_TEST_IMAP_PASSWORD` — for a provider
/// that requires one, an app-specific password. The host comes from Postio's
/// preset table when it ships one for the address's domain, and
/// `POSTIO_TEST_IMAP_HOST` overrides it for anything else.
fn live_settings() -> Option<(ConnectionSettings, Password)> {
    let user = std::env::var("POSTIO_TEST_IMAP_USER").ok()?;
    let password = std::env::var("POSTIO_TEST_IMAP_PASSWORD").ok()?;

    let mut settings = ConnectionSettings::preset_for(&user)
        .unwrap_or_else(|| ConnectionSettings::new("", IMAPS_PORT, TransportSecurity::Tls, &user));
    if let Ok(host) = std::env::var("POSTIO_TEST_IMAP_HOST") {
        settings.host = host;
    }
    Some((settings, Password::new(password)))
}

#[tokio::test]
#[ignore = "talks to a live IMAP server; set POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD"]
async fn live_server_advertises_the_extensions_the_sync_design_needs() {
    let Some((settings, password)) = live_settings() else {
        panic!("POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD must be set");
    };

    let connector = RustlsConnector::new().expect("TLS configuration");
    let session = ImapSession::open(&settings, &password, &connector)
        .await
        .expect("live connect");

    // The last unverified assumption in ADR 0001. Print it, then assert it.
    println!(
        "{} post-auth CAPABILITY: {}",
        session.endpoint(),
        session.capabilities().names().join(" ")
    );

    for capability in [
        Capability::CondStore,
        Capability::QResync,
        Capability::Idle,
        Capability::UidPlus,
    ] {
        assert!(
            session.capabilities().contains(capability),
            "{} does not advertise {capability} after authentication",
            session.endpoint()
        );
    }

    assert!(session.is_encrypted());
    session.logout().await.expect("logout");
}

#[tokio::test]
#[ignore = "talks to a live IMAP server; set POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD"]
async fn live_server_rejects_a_wrong_password_without_asking_us_to_retry() {
    let Some((settings, _)) = live_settings() else {
        panic!("POSTIO_TEST_IMAP_USER and POSTIO_TEST_IMAP_PASSWORD must be set");
    };

    let connector = RustlsConnector::new().expect("TLS configuration");
    let error = ImapSession::open(&settings, &Password::new("not-the-password"), &connector)
        .await
        .expect_err("a wrong password must not open a session");

    assert!(error.is_authentication_failure());
    assert!(!error.is_transient());
}
