//! Sockets and TLS, kept out of the protocols: `io-imap` and `io-smtp` are sans-I/O, so a handshake
//! can be driven over a canned transcript with no socket. The half both share: the error, the two
//! traits a session drives, and the real tokio and `tokio-rustls` implementation.
//! No plaintext fallback: a failed TLS handshake is [`TransportError::Tls`] and the connection
//! ends.

mod deflate;

pub use deflate::{DeflateCodec, DeflateStream};

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

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

const WRITE_CHUNK: usize = 16 * 1024;

/// Everything the transport layer can fail with.
///
/// No variant carries a credential: the handshake bytes never reach an error
/// message, only the reason a socket or a certificate did not work out.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("could not reach {host}:{port}: {reason}")]
    Connect {
        host: String,
        port: u16,
        reason: String,
    },

    #[error("TLS failed for {host}: {reason}")]
    Tls { host: String, reason: String },

    #[error("the server closed the connection")]
    Closed,

    #[error("{context} failed: {reason}")]
    Io { context: String, reason: String },

    #[error("{context} timed out after {after:?}")]
    TimedOut { context: String, after: Duration },

    #[error("{0}")]
    Unsupported(String),
}

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

    /// Writes every byte, giving up if the peer takes nothing for `timeout`.
    ///
    /// The write-side twin of [`read_within`](Self::read_within), and bounded
    /// the same way: on silence, not on duration. A peer that has stopped
    /// reading fills every buffer between us and it and blocks `write_all`
    /// for ever; a large upload over a slow link makes steady progress. The
    /// bytes go in chunks, each of which must be accepted within `timeout`,
    /// so only the first kind is caught. `Duration::ZERO` waits forever.
    async fn write_within(
        &mut self,
        bytes: &[u8],
        timeout: Duration,
        context: &str,
    ) -> Result<(), TransportError> {
        if timeout.is_zero() {
            return self.write_all(bytes).await;
        }
        for chunk in bytes.chunks(WRITE_CHUNK) {
            match tokio::time::timeout(timeout, self.write_all(chunk)).await {
                Ok(written) => written?,
                Err(_) => {
                    return Err(TransportError::TimedOut {
                        context: context.to_owned(),
                        after: timeout,
                    });
                }
            }
        }
        Ok(())
    }

    /// Wraps this connection in TLS, the `STARTTLS` half the protocol cannot
    /// perform itself.
    ///
    /// Consumes the plaintext stream, so there is no way to keep using it.
    async fn upgrade_tls(self: Box<Self>, host: &str) -> Result<Box<dyn Stream>, TransportError>;

    fn is_encrypted(&self) -> bool;
}

#[async_trait]
pub trait Connector: Send + Sync + fmt::Debug {
    async fn connect_tcp(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError>;

    async fn connect_tls(&self, host: &str, port: u16) -> Result<Box<dyn Stream>, TransportError>;

    async fn connect_unix(&self, path: &str) -> Result<Box<dyn Stream>, TransportError> {
        Err(TransportError::Unsupported(format!(
            "this connector cannot open the unix socket at {path}"
        )))
    }
}

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

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Success and failure alike: a log that only showed successes would hide exactly the traffic a
    /// user most wants to see. `subsystem` is the protocol the log files them under: one connector
    /// type serves both IMAP and SMTP.
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

#[derive(Debug)]
enum TokioStream {
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

pub fn map_io(context: &str, error: io::Error) -> TransportError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return TransportError::Closed;
    }
    TransportError::Io {
        context: context.to_owned(),
        reason: error.to_string(),
    }
}
