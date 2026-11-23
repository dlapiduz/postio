//! Signing in with a browser, without a toolkit (#1276): endpoints, consent in the user's browser,
//! a proof that the token opens the account's IMAP session, then the secret and the row, secret
//! first and rolled back if the row fails (`postio-67`).
//! A consent path is the last thing that should exist twice, so every frontend signs in through
//! here. Postio never draws a sign-in form (ADR 0006 Q3).

use std::sync::Arc;

use postio_account::cancel::CancelToken;
use postio_account::discovery::AccountSettings;
use postio_account::imap::{ConnectionSettings, ImapSession, RustlsConnector};
use postio_account::oauth::{self, BrowserOpener, Endpoints, OAuthError, TokenResponse};
use postio_account::secret::{AccountKey, Password, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{EmailAddress, Identity};
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

/// The user's own OAuth client (ADR 0006 Q1). Postio ships no client id or secret: shared
/// credentials in an open-source client get revoked for everybody at once.
#[derive(Clone, Debug)]
pub struct OAuthClient {
    pub client_id: String,
    /// A client secret, when the token endpoint insists on one even for a public PKCE client;
    /// stored in the keyring.
    pub client_secret: Option<String>,
}

/// Why a sign-in did not finish. Cancelled is not a failure: it is what closing the browser tab
/// looks like.
#[derive(Debug)]
pub enum SignInError {
    Cancelled,
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

pub struct SignedIn {
    pub endpoints: Endpoints,
    pub tokens: TokenResponse,
}

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

/// Run the flow and prove the result works, without writing anything. `bound` is told the loopback
/// port as soon as it is listening.
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

/// The connection the proof opens: the account's own IMAP server, with the token presented as
/// `XOAUTH2`.
pub fn verification_settings(settings: &AccountSettings) -> ConnectionSettings {
    let mut connection = ConnectionSettings::new(
        settings.imap.host.clone(),
        settings.imap.port,
        match settings.imap.encryption {
            postio_account::discovery::Encryption::Tls => TransportSecurity::Tls,
            postio_account::discovery::Encryption::StartTls => TransportSecurity::StartTls,
            postio_account::discovery::Encryption::None => TransportSecurity::None,
        },
        settings.login.clone(),
    );
    connection.auth = AuthMethod::XOAuth2;
    connection
}

async fn verify(settings: &AccountSettings, tokens: &TokenResponse) -> Result<(), SignInError> {
    let connection = verification_settings(settings);
    let connector = RustlsConnector::new().map_err(|error| {
        SignInError::Failed(format!(
            "Postio could not start a TLS connection on this machine: {error}"
        ))
    })?;
    ImapSession::open(&connection, &tokens.access_token, &connector)
        .await
        .map(|_| ())
        .map_err(|error| SignInError::Failed(crate::onboarding::explain(&error)))
}

/// Write the account and its refresh token, the secret first, then the row. Answers the same
/// `Provisioned` as a password sign-in; an empty `name` means the provider's name or the address
/// stands in.
#[allow(clippy::too_many_arguments)]
pub async fn provision_oauth(
    database: &Store,
    secrets: Arc<dyn SecretStore>,
    settings: &AccountSettings,
    client: &OAuthClient,
    signed_in: SignedIn,
    scopes: &[String],
    refresh_token_lifetime_days: Option<u32>,
    name: &str,
) -> Result<crate::provision::Provisioned, String> {
    let address = settings.email.clone();
    let key = AccountKey::new(address.clone());

    // An existing account makes the row write inert, but the credential must still be seeded: an
    // expired or revoked grant is repaired by signing in again, and returning early dropped the new
    // refresh token while reporting success.
    let existing = {
        let connection = database
            .connect()
            .await
            .map_err(|error| format!("Postio could not open its local store: {error}"))?;
        AccountRepository::new(&connection)
            .list()
            .await
            .map_err(|error| format!("Postio could not read its local store: {error}"))?
            .into_iter()
            .find(|account| account.address.address.eq_ignore_ascii_case(&address))
            .map(|account| account.id)
    };

    let source = oauth::OwnClientTokenSource::new(
        secrets.clone(),
        signed_in.endpoints.token.clone(),
        client.client_id.clone(),
        client.client_secret.clone(),
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

    // The row is repaired, not left: the person may have moved servers or come over from a
    // password, and it must name the grant just approved. `AlreadyProvisioned` tells the caller not
    // to start another engine. Identities stay, since drafts point at them.
    let oauth = postio_model::account::OAuthConfig {
        client_id: client.client_id.clone(),
        token_url: signed_in.endpoints.token.to_string(),
        authorize_url: signed_in.endpoints.authorize.to_string(),
        scopes: scopes.join(" "),
        refresh_token_lifetime_days,
    };
    let connection = database
        .connect()
        .await
        .map_err(|error| format!("Postio could not open its local store: {error}"))?;
    let repository = AccountRepository::new(&connection);
    if let Some(id) = existing {
        let mut row = repository
            .list()
            .await
            .map_err(|error| format!("Postio could not read its local store: {error}"))?
            .into_iter()
            .find(|account| account.id == id)
            .ok_or_else(|| "The account row vanished while it was being repaired.".to_owned())?;
        apply_servers(&mut row, settings);
        row.auth = AuthMethod::XOAuth2;
        row.oauth = Some(oauth);
        row.enabled = true;
        repository
            .update(&mut row)
            .await
            .map_err(|error| format!("Postio could not record the sign-in: {error}"))?;
        return Ok(crate::provision::Provisioned::AlreadyProvisioned(id));
    }

    let mut account = crate::provision::account_from(settings);
    let name = name.trim();
    if !name.is_empty() {
        let email = EmailAddress::new(Some(name.to_owned()), address.clone());
        account.display_name = name.to_owned();
        account.address = email.clone();
        let mut identity = Identity::new(AccountId::UNASSIGNED, email);
        identity.is_default = true;
        account.identities = vec![identity];
    }
    // A browser sign-in is an IMAP account today; the Gmail REST backend is #546.
    account.auth = AuthMethod::XOAuth2;
    account.oauth = Some(oauth);
    match repository.create(&mut account).await {
        Ok(id) => Ok(crate::provision::Provisioned::Created(id)),
        Err(error) => {
            let _ = secrets
                .delete(&AccountKey::new(format!("{}#oauth-refresh", key.account())))
                .await;
            Err(format!("Postio could not record the sign-in: {error}"))
        }
    }
}

fn apply_servers(account: &mut postio_model::account::Account, settings: &AccountSettings) {
    let fresh = crate::provision::account_from(settings);
    account.incoming = fresh.incoming;
    account.outgoing = fresh.outgoing;
}

fn translate(error: OAuthError, context: &str) -> SignInError {
    if matches!(error, OAuthError::Cancelled) {
        SignInError::Cancelled
    } else {
        SignInError::Failed(format!("{context}: {error}"))
    }
}
