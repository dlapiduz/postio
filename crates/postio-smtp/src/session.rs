//! The SMTP session: connect, authenticate, send one message, quit.
//!
//! Unlike IMAP there is no long-lived mailbox state to cache — a send opens
//! a connection, runs one mail transaction, and closes it. So there is no
//! pool here, only a session type a caller opens once per send.
//!
//! # Reply codes decide retryability, not which command failed
//!
//! RFC 5321's first reply digit is the signal that matters: `4xx` means "try
//! again later" and `5xx` means "this will never work as sent," regardless
//! of whether it arrived answering `MAIL FROM`, `RCPT TO`, `DATA` or `AUTH`.
//! [`classify`] is the one place that rule lives; every error-mapping
//! function here routes through it rather than assuming a command's reply
//! is always permanent.
//!
//! # No plaintext fallback, ever
//!
//! A failed TLS handshake ends the connection. [`ConnectionSettings::validate`]
//! refuses cleartext to anything but loopback before a socket is even
//! opened, so a mistyped port cannot hand an app-specific password to
//! whoever is listening.

use std::borrow::Cow;
use std::fmt;
use std::io;

use io_sasl::mechanism::Sasl;
use io_sasl::rfc4616::plain::SaslPlainCreds;
use io_sasl::rfc7628::oauthbearer::SaslOauthbearerCreds;
use io_sasl::xoauth2::SaslXoauth2Creds;
use io_smtp::client::{SmtpClientAsync, SmtpClientError};
use io_smtp::coroutine::{SmtpCoroutine, SmtpCoroutineState, SmtpYield};
use io_smtp::rfc5321::data::{SmtpData, SmtpDataError};
use io_smtp::rfc5321::mail::SmtpMailError;
use io_smtp::rfc5321::quit::SmtpQuit;
use io_smtp::rfc5321::rcpt::SmtpRcptError;
use io_smtp::rfc5321::{
    SmtpDomain, SmtpEhloDomain, SmtpForwardPath, SmtpLocalPart, SmtpMailbox, SmtpReversePath,
};
use io_smtp::sasl::auth_plain::SmtpAuthPlainError;
use io_smtp::session::{
    SmtpSessionOpen, SmtpSessionOpenError, SmtpSessionOpenOptions, SmtpSessionOpenYield,
    SmtpSessionTransport,
};
use postio_model::{AuthMethod, TransportSecurity};
use secrecy::SecretString;

use crate::cancel::CancelToken;
use crate::error::{SmtpError, SmtpResult};
use crate::settings::ConnectionSettings;
use crate::transport::{SmtpConnector, SmtpStream, TransportError};

/// Read buffer, sized for line-oriented protocol traffic.
const READ_BUFFER: usize = 16 * 1024;

/// An authenticated SMTP session over one connection.
pub struct SmtpSession {
    stream: Box<dyn SmtpStream>,
    endpoint: String,
    account: String,
}

impl fmt::Debug for SmtpSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmtpSession")
            .field("endpoint", &self.endpoint)
            .field("account", &self.account)
            .field("encrypted", &self.stream.is_encrypted())
            .finish()
    }
}

impl SmtpSession {
    /// Opens a session: connect, secure, `EHLO`, authenticate.
    pub async fn open(
        settings: &ConnectionSettings,
        password: &SecretString,
        connector: &dyn SmtpConnector,
    ) -> SmtpResult<Self> {
        settings.validate()?;

        let transport = match settings.security {
            TransportSecurity::Tls => SmtpSessionTransport::Tls {
                host: settings.host.clone(),
                port: settings.port,
            },
            TransportSecurity::StartTls | TransportSecurity::None => SmtpSessionTransport::Tcp {
                host: settings.host.clone(),
                port: settings.port,
            },
        };

        let options = SmtpSessionOpenOptions {
            starttls: settings.security == TransportSecurity::StartTls,
        };

        let credentials = sasl_for(settings, password);

        let mut coroutine = SmtpSessionOpen::new(
            transport,
            client_identity(&settings.username),
            Some(credentials),
            options,
        );
        let mut stream: Option<Box<dyn SmtpStream>> = None;
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;

        // `WantsTlsUpgrade` carries no payload, so the host of the plaintext
        // connect is kept for the certificate check the upgrade performs.
        let mut upgrade_host = settings.host.clone();

        loop {
            match coroutine.resume(resume.take()) {
                SmtpCoroutineState::Complete(Ok(_data)) => break,
                SmtpCoroutineState::Complete(Err(error)) => {
                    return Err(map_open_error(settings, error));
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsTcpConnect {
                    host,
                    port,
                }) => {
                    upgrade_host = host.clone();
                    stream = Some(connector.connect_tcp(&host, port).await?);
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsTlsConnect {
                    host,
                    port,
                }) => {
                    stream = Some(connector.connect_tls(&host, port).await?);
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsUnixConnect(path)) => {
                    stream = Some(connector.connect_unix(&path).await?);
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsTlsUpgrade) => {
                    let plain = stream.take().ok_or_else(no_stream)?;
                    stream = Some(plain.upgrade_tls(&upgrade_host).await?);
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsRead) => {
                    let stream = stream.as_mut().ok_or_else(no_stream)?;
                    let read = stream.read(&mut buffer).await?;
                    resume = Some(&buffer[..read]);
                }
                SmtpCoroutineState::Yielded(SmtpSessionOpenYield::WantsWrite(bytes)) => {
                    let stream = stream.as_mut().ok_or_else(no_stream)?;
                    stream.write_all(&bytes).await?;
                }
            }
        }

        let stream = stream.ok_or_else(no_stream)?;

        Ok(Self {
            stream,
            endpoint: settings.endpoint(),
            account: settings.username.clone(),
        })
    }

    /// The `host:port` this session is connected to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The account this session authenticated as.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Whether the bytes on this connection are encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.stream.is_encrypted()
    }

    /// Sends one message: `MAIL FROM`, one `RCPT TO` per recipient, then
    /// `DATA`.
    ///
    /// `from` and every entry of `to` are bare addresses
    /// (`local@domain`) — the message's own `From`/`To` headers are inside
    /// `raw` already and are not derived from these. Recipients are tried
    /// in order and the first rejection ends the send; there is no partial
    /// delivery to the recipients that would have succeeded.
    pub async fn send_message(
        &mut self,
        from: &str,
        to: &[String],
        raw: &[u8],
        cancel: &CancelToken,
    ) -> SmtpResult<()> {
        if cancel.is_cancelled() {
            return Err(SmtpError::Cancelled);
        }
        if to.is_empty() {
            return Err(SmtpError::Configuration {
                reason: "a message needs at least one recipient".to_owned(),
            });
        }

        let reverse_path = SmtpReversePath::from(parse_mailbox(from)?);
        self.mail(reverse_path, Vec::new())
            .await
            .map_err(|error| map_mail_error(from, error))?;

        for recipient in to {
            if cancel.is_cancelled() {
                return Err(SmtpError::Cancelled);
            }
            let forward_path = SmtpForwardPath::from(parse_mailbox(recipient)?);
            self.rcpt(forward_path, Vec::new())
                .await
                .map_err(|error| map_rcpt_error(recipient, error))?;
        }

        if cancel.is_cancelled() {
            return Err(SmtpError::Cancelled);
        }
        self.send_payload(raw.to_vec()).await
    }

    /// Runs the `DATA` exchange, tagging whatever fails once the message
    /// payload has begun going out.
    ///
    /// Written out rather than calling [`SmtpClientAsync::data`] because the
    /// boundary ADR 0021 turns on is invisible from outside that call: it
    /// writes the command, reads the `354`, writes the body and reads the
    /// reply, and hands back one error for any of it. From here the shape is
    /// legible — `SmtpData` yields exactly `WantsWrite("DATA\r\n")`,
    /// `WantsRead`, `WantsWrite(body)`, `WantsRead` — so **the first write
    /// after a reply has been read is the payload going onto the wire**, and
    /// everything from that instant until the last reply is read is a failure
    /// the client cannot resolve.
    ///
    /// Derived from the exchange rather than from a byte count or a state
    /// name: `SmtpData`'s states are private, and a rule that said "the
    /// second write" would quietly stop being true if the body were ever
    /// chunked.
    ///
    /// It also maps its own transport errors instead of routing them through
    /// [`SmtpClientAsync::run`], which flattens every [`TransportError`] into
    /// an opaque `io::Error`. That is what lets a timeout waiting for the
    /// reply to the terminating `.` arrive as [`SmtpError::TimedOut`] rather
    /// than as an `Io` that merely mentions the word.
    async fn send_payload(&mut self, raw: Vec<u8>) -> SmtpResult<()> {
        let mut coroutine = SmtpData::new(raw);
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;
        let mut answered = false;
        let mut payload_begun = false;

        // Every exit goes through this, and it is read at the moment of
        // failure rather than at the end: a write that fails *is* the payload
        // beginning, so the byte that never made it counts the same as one
        // that did. There is no way to know which.
        fn tag(payload_begun: bool, error: SmtpError) -> SmtpError {
            if payload_begun {
                error.once_the_payload_was_on_the_wire()
            } else {
                error
            }
        }

        loop {
            match coroutine.resume(resume.take()) {
                SmtpCoroutineState::Complete(Ok(())) => return Ok(()),
                SmtpCoroutineState::Complete(Err(error)) => {
                    return Err(tag(
                        payload_begun,
                        map_data_error(SmtpClientError::from(error)),
                    ));
                }
                SmtpCoroutineState::Yielded(SmtpYield::WantsRead) => {
                    let read = self
                        .stream
                        .read(&mut buffer)
                        .await
                        .map_err(|error| tag(payload_begun, SmtpError::from(error)))?;
                    answered = true;
                    resume = Some(&buffer[..read]);
                }
                SmtpCoroutineState::Yielded(SmtpYield::WantsWrite(bytes)) => {
                    // The `354` has been read, so this is the body.
                    payload_begun |= answered;
                    self.stream
                        .write_all(&bytes)
                        .await
                        .map_err(|error| tag(payload_begun, SmtpError::from(error)))?;
                }
            }
        }
    }

    /// Ends the session politely.
    ///
    /// Best-effort: a message [`send_message`](Self::send_message) already
    /// delivered is not undone by a `QUIT` that fails, so a caller that only
    /// cares about the send may ignore this result.
    pub async fn quit(mut self) -> SmtpResult<()> {
        self.run(SmtpQuit::new())
            .await
            .map(|_| ())
            .map_err(|error| map_transport_error("QUIT", error))
    }
}

impl SmtpClientAsync for SmtpSession {
    // Clippy asks to collapse this into an `async fn`. Refuse: an `async fn`
    // in a trait cannot state that its future is `Send`, and that bound is
    // what lets a command built on this method move onto a spawned task.
    #[allow(clippy::manual_async_fn)]
    fn run<C, T, E>(
        &mut self,
        mut coroutine: C,
    ) -> impl Future<Output = Result<T, SmtpClientError>> + Send
    where
        C: SmtpCoroutine<Yield = SmtpYield, Return = Result<T, E>> + Send,
        T: Send,
        E: Send,
        SmtpClientError: From<E>,
    {
        async move {
            let mut buffer = [0u8; READ_BUFFER];
            let mut resume: Option<&[u8]> = None;

            loop {
                match coroutine.resume(resume.take()) {
                    SmtpCoroutineState::Complete(Ok(value)) => return Ok(value),
                    SmtpCoroutineState::Complete(Err(error)) => return Err(error.into()),
                    SmtpCoroutineState::Yielded(SmtpYield::WantsRead) => {
                        let read = self.stream.read(&mut buffer).await.map_err(as_io)?;
                        resume = Some(&buffer[..read]);
                    }
                    SmtpCoroutineState::Yielded(SmtpYield::WantsWrite(bytes)) => {
                        self.stream.write_all(&bytes).await.map_err(as_io)?;
                    }
                }
            }
        }
    }
}

/// The SASL credentials for `settings`' auth method (#193).
///
/// The IMAP side has the same mapping, and the two are deliberately identical
/// in shape: an account authenticates one way, and both of its sessions have
/// to agree about which. The credential itself is the same `SecretString`
/// whichever branch is taken — this crate still never fetches one, it takes
/// it as a parameter (ADR 0006 Q1).
fn sasl_for(settings: &ConnectionSettings, password: &SecretString) -> Sasl {
    match settings.auth {
        // An app-specific password is a password; the distinction is for the
        // user interface and reaches the wire not at all.
        AuthMethod::Password | AuthMethod::AppPassword => Sasl::Plain(SaslPlainCreds {
            authzid: None,
            authcid: settings.username.clone(),
            passwd: password.clone(),
        }),
        // RFC 7628. `host` and `port` go verbatim into the GS2 header, so
        // they must be the server actually being contacted.
        AuthMethod::OAuth2 => Sasl::Oauthbearer(SaslOauthbearerCreds {
            username: settings.username.clone(),
            host: settings.host.clone(),
            port: settings.port,
            token: password.clone(),
        }),
        AuthMethod::XOAuth2 => Sasl::Xoauth2(SaslXoauth2Creds {
            username: settings.username.clone(),
            token: password.clone(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Address parsing
// ---------------------------------------------------------------------------

/// The `EHLO`/`HELO` identity this client announces: the account's own
/// domain, or `localhost` when the username is not shaped like an address.
///
/// Cosmetic rather than security-relevant — servers log it, none gate on it.
fn client_identity(username: &str) -> SmtpEhloDomain<'static> {
    let domain = username
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or("localhost");
    SmtpEhloDomain::SmtpDomain(SmtpDomain(Cow::Owned(domain.to_owned())))
}

/// Splits a bare `local@domain` address into the wire type, refusing
/// anything that could inject a second command into the stream.
///
/// `SmtpLocalPart`/`SmtpDomain` are unchecked `Cow<str>` wrappers with no
/// validating constructor of their own — they are formatted straight into
/// the command line — so an address containing CR, LF or the `<>` path
/// delimiters must be rejected here, before it reaches `Display`.
fn parse_mailbox(address: &str) -> SmtpResult<SmtpMailbox<'static>> {
    let invalid = || SmtpError::Configuration {
        reason: format!("{address:?} is not a usable email address"),
    };

    let (local, domain) = address.split_once('@').ok_or_else(invalid)?;
    if local.is_empty() || domain.is_empty() {
        return Err(invalid());
    }
    if !is_safe_address_component(local) || !is_safe_address_component(domain) {
        return Err(invalid());
    }

    Ok(SmtpMailbox {
        local_part: SmtpLocalPart(Cow::Owned(local.to_owned())),
        domain: SmtpEhloDomain::SmtpDomain(SmtpDomain(Cow::Owned(domain.to_owned()))),
    })
}

fn is_safe_address_component(part: &str) -> bool {
    part.chars()
        .all(|c| !c.is_control() && !c.is_whitespace() && c != '<' && c != '>')
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

fn no_stream() -> SmtpError {
    SmtpError::Protocol {
        reason: "the SMTP session asked for I/O before it asked for a connection".to_owned(),
    }
}

fn as_io(error: TransportError) -> SmtpClientError {
    SmtpClientError::Io(io::Error::other(error.to_string()))
}

/// RFC 5321's rule: the first reply digit decides retryability, whichever
/// command it answered. `4xx` is [`SmtpError::Transient`]; anything else is
/// `permanent`'s business.
fn classify(
    code: u16,
    reason: String,
    permanent: impl FnOnce(u16, String) -> SmtpError,
) -> SmtpError {
    if code / 100 == 4 {
        SmtpError::Transient { code, reason }
    } else {
        permanent(code, reason)
    }
}

fn map_open_error(settings: &ConnectionSettings, error: SmtpSessionOpenError) -> SmtpError {
    match error {
        SmtpSessionOpenError::StartTlsOverTls | SmtpSessionOpenError::StartTlsInjection => {
            SmtpError::Tls {
                host: settings.host.clone(),
                reason: error.to_string(),
            }
        }
        SmtpSessionOpenError::AuthPlain(SmtpAuthPlainError::Rejected { code, message }) => {
            let account = settings.username.clone();
            classify(code, message, |code, reason| SmtpError::Auth {
                account,
                code,
                reason,
            })
        }
        other => SmtpError::Protocol {
            reason: other.to_string(),
        },
    }
}

fn map_mail_error(sender: &str, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Mail(SmtpMailError::Rejected { code, message }) => {
            let sender = sender.to_owned();
            classify(code, message, |code, reason| SmtpError::SenderRejected {
                sender,
                code,
                reason,
            })
        }
        other => map_transport_error("MAIL FROM", other),
    }
}

fn map_rcpt_error(recipient: &str, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Rcpt(SmtpRcptError::Rejected { code, message }) => {
            let recipient = recipient.to_owned();
            classify(code, message, |code, reason| SmtpError::RecipientRejected {
                recipient,
                code,
                reason,
            })
        }
        other => map_transport_error("RCPT TO", other),
    }
}

fn map_data_error(error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Data(
            SmtpDataError::CommandRejected { code, message }
            | SmtpDataError::BodyRejected { code, message },
        ) => classify(code, message, |code, reason| SmtpError::MessageRejected {
            code,
            reason,
        }),
        other => map_transport_error("DATA", other),
    }
}

/// The coarse mapping for whatever a command failed with beyond a reply
/// code: a transport hiccup is [`SmtpError::Io`], everything else — a
/// framing or parse failure this crate did not name a variant for — is
/// [`SmtpError::Protocol`].
fn map_transport_error(command: &str, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Io(io_error) => SmtpError::Io {
            context: command.to_owned(),
            reason: io_error.to_string(),
        },
        other => SmtpError::Protocol {
            reason: format!("{command}: {other}"),
        },
    }
}
