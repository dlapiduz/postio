//! Hand-written reader, not `imap-codec`: a server parsing with the client's own codec would share
//! its mistakes, and this one must send bytes no encoder produces (a `-1` sequence number, a short
//! literal).

use std::io;
use std::time::Duration;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_transport::DeflateCodec;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub(super) trait Io: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug> Io for T {}

#[derive(Debug, Default)]
pub(super) struct WireStats {
    pub(super) logical_out: AtomicUsize,
    pub(super) wire_out: AtomicUsize,
    pub(super) compressed_connections: AtomicUsize,
}

/// Literals carry arbitrary bytes, so each is replaced by `\x01<index>\x01` and kept beside the
/// command line.
const MARK: char = '\u{1}';

/// A count rather than a size, so a trickle's delay is the same for a tagged OK or a megabyte of
/// attachment.
const TRICKLE_PIECES: usize = 8;

#[derive(Debug)]
pub(super) struct Conn {
    stream: Box<dyn Io>,
    buffer: Vec<u8>,
    trickle: Option<Duration>,
    codec: Option<DeflateCodec>,
    stats: Arc<WireStats>,
}

#[derive(Clone, Debug)]
pub(super) struct Command {
    pub(super) tag: String,
    pub(super) name: String,
    pub(super) uid: bool,
    pub(super) args: String,
    pub(super) literals: Vec<Vec<u8>>,
    pub(super) raw: String,
}

impl Command {
    pub(super) fn tokens(&self) -> Vec<String> {
        tokens(&self.args)
    }

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

    pub(super) fn bytes(&self, token: &str) -> Vec<u8> {
        if let Some(index) = mark_index(token) {
            return self.literals.get(index).cloned().unwrap_or_default();
        }
        unquote(token).into_bytes()
    }
}

impl Conn {
    pub(super) fn new(stream: impl Io + 'static, stats: Arc<WireStats>) -> Self {
        Self {
            stream: Box::new(stream),
            buffer: Vec::new(),
            trickle: None,
            codec: None,
            stats,
        }
    }

    /// Bytes already buffered stay plain: a client waits for the `OK` before sending anything
    /// compressed.
    pub(super) fn start_deflate(&mut self) {
        self.codec = Some(DeflateCodec::new());
        self.stats
            .compressed_connections
            .fetch_add(1, Ordering::SeqCst);
    }

    pub(super) fn is_deflating(&self) -> bool {
        self.codec.is_some()
    }

    pub(super) fn set_trickle(&mut self, gap: Option<Duration>) {
        self.trickle = gap;
    }

    /// Cancel-safe: bytes that arrived stay buffered, so a read dropped by `select!` loses nothing.
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

    /// `deaf` is asked when a literal is announced: if yes, the `+` goes out and nothing is read
    /// again, so the peer's writes back up.
    pub(super) async fn read_command(
        &mut self,
        deaf: impl Fn() -> bool,
    ) -> io::Result<Option<Command>> {
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
                    if deaf() {
                        // Never read another byte: not a close, which the
                        // client would see, but a peer whose receive window
                        // stays shut.
                        std::future::pending::<()>().await;
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

/// OAUTHBEARER's failure path is a challenge, not a bare `NO` (RFC 7628 §3.2.3); skipping it would
/// hang the client's coroutine.
pub(super) fn base64_encode(input: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(input)
}

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
