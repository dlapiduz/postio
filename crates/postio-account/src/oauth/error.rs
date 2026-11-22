use std::io;

use crate::secret::SecretError;

#[derive(Debug, thiserror::Error)]
pub enum OAuthError {
    #[error("could not open a local port for the OAuth redirect: {0}")]
    Bind(#[source] io::Error),

    /// Not an error banner: the user asked for this.
    #[error("the sign-in attempt was cancelled")]
    Cancelled,

    #[error("the provider declined: {0}")]
    Denied(String),

    #[error("could not open the system browser: {0}")]
    Browser(#[source] io::Error),

    #[error("the OAuth redirect connection failed: {0}")]
    Redirect(#[source] io::Error),

    #[error("could not reach the OAuth server: {0}")]
    Http(String),

    #[error("the OAuth server rejected the request (status {status}): {body}")]
    Status { status: u16, body: String },

    #[error("could not parse the OAuth server's response: {0}")]
    Parse(String),

    /// Should be unreachable past `providers.toml` validation, but a caller assembling a request by
    /// hand can hit it.
    #[error("no {0} endpoint is known for this provider")]
    MissingEndpoint(&'static str),

    #[error(transparent)]
    Secret(#[from] SecretError),
}
