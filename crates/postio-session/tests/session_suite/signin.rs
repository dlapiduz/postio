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

#[tokio::test]
async fn endpoints_given_directly_are_used_without_discovering_anything() {
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

#[tokio::test]
async fn a_chosen_name_is_the_accounts_name_and_its_identitys() {
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
    let proof = postio_session::signin::verification_settings(&settings());
    assert_eq!(proof.auth, postio_model::account::AuthMethod::XOAuth2);
    assert_eq!(proof.host, "imap.example.com");
}
