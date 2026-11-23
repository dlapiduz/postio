//! Sockets and TLS, kept out of the protocol: `io-smtp` is sans-I/O, so the whole handshake can be
//! driven over a canned [`ScriptedConnector`] transcript with no socket (every default-suite test
//! does).
//! No plaintext fallback: a failed TLS handshake is [`TransportError::Tls`] and the connection
//! ends.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use crate::error::SmtpError;

pub use postio_transport::Connector as SmtpConnector;
pub use postio_transport::Stream as SmtpStream;
pub use postio_transport::{RustlsConnector, TransportError};

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

/// Rules match on a substring of the command the client wrote, tried in order, so a test need not
/// predict how many round trips a handshake takes.
#[derive(Clone, Debug)]
pub struct SmtpScript {
    greeting: String,
    rules: Vec<(String, String)>,
    data_body_reply: String,
}

impl SmtpScript {
    pub fn new(greeting: impl Into<String>) -> Self {
        Self {
            greeting: greeting.into(),
            rules: Vec::new(),
            data_body_reply: "250 2.0.0 message accepted".to_owned(),
        }
    }

    /// Rules are tried in order, so a narrow one goes before a broad one. Not for the `DATA` body:
    /// see [`Self::on_data_body`].
    pub fn on(mut self, keyword: impl Into<String>, reply: impl Into<String>) -> Self {
        self.rules.push((keyword.into(), reply.into()));
        self
    }

    /// Matched structurally (the write ends with the dot-stuffing terminator), because the body is
    /// the test's own message. Defaults to a plain `250` accept.
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

fn crlf(text: &str) -> String {
    let mut out = text.replace("\r\n", "\n").replace('\n', "\r\n");
    if !out.ends_with("\r\n") {
        out.push_str("\r\n");
    }
    out
}

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

/// The two cases ADR 0021 turns on; which one a test picks decides whether retrying the send is
/// safe.
#[derive(Clone, Debug)]
enum Vanish {
    /// Reset before the server has said anything: nothing of the message is on the wire, so a retry
    /// is safe.
    OnWrite(String),
    /// Take the payload and never answer: "never arrived" and "arrived, unacknowledged" are
    /// indistinguishable.
    AfterPayload(AfterPayload),
}

#[derive(Clone, Copy, Debug)]
enum AfterPayload {
    Closed,
    TimedOut(Duration),
}

/// Public because crates above this one test their connection handling against it too.
#[derive(Clone, Debug)]
pub struct ScriptedConnector {
    script: SmtpScript,
    log: Arc<Mutex<ConnectionLog>>,
    tls_failure: Option<String>,
    tcp_failure: Option<String>,
    vanish: Option<Vanish>,
}

impl ScriptedConnector {
    pub fn new(script: SmtpScript) -> Self {
        Self {
            script,
            log: Arc::new(Mutex::new(ConnectionLog::default())),
            tls_failure: None,
            tcp_failure: None,
            vanish: None,
        }
    }

    pub fn failing_tls(mut self, reason: impl Into<String>) -> Self {
        self.tls_failure = Some(reason.into());
        self
    }

    /// The safe half of ADR 0021's distinction: the server never saw the message. Matched against
    /// the command text, never the payload.
    pub fn vanishing_at(mut self, keyword: impl Into<String>) -> Self {
        self.vanish = Some(Vanish::OnWrite(keyword.into()));
        self
    }

    /// The dangerous half: nothing the client can observe says whether the bytes arrived, so a send
    /// that fails this way must be reported rather than retried (ADR 0021, #461).
    pub fn vanishing_after_the_payload(mut self) -> Self {
        self.vanish = Some(Vanish::AfterPayload(AfterPayload::Closed));
        self
    }

    /// A timeout and a close arrive as different errors; the send path must reach the same
    /// conclusion from both.
    pub fn timing_out_after_the_payload(mut self, after: Duration) -> Self {
        self.vanish = Some(Vanish::AfterPayload(AfterPayload::TimedOut(after)));
        self
    }

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

#[derive(Debug)]
struct ScriptedStream {
    script: SmtpScript,
    log: Arc<Mutex<ConnectionLog>>,
    pending: VecDeque<u8>,
    tls_failure: Option<String>,
    encrypted: bool,
    vanish: Option<Vanish>,
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
        // The failure lands on the read, not the write, on purpose: the bytes did leave, which is
        // what makes the outcome unknowable.
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
        // Structurally, not by keyword: the payload is the test's own message and could contain any
        // word, so matching on content would make `vanishing_at` fire on somebody's mail.
        let is_payload = bytes.ends_with(b"\r\n.\r\n");

        if let Some(Vanish::OnWrite(keyword)) = &self.vanish
            && !is_payload
            && String::from_utf8_lossy(bytes)
                .to_ascii_uppercase()
                .contains(&keyword.to_ascii_uppercase())
        {
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
            self.swallowed_payload = Some(*after);
            return Ok(());
        }

        // Every SMTP write is one logical unit awaiting exactly one reply; the `DATA` body is
        // matched structurally, the rest by keyword.
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
