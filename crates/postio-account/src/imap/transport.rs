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
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::backend::BackendError;

// The half IMAP and SMTP share -- the error, the two traits a session
// drives, and the tokio + rustls implementation -- lives in
// `postio-transport`, under the names this crate has always used for them.
pub use postio_transport::Connector as ImapConnector;
pub use postio_transport::Stream as ImapStream;
pub use postio_transport::{RustlsConnector, TransportError};

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
            TransportError::TimedOut { context, after } => Self::TimedOut { context, after },
            TransportError::Unsupported(reason) => Self::Protocol { reason },
        }
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
/// # use postio_account::imap::ImapScript;
/// let script = ImapScript::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN] ready")
///     .on("AUTHENTICATE", "{tag} OK authenticated")
///     .on("CAPABILITY", "* CAPABILITY IMAP4rev1 CONDSTORE QRESYNC\n{tag} OK done");
/// # let _ = script;
/// ```
#[derive(Clone, Debug)]
pub struct ImapScript {
    greeting: String,
    rules: Vec<(String, Reply)>,
}

/// What a matched rule sends back.
#[derive(Clone, Debug)]
enum Reply {
    /// Held in memory and sent verbatim, `{tag}` substituted.
    Fixed(String),
    /// A literal body synthesized a chunk at a time as it is read, rather
    /// than held in memory — see [`ImapScript::on_generated`].
    Generated {
        header: String,
        len: u32,
        trailer: String,
    },
}

impl ImapScript {
    /// A transcript that opens with `greeting`.
    pub fn new(greeting: impl Into<String>) -> Self {
        Self {
            greeting: greeting.into(),
            rules: Vec::new(),
        }
    }

    /// A server whose banner hides every extension until you log in.
    ///
    /// The case ADR 0001 Q3 exists for, and the behaviour of at least one
    /// mainstream provider: gate anything on the banner and CONDSTORE,
    /// QRESYNC, IDLE and UIDPLUS all silently vanish.
    pub fn extensions_hidden_until_login() -> Self {
        Self::new("* OK [CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=LOGIN] ready")
            .on("AUTHENTICATE", "{tag} OK AUTHENTICATE completed")
            .on(
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 SASL-IR AUTH=PLAIN AUTH=LOGIN ENABLE CONDSTORE \
                 QRESYNC IDLE UIDPLUS MOVE NAMESPACE UNSELECT ID X-VENDOR-PUSH\n\
                 {tag} OK CAPABILITY completed",
            )
    }

    /// Replies with `reply` to any command containing `keyword`.
    ///
    /// Rules are tried in order, so a narrow one goes before a broad one.
    pub fn on(mut self, keyword: impl Into<String>, reply: impl Into<String>) -> Self {
        self.rules
            .push((keyword.into(), Reply::Fixed(reply.into())));
        self
    }

    /// Replies to `keyword` with a literal body of `len` synthesized bytes,
    /// generated a chunk at a time as the stream is read rather than
    /// held in memory — for a test that must prove a fetch streams a large
    /// response instead of buffering it, where an ordinary [`Self::on`]
    /// reply of the same size would itself dominate the measurement.
    ///
    /// `prefix` is the response line up to and including the `{` that opens
    /// the literal announcement, e.g. `"* 1 FETCH (BODY[] "` — the `len}`
    /// and its terminating CRLF are added for you. `trailer` closes the
    /// response after the literal, e.g. `")\n{tag} OK FETCH completed"`.
    /// Both accept `{tag}`.
    pub fn on_generated(
        mut self,
        keyword: impl Into<String>,
        prefix: impl Into<String>,
        len: u32,
        trailer: impl Into<String>,
    ) -> Self {
        let header = format!("{}{len}}}", prefix.into());
        self.rules.push((
            keyword.into(),
            Reply::Generated {
                header,
                len,
                trailer: trailer.into(),
            },
        ));
        self
    }

    fn reply_to(&self, command: &str) -> Reply {
        let tag = command.split_whitespace().next().unwrap_or("*");
        match self
            .rules
            .iter()
            .find(|(keyword, _)| {
                command
                    .to_ascii_uppercase()
                    .contains(&keyword.to_ascii_uppercase())
            })
            .map(|(_, reply)| reply.clone())
        {
            Some(Reply::Fixed(reply)) => Reply::Fixed(crlf(&reply.replace("{tag}", tag))),
            Some(Reply::Generated {
                header,
                len,
                trailer,
            }) => Reply::Generated {
                header: crlf(&header.replace("{tag}", tag)),
                len,
                trailer: crlf(&trailer.replace("{tag}", tag)),
            },
            None => Reply::Fixed(crlf(&format!(
                "{tag} BAD the transcript has no reply for this command"
            ))),
        }
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
    close_after: Option<usize>,
}

impl ScriptedConnector {
    /// A connector replaying `script`.
    pub fn new(script: ImapScript) -> Self {
        Self {
            script,
            log: Arc::new(Mutex::new(ConnectionLog::default())),
            tls_failure: None,
            tcp_failure: None,
            close_after: None,
        }
    }

    /// A connector replaying [`ImapScript::extensions_hidden_until_login`].
    pub fn extensions_hidden_until_login() -> Self {
        Self::new(ImapScript::extensions_hidden_until_login())
    }

    /// Makes every TLS connect and upgrade fail with `reason`.
    pub fn failing_tls(mut self, reason: impl Into<String>) -> Self {
        self.tls_failure = Some(reason.into());
        self
    }

    /// Makes every connection go silent after it has served `commands`.
    ///
    /// The server stops answering, so the next read sees EOF — a connection
    /// the far end dropped while the client still believed in it, which is
    /// what an idle timeout or a server restart looks like from here.
    pub fn closing_after(mut self, commands: usize) -> Self {
        self.close_after = Some(commands);
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
            self.close_after,
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
            self.close_after,
        )))
    }
}

/// One queued piece of what [`ScriptedStream`] has left to serve.
///
/// Most replies are small enough to hold outright, but [`Reply::Generated`]
/// exists precisely so a large one is not: [`Segment::Generated`] carries
/// only a remaining byte count, and `read` synthesizes bytes on demand
/// instead of ever materializing them all.
#[derive(Debug)]
enum Segment {
    Bytes(VecDeque<u8>),
    Generated(u32),
}

/// Bytes a [`Segment::Generated`] run is filled with. Content is arbitrary —
/// nothing in this crate's tests reads it back — only the byte count matters.
const GENERATED_PATTERN: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// One replayed connection.
#[derive(Debug)]
struct ScriptedStream {
    script: ImapScript,
    log: Arc<Mutex<ConnectionLog>>,
    segments: VecDeque<Segment>,
    tls_failure: Option<String>,
    encrypted: bool,
    /// Commands still to be answered before the server goes silent.
    budget: Option<usize>,
}

impl ScriptedStream {
    fn new(
        script: ImapScript,
        log: Arc<Mutex<ConnectionLog>>,
        tls_failure: Option<String>,
        encrypted: bool,
        budget: Option<usize>,
    ) -> Self {
        let greeting = crlf(&script.greeting);
        let mut segments = VecDeque::new();
        segments.push_back(Segment::Bytes(greeting.into_bytes().into()));
        Self {
            script,
            log,
            segments,
            tls_failure,
            encrypted,
            budget,
        }
    }

    fn queue_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Fixed(text) => self
                .segments
                .push_back(Segment::Bytes(text.into_bytes().into())),
            Reply::Generated {
                header,
                len,
                trailer,
            } => {
                self.segments
                    .push_back(Segment::Bytes(header.into_bytes().into()));
                self.segments.push_back(Segment::Generated(len));
                self.segments
                    .push_back(Segment::Bytes(trailer.into_bytes().into()));
            }
        }
    }
}

#[async_trait]
impl ImapStream for ScriptedStream {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        loop {
            match self.segments.front_mut() {
                None => return Err(TransportError::Closed),
                Some(Segment::Bytes(bytes)) if bytes.is_empty() => {
                    self.segments.pop_front();
                }
                Some(Segment::Bytes(bytes)) => {
                    let count = buf.len().min(bytes.len());
                    for slot in buf.iter_mut().take(count) {
                        *slot = bytes.pop_front().expect("pending byte");
                    }
                    return Ok(count);
                }
                Some(Segment::Generated(0)) => {
                    self.segments.pop_front();
                }
                Some(Segment::Generated(remaining)) => {
                    let count = buf.len().min(*remaining as usize);
                    for (slot, byte) in buf
                        .iter_mut()
                        .zip(GENERATED_PATTERN.iter().cycle())
                        .take(count)
                    {
                        *slot = *byte;
                    }
                    *remaining -= count as u32;
                    return Ok(count);
                }
            }
        }
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
            match &mut self.budget {
                Some(0) => return Ok(()),
                Some(remaining) => *remaining -= 1,
                None => {}
            }
            let reply = self.script.reply_to(line);
            self.queue_reply(reply);
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
