//! Sockets and TLS, kept out of the protocol.
//!
//! `io-smtp` is sans-I/O: the session-opening coroutine *asks* for a TCP
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

use crate::error::SmtpError;

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

    /// The peer took too long to answer.
    ///
    /// Distinct from [`Io`](Self::Io) because the send path has to tell a
    /// server that refused from one that stopped talking: a timeout waiting
    /// for the reply to the terminating `.` is the case where the client
    /// cannot know whether the message was accepted (ADR 0021).
    #[error("{context} timed out after {}s", after.as_secs_f32())]
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

impl From<TransportError> for SmtpError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Tls { host, reason } => Self::Tls { host, reason },
            TransportError::Closed => Self::Disconnected {
                context: "the SMTP session".to_owned(),
                reason: "the server closed the connection".to_owned(),
            },
            TransportError::Connect { host, port, reason } => Self::Io {
                context: format!("connecting to {host}:{port}"),
                reason,
            },
            TransportError::Io { context, reason } => Self::Io { context, reason },
            TransportError::TimedOut { context, after } => Self::TimedOut { context, after },
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
pub trait SmtpStream: Send + fmt::Debug {
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
    ) -> Result<Box<dyn SmtpStream>, TransportError>;

    /// Whether the bytes on this connection are encrypted.
    fn is_encrypted(&self) -> bool;
}

/// Opens the connections the session coroutine asks for.
#[async_trait]
pub trait SmtpConnector: Send + Sync + fmt::Debug {
    /// Opens a plaintext TCP connection.
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError>;

    /// Opens a TLS connection, verifying the certificate against `host`.
    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError>;

    /// Opens a local unix socket, for a pre-authenticated socket proxy.
    async fn connect_unix(&self, path: &str) -> Result<Box<dyn SmtpStream>, TransportError> {
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
#[derive(Clone)]
pub struct RustlsConnector {
    connector: TlsConnector,
    timeout: Duration,
    /// Where every connection attempt is reported (#151), or nowhere: a
    /// connector a test builds without a sink still connects, it just
    /// proves nothing.
    egress: Option<Arc<dyn postio_model::egress::EgressSink>>,
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
        })
    }

    /// Sets how long a connect or handshake may take.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Report every connection attempt to `sink` — the egress log's seam
    /// (#151). Success and failure alike, same as the IMAP transport.
    pub fn with_egress(mut self, sink: Arc<dyn postio_model::egress::EgressSink>) -> Self {
        self.egress = Some(sink);
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
        // At the TCP stage, before any handshake — "a connection was opened
        // to this host" is the privacy-relevant fact.
        if let Some(egress) = &self.egress {
            egress.record(postio_model::egress::EgressEvent {
                at: chrono::Utc::now(),
                subsystem: postio_model::egress::EgressSubsystem::Smtp,
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
impl SmtpConnector for RustlsConnector {
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
impl SmtpStream for TokioStream {
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
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
/// not have to predict how many round trips a handshake takes or what the
/// server text says. Rules are tried in order.
///
/// ```
/// # use postio_smtp::transport::SmtpScript;
/// let script = SmtpScript::new("220 mail.example.com ESMTP ready")
///     .on("EHLO", "250-mail.example.com\r\n250 AUTH PLAIN")
///     .on("AUTH PLAIN", "235 authenticated");
/// # let _ = script;
/// ```
#[derive(Clone, Debug)]
pub struct SmtpScript {
    greeting: String,
    rules: Vec<(String, String)>,
    data_body_reply: String,
}

impl SmtpScript {
    /// A transcript that opens with `greeting`.
    pub fn new(greeting: impl Into<String>) -> Self {
        Self {
            greeting: greeting.into(),
            rules: Vec::new(),
            data_body_reply: "250 2.0.0 message accepted".to_owned(),
        }
    }

    /// Replies with `reply` to any command containing `keyword`.
    ///
    /// Rules are tried in order, so a narrow one goes before a broad one.
    /// Not for the `DATA` body itself — see [`Self::on_data_body`].
    pub fn on(mut self, keyword: impl Into<String>, reply: impl Into<String>) -> Self {
        self.rules.push((keyword.into(), reply.into()));
        self
    }

    /// Sets the reply to the message body that follows `DATA`'s `354`.
    ///
    /// Matched structurally by [`ScriptedStream`] — the write ends with the
    /// dot-stuffing terminator — rather than by keyword, because the body's
    /// content is the test's own message and cannot be predicted here.
    /// Defaults to a plain `250` accept.
    pub fn on_data_body(mut self, reply: impl Into<String>) -> Self {
        self.data_body_reply = reply.into();
        self
    }

    fn reply_to(&self, command: &str) -> String {
        let reply = self
            .rules
            .iter()
            .find(|(keyword, _)| {
                command
                    .to_ascii_uppercase()
                    .contains(&keyword.to_ascii_uppercase())
            })
            .map(|(_, reply)| reply.clone())
            .unwrap_or_else(|| "500 the transcript has no reply for this command".to_owned());

        crlf(&reply)
    }

    fn reply_to_data_body(&self) -> String {
        crlf(&self.data_body_reply)
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
    /// Useful for asserting that a command was issued — and, more often,
    /// that one was not.
    pub fn commands(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.written)
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// How a scripted connection stops answering partway through a transaction.
///
/// The two cases ADR 0021 turns on, and neither was expressible before #673.
/// Which one a test picks decides whether retrying the send is safe, so they
/// are deliberately separate constructors rather than one with a flag.
#[derive(Clone, Debug)]
enum Vanish {
    /// Reset the connection as the client writes the command containing this
    /// keyword, before the server has said anything about it. Nothing of the
    /// message is on the wire, so a retry is safe.
    OnWrite(String),
    /// Take the message payload and then never answer it — the one window
    /// where "it never arrived" and "it arrived and the acknowledgement did
    /// not" are indistinguishable from the client.
    AfterPayload(AfterPayload),
}

/// What the silence after an accepted payload looks like.
#[derive(Clone, Copy, Debug)]
enum AfterPayload {
    /// The connection closes.
    Closed,
    /// The read waiting for the reply to the terminating `.` times out.
    TimedOut(Duration),
}

/// A connector that replays a transcript instead of opening a socket.
///
/// Public on purpose: crates above this one test their connection handling
/// against it too, and CLAUDE.md forbids a default-suite test from touching
/// the network.
#[derive(Clone, Debug)]
pub struct ScriptedConnector {
    script: SmtpScript,
    log: Arc<Mutex<ConnectionLog>>,
    tls_failure: Option<String>,
    tcp_failure: Option<String>,
    vanish: Option<Vanish>,
}

impl ScriptedConnector {
    /// A connector replaying `script`.
    pub fn new(script: SmtpScript) -> Self {
        Self {
            script,
            log: Arc::new(Mutex::new(ConnectionLog::default())),
            tls_failure: None,
            tcp_failure: None,
            vanish: None,
        }
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

    /// Makes the connection reset as the client writes the command
    /// containing `keyword`, before any reply to it.
    ///
    /// The safe half of ADR 0021's distinction: the server never saw the
    /// message, so whatever failed can be tried again. `vanishing_at("MAIL
    /// FROM")` is the canonical case.
    ///
    /// Matched against the command text, so it never fires on the message
    /// payload — that one is [`vanishing_after_the_payload`
    /// ](Self::vanishing_after_the_payload), and the two must not be
    /// confusable.
    pub fn vanishing_at(mut self, keyword: impl Into<String>) -> Self {
        self.vanish = Some(Vanish::OnWrite(keyword.into()));
        self
    }

    /// Makes the connection accept the message payload and then close
    /// without replying to the terminating `.`.
    ///
    /// The dangerous half: the bytes reached the server, or did not, and
    /// nothing the client can observe tells it which. A send that fails this
    /// way must be reported rather than retried (ADR 0021, #461).
    pub fn vanishing_after_the_payload(mut self) -> Self {
        self.vanish = Some(Vanish::AfterPayload(AfterPayload::Closed));
        self
    }

    /// As [`vanishing_after_the_payload`](Self::vanishing_after_the_payload),
    /// but the read times out rather than the connection closing.
    ///
    /// A separate constructor because a timeout and a close arrive as
    /// different errors and the send path has to reach the same conclusion
    /// from both.
    pub fn timing_out_after_the_payload(mut self, after: Duration) -> Self {
        self.vanish = Some(Vanish::AfterPayload(AfterPayload::TimedOut(after)));
        self
    }

    /// What the connector was asked to do, so far.
    pub fn log(&self) -> ConnectionLog {
        self.log.lock().expect("connection log").clone()
    }
}

#[async_trait]
impl SmtpConnector for ScriptedConnector {
    async fn connect_tcp(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
            self.vanish.clone(),
        )))
    }

    async fn connect_tls(
        &self,
        host: &str,
        port: u16,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
            self.vanish.clone(),
        )))
    }
}

/// One replayed connection.
#[derive(Debug)]
struct ScriptedStream {
    script: SmtpScript,
    log: Arc<Mutex<ConnectionLog>>,
    pending: VecDeque<u8>,
    tls_failure: Option<String>,
    encrypted: bool,
    vanish: Option<Vanish>,
    /// Set once the payload has been written and the connection is due to
    /// stop answering, so the *read* that follows is what fails.
    swallowed_payload: Option<AfterPayload>,
}

impl ScriptedStream {
    fn new(
        script: SmtpScript,
        log: Arc<Mutex<ConnectionLog>>,
        tls_failure: Option<String>,
        encrypted: bool,
        vanish: Option<Vanish>,
    ) -> Self {
        let greeting = crlf(&script.greeting);
        Self {
            script,
            log,
            pending: greeting.into_bytes().into(),
            tls_failure,
            encrypted,
            vanish,
            swallowed_payload: None,
        }
    }
}

#[async_trait]
impl SmtpStream for ScriptedStream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        // The payload went out and the server is never going to answer it.
        // The failure lands here rather than on the write on purpose: that is
        // what makes the outcome unknowable, because the bytes did leave.
        if let Some(after) = self.swallowed_payload {
            return Err(match after {
                AfterPayload::Closed => TransportError::Closed,
                AfterPayload::TimedOut(after) => TransportError::TimedOut {
                    context: "waiting for the reply to the message".to_owned(),
                    after,
                },
            });
        }
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
        // Structurally, not by keyword: the payload is the test's own message
        // and could contain any word at all, so matching it on content would
        // make `vanishing_at` fire on somebody's mail.
        let is_payload = bytes.ends_with(b"\r\n.\r\n");

        if let Some(Vanish::OnWrite(keyword)) = &self.vanish
            && !is_payload
            && String::from_utf8_lossy(bytes)
                .to_ascii_uppercase()
                .contains(&keyword.to_ascii_uppercase())
        {
            // Nothing is logged: the command never left, which is exactly the
            // fact a test asserting "the message was not submitted" needs.
            return Err(TransportError::Closed);
        }

        self.log
            .lock()
            .expect("connection log")
            .written
            .extend_from_slice(bytes);

        if let Some(Vanish::AfterPayload(after)) = &self.vanish
            && is_payload
        {
            // The write succeeds and is logged -- the bytes are on the wire.
            // Everything after this read fails.
            self.swallowed_payload = Some(*after);
            return Ok(());
        }

        // Every SMTP write from this crate's coroutines is one logical unit
        // awaiting exactly one reply: a single-line command, or — for
        // `DATA` — the dot-stuffed body in one shot, however many `\r\n` it
        // contains internally. So one write gets one reply, matched
        // structurally for the body (its content is the test's own message
        // and cannot be a keyword) and by keyword otherwise.
        let reply = if bytes.ends_with(b"\r\n.\r\n") {
            self.script.reply_to_data_body()
        } else {
            let text = String::from_utf8_lossy(bytes);
            self.script.reply_to(text.trim_end_matches("\r\n"))
        };
        self.pending.extend(reply.into_bytes());
        Ok(())
    }

    async fn upgrade_tls(
        self: Box<Self>,
        host: &str,
    ) -> Result<Box<dyn SmtpStream>, TransportError> {
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
