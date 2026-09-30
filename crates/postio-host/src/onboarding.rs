//! Saving an account a frontend already proved.
//!
//! The desktop's first-run screen, its add-account dialog and its
//! credential update prove an account themselves -- the probe, the
//! connection test and the browser sign-in run where the person is, with the
//! transport and the browser opener that frontend was given -- and then hand
//! the writes to the store's owner (`specs/005-tui-frontend` T018). The
//! writes are `postio_session::onboarding`'s, in its order: the credential
//! first, then the row, rolled back if the row will not write.

use std::sync::Arc;

use postio_account::secret::{Password, SecretStore};
use postio_client::protocol::OAuthGrant;
use postio_storage::Store;

/// Write a browser sign-in's tokens to the keyring and its account row to
/// the store (`persist_oauth`). The error is the first-run screen's
/// sentence.
pub async fn save_oauth(
    database: &Store,
    secrets: Arc<dyn SecretStore>,
    grant: OAuthGrant,
) -> Result<(), String> {
    let endpoints = postio_account::oauth::Endpoints {
        authorize: grant
            .authorize_url
            .parse()
            .map_err(|error| format!("The provider's sign-in address is invalid: {error}"))?,
        token: grant
            .token_url
            .parse()
            .map_err(|error| format!("The provider's token address is invalid: {error}"))?,
    };
    let tokens = postio_account::oauth::TokenResponse {
        access_token: Password::new(grant.access_token),
        refresh_token: grant.refresh_token.map(Password::new),
        expires_in: grant.expires_in,
        token_type: grant.token_type,
        scope: grant.scope,
    };
    postio_session::onboarding::persist_oauth(
        database,
        secrets,
        &grant.submission,
        &endpoints,
        &grant.scopes,
        grant.refresh_token_lifetime_days,
        tokens,
    )
    .await
}
