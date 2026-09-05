//! Opening a JMAP connection: the stream, the bearer, the cached session.
//!
//! `io-jmap`'s blocking client wraps one stream; this module opens those
//! streams on the workspace's own cancellable `pimalaya-stream` — the same
//! pattern `postio-account`'s OAuth exchange uses — and runs every blocking
//! request under `spawn_blocking` so the async engine never parks a worker
//! on a socket.
//!
//! The [`JmapSession`] object (api url, account id, blob url templates) is
//! resolved once at [`JmapConnection::connect`] and cached: it changes so
//! rarely that re-fetching it per call would double every request. Each
//! call still opens a fresh stream — connection reuse is a later
//! optimization, and HTTP/1.1 keep-alive across `spawn_blocking` calls is
//! not worth the shared-mutable client it would take today.

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

/// Where a connection's bearer comes from.
#[derive(Clone)]
enum Auth {
    /// A token handed in whole, `Bearer ` prefix included. Tests, mostly.
    Fixed(SecretString),
    /// The account's [`TokenSource`] (ADR 0006): asked per request, so a
    /// refreshed credential is picked up without reconnecting anything.
    Source {
        key: AccountKey,
        tokens: Arc<dyn TokenSource>,
    },
}

/// How to reach one JMAP server, plus the session it resolved.
#[derive(Clone)]
pub struct JmapConnection {
    /// The RFC 8620 session resource URL —
    /// `https://api.fastmail.com/jmap/session/` for Fastmail, or whatever
    /// `.well-known/jmap` redirected to.
    session_url: Url,
    /// Where the `Authorization` header comes from.
    auth: Auth,
    /// Resolved once at connect; see the module docs.
    session: Arc<std::sync::Mutex<Option<JmapSession>>>,
}

impl std::fmt::Debug for JmapConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The bearer never reaches a log (CLAUDE.md: credentials are
        // keyring-and-wire only).
        f.debug_struct("JmapConnection")
            .field("session_url", &self.session_url.as_str())
            .finish_non_exhaustive()
    }
}

impl JmapConnection {
    /// A connection that will resolve its session on first use.
    ///
    /// `token` is the raw access token; the `Bearer ` prefix is added here.
    pub fn new(session_url: Url, token: &str) -> Self {
        Self {
            session_url,
            auth: Auth::Fixed(SecretString::from(format!("Bearer {token}"))),
            session: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// A connection whose bearer comes from the account's token source,
    /// asked per request (ADR 0006: one source per account, shared with
    /// SMTP, so an invalidation is seen everywhere at once).
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

    /// The `Authorization` value for the next request.
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

    /// The host this connection dials, for error reporting.
    pub(crate) fn host(&self) -> String {
        self.session_url.host_str().unwrap_or("unknown").to_owned()
    }

    /// The cached session, if one has been resolved.
    pub(crate) fn session(&self) -> Option<JmapSession> {
        self.session.lock().expect("session lock").clone()
    }

    /// Resolves and caches the session object.
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

    /// Runs one blocking client call on a fresh stream, resolving the
    /// session first if this connection never has.
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

/// A cancellable, bounded stream to `url`'s origin — TLS for `https`,
/// plain for `http` (only ever reached in tests, against a mock server on
/// loopback; the seam's settings validation refuses cleartext anywhere
/// else, and so does this).
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
        "http" if is_loopback(&host) => {
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

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// What [`JmapClientStd::new`] wants of a stream.
pub(crate) trait ReadWriteSend: Read + Write + Send {}
impl<S: Read + Write + Send> ReadWriteSend for S {}

/// A stream that fails fast once its token is spent, so a cancelled sync
/// pass stops at the next read or write instead of finishing the request.
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
