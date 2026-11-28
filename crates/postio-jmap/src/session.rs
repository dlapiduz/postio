//! Opening a JMAP connection: the stream, the bearer, the cached session.
//! The session is resolved once at connect and cached; each call opens a
//! fresh stream under `spawn_blocking`.

use std::io::{Read, Write};
use std::sync::Arc;

use io_jmap::client::JmapClientStd;
use io_jmap::rfc8620::session::JmapSession;
use pimalaya_stream::stream::{Stream, TcpConnectOptions, TlsConnectOptions};
use pimalaya_stream::tls::Tls;
use postio_account::auth::TokenSource;
use postio_account::backend::{BackendError, BackendResult};
use postio_account::cancel::CancelToken;
use postio_account::secret::AccountKey;
use secrecy::SecretString;
use url::Url;

use crate::error::backend_error;

#[derive(Clone)]
enum Auth {
    Fixed(SecretString),
    /// Asked per request, so a refreshed credential is picked up without reconnecting.
    Source {
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
    },
}

#[derive(Clone)]
pub struct JmapConnection {
    session_url: Url,
    auth: Auth,
    session: Arc<std::sync::Mutex<Option<JmapSession>>>,
}

impl std::fmt::Debug for JmapConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bearer never reaches a log.
        f.debug_struct("JmapConnection")
            .field("session_url", &self.session_url.as_str())
            .finish_non_exhaustive()
    }
}

impl JmapConnection {
    /// `token` is the raw access token; the `Bearer ` prefix is added here.
    pub fn new(session_url: Url, token: &str) -> Self {
        Self {
            session_url,
            auth: Auth::Fixed(SecretString::from(format!("Bearer {token}"))),
            session: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// One source per account, shared with SMTP, so an invalidation is seen everywhere (ADR 0006).
    pub fn with_token_source(
        session_url: Url,
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
    ) -> Self {
        Self {
            session_url,
            auth: Auth::Source { key, tokens },
            session: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    async fn http_auth(&self) -> BackendResult<SecretString> {
        match &self.auth {
            Auth::Fixed(auth) => Ok(auth.clone()),
            Auth::Source { key, tokens } => {
                let token = tokens
                    .access_token(key)
                    .await
                    .map_err(|error| BackendError::Auth {
                        account: key.to_string(),
                        reason: error.to_string(),
                    })?;
                Ok(SecretString::from(format!("Bearer {}", token.expose())))
            }
        }
    }

    pub(crate) fn host(&self) -> String {
        self.session_url.host_str().unwrap_or("unknown").to_owned()
    }

    pub(crate) fn session(&self) -> Option<JmapSession> {
        self.session.lock().expect("session lock").clone()
    }

    pub(crate) async fn connect(&self, cancel: &CancelToken) -> BackendResult<JmapSession> {
        let url = self.session_url.clone();
        let auth = self.http_auth().await?;
        let cancel = cancel.clone();
        let session = tokio::task::spawn_blocking(move || {
            let stream = open_stream(&url, &cancel)?;
            let mut client = JmapClientStd::new(stream, auth);
            client
                .session_get(&url)
                .cloned()
                .map_err(|error| backend_error("resolving the JMAP session", error))
        })
        .await
        .map_err(join_error)??;

        *self.session.lock().expect("session lock") = Some(session.clone());
        Ok(session)
    }

    pub(crate) async fn run<T, F>(&self, cancel: &CancelToken, call: F) -> BackendResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut JmapClientStd) -> BackendResult<T> + Send + 'static,
    {
        let session = match self.session() {
            Some(session) => session,
            None => self.connect(cancel).await?,
        };
        let url = self.session_url.clone();
        let auth = self.http_auth().await?;
        let cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            let stream = open_stream(&url, &cancel)?;
            let mut client = JmapClientStd::from_parts(stream, auth, session);
            call(&mut client)
        })
        .await
        .map_err(join_error)?
    }
}

fn join_error(error: tokio::task::JoinError) -> BackendError {
    BackendError::Io {
        context: "the JMAP request task did not finish".to_owned(),
        reason: error.to_string(),
    }
}

/// Plain `http` is only reached in tests, against loopback; cleartext anywhere else is refused.
pub(crate) fn open_stream(
    url: &Url,
    cancel: &CancelToken,
) -> BackendResult<Box<dyn ReadWriteSend>> {
    let host = url
        .host_str()
        .ok_or_else(|| BackendError::Protocol {
            reason: format!("JMAP URL `{url}` has no host"),
        })?
        .to_owned();

    match url.scheme() {
        "https" => {
            let port = url.port_or_known_default().unwrap_or(443);
            let options = TlsConnectOptions {
                tls: Tls::default(),
                ..Default::default()
            };
            let stream = Stream::connect_tls(&host, port, options).map_err(io_error)?;
            Ok(Box::new(Cancellable::new(stream, cancel.clone())))
        }
        "http" if postio_model::net::is_loopback(&host) => {
            let port = url.port_or_known_default().unwrap_or(80);
            let stream =
                Stream::connect_tcp(&host, port, TcpConnectOptions::default()).map_err(io_error)?;
            Ok(Box::new(Cancellable::new(stream, cancel.clone())))
        }
        other => Err(BackendError::Protocol {
            reason: format!("refusing scheme `{other}` for a JMAP server (`{url}`)"),
        }),
    }
}

fn io_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::Io {
        context: "opening the JMAP stream".to_owned(),
        reason: error.to_string(),
    }
}

pub(crate) trait ReadWriteSend: Read + Write + Send {}
impl<S: Read + Write + Send> ReadWriteSend for S {}

/// Fails fast once its token is spent, so a cancelled sync pass stops at the next read or write.
struct Cancellable<S> {
    inner: S,
    cancel: CancelToken,
}

impl<S> Cancellable<S> {
    fn new(inner: S, cancel: CancelToken) -> Self {
        Self { inner, cancel }
    }

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
