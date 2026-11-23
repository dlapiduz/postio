use std::sync::Arc;
use std::time::Duration;

use postio_account::oauth::Url;
use postio_account::oauth::exchange::TokenResponse;
use postio_account::oauth::token_source::OwnClientTokenSource;
use postio_account::secret::{AccountKey, MemorySecretStore, Password, SecretStore};
use postio_ffi::{RepairRouteFfi, Session, SessionOptions};
use postio_model::EmailAddress;
use postio_model::account::{Account, AuthMethod, Backend, OAuthConfig};
use postio_model::ids::AccountId;
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

const ADDRESS: &str = "ada@example.com";

fn account(auth: AuthMethod) -> Account {
    let mut account = Account::new(
        "Ada Lovelace",
        EmailAddress::new(Some("Ada Lovelace"), ADDRESS),
    );
    account.backend = Backend::Imap;
    account.auth = auth;
    account.incoming.host = "imap.example.com".to_owned();
    account.incoming.port = 993;
    account.outgoing.host = "smtp.example.com".to_owned();
    account.outgoing.port = 465;
    if matches!(auth, AuthMethod::OAuth2 | AuthMethod::XOAuth2) {
        account.oauth = Some(OAuthConfig {
            client_id: "a-client-the-user-registered".to_owned(),
            token_url: "https://oauth.example.com/token".to_owned(),
            authorize_url: "https://oauth.example.com/authorize".to_owned(),
            scopes: "https://oauth.example.com/mail".to_owned(),
            refresh_token_lifetime_days: None,
        });
    }
    account
}

async fn store_with(account: Account) -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let mut account = account;
    let connection = database.connect().await.expect("a checkout");
    let id = AccountRepository::new(&connection)
        .create(&mut account)
        .await
        .expect("the account row is written");
    drop(connection);
    (database, id)
}

async fn seed_token(keyring: Arc<MemorySecretStore>, expires_in: Duration) {
    let source = OwnClientTokenSource::new(
        keyring,
        Url::parse("https://oauth.example.com/token").expect("a token endpoint"),
        "a-client-the-user-registered",
        None,
        None,
    );
    source
        .seed(
            &AccountKey::new(ADDRESS),
            TokenResponse {
                access_token: Password::new("an access token"),
                refresh_token: Some(Password::new("a refresh token")),
                expires_in: Some(expires_in),
                token_type: "Bearer".to_owned(),
                scope: None,
            },
        )
        .await
        .expect("the keyring takes the token");
}

fn session(database: Store, keyring: Arc<MemorySecretStore>) -> Arc<Session> {
    Session::open(SessionOptions::in_memory_with(database).with_secrets(keyring))
        .expect("a session over the seeded store")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_token_reaches_the_row_as_a_fact_and_a_flag() {
    // The expiry was on file but never reached the row, so a lapsed grant looked healthy (#1584).
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, _) = store_with(account(AuthMethod::XOAuth2)).await;
    seed_token(keyring.clone(), Duration::ZERO).await;

    let session = session(database, keyring);
    let rows = session.accounts().await;
    let row = rows.first().expect("the pane lists the account");

    assert!(
        row.facts.iter().any(|fact| fact.contains("expired")),
        "the fact line says nothing about a token that has expired: {:?}",
        row.facts
    );
    assert!(
        row.needs_attention,
        "the row draws no warning, so the one state on it that is a problem \
         rather than a fact looks like a fact"
    );
    assert_eq!(
        row.repair,
        RepairRouteFfi::Browser,
        "an expired grant is re-consented in a browser; a password field \
         here would ask for something no provider would accept"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_with_time_left_says_so_without_raising_an_alarm() {
    // A row that flagged every OAuth account would be as useless as one that flagged none.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, _) = store_with(account(AuthMethod::XOAuth2)).await;
    seed_token(keyring.clone(), Duration::from_secs(41 * 24 * 60 * 60)).await;

    let session = session(database, keyring);
    let rows = session.accounts().await;
    let row = rows.first().expect("the pane lists the account");

    assert!(
        row.facts.iter().any(|fact| fact.starts_with("token valid")),
        "a healthy token says how long it is good for: {:?}",
        row.facts
    );
    assert!(
        !row.needs_attention,
        "a token good for weeks was drawn as a problem: {:?}",
        row.facts
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_account_offers_the_typed_route_and_no_token_line() {
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, _) = store_with(account(AuthMethod::AppPassword)).await;

    let session = session(database, keyring);
    let rows = session.accounts().await;
    let row = rows.first().expect("the pane lists the account");

    assert!(
        !row.facts.iter().any(|fact| fact.contains("token")),
        "a password account was told about a token it does not have: {:?}",
        row.facts
    );
    assert!(!row.needs_attention);
    assert_eq!(row.repair, RepairRouteFfi::Password);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rotated_app_password_replaces_the_one_in_the_keyring() {
    // `provision` refuses to overwrite a working credential, which left a rotated app password no
    // route; the repair is that route.
    let keyring = Arc::new(MemorySecretStore::new());
    keyring
        .store(&AccountKey::new(ADDRESS), &Password::new("the old one"))
        .await
        .expect("the keyring takes the first password");
    let (database, id) = store_with(account(AuthMethod::AppPassword)).await;

    let session = session(database, keyring.clone());
    assert_eq!(
        session
            .repair_credential(id.get(), "the rotated one".to_owned())
            .await,
        None,
        "the repair reported a problem"
    );

    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "the rotated one",
        "the account still signs in with the password its provider retired"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_with_nothing_in_the_keyring_can_be_given_a_credential() {
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::Password)).await;

    let session = session(database, keyring.clone());
    assert_eq!(
        session
            .repair_credential(id.get(), "at last".to_owned())
            .await,
        None,
        "the repair reported a problem"
    );

    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "at last"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_browser_account_refuses_a_typed_password_rather_than_storing_one() {
    // An OAuth account's plain keyring entry is read by nothing; storing a password would report
    // success and change nothing.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::XOAuth2)).await;

    let session = session(database, keyring.clone());
    let complaint = session
        .repair_credential(id.get(), "a password no provider would take".to_owned())
        .await
        .expect("a browser account refuses a typed password");
    assert!(
        complaint.to_lowercase().contains("browser"),
        "the refusal has to name the route that does work, got: {complaint}"
    );
    assert!(
        keyring.retrieve(&AccountKey::new(ADDRESS)).await.is_err(),
        "a password was written for an account that signs in with a token"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repair_over_an_account_that_is_gone_says_so() {
    // Read the sentence and the keyring: a repair that cannot find its row must not fall back to
    // another account and overwrite its password.
    let keyring = Arc::new(MemorySecretStore::new());
    keyring
        .store(&AccountKey::new(ADDRESS), &Password::new("the working one"))
        .await
        .expect("the keyring takes the first password");
    let (database, _) = store_with(account(AuthMethod::Password)).await;

    let session = session(database, keyring.clone());
    let complaint = session
        .repair_credential(4_242, "anything".to_owned())
        .await
        .expect("an account that is not there cannot be repaired");
    assert!(
        complaint.to_lowercase().contains("not in the store"),
        "the refusal has to say which fault this is — a missing row reads \
         nothing like a store that would not open, and the two have \
         different fixes. Got: {complaint}"
    );
    assert_eq!(
        keyring
            .retrieve(&AccountKey::new(ADDRESS))
            .await
            .expect("the credential")
            .expose(),
        "the working one",
        "a repair aimed at an id nothing answers to landed on the account \
         that was there, so a healthy credential was replaced by one typed \
         for something else"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_password_is_refused_rather_than_stored() {
    // An empty secret is indistinguishable downstream from a real one, so it would hide the
    // *Partial* state.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::Password)).await;

    let session = session(database, keyring.clone());
    assert!(
        session
            .repair_credential(id.get(), String::new())
            .await
            .is_some(),
        "an empty password was accepted"
    );
    assert!(
        keyring.retrieve(&AccountKey::new(ADDRESS)).await.is_err(),
        "an empty secret was written to the keyring"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repair_turns_a_disabled_account_back_on() {
    // Same as GTK's `onboarding::configure`: repairing a disabled account is still a repair.
    let keyring = Arc::new(MemorySecretStore::new());
    let mut disabled = account(AuthMethod::AppPassword);
    disabled.enabled = false;
    let (database, id) = store_with(disabled).await;

    let session = session(database, keyring);
    assert_eq!(
        session
            .repair_credential(id.get(), "a fresh app password".to_owned())
            .await,
        None,
        "the repair reported a problem"
    );

    let rows = session.accounts().await;
    let row = rows.first().expect("the pane lists the account");
    assert!(
        !row.facts.iter().any(|fact| fact == "disabled"),
        "the account the user just re-credentialled is still switched off: {:?}",
        row.facts
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_signs_in_with_the_client_the_account_already_has() {
    // Reconnect has no person to supply a client id and secret, so the boundary resolves both from
    // the first sign-in.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::XOAuth2)).await;
    // Written through the token source that writes one at sign-in; the key is that module's
    // business.
    OwnClientTokenSource::new(
        keyring.clone(),
        Url::parse("https://oauth.example.com/token").expect("a token endpoint"),
        "a-client-the-user-registered",
        None,
        None,
    )
    .store_client_secret(
        &AccountKey::new(ADDRESS),
        &Password::new("a desktop client secret"),
    )
    .await
    .expect("the keyring takes the client secret");

    let session = session(database, keyring);
    let resumed = session
        .browser_sign_in_for(id.get())
        .await
        .expect("an account signed in through Postio's own flow can resume it");

    assert_eq!(resumed.address, ADDRESS);
    assert_eq!(
        resumed.client_id, "a-client-the-user-registered",
        "the reconnect would ask the provider to authorize a client the user \
         never registered"
    );
    assert_eq!(
        resumed
            .client_secret
            .as_ref()
            .map(|secret| secret.expose().to_owned()),
        Some("a desktop client secret".to_owned()),
        "a provider that issued a client secret refuses the token exchange \
         without it, and the error it gives back says nothing about why"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_password_account_has_no_browser_sign_in_to_resume() {
    // Both halves: a refusal only in the join would let the exported call open the system browser.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::AppPassword)).await;

    let session = session(database, keyring);
    let complaint = session
        .browser_sign_in_for(id.get())
        .await
        .expect_err("a password account has no grant to re-consent");
    assert!(
        complaint.contains(ADDRESS),
        "the refusal names no account, so a person with several cannot tell \
         which row it is about: {complaint}"
    );
    assert_eq!(
        session.reconnect_account(id.get()).await,
        Some(complaint),
        "the exported call answered differently from the resolution behind \
         it, which means it went past the refusal and opened a browser"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_account_whose_token_a_broker_mints_is_not_offered_a_reconnect() {
    // No OAuth client on the row means an external tool owns the grant: no repair is offered, and
    // resolution refuses to match.
    let keyring = Arc::new(MemorySecretStore::new());
    let mut brokered = account(AuthMethod::XOAuth2);
    brokered.oauth = None;
    let (database, id) = store_with(brokered).await;

    let session = session(database, keyring);
    let rows = session.accounts().await;
    assert_eq!(
        rows.first().expect("the pane lists the account").repair,
        RepairRouteFfi::Nothing
    );
    assert!(
        session.browser_sign_in_for(id.get()).await.is_err(),
        "the boundary offered to reconnect an account it holds no client for"
    );
    assert!(
        session.reconnect_account(id.get()).await.is_some(),
        "the exported call went past the refusal, which means it opened a \
         browser to ask a provider to authorize a client that does not exist"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_never_offers_a_reconnect_the_boundary_would_refuse() {
    // `oauth.is_some()` is not "there is a client": a blank client-id column must offer no
    // Reconnect.
    let keyring = Arc::new(MemorySecretStore::new());
    let mut blank = account(AuthMethod::XOAuth2);
    blank.oauth.as_mut().expect("an oauth row").client_id = "   ".to_owned();
    let (database, id) = store_with(blank).await;

    let session = session(database, keyring);
    let rows = session.accounts().await;
    assert_eq!(
        rows.first().expect("the pane lists the account").repair,
        RepairRouteFfi::Nothing,
        "the row offers a repair the boundary below it will not perform"
    );
    assert!(
        session.browser_sign_in_for(id.get()).await.is_err(),
        "the resolution accepted a client id that `sign_in_with_browser` \
         refuses, so the failure would surface one call later"
    );
    session.shutdown();
}
