//! The SMTP session: connect, authenticate, send one message, quit. No pool: a send opens a
//! connection, runs one mail transaction and closes it.
//! The reply code decides retryability (4xx try again, 5xx never), regardless of which command it
//! answered; [`classify`] is the one place that rule lives.
//! No plaintext fallback: a failed TLS handshake ends the connection, and
//! [`ConnectionSettings::validate`] refuses cleartext to anything but loopback.

use bounded_static::IntoBoundedStatic;
use std::fmt;
use std::io;
use std::net::Ipv4Addr;
use std::ops::Not;

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
    SmtpDomain, SmtpEhloDomain, SmtpForwardPath, SmtpLocalPart, SmtpMailbox, SmtpParameter,
    SmtpReversePath,
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
use crate::reply::ReplyTap;
use crate::settings::ConnectionSettings;
use crate::transport::{SmtpConnector, SmtpStream, TransportError};

const READ_BUFFER: usize = 16 * 1024;

pub struct SmtpSession {
    stream: Box<dyn SmtpStream>,
    replies: ReplyTap,
    endpoint: String,
    account: String,
    /// Read for exactly one thing: [`SmtpSession::supports`].
    capabilities: Vec<String>,
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
            ..SmtpSessionOpenOptions::default()
        };

        let credentials = sasl_for(settings, password);

        let mut coroutine =
            SmtpSessionOpen::new(transport, client_identity(), Some(credentials), options);
        let mut stream: Option<Box<dyn SmtpStream>> = None;
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;

        // `WantsTlsUpgrade` carries no payload, so the host of the plaintext
        // connect is kept for the certificate check the upgrade performs.
        let mut upgrade_host = settings.host.clone();
        let mut replies = ReplyTap::default();
        // Kept for one case: RFC 6531, where relying on nothing is not enough because the address
        // itself carries the 8-bit octets (#922). Uppercased once, since EHLO keywords are
        // case-insensitive (RFC 5321 §2.4).
        let capabilities: Vec<String>;

        loop {
            match coroutine.resume(resume.take()) {
                SmtpCoroutineState::Complete(Ok(data)) => {
                    capabilities = data
                        .capabilities
                        .iter()
                        .map(|capability| capability.to_ascii_uppercase())
                        .collect();
                    break;
                }
                SmtpCoroutineState::Complete(Err(error)) => {
                    return Err(map_open_error(settings, &replies, error));
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
                    replies.saw(&buffer[..read]);
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
            replies,
            endpoint: settings.endpoint(),
            account: settings.username.clone(),
            capabilities,
        })
    }

    /// Matched case-insensitively, against the keyword only. One caller, deliberately: Postio
    /// announces nothing and relies on nothing for `SIZE` and `8BITMIME`, and a capability list
    /// erodes that one `if server_supports` at a time; RFC 6531 is the exception (#922).
    pub fn supports(&self, keyword: &str) -> bool {
        let keyword = keyword.to_ascii_uppercase();
        self.capabilities
            .iter()
            .any(|advertised| advertised.split_whitespace().next() == Some(keyword.as_str()))
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn is_encrypted(&self) -> bool {
        self.stream.is_encrypted()
    }

    /// `from` and every entry of `to` are bare addresses; the message's own headers are in `raw`.
    /// The first rejected recipient ends the send, with no partial delivery.
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

        // RFC 6531, before anything reaches the wire: a server that never advertised `SMTPUTF8` may
        // reject an 8-bit address or accept and mangle it. Refusing here keeps the failure legible
        // and nothing half-sent (#922).
        let needs_utf8 = std::iter::once(from)
            .chain(to.iter().map(String::as_str))
            .find(|address| local_part_needs_utf8(address));
        if let Some(address) = needs_utf8
            && !self.supports(SMTPUTF8)
        {
            return Err(SmtpError::Configuration {
                reason: format!(
                    "{address} has a non-ASCII local part, and this server did not offer \
                     SMTPUTF8 — it cannot carry that address"
                ),
            });
        }
        // The parameter RFC 6531 §3.4 requires: sending the octets without it relies on an
        // extension nobody asked for.
        let parameters = match needs_utf8 {
            // Through `new`: `SmtpParameter`'s fields are private to `io-smtp`. The input is a
            // `&'static str`, so the failure is impossible.
            Some(_) => {
                vec![SmtpParameter::new(SMTPUTF8, None).expect("SMTPUTF8 is a valid ESMTP keyword")]
            }
            None => Vec::new(),
        };

        let reverse_path = SmtpReversePath::from(parse_mailbox(from)?);
        self.mail(reverse_path, parameters)
            .await
            .map_err(|error| map_mail_error(from, &self.replies, error))?;

        for recipient in to {
            if cancel.is_cancelled() {
                return Err(SmtpError::Cancelled);
            }
            let forward_path = SmtpForwardPath::from(parse_mailbox(recipient)?);
            self.rcpt(forward_path, Vec::new())
                .await
                .map_err(|error| map_rcpt_error(recipient, &self.replies, error))?;
        }

        if cancel.is_cancelled() {
            return Err(SmtpError::Cancelled);
        }
        self.send_payload(crate::bcc::strip_bcc(raw).into_owned())
            .await
    }

    /// Written out rather than calling [`SmtpClientAsync::data`] because the boundary ADR 0021
    /// turns on is invisible from outside it. `SmtpData` yields `WantsWrite("DATA\r\n")`,
    /// `WantsRead`, `WantsWrite(body)`, `WantsRead`, so the first write after a reply has been read
    /// is the payload going out, and every failure from then until the last reply is read is one
    /// the client cannot resolve.
    /// Derived from the exchange, not a byte count or state name, so chunking the body would not
    /// break it. It maps its own transport errors so a timeout waiting for the reply to the
    /// terminating `.` arrives as [`SmtpError::TimedOut`] rather than an `Io`.
    async fn send_payload(&mut self, raw: Vec<u8>) -> SmtpResult<()> {
        let mut coroutine = SmtpData::new(raw);
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;
        let mut answered = false;
        let mut payload_begun = false;

        // Read at the moment of failure: a write that fails is the payload beginning, and there is
        // no way to know which byte made it.
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
                        map_data_error(&self.replies, SmtpClientError::from(error)),
                    ));
                }
                SmtpCoroutineState::Yielded(SmtpYield::WantsRead) => {
                    let read = self
                        .stream
                        .read(&mut buffer)
                        .await
                        .map_err(|error| tag(payload_begun, SmtpError::from(error)))?;
                    answered = true;
                    self.replies.saw(&buffer[..read]);
                    resume = Some(&buffer[..read]);
                }
                SmtpCoroutineState::Yielded(SmtpYield::WantsWrite(bytes)) => {
                    payload_begun |= answered;
                    self.stream
                        .write_all(&bytes)
                        .await
                        .map_err(|error| tag(payload_begun, SmtpError::from(error)))?;
                }
            }
        }
    }

    /// Best-effort: a failed `QUIT` does not undo a delivered message, so a caller may ignore this
    /// result.
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
                        self.replies.saw(&buffer[..read]);
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

/// Deliberately identical in shape to the IMAP side: an account authenticates one way and both
/// sessions must agree. This crate never fetches the credential, it takes it as a parameter (ADR
/// 0006 Q1).
fn sasl_for(settings: &ConnectionSettings, password: &SecretString) -> Sasl {
    match settings.auth {
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

/// RFC 5321 §4.1.3 reserves the address literal for a client without an FQDN. Servers gate on it:
/// Stalwart refuses `localhost` with `550 5.5.0 Invalid EHLO domain`, and a server checking HELO
/// against its own domains refuses a client claiming to be it.
fn client_identity() -> SmtpEhloDomain<'static> {
    SmtpEhloDomain::from(Ipv4Addr::LOCALHOST)
}

/// `SmtpLocalPart`/`SmtpDomain` are unchecked wrappers formatted straight into the command line, so
/// an address containing CR, LF or `<>` must be rejected here.
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

    // The domain becomes ASCII whatever the server offered (IDNA is a conversion, not a
    // negotiation). The local part is untouched: RFC 6531 exists because the only way to carry one
    // is to send the octets, and `send_message` decides whether the connection may.
    let domain = idna::domain_to_ascii(domain).map_err(|_| invalid())?;

    // Through `try_from`, which checks each piece against RFC 5321's grammar (non-ASCII allowed,
    // RFC 6531 §3.3), before a byte reaches the server.
    let local_part = SmtpLocalPart::try_from(local).map_err(|_| invalid())?;
    let domain = SmtpDomain::try_from(domain.as_str()).map_err(|_| invalid())?;
    Ok(SmtpMailbox {
        local_part,
        domain: SmtpEhloDomain::SmtpDomain(domain),
    }
    .into_static())
}

const SMTPUTF8: &str = "SMTPUTF8";

/// The domain never does ([`parse_mailbox`] punycodes it); this asks about the local part as typed.
fn local_part_needs_utf8(address: &str) -> bool {
    address
        .split_once('@')
        .map(|(local, _)| local)
        .unwrap_or(address)
        .is_ascii()
        .not()
}

fn is_safe_address_component(part: &str) -> bool {
    part.chars()
        .all(|c| !c.is_control() && !c.is_whitespace() && c != '<' && c != '>')
}

fn no_stream() -> SmtpError {
    SmtpError::Protocol {
        reason: "the SMTP session asked for I/O before it asked for a connection".to_owned(),
    }
}

fn as_io(error: TransportError) -> SmtpClientError {
    SmtpClientError::Io(io::Error::other(error.to_string()))
}

/// RFC 5321: the first reply digit decides retryability, whichever command it answered; `4xx` is
/// [`SmtpError::Transient`].
/// `message` is only the reply's first line (`io-smtp` builds every rejection from
/// `response.text()`), so `replies` recovers the continuation lines (#921, `crate::reply`).
fn classify(
    replies: &ReplyTap,
    code: u16,
    message: String,
    permanent: impl FnOnce(u16, String) -> SmtpError,
) -> SmtpError {
    let reason = replies.reason(code).unwrap_or(message);
    if code / 100 == 4 {
        SmtpError::Transient { code, reason }
    } else {
        permanent(code, reason)
    }
}

fn map_open_error(
    settings: &ConnectionSettings,
    replies: &ReplyTap,
    error: SmtpSessionOpenError,
) -> SmtpError {
    match error {
        SmtpSessionOpenError::StartTlsOverTls | SmtpSessionOpenError::StartTlsInjection => {
            SmtpError::Tls {
                host: settings.host.clone(),
                reason: error.to_string(),
            }
        }
        SmtpSessionOpenError::AuthPlain(SmtpAuthPlainError::Rejected { code, message }) => {
            let account = settings.username.clone();
            classify(replies, code, message, |code, reason| SmtpError::Auth {
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

fn map_mail_error(sender: &str, replies: &ReplyTap, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Mail(SmtpMailError::Rejected { code, message }) => {
            let sender = sender.to_owned();
            classify(replies, code, message, |code, reason| {
                SmtpError::SenderRejected {
                    sender,
                    code,
                    reason,
                }
            })
        }
        other => map_transport_error("MAIL FROM", other),
    }
}

fn map_rcpt_error(recipient: &str, replies: &ReplyTap, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Rcpt(SmtpRcptError::Rejected { code, message }) => {
            let recipient = recipient.to_owned();
            classify(replies, code, message, |code, reason| {
                SmtpError::RecipientRejected {
                    recipient,
                    code,
                    reason,
                }
            })
        }
        other => map_transport_error("RCPT TO", other),
    }
}

fn map_data_error(replies: &ReplyTap, error: SmtpClientError) -> SmtpError {
    match error {
        SmtpClientError::Data(
            SmtpDataError::CommandRejected { code, message }
            | SmtpDataError::BodyRejected { code, message },
        ) => classify(replies, code, message, |code, reason| {
            SmtpError::MessageRejected { code, reason }
        }),
        other => map_transport_error("DATA", other),
    }
}

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
