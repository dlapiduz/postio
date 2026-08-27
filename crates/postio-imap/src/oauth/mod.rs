//! The OAuth 2 authorization flow: system browser, loopback redirect, PKCE.
//!
//! ADR 0006 Q3, implemented as `OwnClientTokenSource`'s flow — the user's
//! own client credentials, no verification burden, and the same code
//! Postio's verified client will reuse once it has one.
//!
//! * The consent screen opens in the **user's own browser**
//!   ([`browser::BrowserOpener`]) — never Postio's hardened `WebView`, which
//!   has JavaScript off and would let an embedded page read the password
//!   being typed into it.
//! * The redirect lands on a [`redirect::LoopbackRedirect`] bound for the
//!   duration of this one attempt and closed the moment the code arrives or
//!   the caller's [`CancelToken`] fires.
//! * [`pkce::Pkce`] is generated fresh per attempt and sent with the
//!   authorization request; [`pkce::State`] is compared against the
//!   callback before anything is exchanged.
//! * [`exchange`] does the token POST and, when a provider names only an
//!   `issuer`, the RFC 8414 metadata GET that resolves it to concrete
//!   endpoints — both over the same `pimalaya-stream`/`io-http` transport
//!   [`crate::discovery::transport`] already uses, so no new dependency
//!   class arrives for it.
//! * [`token_source::OwnClientTokenSource`] is the [`crate::auth::TokenSource`]
//!   this flow feeds: the refresh token goes to the keyring, the access
//!   token stays a [`crate::secret::Password`] in memory and is never
//!   written to disk.
//!
//! What this module deliberately does not do: decide *when* to start a
//! flow (that is a UI action, one click) or route a rejected token to
//! `Attention` (#194). It is the mechanism those call into.

pub mod browser;
mod error;
pub mod exchange;
pub mod pkce;
pub mod redirect;
pub mod token_source;

use url::Url;

use crate::cancel::CancelToken;

pub use browser::BrowserOpener;
pub use error::OAuthError;
pub use exchange::{Endpoints, TokenResponse};
pub use pkce::{Pkce, State};
pub use redirect::LoopbackRedirect;
pub use token_source::OwnClientTokenSource;

/// Everything one authorization attempt needs to know before it starts.
///
/// `authorize_endpoint` and `token_endpoint` are already resolved — from a
/// preset row directly, or via [`exchange::resolve_endpoints`] against an
/// `issuer` — so this type carries no `Option`s a caller could forget to
/// handle.
#[derive(Clone, Debug)]
pub struct AuthorizeRequest {
    /// The user's own OAuth client id (ADR 0006 Q1 — `OwnClientTokenSource`
    /// never ships a Postio-verified one).
    pub client_id: String,
    /// The user's own client secret, when the provider's token endpoint
    /// requires one even for a public, PKCE-protected client. Rare, and
    /// never written to disk unencrypted — same rule as any other secret.
    pub client_secret: Option<String>,
    /// Where consent is asked.
    pub authorize_endpoint: Url,
    /// Where the code is exchanged.
    pub token_endpoint: Url,
    /// Requested scopes, space-joined into the authorization request per
    /// RFC 6749 §3.3.
    pub scopes: Vec<String>,
}

/// Runs one authorization attempt end to end: bind the loopback listener,
/// open the consent screen in the caller's browser, wait for the redirect,
/// and exchange the code for tokens.
///
/// Cancelling `cancel` at any point — before the browser opens, while
/// waiting for the redirect, or during the token exchange — unwinds
/// cleanly: the listener is dropped (closing its socket) and no token
/// exchange fires for a flow that never produced a validated code.
pub async fn authorize(
    request: AuthorizeRequest,
    opener: &dyn BrowserOpener,
    cancel: &CancelToken,
) -> Result<TokenResponse, OAuthError> {
    let pkce = Pkce::generate();
    let state = State::generate();

    let listener = LoopbackRedirect::bind().await?;
    let redirect_uri = listener.redirect_uri();

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

/// Builds the URL the browser is sent to, per RFC 6749 §4.1.1 plus RFC 7636.
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
