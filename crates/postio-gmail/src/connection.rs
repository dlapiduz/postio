//! Opening a Gmail REST connection: the stream and the bearer.
//!
//! Same story as the JMAP adapter's session module: `io-gmail`'s blocking
//! client wraps one stream; streams open on the workspace's own
//! cancellable `pimalaya-stream`, every call runs under `spawn_blocking`,
//! and each call gets a fresh connection.

use std::io::{Read, Write};
use std::sync::Arc;

use io_gmail::v1::client::{GmailClientStd, GmailClientStdConnectOptions};
use pimalaya_stream::stream::{Stream, TcpConnectOptions, TlsConnectOptions};
use pimalaya_stream::tls::Tls;
use postio_account::auth::TokenSource;
use postio_account::backend::{BackendError, BackendResult};
use postio_account::cancel::CancelToken;
use postio_account::secret::AccountKey;

/// Where the bearer comes from.
#[derive(Clone)]
enum Auth {
    /// A raw token handed in whole. Tests, mostly.
    Fixed(String),
    /// The account's [`TokenSource`] (ADR 0006), asked per request.
    Source {
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
    },
}

/// How to reach the Gmail REST API for one account.
#[derive(Clone)]
pub(crate) struct GmailConnection {
    auth: Auth,
    /// Where to dial. `gmail.googleapis.com:443` over TLS in production;
    /// tests point this at a scripted server on loopback, plaintext.
    endpoint: Endpoint,
}

#[derive(Clone)]
enum Endpoint {
    Production,
    /// Loopback-only, for the scripted test server.
    Loopback {
        host: String,
        port: u16,
    },
}

impl std::fmt::Debug for GmailConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bearer never reaches a log.
        f.debug_struct("GmailConnection").finish_non_exhaustive()
    }
}

impl GmailConnection {
    /// A production connection with a fixed token.
    pub(crate) fn new(token: &str) -> Self {
        Self {
            auth: Auth::Fixed(token.to_owned()),
            endpoint: Endpoint::Production,
        }
    }

    /// A production connection whose bearer comes from the account's
    /// token source, asked per request.
    pub(crate) fn with_token_source(key: AccountKey, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            auth: Auth::Source { key, tokens },
            endpoint: Endpoint::Production,
        }
    }

    /// Point every request at a scripted server on loopback instead of
    /// Google. Refused off loopback — the same rule as everywhere else a
    /// plaintext connection exists for tests.
    pub(crate) fn with_loopback_endpoint(mut self, host: &str, port: u16) -> Self {
        self.endpoint = Endpoint::Loopback {
            host: host.to_owned(),
            port,
        };
        self
    }

    async fn token(&self) -> BackendResult<String> {
        match &self.auth {
            Auth::Fixed(token) => Ok(token.clone()),
            Auth::Source { key, tokens } => {
                let token = tokens
                    .access_token(key)
                    .await
                    .map_err(|error| BackendError::Auth {
                        account: key.to_string(),
                        reason: error.to_string(),
                    })?;
                Ok(token.expose().to_owned())
            }
        }
    }

    /// Runs one blocking client call on a fresh stream.
    pub(crate) async fn run<T, F>(&self, cancel: &CancelToken, call: F) -> BackendResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut GmailClientStd) -> BackendResult<T> + Send + 'static,
    {
        let token = self.token().await?;
        let endpoint = self.endpoint.clone();
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            let stream = open_stream(&endpoint, &cancel)?;
            let mut client =
                GmailClientStd::new(stream, token, GmailClientStdConnectOptions::default());
            call(&mut client)
        })
        .await
        .map_err(|error| BackendError::Io {
            context: "the Gmail request task did not finish".to_owned(),
            reason: error.to_string(),
        })?
    }
}

fn open_stream(
    endpoint: &Endpoint,
    cancel: &CancelToken,
) -> BackendResult<Cancellable<Box<dyn ReadWriteSend>>> {
    let stream: Box<dyn ReadWriteSend> = match endpoint {
        Endpoint::Production => {
            let options = TlsConnectOptions {
                tls: Tls::default(),
                ..Default::default()
            };
            Box::new(Stream::connect_tls("gmail.googleapis.com", 443, options).map_err(io_error)?)
        }
        Endpoint::Loopback { host, port } => {
            if !is_loopback(host) {
                return Err(BackendError::Protocol {
                    reason: format!("refusing a plaintext Gmail endpoint off loopback: {host}"),
                });
            }
            Box::new(
                Stream::connect_tcp(host, *port, TcpConnectOptions::default()).map_err(io_error)?,
            )
        }
    };
    Ok(Cancellable {
        inner: stream,
        cancel: cancel.clone(),
    })
}

fn io_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::Io {
        context: "opening the Gmail stream".to_owned(),
        reason: error.to_string(),
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub(crate) trait ReadWriteSend: Read + Write + Send {}
impl<S: Read + Write + Send> ReadWriteSend for S {}

/// A stream that fails fast once its token is spent.
struct Cancellable<S> {
    inner: S,
    cancel: CancelToken,
}

impl<S> Cancellable<S> {
    fn check(&self) -> std::io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        Ok(())
    }
}

impl<S: Read> Read for Cancellable<S> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        self.inner.read(buf)
    }
}

impl<S: Write> Write for Cancellable<S> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.check()?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.check()?;
        self.inner.flush()
    }
}
