//! Sockets and TLS, kept out of the protocols.
//!
//! `io-imap` and `io-smtp` are both sans-I/O: a session-opening coroutine
//! *asks* for a TCP connect, a TLS connect or an upgrade and never performs
//! one. That is what lets Postio own its runtime and TLS stack -- and what
//! lets a whole handshake be driven over a canned transcript with no socket
//! at all. This is the half both protocols share: the error, the two traits
//! a session drives, and the real tokio and `tokio-rustls` implementation.
//! Each protocol crate keeps its own scripted double and its own mapping of
//! [`TransportError`] into its error type.
//!
//! # There is no plaintext fallback
//!
//! A failed TLS handshake is [`TransportError::Tls`] and the connection ends.
//! Retrying in the clear is a decision no mail client gets to make on the
//! user's behalf, so the code to do it does not exist.

use std::fmt;
use std::io;
use std::sync::{Arc, Once};
use std::time::Duration;

use async_trait::async_trait;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

/// How long to wait for a socket or a TLS handshake.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything the transport layer can fail with.
///
/// No variant carries a credential: the handshake bytes never reach an error
/// message, only the reason a socket or a certificate did not work out.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The TCP connection could not be established.
    #[error("could not reach {host}:{port}: {reason}")]
    Connect {
        /// The host that was unreachable.
        host: String,
        /// The port that was tried.
        port: u16,
        /// What the OS reported.
        reason: String,
    },

    /// The TLS handshake failed or the certificate did not verify.
    #[error("TLS failed for {host}: {reason}")]
    Tls {
        /// The host whose certificate was rejected.
        host: String,
        /// What the TLS stack reported.
        reason: String,
    },

    /// The server closed the connection.
    #[error("the server closed the connection")]
    Closed,

    /// Reading or writing failed.
    #[error("{context} failed: {reason}")]
    Io {
        /// What was being attempted.
        context: String,
        /// The underlying error.
        reason: String,
    },

    /// The server accepted something and then said nothing.
    #[error("{context} timed out after {after:?}")]
    TimedOut {
        /// What was being waited for.
        context: String,
        /// How long we waited.
        after: Duration,
    },

    /// The transport cannot do what the protocol asked of it.
    #[error("{0}")]
    Unsupported(String),
}

/// An open connection to a server.
///
/// Byte-level and deliberately dumb: it reads, it writes, and it can hand
/// itself to a TLS stack once. Everything about *what* those bytes mean lives
/// above it.
#[async_trait]
pub trait Stream: Send + fmt::Debug {
    /// Reads whatever is available. `Ok(0)` means the peer closed.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Writes every byte. A short write would desynchronize the exchange.
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Reads whatever is available, giving up if nothing arrives for
    /// `timeout`.
    ///
    /// The bound is on *silence*, never on how long an exchange takes. A
    /// multi-megabyte attachment over a slow link needs minutes and is not a
    /// failure; a server that accepted a command and went quiet is, and
    /// costs one of a handful of pooled connections until something says so.
    /// `Duration::ZERO` waits forever, which is what a watcher parked in
    /// `IDLE` wants.
    async fn read_within(
        &mut self,
        buf: &mut [u8],
        timeout: Duration,
        context: &str,
    ) -> Result<usize, TransportError> {
        if timeout.is_zero() {
            return self.read(buf).await;
        }
        match tokio::time::timeout(timeout, self.read(buf)).await {
            Ok(read) => read,
            Err(_) => Err(TransportError::TimedOut {
                context: context.to_owned(),
                after: timeout,
            }),
        }
    }

    /// Wraps this connection in TLS, the `STARTTLS` half the protocol cannot
    /// perform itself.
    ///
    /// Consumes the plaintext stream, so there is no way to keep using it.
    async fn upgrade_tls(self: Box<Self>, host: &str) -> Result<Box<dyn Stream>, TransportError>;

    /// Whether the bytes on this connection are encrypted.
    fn is_encrypted(&self) -> bool;
}

/// Opens the connections the session coroutine asks for.
#[async_trait]
pub trait Connector: Send + Sync + fmt::Debug {
    /// Opens a plaintext TCP connection.
    async fn connect_tcp(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError>;

    /// Opens a TLS connection, verifying the certificate against `host`.
    async fn connect_tls(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError>;

    /// Opens a local unix socket, for a pre-authenticated socket proxy.
    async fn connect_unix(&self, path: &str) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Unsupported(format!(
            "this connector cannot open the unix socket at {path}"
        )))
    }
}

// ---------------------------------------------------------------------------
// tokio + rustls
// ---------------------------------------------------------------------------

/// Installs the ring crypto provider exactly once per process.
pub fn install_crypto_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // An error here means another provider is already installed, which is
        // fine: something else in the process got there first.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The real connector: tokio sockets, `tokio-rustls`, platform trust store.
///
/// Certificate verification goes through `rustls-platform-verifier`, so a
/// certificate Postio accepts is one the rest of the desktop accepts, and an
/// enterprise root installed in the system store works without Postio growing
/// a trust-store setting.
#[derive(Clone)]
pub struct RustlsConnector {
    connector: TlsConnector,
    timeout: Duration,
    /// Where every connection attempt is reported (#151), or nowhere: a
    /// connector a test builds without a sink still connects, it just
    /// proves nothing.
    egress: Option<Arc<dyn postio_model::egress::EgressSink>>,
    /// Which protocol this connector's attempts are logged under.
    subsystem: postio_model::egress::EgressSubsystem,
}

impl fmt::Debug for RustlsConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustlsConnector")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl RustlsConnector {
    /// A connector using the platform trust store.
    pub fn new() -> Result<Self, TransportError> {
        install_crypto_provider();
        let config =
            ClientConfig::with_platform_verifier().map_err(|error| TransportError::Tls {
                host: "*".to_owned(),
                reason: format!("could not build a TLS configuration: {error}"),
            })?;
        Ok(Self {
            connector: TlsConnector::from(Arc::new(config)),
            timeout: DEFAULT_CONNECT_TIMEOUT,
            egress: None,
            subsystem: postio_model::egress::EgressSubsystem::Imap,
        })
    }

    /// Sets how long a connect or handshake may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Report every connection attempt to `sink` — the egress log's seam
    /// (#151). Success and failure alike: a log that only showed successes
    /// would hide exactly the traffic a user most wants to see.
    ///
    /// `subsystem` is the protocol the log files them under: one connector
    /// type serves both IMAP and SMTP, and a user reading the log needs to
    /// know which of the two opened a connection.
    pub fn with_egress(
        mut self,
        sink: Arc<dyn postio_model::egress::EgressSink>,
        subsystem: postio_model::egress::EgressSubsystem,
    ) -> Self {
        self.egress = Some(sink);
        self.subsystem = subsystem;
        self
    }

    async fn tcp(&self, host: &str, port: u16) -> Result<TcpStream, TransportError> {
        let attempt = tokio::time::timeout(self.timeout, TcpStream::connect((host, port)));
        let outcome = match attempt.await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(error)) => Err(TransportError::Connect {
                host: host.to_owned(),
                port,
                reason: error.to_string(),
            }),
            Err(_) => Err(TransportError::Connect {
                host: host.to_owned(),
                port,
                reason: format!("no answer within {}s", self.timeout.as_secs()),
            }),
        };
        // At the TCP stage, before any handshake: "a connection was opened
        // to this host" is the privacy-relevant fact, and it is true the
        // moment the socket is, whatever TLS makes of it afterwards.
        if let Some(egress) = &self.egress {
            egress.record(postio_model::egress::EgressEvent {
                at: chrono::Utc::now(),
                subsystem: self.subsystem,
                account: None,
                host: host.to_owned(),
                port,
                outcome: if outcome.is_ok() {
                    postio_model::egress::EgressOutcome::Connected
                } else {
                    postio_model::egress::EgressOutcome::Failed
                },
            });
        }
        outcome
    }

    async fn handshake(
        &self,
        stream: TcpStream,
        host: &str,
    ) -> Result<TlsStream<TcpStream>, TransportError> {
        let name = ServerName::try_from(host.to_owned()).map_err(|error| TransportError::Tls {
            host: host.to_owned(),
            reason: format!("{host} is not a valid certificate name: {error}"),
        })?;

        let attempt = tokio::time::timeout(self.timeout, self.connector.connect(name, stream));
        match attempt.await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(error)) => Err(TransportError::Tls {
                host: host.to_owned(),
                reason: error.to_string(),
            }),
            Err(_) => Err(TransportError::Tls {
                host: host.to_owned(),
                reason: format!(
                    "the handshake did not finish within {}s",
                    self.timeout.as_secs()
                ),
            }),
        }
    }
}

#[async_trait]
impl Connector for RustlsConnector {
    async fn connect_tcp(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError> {
        let stream = self.tcp(host, port).await?;
        Ok(Box::new(TokioStream::Plain {
            stream,
            connector: self.clone(),
        }))
    }

    async fn connect_tls(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError> {
        let stream = self.tcp(host, port).await?;
        let stream = self.handshake(stream, host).await?;
        Ok(Box::new(TokioStream::Encrypted {
            stream: Box::new(stream),
        }))
    }
}

/// The two shapes a real connection takes.
#[derive(Debug)]
enum TokioStream {
    /// Plain TCP, waiting for a `STARTTLS` upgrade that may never come.
    Plain {
        stream: TcpStream,
        connector: RustlsConnector,
    },
    /// A TLS session. Boxed: a rustls session is far larger than a socket,
    /// and an enum is as wide as its widest variant.
    Encrypted { stream: Box<TlsStream<TcpStream>> },
}

#[async_trait]
impl Stream for TokioStream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        let read = match self {
            Self::Plain { stream, .. } => stream.read(buf).await,
            Self::Encrypted { stream } => stream.read(buf).await,
        };
        match read {
            Ok(0) => Err(TransportError::Closed),
            Ok(count) => Ok(count),
            Err(error) => Err(map_io("reading from the server", error)),
        }
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        let written = match self {
            Self::Plain { stream, .. } => stream.write_all(bytes).await,
            Self::Encrypted { stream } => stream.write_all(bytes).await,
        };
        written.map_err(|error| map_io("writing to the server", error))
    }

    async fn upgrade_tls(self: Box<Self>, host: &str) -> Result<Box<dyn Stream>, TransportError> {
        match *self {
            Self::Plain { stream, connector } => {
                let stream = connector.handshake(stream, host).await?;
                Ok(Box::new(Self::Encrypted {
                    stream: Box::new(stream),
                }))
            }
            Self::Encrypted { .. } => Err(TransportError::Unsupported(
                "STARTTLS was requested on a connection that is already encrypted".to_owned(),
            )),
        }
    }

    fn is_encrypted(&self) -> bool {
        matches!(self, Self::Encrypted { .. })
    }
}

/// An I/O error as a [`TransportError`]: a closed peer is
/// [`TransportError::Closed`], a timeout [`TransportError::TimedOut`], anything
/// else [`TransportError::Io`] with `context` saying what was being done.
pub fn map_io(context: &str, error: io::Error) -> TransportError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return TransportError::Closed;
    }
    TransportError::Io {
        context: context.to_owned(),
        reason: error.to_string(),
    }
}
