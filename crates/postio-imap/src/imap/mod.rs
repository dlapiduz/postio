//! The IMAP session: TLS, authentication, and the capability list that
//! everything else is gated on.
//!
//! # The one thing this module exists to get right
//!
//! iCloud does not advertise its real capability set in the pre-auth banner.
//! `CONDSTORE`, `QRESYNC`, `IDLE` and `UIDPLUS` appear only *after* login, and
//! a client that trusts the banner degrades to full resync forever without a
//! single error being logged. `io-imap` makes this easy to get wrong:
//! `ImapLoginOptions::ensure_capabilities` defaults to `false`, and a
//! hand-driven auth coroutine will hand back an empty `Vec<Capability>` rather
//! than failing.
//!
//! So there is exactly one way into a session — [`ImapSession::open`], which
//! goes through `io-imap`'s `ImapSessionOpen` coroutine (that one hard-codes
//! `ensure_capabilities = true`) — and an empty post-authentication capability
//! list is [`BackendError::EmptyCapabilities`], never a silent downgrade. See
//! ADR 0001, Q3.
//!
//! # No plaintext fallback, ever
//!
//! A failed TLS handshake ends the connection. Cleartext to anything but
//! loopback is refused by [`ConnectionSettings::validate`] before a socket is
//! opened, so a mistyped port cannot hand an app-specific password to whoever
//! is listening.
//!
//! # Testing
//!
//! `io-imap` is sans-I/O, so the whole handshake runs over a canned transcript
//! with no socket: see [`ScriptedConnector`]. That covers the exchanges whose
//! exact bytes are the point; for the ones where *state* is the point — a
//! resync, a UIDVALIDITY bump, a torn body fetch — point a [`ConnectionPool`]
//! at the `test_server` module's in-process server, which is a real IMAP
//! server on a loopback port. No test in the default suite touches the
//! network.

mod backend;
mod body;
mod dispatch;
mod fetch;
mod idle;
mod mailboxes;
mod mutate;
mod pool;
mod selection;
mod settings;
mod skip_counter;
mod transport;

use std::fmt;

use io_imap::client::{ImapClientAsync, ImapClientError};
use io_imap::codec::fragmentizer::Fragmentizer;
use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState, ImapYield};
use io_imap::rfc3501::capability::ImapCapabilityGet;
use io_imap::rfc3501::login::ImapLoginError;
use io_imap::rfc3501::logout::ImapLogout;
use io_imap::sasl::auth_login::ImapAuthLoginError;
use io_imap::sasl::auth_plain::ImapAuthPlainError;
use io_imap::session::{
    ImapSessionOpen, ImapSessionOpenError, ImapSessionOpenOptions, ImapSessionOpenYield,
    ImapSessionTransport,
};
use io_imap::types::response::Capability as WireCapability;
use io_sasl::rfc4616::plain::SaslPlainCreds;
use postio_model::TransportSecurity;

use crate::backend::{BackendError, BackendResult, Capabilities};
use crate::secret::Password;

pub use self::backend::ImapBackend;
pub use self::body::{PARTIAL_FETCH_WINDOW, fetch_part};
pub use self::dispatch::{
    Dispatch, ExpungeStrategy, ListingStrategy, MoveStrategy, ResyncStrategy, WatchStrategy,
};
pub use self::fetch::fetch_headers;
pub use self::idle::idle;
pub use self::mailboxes::list_mailboxes;
pub use self::mutate::{append, copy_messages, expunge, move_messages, store_flags};
pub use self::pool::{
    ConnectionPool, DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS,
    PoolConfig, PoolStats, PooledSession, Priority,
};
pub use self::pool::{
    DEFAULT_COMMAND_TIMEOUT, DEFAULT_SELECTION_MAX_AGE, DEFAULT_WATCH_POLL_INTERVAL,
    DEFAULT_WATCH_REFRESH,
};
pub use self::selection::{select, status};
pub use self::settings::{ConnectionSettings, DEFAULT_CONNECT_TIMEOUT, IMAP_PORT, IMAPS_PORT};
pub use self::skip_counter::{
    exclusive_measurement as skip_counter_exclusive_measurement, install as install_skip_counter,
    skipped_untagged_responses,
};
pub use self::transport::{
    ConnectionLog, ImapConnector, ImapScript, ImapStream, RustlsConnector, ScriptedConnector,
    TransportError,
};

/// The largest server message the parser will accept.
///
/// A mailbox can legitimately hold a message far bigger than anything a client
/// wants in one buffer, but a *response* this large is a server misbehaving or
/// a fetch that should have been streamed.
const MAX_MESSAGE_SIZE: u32 = 100 * 1024 * 1024;

/// Read buffer, sized for line-oriented protocol traffic.
const READ_BUFFER: usize = 16 * 1024;

/// An authenticated IMAP session over one connection.
///
/// Owns the socket and the connection-wide parser buffer, and nothing else:
/// every command comes from `io-imap`'s `ImapClientAsync`, which this type
/// implements by pumping coroutines against the stream.
pub struct ImapSession {
    stream: Box<dyn ImapStream>,
    fragmentizer: Fragmentizer,
    capabilities: Capabilities,
    endpoint: String,
    account: String,
    pre_authenticated: bool,
    /// The mailbox this session currently has selected, cached so a fetch
    /// loop over many chunks of the same mailbox does not re-issue `SELECT`
    /// for every one of them. See [`selection`].
    selected: Option<selection::SelectedMailbox>,
    /// The UID generation observed for each mailbox. Shared with every other
    /// session in the same pool, so one connection discovering a renumber
    /// stops the rest from acting on what they cached before it.
    generations: std::sync::Arc<selection::Generations>,
    /// How long [`selected`](Self::selected) may answer without the server
    /// confirming it again. See [`selection`] for why a cached generation is the
    /// dangerous half of that cache.
    selection_max_age: std::time::Duration,
    /// How long a command may go without a byte from the server before it is
    /// given up on. Silence, not duration: see
    /// [`ImapStream::read_within`].
    command_timeout: std::time::Duration,
    /// Set when one of this session's reads hit that bound, so the I/O error
    /// `io-imap` reports back can be turned into the reason it happened.
    /// Taken by [`command_error`](Self::command_error).
    timed_out: Option<BackendError>,
}

impl fmt::Debug for ImapSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImapSession")
            .field("endpoint", &self.endpoint)
            .field("account", &self.account)
            .field("encrypted", &self.stream.is_encrypted())
            .field("capabilities", &self.capabilities.names())
            .finish()
    }
}

impl ImapSession {
    /// Opens a session: connect, secure, authenticate, read capabilities.
    ///
    /// The capability set this returns is the one observed after
    /// authentication. Nothing else in Postio is allowed to build one.
    pub async fn open(
        settings: &ConnectionSettings,
        password: &Password,
        connector: &dyn ImapConnector,
    ) -> BackendResult<Self> {
        settings.validate()?;

        let transport = match settings.security {
            TransportSecurity::Tls => ImapSessionTransport::Tls {
                host: settings.host.clone(),
                port: settings.port,
            },
            TransportSecurity::StartTls | TransportSecurity::None => ImapSessionTransport::Tcp {
                host: settings.host.clone(),
                port: settings.port,
            },
        };

        let options = ImapSessionOpenOptions {
            starttls: settings.security == TransportSecurity::StartTls,
            ..Default::default()
        };

        let credentials = SaslPlainCreds {
            authzid: None,
            authcid: settings.username.clone(),
            passwd: password.expose().to_owned().into(),
        };

        let mut coroutine = ImapSessionOpen::new(transport, Some(credentials), options);
        let mut fragmentizer = Fragmentizer::new(MAX_MESSAGE_SIZE);
        let mut stream: Option<Box<dyn ImapStream>> = None;
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;

        // `WantsTlsUpgrade` carries no payload, so the host of the plaintext
        // connect is kept for the certificate check the upgrade performs.
        let mut upgrade_host = settings.host.clone();

        let opened = loop {
            match coroutine.resume(&mut fragmentizer, resume.take()) {
                ImapCoroutineState::Complete(Ok(data)) => break data,
                ImapCoroutineState::Complete(Err(error)) => {
                    return Err(map_open_error(settings, error));
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsTcpConnect {
                    host,
                    port,
                }) => {
                    upgrade_host = host.clone();
                    stream = Some(connector.connect_tcp(&host, port).await?);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsTlsConnect {
                    host,
                    port,
                }) => {
                    stream = Some(connector.connect_tls(&host, port).await?);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsUnixConnect(path)) => {
                    stream = Some(connector.connect_unix(&path).await?);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsTlsUpgrade) => {
                    // The STARTTLS exchange already completed cleanly; io-imap
                    // refuses the upgrade itself when the server appended
                    // bytes to the tagged response, so an injected command
                    // cannot ride into the TLS session.
                    let plain = stream.take().ok_or_else(no_stream)?;
                    stream = Some(plain.upgrade_tls(&upgrade_host).await?);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsRead) => {
                    let stream = stream.as_mut().ok_or_else(no_stream)?;
                    // The socket connected and TLS finished, so
                    // `connect_timeout` has already been satisfied — but the
                    // exchange after it can still hang, and an opening that
                    // never completes holds a pool slot forever.
                    let read = stream
                        .read_within(&mut buffer, settings.connect_timeout, "the IMAP handshake")
                        .await?;
                    resume = Some(&buffer[..read]);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsWrite(bytes)) => {
                    let stream = stream.as_mut().ok_or_else(no_stream)?;
                    stream.write_all(&bytes).await?;
                }
            }
        };

        let stream = stream.ok_or_else(no_stream)?;
        let capabilities = post_auth_capabilities(&settings.host, &opened.capability)?;

        Ok(Self {
            stream,
            fragmentizer,
            capabilities,
            endpoint: settings.endpoint(),
            account: settings.username.clone(),
            pre_authenticated: opened.pre_authenticated,
            selected: None,
            // A session opened outside a pool answers only to itself; the
            // pool replaces both of these when it opens one.
            generations: std::sync::Arc::new(selection::Generations::new()),
            selection_max_age: DEFAULT_SELECTION_MAX_AGE,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            timed_out: None,
        })
    }

    /// The capabilities this server advertised after authentication.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// The `host:port` this session is connected to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The account this session authenticated as.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// Whether the session opened already authenticated (a `PREAUTH`
    /// greeting, as a local socket proxy sends).
    pub fn is_pre_authenticated(&self) -> bool {
        self.pre_authenticated
    }

    /// Whether the bytes on this connection are encrypted.
    pub fn is_encrypted(&self) -> bool {
        self.stream.is_encrypted()
    }

    /// Re-reads the capability list.
    ///
    /// Needed after anything that changes what the server will admit to —
    /// a `COMPRESS` negotiation, or a provider that revises its list mid
    /// session. Not needed after authentication: [`open`](Self::open) already
    /// did it.
    pub async fn refresh_capabilities(&mut self) -> BackendResult<&Capabilities> {
        let advertised = self.run(ImapCapabilityGet::new()).await;
        let advertised = advertised.map_err(|error| self.command_error("CAPABILITY", error))?;

        self.capabilities = post_auth_capabilities(&self.endpoint, &advertised)?;
        Ok(&self.capabilities)
    }

    /// How long this session waits on a silent server.
    pub(super) fn command_timeout(&self) -> std::time::Duration {
        self.command_timeout
    }

    /// Sets how long a command may go without a byte from the server.
    pub(super) fn set_command_timeout(&mut self, timeout: std::time::Duration) {
        self.command_timeout = timeout;
    }

    /// Maps a command failure, recovering a deadline this session imposed.
    ///
    /// `io-imap` reports our own timeout back as a plain I/O error, which
    /// would surface as `BackendError::Io` — transient, so it would still be
    /// retried, but indistinguishable in a log from a socket that broke. The
    /// distinction is worth keeping: "the server stopped answering" is a
    /// different thing to explain than "the connection dropped".
    pub(crate) fn command_error(&mut self, command: &str, error: ImapClientError) -> BackendError {
        match self.timed_out.take() {
            Some(BackendError::TimedOut { after, .. }) => BackendError::TimedOut {
                context: format!("{command} on {}", self.endpoint),
                after,
            },
            Some(other) => other,
            None => map_client_error(command, &self.account, error),
        }
    }

    /// Ends the session politely.
    pub async fn logout(mut self) -> BackendResult<()> {
        let result = self.run(ImapLogout::new()).await;
        result.map_err(|error| self.command_error("LOGOUT", error))
    }
}

impl ImapClientAsync for ImapSession {
    // Clippy asks to collapse this into an `async fn`. Refuse: an `async fn`
    // in a trait cannot state that its future is `Send`, and that bound is
    // what lets a command built on this method move onto a spawned task —
    // which is exactly what the connection pool does.
    #[allow(clippy::manual_async_fn)]
    fn run<C, T, E>(
        &mut self,
        mut coroutine: C,
    ) -> impl Future<Output = Result<T, ImapClientError>> + Send
    where
        C: ImapCoroutine<Yield = ImapYield, Return = Result<T, E>> + Send,
        T: Send,
        E: Send,
        ImapClientError: From<E>,
    {
        async move {
            let mut buffer = [0u8; READ_BUFFER];
            let mut resume: Option<&[u8]> = None;
            self.timed_out = None;

            loop {
                match coroutine.resume(&mut self.fragmentizer, resume.take()) {
                    ImapCoroutineState::Complete(Ok(value)) => return Ok(value),
                    ImapCoroutineState::Complete(Err(error)) => return Err(error.into()),
                    ImapCoroutineState::Yielded(ImapYield::WantsRead) => {
                        let timeout = self.command_timeout;
                        let read = match self
                            .stream
                            .read_within(&mut buffer, timeout, "a command")
                            .await
                        {
                            Ok(read) => read,
                            Err(error) => {
                                // `io-imap` can only carry this back as an I/O
                                // error, so the reason is kept here and
                                // recovered by `command_error`.
                                let error: BackendError = error.into();
                                let carried = std::io::Error::other(error.to_string());
                                self.timed_out = Some(error);
                                return Err(ImapClientError::Io(carried));
                            }
                        };
                        resume = Some(&buffer[..read]);
                    }
                    ImapCoroutineState::Yielded(ImapYield::WantsWrite(bytes)) => {
                        self.stream.write_all(&bytes).await.map_err(as_io)?;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Capability and error mapping
// ---------------------------------------------------------------------------

/// Turns the wire capability list into Postio's, refusing an empty one.
///
/// The refusal is the whole point. `io-imap` returns `Ok(vec![])` when the
/// capability round trip was skipped, and every gate downstream would read
/// that as "this server supports nothing" and quietly fall back to full
/// resync forever. A server that authenticated us always advertises
/// something.
fn post_auth_capabilities(
    host: &str,
    advertised: &[WireCapability<'static>],
) -> BackendResult<Capabilities> {
    // `Capability`'s `Display` is the wire spelling, including for the
    // extensions imap-types does not model, so nothing is lost in translation.
    let capabilities = Capabilities::from_names(advertised.iter().map(ToString::to_string));

    if capabilities.is_empty() {
        return Err(BackendError::EmptyCapabilities {
            host: host.to_owned(),
        });
    }
    Ok(capabilities)
}

fn no_stream() -> BackendError {
    BackendError::Protocol {
        reason: "the IMAP session asked for I/O before it asked for a connection".to_owned(),
    }
}

fn as_io(error: TransportError) -> ImapClientError {
    let error: BackendError = error.into();
    ImapClientError::Io(std::io::Error::other(error.to_string()))
}

/// Maps a session-opening failure onto the predicate the caller branches on.
///
/// A rejected password must come out as [`BackendError::Auth`] — retrying it
/// forever is the difference between "ask the user for a new app-specific
/// password" and a backoff loop that never ends.
fn map_open_error(settings: &ConnectionSettings, error: ImapSessionOpenError) -> BackendError {
    let account = settings.username.clone();
    match error {
        ImapSessionOpenError::StartTlsOverTls | ImapSessionOpenError::StartTlsInjection => {
            BackendError::Tls {
                host: settings.host.clone(),
                reason: error.to_string(),
            }
        }
        ImapSessionOpenError::AuthPlain(inner) if is_rejection_plain(&inner) => {
            BackendError::Auth {
                account,
                reason: inner.to_string(),
            }
        }
        ImapSessionOpenError::AuthLogin(inner) if is_rejection_login(&inner) => {
            BackendError::Auth {
                account,
                reason: inner.to_string(),
            }
        }
        ImapSessionOpenError::Login(inner) if is_rejection_imap_login(&inner) => {
            BackendError::Auth {
                account,
                reason: inner.to_string(),
            }
        }
        other => BackendError::Protocol {
            reason: other.to_string(),
        },
    }
}

/// Maps a command failure. Commands cannot fail authentication, so this is
/// the coarse mapping: a rejection stays a rejection, everything else is a
/// protocol problem.
fn map_client_error(command: &str, account: &str, error: ImapClientError) -> BackendError {
    match error {
        ImapClientError::Io(inner) => BackendError::Io {
            context: command.to_owned(),
            reason: inner.to_string(),
        },
        ImapClientError::SessionOpen(inner) => BackendError::Auth {
            account: account.to_owned(),
            reason: inner.to_string(),
        },
        other => BackendError::Rejected {
            command: command.to_owned(),
            reason: other.to_string(),
        },
    }
}

/// Whether the server said "no" to the credentials, as opposed to the
/// exchange going wrong.
fn is_rejection_plain(error: &ImapAuthPlainError) -> bool {
    matches!(
        error,
        ImapAuthPlainError::No(_) | ImapAuthPlainError::Bad(_) | ImapAuthPlainError::Bye(_)
    )
}

fn is_rejection_login(error: &ImapAuthLoginError) -> bool {
    matches!(
        error,
        ImapAuthLoginError::No(_) | ImapAuthLoginError::Bad(_) | ImapAuthLoginError::Bye(_)
    )
}

fn is_rejection_imap_login(error: &ImapLoginError) -> bool {
    matches!(
        error,
        ImapLoginError::No(_) | ImapLoginError::Bad(_) | ImapLoginError::Bye(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Capability;

    #[test]
    fn the_wire_capability_list_maps_onto_ours_without_losing_names() {
        let advertised = vec![
            WireCapability::Imap4Rev1,
            WireCapability::CondStore,
            WireCapability::QResync,
            WireCapability::Idle,
            WireCapability::UidPlus,
            WireCapability::Move,
        ];

        let capabilities = post_auth_capabilities("imap.example.com", &advertised).unwrap();

        assert!(capabilities.supports_incremental_sync());
        assert!(capabilities.contains(Capability::Idle));
        assert!(capabilities.contains(Capability::UidPlus));
        assert!(capabilities.contains(Capability::Move));
    }

    #[test]
    fn an_empty_post_auth_capability_list_is_an_error() {
        // ADR 0001 rule 1: io-imap returns Ok(vec![]) rather than failing when
        // the capability round trip was skipped, and every gate downstream
        // would read that as "supports nothing".
        let error = post_auth_capabilities("imap.example.com", &[]).unwrap_err();

        assert!(matches!(error, BackendError::EmptyCapabilities { .. }));
        assert!(error.to_string().contains("imap.example.com"));
        assert!(!error.is_transient());
    }

    #[test]
    fn a_rejected_password_is_an_authentication_failure_not_a_retry() {
        let settings = ConnectionSettings::new(
            "imap.example.com",
            IMAPS_PORT,
            TransportSecurity::Tls,
            "someone@example.com",
        );
        let error = map_open_error(
            &settings,
            ImapSessionOpenError::AuthPlain(ImapAuthPlainError::No(
                "[AUTHENTICATIONFAILED] Authentication failed".to_owned(),
            )),
        );

        assert!(error.is_authentication_failure());
        assert!(!error.is_transient());
        assert!(error.to_string().contains("someone@example.com"));
    }

    #[test]
    fn a_starttls_injection_is_a_tls_failure_and_never_a_downgrade() {
        let settings = ConnectionSettings::new(
            "imap.example.com",
            143,
            TransportSecurity::StartTls,
            "someone@example.com",
        );

        let error = map_open_error(&settings, ImapSessionOpenError::StartTlsInjection);

        assert!(matches!(error, BackendError::Tls { .. }));
        assert!(!error.is_transient());
    }

    #[test]
    fn a_broken_exchange_is_not_mistaken_for_bad_credentials() {
        let settings = ConnectionSettings::new(
            "imap.example.com",
            IMAPS_PORT,
            TransportSecurity::Tls,
            "someone@example.com",
        );
        let error = map_open_error(
            &settings,
            ImapSessionOpenError::AuthPlain(ImapAuthPlainError::MissingTagged),
        );

        assert!(!error.is_authentication_failure());
    }
}
