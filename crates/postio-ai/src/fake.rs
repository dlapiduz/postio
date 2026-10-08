//! A runtime that is not there: the fake transport every default test runs
//! the client against, so no test in the default suite opens a connection,
//! loopback included.
//!
//! It answers each request with what its answerer says -- a model's message
//! content, an error status, or a refused connection -- and keeps what it
//! was asked, so a test can read the request the client made.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use postio_config::ModelEndpoint;
use serde_json::{Value, json};

use crate::transport::{Stream, Transport};

/// What the fake runtime answers one request with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// A chat completion whose message content is this.
    Content(String),
    /// An error status, with no completion.
    Status(u16),
    /// The connection is refused: the runtime is not running.
    Refused,
}

impl Reply {
    /// A completion whose message content is `content`.
    pub fn content(content: impl Into<String>) -> Self {
        Reply::Content(content.into())
    }
}

/// A request the fake runtime was sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Its method.
    pub method: String,
    /// Its path.
    pub path: String,
    /// Its `Host` header.
    pub host: String,
    /// Its body, as JSON.
    pub body: Value,
}

impl Request {
    /// The fenced data the request carried: its user message.
    pub fn data(&self) -> &str {
        self.body["messages"][1]["content"]
            .as_str()
            .unwrap_or_default()
    }
}

type Answerer = dyn Fn(&Request) -> Reply + Send + Sync;

/// The fake runtime.
#[derive(Clone)]
pub struct FakeRuntime {
    answer: Arc<Answerer>,
    /// Refuse every connection once nothing is queued.
    refusing: bool,
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    connections: usize,
    requests: Vec<Request>,
    queued: VecDeque<Reply>,
}

impl std::fmt::Debug for FakeRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeRuntime").finish_non_exhaustive()
    }
}

impl FakeRuntime {
    /// A runtime that answers each request as `answer` says.
    pub fn new(answer: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        Self {
            answer: Arc::new(answer),
            refusing: false,
            state: Arc::default(),
        }
    }

    /// A runtime that answers every request with `reply`.
    pub fn always(reply: Reply) -> Self {
        let refusing = reply == Reply::Refused;
        Self {
            refusing,
            ..Self::new(move |_| reply.clone())
        }
    }

    /// A runtime that answers with `replies`, in order, and then refuses.
    pub fn replying(replies: impl IntoIterator<Item = Reply>) -> Self {
        let runtime = Self::refusing();
        runtime
            .state
            .lock()
            .expect("never poisoned")
            .queued
            .extend(replies);
        runtime
    }

    /// A runtime that is not running: every connection is refused.
    pub fn refusing() -> Self {
        Self::always(Reply::Refused)
    }

    /// How many connections were attempted.
    pub fn connections(&self) -> usize {
        self.state.lock().expect("never poisoned").connections
    }

    /// Every request it was sent, in order.
    pub fn requests(&self) -> Vec<Request> {
        self.state.lock().expect("never poisoned").requests.clone()
    }

    fn next_reply(&self, request: Option<&Request>) -> Reply {
        let queued = self
            .state
            .lock()
            .expect("never poisoned")
            .queued
            .pop_front();
        match (queued, request) {
            (Some(reply), _) => reply,
            (None, Some(request)) => (self.answer)(request),
            (None, None) => Reply::Status(400),
        }
    }
}

impl Transport for FakeRuntime {
    fn connect(&self, _: &ModelEndpoint, _: Instant) -> io::Result<Box<dyn Stream>> {
        self.state.lock().expect("never poisoned").connections += 1;
        // A refusal refuses the connection itself; anything else is decided
        // once the request is read.
        let refuses = {
            let mut state = self.state.lock().expect("never poisoned");
            match state.queued.front() {
                Some(Reply::Refused) => {
                    state.queued.pop_front();
                    true
                }
                Some(_) => false,
                None => self.refusing,
            }
        };
        if refuses {
            return Err(io::Error::from(io::ErrorKind::ConnectionRefused));
        }
        Ok(Box::new(FakeStream {
            runtime: self.clone(),
            written: Vec::new(),
            reply: None,
            at: 0,
        }))
    }
}

/// One connection to the fake runtime.
struct FakeStream {
    runtime: FakeRuntime,
    written: Vec<u8>,
    reply: Option<Vec<u8>>,
    at: usize,
}

impl Write for FakeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Read for FakeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.reply.is_none() {
            let request = parse(&self.written);
            let reply = self.runtime.next_reply(request.as_ref());
            if let Some(request) = request {
                self.runtime
                    .state
                    .lock()
                    .expect("never poisoned")
                    .requests
                    .push(request);
            }
            self.reply = Some(response(&reply));
        }
        let reply = self.reply.as_deref().unwrap_or_default();
        let left = &reply[self.at..];
        let n = left.len().min(buf.len());
        buf[..n].copy_from_slice(&left[..n]);
        self.at += n;
        Ok(n)
    }
}

/// The request the client wrote: its head and its JSON body.
fn parse(written: &[u8]) -> Option<Request> {
    let text = std::str::from_utf8(written).ok()?;
    let (head, body) = text.split_once("\r\n\r\n")?;
    let mut lines = head.lines();
    let mut start = lines.next()?.split_whitespace();
    let method = start.next()?.to_owned();
    let path = start.next()?.to_owned();
    let host = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim().to_owned())
        .unwrap_or_default();
    Some(Request {
        method,
        path,
        host,
        body: serde_json::from_str(body).ok()?,
    })
}

/// The bytes a runtime would send back for `reply`.
fn response(reply: &Reply) -> Vec<u8> {
    let (status, body) = match reply {
        Reply::Content(content) => (
            "200 OK".to_owned(),
            json!({
                "object": "chat.completion",
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": content },
                    "finish_reason": "stop",
                }],
            })
            .to_string(),
        ),
        Reply::Status(code) => (
            format!("{code} Refused"),
            json!({ "error": { "message": "no" } }).to_string(),
        ),
        Reply::Refused => ("503 Service Unavailable".to_owned(), String::new()),
    };
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
