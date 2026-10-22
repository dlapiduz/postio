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
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();

    let outcome = provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &["https://provider.example/mail".to_owned()],
        Some(90),
        "",
    )
    .await
    .expect("the sign-in persists");

    assert!(matches!(outcome, Provisioned::Created(_)));

    let connection = database.connect().await.expect("a connection");
    let stored = AccountRepository::new(&connection)
        .list()
        .await
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
    let database = test_support::temp().await;
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
            "",
        )
        .await
        .expect("both attempts answer");
    }

    let connection = database.connect().await.expect("a connection");
    assert_eq!(
        AccountRepository::new(&connection)
            .list()
            .await
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
    let database = test_support::temp().await;
    let locked = MemorySecretStore::locked();

    let error = provision_oauth(
        &database,
        Arc::new(locked.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &[],
        None,
        "",
    )
    .await
    .expect_err("a locked keyring refuses");

    assert!(error.contains("keyring"), "{error}");
    let connection = database.connect().await.expect("a connection");
    assert!(
        AccountRepository::new(&connection)
            .list()
            .await
            .expect("a list")
            .is_empty(),
        "nothing was written"
    );
}

#[tokio::test]
async fn signing_in_again_replaces_the_token_rather_than_throwing_it_away() {
    // The repair path, and the one that lied. An OAuth grant expires or is
    // revoked; the settings pane offers Reconnect; the browser opens, the
    // person approves, and `provision_oauth` returned
    // `AlreadyProvisioned` **before** seeding the keyring -- so the fresh
    // refresh token was discarded, the boundary mapped `Ok(_)` to "no
    // complaint", and the sheet closed reporting success over an account
    // that still could not sign in.
    //
    // Adding the same account twice staying inert is right *about the row*.
    // It was never right about the credential: signing in again is what
    // somebody does precisely because the credential is wrong.
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();
    let scopes = ["https://provider.example/mail".to_owned()];
    let key = AccountKey::new(format!("{ADDRESS}#oauth-refresh"));

    let first = provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &scopes,
        None,
        "",
    )
    .await
    .expect("the first sign-in");
    assert!(matches!(first, Provisioned::Created(_)));

    // The grant is revoked and re-granted: same account, a new token.
    let mut again = signed_in();
    again.tokens.refresh_token = Some(Password::new("a-second-refresh-token"));

    let repaired = provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &settings(),
        &client(),
        again,
        &scopes,
        None,
        "",
    )
    .await
    .expect("the repair");
    assert!(
        matches!(repaired, Provisioned::AlreadyProvisioned(_)),
        "one account, not two: {repaired:?}"
    );

    let held = keyring
        .retrieve(&key)
        .await
        .expect("a token is in the keyring");
    assert_eq!(
        held.expose(),
        "a-second-refresh-token",
        "the person approved a new grant and Postio kept the dead one"
    );
}

// -- what signing in over an existing row keeps and changes -----------------

#[tokio::test]
async fn a_chosen_name_is_the_accounts_name_and_its_identitys() {
    // The person typed a name on the sheet; it is what the sidebar and the
    // From header show, ahead of the provider's own name.
    let database = test_support::temp().await;
    provision_oauth(
        &database,
        Arc::new(MemorySecretStore::new()),
        &settings(),
        &client(),
        signed_in(),
        &[],
        None,
        "  Mara Example ",
    )
    .await
    .expect("persists");

    let connection = database.connect().await.expect("a connection");
    let stored = AccountRepository::new(&connection)
        .list()
        .await
        .expect("a list")
        .into_iter()
        .next()
        .expect("one account");
    assert_eq!(stored.display_name, "Mara Example");
    assert_eq!(
        stored.identities[0].address.name.as_deref(),
        Some("Mara Example")
    );
}

#[tokio::test]
async fn signing_in_over_an_existing_row_repairs_it_and_keeps_its_identities() {
    // The repair screen is reachable for an account that exists: the servers
    // may have changed, the account may have been disabled, and it may have
    // been a password account. Signing in again must leave the row naming
    // the grant the person just approved, not the one they replaced.
    let database = test_support::temp().await;
    let keyring = MemorySecretStore::new();
    provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &settings(),
        &client(),
        signed_in(),
        &[],
        None,
        "",
    )
    .await
    .expect("first");
    {
        let connection = database.connect().await.expect("a connection");
        let repository = AccountRepository::new(&connection);
        let mut row = repository.list().await.unwrap().remove(0);
        row.enabled = false;
        row.auth = postio_model::account::AuthMethod::Password;
        row.oauth = None;
        repository.update(&mut row).await.unwrap();
    }

    let mut moved = settings();
    moved.imap = ServerSettings::new("imap2.example.com", 993, Encryption::Tls);
    let newer = OAuthClient {
        client_id: "a-newer-client".to_owned(),
        client_secret: None,
    };
    let outcome = provision_oauth(
        &database,
        Arc::new(keyring.reopen()),
        &moved,
        &newer,
        signed_in(),
        &["scope-b".to_owned()],
        Some(30),
        "",
    )
    .await
    .expect("repair");
    assert!(matches!(outcome, Provisioned::AlreadyProvisioned(_)));

    let connection = database.connect().await.expect("a connection");
    let rows = AccountRepository::new(&connection).list().await.unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert!(row.enabled, "a repair re-enables the account");
    assert_eq!(row.incoming.host, "imap2.example.com");
    assert_eq!(row.auth, postio_model::account::AuthMethod::XOAuth2);
    let oauth = row.oauth.as_ref().expect("the new grant is recorded");
    assert_eq!(oauth.client_id, "a-newer-client");
    assert_eq!(oauth.scopes, "scope-b");
    assert_eq!(oauth.refresh_token_lifetime_days, Some(30));
    assert_eq!(row.identities.len(), 1);
}

#[test]
fn the_proof_presents_the_token_as_xoauth2_not_as_a_password() {
    // A bearer token sent as a PLAIN password is refused by every provider
    // this path exists for.
    let proof = postio_session::signin::verification_settings(&settings());
    assert_eq!(proof.auth, postio_model::account::AuthMethod::XOAuth2);
    assert_eq!(proof.host, "imap.example.com");
}
