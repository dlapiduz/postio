//! Sockets and TLS, kept out of the protocol.
//!
//! `io-imap` is sans-I/O: the session-opening coroutine *asks* for a TCP
//! connect, a TLS connect or an upgrade and never performs one. That is what
//! lets Postio own its own runtime and TLS stack — and, more usefully here,
//! what lets the whole handshake be driven over a canned transcript with no
//! socket at all. Every test in the default suite takes that path.
//!
//! Two implementations:
//!
//! * [`RustlsConnector`] — tokio sockets and `tokio-rustls`, verifying
//!   certificates against the platform trust store.
//! * [`ScriptedConnector`] — a recorded server transcript in memory.
//!
//! # There is no plaintext fallback
//!
//! A failed TLS handshake is [`TransportError::Tls`] and the connection ends.
//! Retrying in the clear is a decision no mail client gets to make on the
//! user's behalf, so the code to do it does not exist.

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use async_trait::async_trait;
use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use rustls_platform_verifier::ConfigVerifierExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::backend::BackendError;

/// How long to wait for a socket or a TLS handshake.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

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

    /// The transport cannot do what the protocol asked of it.
    #[error("{0}")]
    Unsupported(String),
}

impl From<TransportError> for BackendError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Tls { host, reason } => Self::Tls { host, reason },
            TransportError::Closed => Self::Disconnected {
                context: "the IMAP session".to_owned(),
                reason: "the server closed the connection".to_owned(),
            },
            TransportError::Connect { host, port, reason } => Self::Io {
                context: format!("connecting to {host}:{port}"),
                reason,
            },
            TransportError::Io { context, reason } => Self::Io { context, reason },
            TransportError::Unsupported(reason) => Self::Protocol { reason },
        }
    }
}

/// An open connection to a server.
///
/// Byte-level and deliberately dumb: it reads, it writes, and it can hand
/// itself to a TLS stack once. Everything about *what* those bytes mean lives
/// above it.
#[async_trait]
pub trait ImapStream: Send + fmt::Debug {
    /// Reads whatever is available. `Ok(0)` means the peer closed.
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;

    /// Writes every byte. A short write would desynchronize the exchange.
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError>;

    /// Wraps this connection in TLS, the `STARTTLS` half the protocol cannot
    /// perform itself.
    ///
    /// Consumes the plaintext stream, so there is no way to keep using it.
    async fn upgrade_tls(
        self: Box<Self>,
        host: &str,
    ) -> Result<Box<dyn ImapStream>, TransportError>;

    /// Whether the bytes on this connection are encrypted.
    fn is_encrypted(&self) -> bool;
}

/// Opens the connections the session coroutine asks for.
#[async_trait]
pub trait ImapConnector: Send + Sync + fmt::Debug {
    /// Opens a plaintext TCP connection.
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError>;

    /// Opens a TLS connection, verifying the certificate against `host`.
    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError>;

    /// Opens a local unix socket, for a pre-authenticated socket proxy.
    async fn connect_unix(&self, path: &str) -> Result<Box<dyn ImapStream>, TransportError> {
        Err(TransportError::Unsupported(format!(
            "this connector cannot open the unix socket at {path}"
        )))
    }
}

// ---------------------------------------------------------------------------
// tokio + rustls
// ---------------------------------------------------------------------------

/// Installs the ring crypto provider exactly once per process.
fn install_crypto_provider() {
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
        })
    }

    /// Sets how long a connect or handshake may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    async fn tcp(&self, host: &str, port: u16) -> Result<TcpStream, TransportError> {
        let attempt = tokio::time::timeout(self.timeout, TcpStream::connect((host, port)));
        match attempt.await {
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
        }
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
impl ImapConnector for RustlsConnector {
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
        let stream = self.tcp(host, port).await?;
        Ok(Box::new(TokioStream::Plain {
            stream,
            connector: self.clone(),
        }))
    }

    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
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
impl ImapStream for TokioStream {
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

    async fn upgrade_tls(
        self: Box<Self>,
        host: &str,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
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

fn map_io(context: &str, error: io::Error) -> TransportError {
    if error.kind() == io::ErrorKind::UnexpectedEof {
        return TransportError::Closed;
    }
    TransportError::Io {
        context: context.to_owned(),
        reason: error.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Scripted transcript
// ---------------------------------------------------------------------------

/// A canned server transcript.
///
/// Rules match on a substring of the command the client wrote, so a test does
/// not have to predict how many round trips a handshake takes or what tag the
/// generator picked. `{tag}` in a reply is replaced with the tag of the
/// command that matched, and a bare `\n` is rewritten to CRLF so transcripts
/// stay readable in source.
///
/// ```
/// # use postio_imap::imap::ImapScript;
/// let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
///     .on("AUTHENTICATE", "{tag} OK authenticated")
///     .on("CAPABILITY", "* CAPABILITY IMAP4rev1 CONDSTORE QRESYNC\n{tag} OK done");
/// # let _ = script;
/// ```
#[derive(Clone, Debug)]
pub struct ImapScript {
    greeting: String,
    rules: Vec<(String, String)>,
}

impl ImapScript {
    /// A transcript that opens with `greeting`.
    pub fn new(greeting: impl Into<String>) -> Self {
        Self {
            greeting: greeting.into(),
            rules: Vec::new(),
        }
    }

    /// A transcript shaped like iCloud's: a banner that hides everything, and
    /// the real capability list only after authentication.
    ///
    /// This is the case ADR 0001 Q3 exists for. Gate anything on the banner
    /// and CONDSTORE, QRESYNC, IDLE and UIDPLUS all silently vanish.
    pub fn icloud() -> Self {
        Self::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=LOGIN] iCloud ready")
            .on("AUTHENTICATE", "{tag} OK AUTHENTICATE completed")
            .on(
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=LOGIN ENABLE CONDSTORE \
                 QRESYNC IDLE UIDPLUS MOVE NAMESPACE UNSELECT ID X-APPLEPUSHSERVICE\n\
                 {tag} OK CAPABILITY completed",
            )
    }

    /// Replies with `reply` to any command containing `keyword`.
    ///
    /// Rules are tried in order, so a narrow one goes before a broad one.
    pub fn on(mut self, keyword: impl Into<String>, reply: impl Into<String>) -> Self {
        self.rules.push((keyword.into(), reply.into()));
        self
    }

    fn reply_to(&self, command: &str) -> String {
        let tag = command.split_whitespace().next().unwrap_or("*");
        let reply = self
            .rules
            .iter()
            .find(|(keyword, _)| {
                command
                    .to_ascii_uppercase()
                    .contains(&keyword.to_ascii_uppercase())
            })
            .map(|(_, reply)| reply.clone())
            .unwrap_or_else(|| "{tag} BAD the transcript has no reply for this command".to_owned());

        crlf(&reply.replace("{tag}", tag))
    }
}

/// Rewrites bare LF as CRLF so transcripts can be written readably.
fn crlf(text: &str) -> String {
    let mut out = text.replace("\r\n", "\n").replace('\n', "\r\n");
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out
}

/// What a [`ScriptedConnector`] was asked to do.
///
/// The point of recording connects separately from upgrades is the "never
/// silently downgraded" rule: a test asserts that a failed TLS connect was
/// followed by *no* plaintext attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionLog {
    /// Plaintext connects attempted, as `(host, port)`.
    pub tcp: Vec<(String, u16)>,
    /// TLS connects attempted, as `(host, port)`.
    pub tls: Vec<(String, u16)>,
    /// Hosts a `STARTTLS` upgrade was performed against.
    pub upgrades: Vec<String>,
    /// Every byte the client wrote, in order.
    pub written: Vec<u8>,
}

impl ConnectionLog {
    /// The commands the client sent, one per line, without the CRLFs.
    ///
    /// Useful for asserting that a command was issued — and, more often, that
    /// one was not.
    pub fn commands(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.written)
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// A connector that replays a transcript instead of opening a socket.
///
/// Public on purpose: crates above this one test their connection handling
/// against it too, and CLAUDE.md forbids a default-suite test from touching
/// the network.
#[derive(Clone, Debug)]
pub struct ScriptedConnector {
    script: ImapScript,
    log: Arc<Mutex<ConnectionLog>>,
    tls_failure: Option<String>,
    tcp_failure: Option<String>,
}

impl ScriptedConnector {
    /// A connector replaying `script`.
    pub fn new(script: ImapScript) -> Self {
        Self {
            script,
            log: Arc::new(Mutex::new(ConnectionLog::default())),
            tls_failure: None,
            tcp_failure: None,
        }
    }

    /// A connector replaying [`ImapScript::icloud`].
    pub fn icloud() -> Self {
        Self::new(ImapScript::icloud())
    }

    /// Makes every TLS connect and upgrade fail with `reason`.
    pub fn failing_tls(mut self, reason: impl Into<String>) -> Self {
        self.tls_failure = Some(reason.into());
        self
    }

    /// Makes every plaintext connect fail with `reason`.
    pub fn failing_tcp(mut self, reason: impl Into<String>) -> Self {
        self.tcp_failure = Some(reason.into());
        self
    }

    /// What the connector was asked to do, so far.
    pub fn log(&self) -> ConnectionLog {
        self.log.lock().expect("connection log").clone()
    }
}

#[async_trait]
impl ImapConnector for ScriptedConnector {
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
        self.log
            .lock()
            .expect("connection log")
            .tcp
            .push((host.to_owned(), port));

        if let Some(reason) = &self.tcp_failure {
            return Err(TransportError::Connect {
                host: host.to_owned(),
                port,
                reason: reason.clone(),
            });
        }

        Ok(Box::new(ScriptedStream::new(
            self.script.clone(),
            Arc::clone(&self.log),
            self.tls_failure.clone(),
            false,
        )))
    }

    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
        self.log
            .lock()
            .expect("connection log")
            .tls
            .push((host.to_owned(), port));

        if let Some(reason) = &self.tls_failure {
            return Err(TransportError::Tls {
                host: host.to_owned(),
                reason: reason.clone(),
            });
        }

        Ok(Box::new(ScriptedStream::new(
            self.script.clone(),
            Arc::clone(&self.log),
            None,
            true,
        )))
    }
}

/// One replayed connection.
#[derive(Debug)]
struct ScriptedStream {
    script: ImapScript,
    log: Arc<Mutex<ConnectionLog>>,
    pending: VecDeque<u8>,
    tls_failure: Option<String>,
    encrypted: bool,
}

impl ScriptedStream {
    fn new(
        script: ImapScript,
        log: Arc<Mutex<ConnectionLog>>,
        tls_failure: Option<String>,
        encrypted: bool,
    ) -> Self {
        let greeting = crlf(&script.greeting);
        Self {
            script,
            log,
            pending: greeting.into_bytes().into(),
            tls_failure,
            encrypted,
        }
    }
}

#[async_trait]
impl ImapStream for ScriptedStream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        if self.pending.is_empty() {
            return Err(TransportError::Closed);
        }
        let count = buf.len().min(self.pending.len());
        for slot in buf.iter_mut().take(count) {
            *slot = self.pending.pop_front().expect("pending byte");
        }
        Ok(count)
    }

    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.log
            .lock()
            .expect("connection log")
            .written
            .extend_from_slice(bytes);

        // A command may arrive in several writes; reply once the line is
        // terminated, which is where the server would.
        let text = String::from_utf8_lossy(bytes);
        for line in text.split("\r\n").filter(|line| !line.is_empty()) {
            self.pending.extend(self.script.reply_to(line).into_bytes());
        }
        Ok(())
    }

    async fn upgrade_tls(
        self: Box<Self>,
        host: &str,
    ) -> Result<Box<dyn ImapStream>, TransportError> {
        self.log
            .lock()
            .expect("connection log")
            .upgrades
            .push(host.to_owned());

        if let Some(reason) = &self.tls_failure {
            return Err(TransportError::Tls {
                host: host.to_owned(),
                reason: reason.clone(),
            });
        }

        let mut upgraded = self;
        upgraded.encrypted = true;
        Ok(upgraded)
    }

    fn is_encrypted(&self) -> bool {
        self.encrypted
    }
}
