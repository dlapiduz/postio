//! Signing in with a browser, without a toolkit (#1276).
//!
//! The flow itself — PKCE, the loopback listener, the code exchange — is
//! `postio-account`'s and is tested there against a mock authorization
//! server. What is tested here is the part that used to live in the GTK
//! composition root and could not be reached from a second frontend: which
//! endpoints a sign-in uses, and the order the account and its refresh token
//! are written in.
//!
//! **The order is the whole of the risk**, and it is `provision`'s rule
//! applied to a token: the credential first, then the row, rolling the
//! credential back if the row will not write. `postio-67` is what the other
//! order cost.
//!
//! Nothing here reaches the network or a browser.

use std::sync::Arc;

use postio_account::cancel::CancelToken;
use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::oauth::{Endpoints, TokenResponse};
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_session::provision::Provisioned;
use postio_session::signin::{OAuthClient, SignInError, SignedIn, endpoints_for, provision_oauth};
use postio_storage::repository::AccountRepository;
use postio_storage::test_support;

const ADDRESS: &str = "mara@example.com";

fn settings() -> AccountSettings {
    AccountSettings {
        email: ADDRESS.to_owned(),
        imap: ServerSettings::new("imap.example.com", 993, Encryption::Tls),
        smtp: ServerSettings::new("smtp.example.com", 465, Encryption::StartTls),
        login: ADDRESS.to_owned(),
        source: SettingsSource::Builtin,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: Some("Example Mail".to_owned()),
        oauth: None,
        jmap: None,
        backends: Vec::new(),
    }
}

fn client() -> OAuthClient {
    // The user's own, always: Postio ships no client id and no secret, and a
    // registered application's credentials in an open-source mail client are
    // credentials every user of it shares (ADR 0006 Q1).
    OAuthClient {
        client_id: "the-users-own-client".to_owned(),
        client_secret: None,
    }
}

fn endpoints() -> Endpoints {
    Endpoints {
        authorize: "https://provider.example/authorize".parse().unwrap(),
        token: "https://provider.example/token".parse().unwrap(),
    }
}

fn signed_in() -> SignedIn {
    SignedIn {
        endpoints: endpoints(),
        tokens: TokenResponse {
            access_token: Password::new("an-access-token"),
            refresh_token: Some(Password::new("a-refresh-token")),
            expires_in: Some(std::time::Duration::from_secs(3600)),
            token_type: "Bearer".to_owned(),
            scope: None,
        },
    }
}

// -- which endpoints a sign-in uses -----------------------------------------

#[tokio::test]
async fn endpoints_given_directly_are_used_without_discovering_anything() {
    // A preset row that names both is the ordinary case, and it must not
    // cost a network round trip to a discovery document.
    let resolved = endpoints_for(
        Some("https://provider.example/authorize"),
        Some("https://provider.example/token"),
        Some("https://provider.example"),
        &CancelToken::new(),
    )
    .await
    .expect("the endpoints were given");

    assert_eq!(
        resolved.authorize.as_str(),
        "https://provider.example/authorize"
    );
    assert_eq!(resolved.token.as_str(), "https://provider.example/token");
}

#[tokio::test]
async fn a_provider_with_no_endpoints_and_no_issuer_says_so_rather_than_guessing() {
    // Guessing a consent URL is sending somebody's credentials somewhere
    // nobody chose.
    let error = endpoints_for(None, None, None, &CancelToken::new())
        .await
        .expect_err("nothing to go on");

    assert!(matches!(error, SignInError::Failed(message) if message.contains("consent")));
}

#[tokio::test]
async fn an_unparseable_endpoint_is_refused_before_a_browser_opens() {
    let error = endpoints_for(
        Some("not a url"),
        Some("also not"),
        None,
        &CancelToken::new(),
    )
    .await
    .expect_err("neither is a URL");

    assert!(matches!(error, SignInError::Failed(_)));
}

// -- what a completed sign-in writes ----------------------------------------

#[tokio::test]
async fn the_account_is_written_with_its_client_and_the_token_goes_to_the_keyring() {
    let database = test_support::temp();
    let keyring = MemorySecretStore::new();

    let outcome = provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &["https://provider.example/mail".to_owned()],
        Some(90),
    )
    .await
    .expect("the sign-in persists");

    assert!(matches!(outcome, Provisioned::Created(_)));

    let connection = database.connection().expect("a connection");
    let stored = AccountRepository::new(&connection)
        .list()
        .expect("a list")
        .into_iter()
        .find(|account| account.address.address == ADDRESS)
        .expect("the account is in the store");

    assert_eq!(stored.auth, postio_model::account::AuthMethod::XOAuth2);
    let oauth = stored
        .oauth
        .clone()
        .expect("the row records how to sign in again");
    assert_eq!(oauth.client_id, "the-users-own-client");
    assert_eq!(oauth.token_url, "https://provider.example/token");
    assert_eq!(oauth.refresh_token_lifetime_days, Some(90));

    // The token is in the keyring and **not** in the row: a refresh token in
    // `config.toml` or in the store is the thing ADR 0006 exists to prevent.
    let held = keyring
        .retrieve(&AccountKey::new(format!("{ADDRESS}#oauth-refresh")))
        .await
        .expect("the refresh token is in the keyring");
    assert_eq!(held.expose(), "a-refresh-token");
    assert!(
        !format!("{stored:?}").contains("a-refresh-token"),
        "a token must not be in the account row"
    );
}

#[tokio::test]
async fn signing_in_twice_leaves_one_account() {
    // Somebody who stumbles and starts again has one account, not two — the
    // same rule the password path keeps.
    let database = test_support::temp();
    let keyring = MemorySecretStore::new();
    let scopes = ["https://provider.example/mail".to_owned()];

    for _ in 0..2 {
        provision_oauth(
            &database,
            Arc::new(keyring.reopen()),
            &settings(),
            &client(),
            signed_in(),
            &scopes,
            None,
        )
        .await
        .expect("both attempts answer");
    }

    let connection = database.connection().expect("a connection");
    assert_eq!(
        AccountRepository::new(&connection)
            .list()
            .expect("a list")
            .into_iter()
            .filter(|account| account.address.address == ADDRESS)
            .count(),
        1
    );
}

#[tokio::test]
async fn a_keyring_that_will_not_take_the_token_writes_no_account_row() {
    // The order that cannot strand an account: the credential first. An
    // account row with no reachable token could not sync, could not
    // authenticate, and could not be repaired from inside the application.
    let database = test_support::temp();
    let locked = MemorySecretStore::locked();

    let error = provision_oauth(
        &database,
        Arc::new(locked.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &[],
        None,
    )
    .await
    .expect_err("a locked keyring refuses");

    assert!(error.contains("keyring"), "{error}");
    let connection = database.connection().expect("a connection");
    assert!(
        AccountRepository::new(&connection)
            .list()
            .expect("a list")
            .is_empty(),
        "nothing was written"
    );
}
