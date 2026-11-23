//! ADR 0006 Q3: the user's own client credentials, system browser, loopback redirect and PKCE.
//! Deciding when to start a flow (a UI action) and routing a rejected token to `Attention` (#194)
//! are not here.

pub mod browser;
mod error;
pub mod exchange;
pub mod pkce;
pub mod redirect;
pub mod token_source;

use crate::cancel::CancelToken;

pub use url::Url;

pub use browser::BrowserOpener;
pub use error::OAuthError;
pub use exchange::{Endpoints, TokenResponse};
pub use pkce::{Pkce, State};
pub use redirect::LoopbackRedirect;
pub use token_source::OwnClientTokenSource;

#[derive(Clone, Debug)]
pub struct AuthorizeRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub authorize_endpoint: Url,
    pub token_endpoint: Url,
    pub scopes: Vec<String>,
}

/// Cancelling at any point unwinds cleanly: the listener is dropped and no token exchange fires for
/// a flow that never produced a validated code.
pub async fn authorize(
    request: AuthorizeRequest,
    opener: &dyn BrowserOpener,
    cancel: &CancelToken,
) -> Result<TokenResponse, OAuthError> {
    authorize_watching(request, opener, cancel, &|_| {}).await
}

/// A callback rather than a returned port: the port exists only for the span of the wait, and the
/// call that would return it blocks until the flow ends. The listener binds `:0` because a fixed
/// port may be held.
pub async fn authorize_watching(
    request: AuthorizeRequest,
    opener: &dyn BrowserOpener,
    cancel: &CancelToken,
    bound: &(dyn Fn(u16) + Sync),
) -> Result<TokenResponse, OAuthError> {
    let pkce = Pkce::generate();
    let state = State::generate();

    let listener = LoopbackRedirect::bind().await?;
    let redirect_uri = listener.redirect_uri();
    bound(listener.port());

    let authorize_url = authorization_url(&request, &pkce, &state, &redirect_uri);
    opener.open(&authorize_url).map_err(OAuthError::Browser)?;

    let code = listener.wait_for_code(&state, cancel).await?;

    exchange::exchange_code(
        &request.token_endpoint,
        exchange::CodeExchange {
            client_id: &request.client_id,
            client_secret: request.client_secret.as_deref(),
            code: code.as_str(),
            code_verifier: pkce.verifier(),
            redirect_uri: redirect_uri.as_str(),
        },
        cancel,
    )
    .await
}

fn authorization_url(
    request: &AuthorizeRequest,
    pkce: &Pkce,
    state: &State,
    redirect_uri: &Url,
) -> Url {
    let mut url = request.authorize_endpoint.clone();
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &request.client_id)
        .append_pair("redirect_uri", redirect_uri.as_str())
        .append_pair("scope", &request.scopes.join(" "))
        .append_pair("state", state.as_str())
        .append_pair("code_challenge", &pkce.challenge())
        .append_pair("code_challenge_method", "S256");
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AuthorizeRequest {
        AuthorizeRequest {
            client_id: "the-client-id".to_string(),
            client_secret: None,
            authorize_endpoint: "https://example.com/authorize".parse().unwrap(),
            token_endpoint: "https://example.com/token".parse().unwrap(),
            scopes: vec!["mail.read".to_string(), "mail.send".to_string()],
        }
    }

    #[test]
    fn the_authorization_url_carries_pkce_and_state() {
        let pkce = Pkce::generate();
        let state = State::generate();
        let redirect_uri: Url = "http://127.0.0.1:4242/".parse().unwrap();

        let url = authorization_url(&request(), &pkce, &state, &redirect_uri);

        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["response_type"], "code");
        assert_eq!(pairs["client_id"], "the-client-id");
        assert_eq!(pairs["redirect_uri"], "http://127.0.0.1:4242/");
        assert_eq!(pairs["scope"], "mail.read mail.send");
        assert_eq!(pairs["state"], state.as_str());
        assert_eq!(pairs["code_challenge"], pkce.challenge());
        assert_eq!(pairs["code_challenge_method"], "S256");
    }
}
