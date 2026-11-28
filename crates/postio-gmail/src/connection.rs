//! Opening a Gmail REST connection: each call runs under `spawn_blocking` on
//! a fresh cancellable stream.

use std::io::{Read, Write};
use std::sync::Arc;

use io_gmail::v1::client::{GmailClientStd, GmailClientStdConnectOptions};
use pimalaya_stream::stream::{Stream, TcpConnectOptions, TlsConnectOptions};
use pimalaya_stream::tls::Tls;
use postio_account::auth::TokenSource;
use postio_account::backend::{BackendError, BackendResult};
use postio_account::cancel::CancelToken;
use postio_account::secret::AccountKey;

#[derive(Clone)]
enum Auth {
    Fixed(String),
    Source {
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
    },
}

#[derive(Clone)]
pub(crate) struct GmailConnection {
    auth: Auth,
    /// `gmail.googleapis.com:443` over TLS in production; tests use plaintext loopback.
    endpoint: Endpoint,
}

#[derive(Clone)]
enum Endpoint {
    Production,
    Loopback { host: String, port: u16 },
}

impl std::fmt::Debug for GmailConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bearer never reaches a log.
        f.debug_struct("GmailConnection").finish_non_exhaustive()
    }
}

impl GmailConnection {
    pub(crate) fn new(token: &str) -> Self {
        Self {
            auth: Auth::Fixed(token.to_owned()),
            endpoint: Endpoint::Production,
        }
    }

    pub(crate) fn with_token_source(key: AccountKey, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            auth: Auth::Source { key, tokens },
            endpoint: Endpoint::Production,
        }
    }

    /// Refused off loopback.
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
            if !postio_model::net::is_loopback(host) {
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

pub(crate) trait ReadWriteSend: Read + Write + Send {}
impl<S: Read + Write + Send> ReadWriteSend for S {}

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
