//! `io-imap` is sans-I/O: the session coroutine asks for connects and never performs one, so the
//! handshake runs over a canned transcript with no socket (`ScriptedConnector`).
//! No plaintext fallback: a failed TLS handshake is `TransportError::Tls` and the connection ends;
//! no mail client may retry in the clear on the user's behalf.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::backend::BackendError;

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

/// Rules match on a substring of the command, so a test need not predict round trips or tags.
/// `{tag}` in a reply is replaced by the matched command's tag; bare `\n` becomes CRLF.
#[derive(Clone, Debug)]
pub struct ImapScript {
    greeting: String,
    rules: Vec<(String, Reply)>,
}

#[derive(Clone, Debug)]
enum Reply {
    Fixed(String),
    Generated {
        header: String,
        len: u32,
        trailer: String,
    },
}

impl ImapScript {
    pub fn new(greeting: impl Into<String>) -> Self {
        Self {
            greeting: greeting.into(),
            rules: Vec::new(),
        }
    }

    /// ADR 0001 Q3: gate anything on the banner and CONDSTORE, QRESYNC, IDLE and UIDPLUS silently
    /// vanish.
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

    /// Rules are tried in order, so a narrow one goes before a broad one.
    pub fn on(mut self, keyword: impl Into<String>, reply: impl Into<String>) -> Self {
        self.rules
            .push((keyword.into(), Reply::Fixed(reply.into())));
        self
    }

    /// Generated a chunk at a time, not held in memory, for a test that must prove a fetch streams
    /// a large response. `prefix` ends at the `{` opening the literal announcement; `len}` and CRLF
    /// are added. `trailer` closes the response. Both accept `{tag}`.
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

fn crlf(text: &str) -> String {
    let mut out = text.replace("\r\n", "\n").replace('\n', "\r\n");
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out
}

/// Connects are recorded apart from upgrades so a test can assert that a failed TLS connect was
/// followed by no plaintext attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectionLog {
    pub tcp: Vec<(String, u16)>,
    pub tls: Vec<(String, u16)>,
    pub upgrades: Vec<String>,
    pub written: Vec<u8>,
}

impl ConnectionLog {
    pub fn commands(&self) -> Vec<String> {
        String::from_utf8_lossy(&self.written)
            .split("\r\n")
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect()
    }
}

/// Public so crates above test their connection handling against it; no default-suite test touches
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
    pub fn new(script: ImapScript) -> Self {
        Self {
            script,
            log: Arc::new(Mutex::new(ConnectionLog::default())),
            tls_failure: None,
            tcp_failure: None,
            close_after: None,
        }
    }

    pub fn extensions_hidden_until_login() -> Self {
        Self::new(ImapScript::extensions_hidden_until_login())
    }

    pub fn failing_tls(mut self, reason: impl Into<String>) -> Self {
        self.tls_failure = Some(reason.into());
        self
    }

    /// The next read sees EOF: what an idle timeout or server restart looks like from here.
    pub fn closing_after(mut self, commands: usize) -> Self {
        self.close_after = Some(commands);
        self
    }

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

/// `Segment::Generated` carries only a remaining byte count and synthesizes bytes on read, so a
/// large reply is never materialized.
#[derive(Debug)]
enum Segment {
    Bytes(VecDeque<u8>),
    Generated(u32),
}

const GENERATED_PATTERN: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";

#[derive(Debug)]
struct ScriptedStream {
    script: ImapScript,
    log: Arc<Mutex<ConnectionLog>>,
    segments: VecDeque<Segment>,
    tls_failure: Option<String>,
    encrypted: bool,
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
