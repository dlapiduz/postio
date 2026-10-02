//! An account whose credential has stopped working, from the row that says
//! so to the call that fixes it (#1584).
//!
//! Every app password is eventually rotated and every OAuth grant eventually
//! expires, so this is not an edge case — it is the ordinary end of every
//! account, and until now macOS had no way through it. The row said nothing,
//! the Reconnect button was gated on a fact nothing produced, and the one
//! call that writes a credential refused to write over an existing one.
//!
//! **The assertions here are on what the boundary produces**, which is the
//! whole reason this file exists. `AccountRowTests` on the Swift side built
//! `facts: ["outlook", "oauth2", "token expired"]` by hand and passed
//! forever over a boundary that has never once emitted the word "expired" —
//! a test about a string the frontend invented, asserting nothing about the
//! software. So nothing below hands a fact in: the store and the keyring are
//! seeded the way a real account seeds them, and the row is read back.
//!
//! Nothing here touches the real keyring or the network: the secret store is
//! injected, and an expiry is written through the same token source a
//! completed sign-in writes it through rather than by spelling out its
//! keyring key a second time — a test that guessed that key would go green
//! on a convention that had moved.

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

/// An account row as the store holds one, before any credential exists.
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

/// A store with one account in it, and the id it was given.
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

/// Writes the expiry a minted OAuth token leaves behind, through the token
/// source that mints one — `expires_in` of zero being a token that was
/// already past by the time it was written down.
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

/// A session over `database`, reading `keyring` rather than this machine's.
fn session(database: Store, keyring: Arc<MemorySecretStore>) -> Arc<Session> {
    Session::open(SessionOptions::in_memory_with(database).with_secrets(keyring))
        .expect("a session over the seeded store")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_token_reaches_the_row_as_a_fact_and_a_flag() {
    // The failure #1584 names. The expiry was on file the whole time; there
    // was simply no path from the keyring to the row, so a person whose
    // grant had lapsed saw an account that looked perfectly healthy and
    // stopped receiving mail.
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
    // The other half of the same wire, and the one that keeps the first
    // honest: a row that flagged every OAuth account would be as useless as
    // one that flagged none.
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
    // There is no token, so there is nothing to count down — and the repair,
    // when one is needed, is a field rather than a browser.
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
    // `provision` deliberately refuses this — a re-run of the headless helper
    // must not overwrite a working credential — which left a rotated app
    // password with no route at all. The repair is that route, and it is
    // explicit rather than a change of the helper's default.
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
    // The pane's *Partial* state: a row that exists with no secret behind
    // it. Before this the only way out was to remove the account and add it
    // again, which throws away every mailbox role and every local body with
    // it.
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
    // An OAuth account's plain keyring entry is read by nothing: its
    // credential lives under a derived key and is minted, not typed. Storing
    // a password here would report success and change nothing a server ever
    // sees, which is the failure the OAuth half of #1584 already was.
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
    // An id nothing answers to, over a store that does have an account in
    // it. `!complaint.is_empty()` would pass over "there is no store open",
    // which is a different fault with a different fix, so the sentence is
    // read — and, far more importantly, so is the keyring: a repair that
    // could not find the row it was named must not fall back to *an*
    // account, which is the shape of mistake that writes one person's new
    // password over another account's working one.
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
    // An empty secret is worse than none: it is indistinguishable from a
    // real one to everything downstream, so the *Partial* state — the one
    // state that says "put a credential in" — would be silently unreachable
    // for an account that had been through this field.
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
    // GTK's repair goes through `onboarding::configure`, which says it in a
    // comment: "a repair over an account somebody had disabled is still a
    // repair — the user just proved they want to sign in to it". The same
    // sentence has to be true on both platforms or the panes describe
    // different software.
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
    // The half of *Reconnect* that has nothing to do with the browser, and
    // the half that can be asserted on: what the sign-in is handed.
    //
    // `signInWithBrowser` takes a client id and secret because the
    // add-account sheet has a person in front of it who can supply them.
    // A Reconnect button has no such person — asking somebody to find their
    // registered client id again, on the row that is already telling them
    // their account is broken, is not a repair. Both are on file from the
    // first sign-in, so the boundary resolves them rather than asking, and
    // a frontend that guessed either would open a browser to nowhere.
    let keyring = Arc::new(MemorySecretStore::new());
    let (database, id) = store_with(account(AuthMethod::XOAuth2)).await;
    // Written through the token source that writes one at sign-in, for the
    // same reason the expiry above is: the key is that module's business.
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
    // Both halves, because `reconnect_account` is what a button calls and
    // `browser_sign_in_for` is only the join inside it. A refusal that lived
    // in the join alone would still let the exported call walk on into
    // `sign_in_with_browser` — which opens the system browser. That is the
    // one thing this must be *unable* to do for an account that has no grant
    // to re-consent: the exported call is asserted here to come back with a
    // sentence, and it can only do that by having refused before the browser.
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
    // No OAuth client on the row means nothing here signed it in: `oama` and
    // its kin own the grant. The row says so by offering no repair at all,
    // and the resolution refuses for the same reason — so the two cannot
    // disagree about which accounts the button appears on.
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
    // The fence on the claim `browser_sign_in_for` makes in its own doc
    // comment: that it "refuses exactly where the row offers nothing". Two
    // answers to one question, derived separately, and this is the case
    // where they used to disagree — an OAuth row whose client id column is
    // present but says nothing.
    //
    // `AccountRepository` builds an `OAuthConfig` whenever the client-id and
    // token-url columns are both non-NULL; neither it nor `Account` asks
    // whether either column says anything. So `oauth.is_some()` was never
    // the same question as "there is a client here", and answering the row
    // with it drew a Reconnect button whose only possible outcome was a
    // sentence about a credential the person never registered.
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
