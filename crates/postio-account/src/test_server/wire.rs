//! Reading commands off a socket and writing responses back.
//!
//! The reader is written by hand rather than over `imap-codec` on purpose.
//! The point of this server is to catch a regression in the protocol crate,
//! and a server that parsed with the same codec the client encodes with would
//! agree with it about any mistake they shared. It also has to be able to
//! send bytes no encoder would produce — a `-1` sequence number, a literal
//! that stops short — which is the other half of the job.

use std::io;
use std::time::Duration;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_transport::DeflateCodec;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What crossed the wire, summed over every connection.
///
/// `logical` is what the protocol said and `wire` is what the socket carried,
/// so the two differ only on a connection that negotiated `COMPRESS`.
#[derive(Debug, Default)]
pub(super) struct WireStats {
    pub(super) logical_out: AtomicUsize,
    pub(super) wire_out: AtomicUsize,
    pub(super) compressed_connections: AtomicUsize,
}

/// The byte that stands in for a literal in a parsed command line.
///
/// Literals carry arbitrary bytes — an appended message, a mailbox name in
/// UTF-7 — so they never go into the command string itself. Each one is
/// replaced by `\x01<index>\x01` and kept beside it.
const MARK: char = '\u{1}';

/// How many pieces a trickled response is broken into.
///
/// A count rather than a piece size, so the delay a
/// [`Fault::Trickle`](super::Fault::Trickle) adds is the same whether the
/// response is a tagged OK or a megabyte of attachment.
const TRICKLE_PIECES: usize = 8;

/// One connection's socket, with the leftovers of the last read.
#[derive(Debug)]
pub(super) struct Conn {
    stream: TcpStream,
    buffer: Vec<u8>,
    /// When set, responses go out in pieces with this long between them —
    /// a server that is slow rather than stuck.
    trickle: Option<Duration>,
    /// Set once `COMPRESS DEFLATE` has been answered; both directions are raw
    /// deflate from then on.
    codec: Option<DeflateCodec>,
    stats: Arc<WireStats>,
}

/// A command line, with its literals lifted out.
#[derive(Clone, Debug)]
pub(super) struct Command {
    /// The client's tag, or `*` if it sent none.
    pub(super) tag: String,
    /// The command name, upper case. `UID FETCH` is `FETCH` with
    /// [`Command::uid`] set.
    pub(super) name: String,
    /// Whether the command was prefixed with `UID`.
    pub(super) uid: bool,
    /// Everything after the command name, literals replaced by marks.
    pub(super) args: String,
    /// The literals, in the order they appeared.
    pub(super) literals: Vec<Vec<u8>>,
    /// The whole line as received, for the command log.
    pub(super) raw: String,
}

impl Command {
    /// The argument tokens, split at top level.
    pub(super) fn tokens(&self) -> Vec<String> {
        tokens(&self.args)
    }

    /// One argument as a string: unquoted, or the literal it marks.
    pub(super) fn text(&self, token: &str) -> String {
        if let Some(index) = mark_index(token) {
            return self
                .literals
                .get(index)
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
        }
        unquote(token)
    }

    /// One argument as raw bytes — the appended message, in practice.
    pub(super) fn bytes(&self, token: &str) -> Vec<u8> {
        if let Some(index) = mark_index(token) {
            return self.literals.get(index).cloned().unwrap_or_default();
        }
        unquote(token).into_bytes()
    }
}

impl Conn {
    pub(super) fn new(stream: TcpStream, stats: Arc<WireStats>) -> Self {
        Self {
            stream,
            buffer: Vec::new(),
            trickle: None,
            codec: None,
            stats,
        }
    }

    /// Switches both directions to raw deflate, as RFC 4978 says to do right
    /// after the tagged `OK`. Bytes already buffered stay plain: a client
    /// waits for the `OK` before it sends anything compressed.
    pub(super) fn start_deflate(&mut self) {
        self.codec = Some(DeflateCodec::new());
        self.stats
            .compressed_connections
            .fetch_add(1, Ordering::SeqCst);
    }

    /// Whether `COMPRESS` is active on this connection.
    pub(super) fn is_deflating(&self) -> bool {
        self.codec.is_some()
    }

    /// Sends everything from now on in pieces, `gap` apart.
    pub(super) fn set_trickle(&mut self, gap: Option<Duration>) {
        self.trickle = gap;
    }

    /// Reads one CRLF-terminated line, without the CRLF.
    ///
    /// Cancel-safe: bytes that arrived stay in the buffer, so a read dropped
    /// by `select!` while IDLE waits for a notification loses nothing.
    pub(super) async fn read_line(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(at) = self.buffer.windows(2).position(|pair| pair == b"\r\n") {
                let line = self.buffer[..at].to_vec();
                self.buffer.drain(..at + 2);
                return Ok(Some(line));
            }
            if !self.fill().await? {
                return Ok(None);
            }
        }
    }

    /// Reads exactly `len` bytes — a literal's payload.
    pub(super) async fn read_exact(&mut self, len: usize) -> io::Result<Option<Vec<u8>>> {
        while self.buffer.len() < len {
            if !self.fill().await? {
                return Ok(None);
            }
        }
        let bytes = self.buffer[..len].to_vec();
        self.buffer.drain(..len);
        Ok(Some(bytes))
    }

    async fn fill(&mut self) -> io::Result<bool> {
        let mut chunk = [0u8; 8 * 1024];
        loop {
            let Some(codec) = self.codec.as_mut() else {
                let read = self.stream.read(&mut chunk).await?;
                if read == 0 {
                    return Ok(false);
                }
                self.buffer.extend_from_slice(&chunk[..read]);
                return Ok(true);
            };

            let inflated = codec.inflate(&mut chunk)?;
            if inflated > 0 {
                self.buffer.extend_from_slice(&chunk[..inflated]);
                return Ok(true);
            }
            let read = self.stream.read(&mut chunk).await?;
            if read == 0 {
                return Ok(false);
            }
            codec.push(&chunk[..read]);
        }
    }

    /// One socket write, through the compressor when `COMPRESS` is active.
    async fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.stats
            .logical_out
            .fetch_add(bytes.len(), Ordering::SeqCst);
        match self.codec.as_mut() {
            None => {
                self.stats.wire_out.fetch_add(bytes.len(), Ordering::SeqCst);
                self.stream.write_all(bytes).await
            }
            Some(codec) => {
                let wire = codec.deflate(bytes);
                self.stats.wire_out.fetch_add(wire.len(), Ordering::SeqCst);
                self.stream.write_all(&wire).await
            }
        }
    }

    pub(super) async fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let Some(gap) = self.trickle else {
            return self.send(bytes).await;
        };

        let piece = bytes.len().div_ceil(TRICKLE_PIECES).max(1);
        for (index, slice) in bytes.chunks(piece).enumerate() {
            if index > 0 {
                tokio::time::sleep(gap).await;
            }
            self.send(slice).await?;
        }
        Ok(())
    }

    pub(super) async fn write_line(&mut self, text: &str) -> io::Result<()> {
        self.write(format!("{text}\r\n").as_bytes()).await
    }

    /// Reads a whole command, answering the continuation request each
    /// synchronizing literal asks for.
    pub(super) async fn read_command(&mut self) -> io::Result<Option<Command>> {
        let mut text = String::new();
        let mut literals: Vec<Vec<u8>> = Vec::new();

        loop {
            let Some(line) = self.read_line().await? else {
                return Ok(None);
            };
            let line = String::from_utf8_lossy(&line).into_owned();

            match literal_announcement(&line) {
                Some((head, len, synchronizing)) => {
                    text.push_str(head);
                    text.push(MARK);
                    text.push_str(&literals.len().to_string());
                    text.push(MARK);
                    if synchronizing {
                        self.write_line("+ ready for the literal").await?;
                    }
                    let Some(bytes) = self.read_exact(len).await? else {
                        return Ok(None);
                    };
                    literals.push(bytes);
                }
                None => {
                    text.push_str(&line);
                    break;
                }
            }
        }

        Ok(Some(parse(text, literals)))
    }
}

/// Splits a command line into tag, name and arguments.
fn parse(text: String, literals: Vec<Vec<u8>>) -> Command {
    let raw = text.clone();
    let mut rest = text.trim_start();

    let tag = take_word(&mut rest);
    let mut name = take_word(&mut rest).to_ascii_uppercase();
    let mut uid = false;
    if name == "UID" {
        uid = true;
        name = take_word(&mut rest).to_ascii_uppercase();
    }

    Command {
        tag,
        name,
        uid,
        args: rest.trim().to_owned(),
        literals,
        raw,
    }
}

fn take_word(rest: &mut &str) -> String {
    let trimmed = rest.trim_start();
    let end = trimmed.find(' ').unwrap_or(trimmed.len());
    let word = trimmed[..end].to_owned();
    *rest = &trimmed[end..];
    word
}

/// `("{" number ["+"] "}")` at the end of a line, and what precedes it.
fn literal_announcement(line: &str) -> Option<(&str, usize, bool)> {
    let head = line.strip_suffix('}')?;
    let at = head.rfind('{')?;
    let digits = &head[at + 1..];
    let (digits, synchronizing) = match digits.strip_suffix('+') {
        Some(digits) => (digits, false),
        None => (digits, true),
    };
    let len = digits.parse().ok()?;
    Some((&head[..at], len, synchronizing))
}

/// Splits at top level, keeping parenthesised, bracketed and quoted groups
/// whole.
pub(super) fn tokens(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;

    for character in input.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' if quoted => {
                current.push(character);
                escaped = true;
            }
            '"' => {
                quoted = !quoted;
                current.push(character);
            }
            '(' | '[' | '<' if !quoted => {
                depth += 1;
                current.push(character);
            }
            ')' | ']' | '>' if !quoted => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            ' ' if !quoted && depth == 0 => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Strips one layer of quoting, undoing the escapes inside it.
pub(super) fn unquote(token: &str) -> String {
    let inner = match token
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        Some(inner) => inner,
        None => return token.to_owned(),
    };
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Strips one layer of parentheses.
pub(super) fn unwrap_parens(token: &str) -> &str {
    token
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .unwrap_or(token)
}

fn mark_index(token: &str) -> Option<usize> {
    let inner = token.strip_prefix(MARK)?.strip_suffix(MARK)?;
    inner.parse().ok()
}

/// Encodes a SASL server challenge as base64.
///
/// OAUTHBEARER needs it: its failure path is not a bare tagged `NO`. RFC 7628
/// §3.2.3 has the server send a JSON error *as a challenge*, the client
/// acknowledge with a single `0x01`, and only then the `NO`. A server that
/// skipped the challenge would leave the client's coroutine waiting for
/// something that never arrives, and the test would hang rather than fail.
pub(super) fn base64_encode(input: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(input)
}

/// Decodes the base64 of a SASL initial response, padded or not.
pub(super) fn base64_decode(input: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    const LENIENT: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let compact: String = input.split_whitespace().collect();
    LENIENT.decode(compact).ok()
}
