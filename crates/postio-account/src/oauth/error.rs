//! What can go wrong across the whole authorization flow.
//!
//! One enum for [`redirect`](super::redirect), [`exchange`](super::exchange)
//! and [`super::authorize`] itself, because a caller showing the user a
//! failure does not care which stage produced it — it cares whether trying
//! again is reasonable ([`OAuthError::Cancelled`] and
//! [`OAuthError::Denied`] are not the same kind of failure as
//! [`OAuthError::Http`]).

use std::io;

use crate::secret::SecretError;

/// Everything that can end an authorization attempt without tokens.
#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    /// The loopback listener could not be bound.
    #[error("could not open a local port for the OAuth redirect: {0}")]
    Bind(#[source] io::Error),

    /// The user's [`CancelToken`](crate::cancel::CancelToken) fired before
    /// a code was obtained. Not a failure to report as an error banner —
    /// the user asked for this.
    #[error("the sign-in attempt was cancelled")]
    Cancelled,

    /// The provider's own consent screen returned `error=…` — most often
    /// `access_denied`, the user declining consent.
    #[error("the provider declined: {0}")]
    Denied(String),

    /// [`BrowserOpener::open`](super::browser::BrowserOpener::open) failed
    /// to launch anything.
    #[error("could not open the system browser: {0}")]
    Browser(#[source] io::Error),

    /// The redirect socket accepted a connection but reading or writing on
    /// it failed or ran past [`super::redirect::REDIRECT_IO_TIMEOUT`].
    #[error("the OAuth redirect connection failed: {0}")]
    Redirect(#[source] io::Error),

    /// The HTTP exchange with the authorization server itself failed —
    /// connecting, TLS, or the request/response cycle.
    #[error("could not reach the OAuth server: {0}")]
    Http(String),

    /// The server answered with a non-success status.
    #[error("the OAuth server rejected the request (status {status}): {body}")]
    Status {
        /// The HTTP status code returned.
        status: u16,
        /// The response body, truncated to what the server sent — providers
        /// put `error`/`error_description` here on a token exchange
        /// failure.
        body: String,
    },

    /// The response body was not the JSON the token or metadata endpoint is
    /// required to return.
    #[error("could not parse the OAuth server's response: {0}")]
    Parse(String),

    /// A provider row named `oauth2` with neither an `issuer` nor explicit
    /// endpoints — should already be unreachable past `providers.toml`'s
    /// own validation, but a caller assembling an [`super::AuthorizeRequest`]
    /// by hand can still hit it.
    #[error("no {0} endpoint is known for this provider")]
    MissingEndpoint(&'static str),

    /// Storing or reading the refresh token in the keyring failed.
    #[error(transparent)]
    Secret(#[from] SecretError),
}
