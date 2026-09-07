//! Signing in with a browser, without a toolkit (#1276).
//!
//! The whole flow — resolve the provider's endpoints, send the user to their
//! own browser, wait on a loopback port, exchange the code, **prove the token
//! actually opens the account's IMAP session**, then write the secret and the
//! row in the order that cannot strand either — was written in
//! `postio-app::onboarding`, which is the GTK composition root. A second
//! frontend cannot link it, and a *consent* path is the last thing that
//! should exist twice: two implementations means two answers to what Postio
//! asked permission for, and the wrong answer is invisible.
//!
//! So it lives here, beside [`crate::provision`], which is the same shape for
//! a password. `postio-app` adopting it is #1283.
//!
//! # The order everything happens in
//!
//! 1. **Endpoints**, from the preset row or discovered from its issuer.
//! 2. **Consent**, in the user's browser. Postio never draws a sign-in form:
//!    a password typed into a mail client is a password that client could
//!    keep (ADR 0006 Q3).
//! 3. **Verification**, before anything persists. The token opens a real
//!    session against the account's own server — the same test the password
//!    path runs — so a consent screen that granted the wrong scopes fails in
//!    front of the user rather than at the first background sync.
//! 4. **The secret, then the row**, rolling the secret back if the row write
//!    fails. `postio-67` is what the other order cost: a keyring write that
//!    failed after the row committed left an account that could not sync,
//!    could not authenticate, and could not be repaired from inside the
//!    application.

use std::sync::Arc;

use postio_account::cancel::CancelToken;
use postio_account::discovery::AccountSettings;
use postio_account::imap::{ConnectionSettings, ImapSession, RustlsConnector};
use postio_account::oauth::{self, BrowserOpener, Endpoints, OAuthError, TokenResponse};
use postio_account::secret::{AccountKey, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

/// The user's own OAuth client (ADR 0006 Q1).
///
/// Postio ships no client id and no secret. A registered application's
/// credentials in an open-source mail client are credentials every user of it
/// shares, and a provider that notices revokes them for everybody at once.
#[derive(Clone, Debug)]
pub struct OAuthClient {
    /// The client id the user registered.
    pub client_id: String,
    /// A client secret, when the provider's token endpoint insists on one
    /// even for a public PKCE client. Rare, and stored in the keyring.
    pub client_secret: Option<String>,
}

/// Why a sign-in did not finish.
///
/// Cancelled is not a failure and must not be reported as one: it is what
/// closing the browser tab looks like from here.
#[derive(Debug)]
pub enum SignInError {
    /// The user cancelled, or closed the tab.
    Cancelled,
    /// Something went wrong, in words for the person who was signing in.
    Failed(String),
}

impl std::fmt::Display for SignInError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => write!(f, "the sign-in was cancelled"),
            Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SignInError {}

/// What a completed sign-in produced.
pub struct SignedIn {
    /// Where consent was asked and where the token came from, resolved.
    pub endpoints: Endpoints,
    /// The tokens themselves. Not logged, not returned to a frontend.
    pub tokens: TokenResponse,
}

/// Resolve the provider's endpoints: given directly, or discovered from the
/// issuer (RFC 8414).
pub async fn endpoints_for(
    authorize: Option<&str>,
    token: Option<&str>,
    issuer: Option<&str>,
    cancel: &CancelToken,
) -> Result<Endpoints, SignInError> {
    if let (Some(authorize), Some(token)) = (authorize, token) {
        let authorize = authorize.parse().map_err(|error| {
            SignInError::Failed(format!("The authorize URL is invalid: {error}"))
        })?;
        let token = token
            .parse()
            .map_err(|error| SignInError::Failed(format!("The token URL is invalid: {error}")))?;
        return Ok(Endpoints { authorize, token });
    }
    let Some(issuer) = issuer else {
        return Err(SignInError::Failed(
            "Postio does not know where this provider asks for consent.".to_owned(),
        ));
    };
    let issuer = issuer.parse().map_err(|error| {
        SignInError::Failed(format!("The provider's issuer is invalid: {error}"))
    })?;
    oauth::exchange::resolve_endpoints(&issuer, cancel)
        .await
        .map_err(|error| translate(error, "Could not discover the provider's sign-in endpoints"))
}

/// Run the flow and prove the result works, without writing anything.
///
/// `bound` is told the loopback port as soon as it is listening, so a
/// frontend can say where the answer will come back — which is most of what
/// there is to say while somebody is away in their browser.
pub async fn sign_in(
    settings: &AccountSettings,
    client: &OAuthClient,
    endpoints: &Endpoints,
    scopes: &[String],
    opener: &dyn BrowserOpener,
    cancel: &CancelToken,
    bound: &(dyn Fn(u16) + Sync),
) -> Result<SignedIn, SignInError> {
    let tokens = oauth::authorize_watching(
        oauth::AuthorizeRequest {
            client_id: client.client_id.clone(),
            client_secret: client.client_secret.clone(),
            authorize_endpoint: endpoints.authorize.clone(),
            token_endpoint: endpoints.token.clone(),
            scopes: scopes.to_vec(),
        },
        opener,
        cancel,
        bound,
    )
    .await
    .map_err(|error| translate(error, "The sign-in did not complete"))?;

    verify(settings, &tokens).await?;

    Ok(SignedIn {
        endpoints: endpoints.clone(),
        tokens,
    })
}

/// The proof, before anything persists.
async fn verify(settings: &AccountSettings, tokens: &TokenResponse) -> Result<(), SignInError> {
    let connection = ConnectionSettings::new(
        settings.imap.host.clone(),
        settings.imap.port,
        // Carried rather than upgraded: the settings a user is shown are the
        // settings in use, which is `provision`'s rule about the same join.
        match settings.imap.encryption {
            postio_account::discovery::Encryption::Tls => TransportSecurity::Tls,
            postio_account::discovery::Encryption::StartTls => TransportSecurity::StartTls,
            postio_account::discovery::Encryption::None => TransportSecurity::None,
        },
        settings.login.clone(),
    );
    let connector = RustlsConnector::new().map_err(|error| {
        SignInError::Failed(format!(
            "Postio could not start a TLS connection on this machine: {error}"
        ))
    })?;
    ImapSession::open(&connection, &tokens.access_token, &connector)
        .await
        .map(|_| ())
        .map_err(|error| SignInError::Failed(format!("The provider refused the token: {error}")))
}

/// Write the account and its refresh token — the secret first, then the row.
///
/// Answers the same [`crate::provision::Provisioned`] a password sign-in
/// does, so a caller that has both routes has one answer to interpret.
pub async fn provision_oauth(
    database: &Database,
    secrets: Arc<dyn SecretStore>,
    settings: &AccountSettings,
    client: &OAuthClient,
    signed_in: SignedIn,
    scopes: &[String],
    refresh_token_lifetime_days: Option<u32>,
) -> Result<crate::provision::Provisioned, String> {
    let address = settings.email.clone();
    let key = AccountKey::new(address.clone());

    // Already here: adding the same account twice is inert, the same way the
    // password path is. Somebody who signs in again after a stumble has one
    // account, not two.
    {
        let connection = database
            .connection()
            .map_err(|error| format!("Postio could not open its local store: {error}"))?;
        if let Some(found) = AccountRepository::new(&connection)
            .list()
            .map_err(|error| format!("Postio could not read its local store: {error}"))?
            .into_iter()
            .find(|account| account.address.address.eq_ignore_ascii_case(&address))
        {
            return Ok(crate::provision::Provisioned::AlreadyProvisioned(found.id));
        }
    }

    let source = oauth::OwnClientTokenSource::new(
        secrets.clone(),
        signed_in.endpoints.token.clone(),
        client.client_id.clone(),
        client.client_secret.clone(),
        // So the mint records the grant's deadline, not just its rotations.
        refresh_token_lifetime_days
            .map(|days| std::time::Duration::from_secs(u64::from(days) * 86_400)),
    );
    source.seed(&key, signed_in.tokens).await.map_err(|error| {
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

    match write_row(
        database,
        settings,
        client,
        &signed_in.endpoints,
        scopes,
        refresh_token_lifetime_days,
    ) {
        Ok(id) => Ok(crate::provision::Provisioned::Created(id)),
        Err(reason) => {
            // Rolled back the way `provision` rolls back a password: nothing
            // reads a credential no account row names, but leaving one is
            // untidy.
            let _ = secrets
                .delete(&AccountKey::new(format!("{}#oauth-refresh", key.account())))
                .await;
            Err(reason)
        }
    }
}

/// The row write: the account, then its auth method and OAuth client.
fn write_row(
    database: &Database,
    settings: &AccountSettings,
    client: &OAuthClient,
    endpoints: &Endpoints,
    scopes: &[String],
    refresh_token_lifetime_days: Option<u32>,
) -> Result<postio_model::ids::AccountId, String> {
    let mut account = crate::provision::account_from(settings);
    // A browser sign-in is an IMAP account today; the Gmail REST backend is
    // #546, gated on its preset row flipping after #195.
    account.auth = AuthMethod::XOAuth2;
    account.oauth = Some(postio_model::account::OAuthConfig {
        client_id: client.client_id.clone(),
        token_url: endpoints.token.to_string(),
        authorize_url: endpoints.authorize.to_string(),
        scopes: scopes.join(" "),
        refresh_token_lifetime_days,
    });

    let connection = database
        .connection()
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    AccountRepository::new(&connection)
        .create(&mut account)
        .map_err(|error| format!("Postio could not record the sign-in: {error}"))
}

/// A cancelled flow is not a failure, and must not be reported as one.
fn translate(error: OAuthError, context: &str) -> SignInError {
    if matches!(error, OAuthError::Cancelled) {
        SignInError::Cancelled
    } else {
        SignInError::Failed(format!("{context}: {error}"))
    }
}
