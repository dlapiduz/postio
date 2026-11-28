//! The one way into a session is `ImapSession::open`, via io-imap's `ImapSessionOpen`, which
//! hard-codes `ensure_capabilities = true`; an empty post-auth capability list is
//! `EmptyCapabilities`. iCloud advertises `CONDSTORE`, `QRESYNC`, `IDLE` and `UIDPLUS` only after
//! login (ADR 0001 Q3).
//! No plaintext fallback: a failed TLS handshake ends the connection, and
//! `ConnectionSettings::validate` refuses cleartext to anything but loopback.
//! `ScriptedConnector` runs the handshake over a canned transcript; the `test_server` module serves
//! state tests on loopback.

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
mod transport;

use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU32;

use io_imap::client::{ImapClientAsync, ImapClientError};
use io_imap::codec::fragmentizer::Fragmentizer;
use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState, ImapYield};
use io_imap::rfc3501::capability::ImapCapabilityGet;
use io_imap::rfc3501::login::ImapLoginError;
use io_imap::rfc3501::logout::ImapLogout;
use io_imap::rfc7628::auth_oauthbearer::ImapAuthOauthbearerError;
use io_imap::sasl::auth_login::ImapAuthLoginError;
use io_imap::sasl::auth_plain::ImapAuthPlainError;
use io_imap::sasl::auth_xoauth2::ImapAuthXoauth2Error;
use io_imap::send::ImapOutput;
use io_imap::session::{
    ImapSessionOpen, ImapSessionOpenError, ImapSessionOpenOptions, ImapSessionOpenYield,
    ImapSessionTransport,
};
use io_imap::types::core::{Tag, TagGenerator};
use io_imap::types::response::Capability as WireCapability;
use io_sasl::mechanism::Sasl;
use io_sasl::rfc4616::plain::SaslPlainCreds;
use io_sasl::rfc7628::oauthbearer::SaslOauthbearerCreds;
use io_sasl::xoauth2::SaslXoauth2Creds;
use postio_model::{AuthMethod, TransportSecurity};
use postio_transport::DeflateStream;

use crate::backend::{BackendError, BackendResult, Capabilities, Capability};
use crate::secret::Password;

pub use self::backend::ImapBackend;
pub use self::body::{PARTIAL_FETCH_WINDOW, fetch_part, fetch_sections};
pub use self::dispatch::{
    Dispatch, ExpungeStrategy, ListingStrategy, MoveStrategy, ResyncStrategy, WatchStrategy,
};
pub use self::fetch::{fetch_flags, fetch_headers, fetch_headers_for_filing};
pub use self::idle::idle;
pub use self::mailboxes::{create_mailbox, list_mailboxes};
pub use self::mutate::{append, copy_messages, expunge, move_messages, store_flags};
pub use self::pool::{
    ConnectionPool, DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_IDLE_TIMEOUT, DEFAULT_MAX_CONNECTIONS,
    PoolConfig, PoolStats, PooledSession, Priority,
};
pub use self::pool::{DEFAULT_COMMAND_TIMEOUT, DEFAULT_SELECTION_MAX_AGE};
pub use self::selection::{select, status};
pub use self::settings::{ConnectionSettings, DEFAULT_CONNECT_TIMEOUT, IMAP_PORT, IMAPS_PORT};
pub use self::transport::{
    ConnectionLog, ImapConnector, ImapScript, ImapStream, RustlsConnector, ScriptedConnector,
    TransportError,
};

/// A response this large is a server misbehaving or a fetch that should have streamed.
pub(crate) const MAX_MESSAGE_SIZE: u32 = 100 * 1024 * 1024;

const COMPRESS_COMMAND: &[u8] = b"pz1 COMPRESS DEFLATE\r\n";
const COMPRESS_TAG_OK: &str = "pz1 OK";

const READ_BUFFER: usize = 16 * 1024;

pub struct ImapSession {
    stream: Box<dyn ImapStream>,
    fragmentizer: Fragmentizer,
    tags: TagGenerator,
    capabilities: Capabilities,
    endpoint: String,
    account: String,
    selected: Option<selection::SelectedMailbox>,
    /// Shared with every other session in the pool, so one connection discovering a renumber stops
    /// the rest acting on stale generations.
    generations: std::sync::Arc<selection::Generations>,
    selection_max_age: std::time::Duration,
    /// Silence, not duration; see `ImapStream::read_within`.
    command_timeout: std::time::Duration,
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
    /// Nothing else in Postio may build a capability set.
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

        let credentials = sasl_for(settings, password);

        let mut coroutine = ImapSessionOpen::new(transport, Some(credentials), options);
        let mut fragmentizer = Fragmentizer::new(MAX_MESSAGE_SIZE);
        let mut stream: Option<Box<dyn ImapStream>> = None;
        let mut buffer = [0u8; READ_BUFFER];
        let mut resume: Option<&[u8]> = None;

        let mut upgrade_host = settings.host.clone();
        let mut tags = TagGenerator::new();
        let mut tag: Tag<'static>;

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
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsTag) => {
                    tag = tags.generate();
                    resume = Some(tag.as_ref().as_bytes());
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
                    // The handshake can still hang after connect; an opening that never completes
                    // holds a pool slot forever.
                    let read = stream
                        .read_within(&mut buffer, settings.connect_timeout, "the IMAP handshake")
                        .await?;
                    resume = Some(&buffer[..read]);
                }
                ImapCoroutineState::Yielded(ImapSessionOpenYield::WantsWrite(bytes)) => {
                    let stream = stream.as_mut().ok_or_else(no_stream)?;
                    stream
                        .write_within(&bytes, settings.connect_timeout, "the IMAP handshake")
                        .await?;
                }
            }
        };

        let stream = stream.ok_or_else(no_stream)?;
        let capabilities = post_auth_capabilities(&settings.host, &opened.capability)?;

        let mut session = Self {
            stream,
            fragmentizer,
            tags,
            capabilities,
            endpoint: settings.endpoint(),
            account: settings.username.clone(),
            selected: None,
            generations: std::sync::Arc::new(selection::Generations::new()),
            selection_max_age: DEFAULT_SELECTION_MAX_AGE,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            timed_out: None,
        };
        session.negotiate_compression().await?;
        Ok(session)
    }

    /// `io-imap` has no coroutine for it, so the command goes through its raw passthrough. A
    /// refusal leaves the session working in the clear.
    async fn negotiate_compression(&mut self) -> BackendResult<()> {
        if !self.capabilities.contains(Capability::Compress) {
            return Ok(());
        }

        let response = self.raw(COMPRESS_COMMAND).await;
        let response = response.map_err(|error| self.command_error("COMPRESS", error))?;
        if !response
            .iter()
            .any(|line| line.starts_with(COMPRESS_TAG_OK.as_bytes()))
        {
            tracing::debug!(endpoint = %self.endpoint, "the server declined COMPRESS DEFLATE");
            return Ok(());
        }

        // Nothing follows the `OK` in the clear, so the parser holds no
        // plaintext that belongs to the compressed stream.
        DeflateStream::wrap(&mut self.stream);
        tracing::debug!(endpoint = %self.endpoint, "COMPRESS DEFLATE active");
        Ok(())
    }

    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
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

    pub async fn refresh_capabilities(&mut self) -> BackendResult<&Capabilities> {
        let advertised = self.run(ImapCapabilityGet::new()).await;
        let advertised = advertised.map_err(|error| self.command_error("CAPABILITY", error))?;

        self.capabilities = post_auth_capabilities(&self.endpoint, &advertised)?;
        Ok(&self.capabilities)
    }

    pub(super) fn command_timeout(&self) -> std::time::Duration {
        self.command_timeout
    }

    pub(super) fn set_command_timeout(&mut self, timeout: std::time::Duration) {
        self.command_timeout = timeout;
    }

    /// `io-imap` reports our own timeout as a plain I/O error; recovered here so a log tells "the
    /// server stopped answering" from "the connection dropped".
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
            let mut tag: Tag<'static>;
            self.timed_out = None;

            loop {
                match coroutine.resume(&mut self.fragmentizer, resume.take()) {
                    ImapCoroutineState::Complete(Ok(value)) => return Ok(value),
                    ImapCoroutineState::Complete(Err(error)) => return Err(error.into()),
                    ImapCoroutineState::Yielded(ImapYield::WantsTag) => {
                        tag = self.tags.generate();
                        resume = Some(tag.as_ref().as_bytes());
                    }
                    ImapCoroutineState::Yielded(ImapYield::WantsRead) => {
                        let timeout = self.command_timeout;
                        let read = match self
                            .stream
                            .read_within(&mut buffer, timeout, "a command")
                            .await
                        {
                            Ok(read) => read,
                            Err(error) => {
                                let error: BackendError = error.into();
                                let carried = std::io::Error::other(error.to_string());
                                self.timed_out = Some(error);
                                return Err(ImapClientError::Io(carried));
                            }
                        };
                        resume = Some(&buffer[..read]);
                    }
                    ImapCoroutineState::Yielded(ImapYield::WantsWrite(bytes)) => {
                        let timeout = self.command_timeout;
                        if let Err(error) =
                            self.stream.write_within(&bytes, timeout, "a command").await
                        {
                            if matches!(error, TransportError::TimedOut { .. }) {
                                let error: BackendError = error.into();
                                let carried = std::io::Error::other(error.to_string());
                                self.timed_out = Some(error);
                                return Err(ImapClientError::Io(carried));
                            }
                            return Err(as_io(error));
                        }
                    }
                }
            }
        }
    }
}

/// io-imap completes `Ok` however much it failed to read: unscanned lines are `skipped`, a `FETCH`
/// with an undecoded item is `partial`. iCloud has sent `FETCH` sequence numbers no decoder accepts
/// under QRESYNC.
pub(crate) fn undecoded<T>(output: &ImapOutput<T>) -> u64 {
    (output.skipped.len() + output.partial.len()) as u64
}

/// A message with an undecoded item is taken out: storing it would store something the server did
/// not say.
pub(crate) fn whole_messages<T>(
    mut output: ImapOutput<BTreeMap<NonZeroU32, T>>,
) -> (BTreeMap<NonZeroU32, T>, u64) {
    let undecoded = undecoded(&output);
    for partial in &output.partial {
        output.data.remove(&partial.seq);
    }
    (output.data, undecoded)
}

/// `io-imap` returns `Ok(vec![])` when the capability round trip was skipped, which downstream
/// gates would read as "supports nothing".
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

/// A rejected password must come out as `Auth`, or a backoff loop never ends.
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
        // A rejected bearer token must arrive as `Auth`, which stops blind retries and drives
        // `TokenSource` invalidation (#193); as `Protocol` a stale token would be retried for ever.
        ImapSessionOpenError::AuthOauthbearer(inner) if is_rejection_oauthbearer(&inner) => {
            BackendError::Auth {
                account,
                reason: inner.to_string(),
            }
        }
        ImapSessionOpenError::AuthXoauth2(inner) if is_rejection_xoauth2(&inner) => {
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
        other => {
            let reason = other.to_string();
            // A throttle is not a refusal: `Rejected` is not transient, `RateLimited` is. IMAP has
            // no status code for "slow down", so it is told apart by wording. iCloud answers a
            // sustained backfill with `NO Service temporarily unavailable` (#1438).
            if reads_as_throttling(&reason) {
                return BackendError::RateLimited {
                    // No `retry_after`: iCloud names no interval, and the backoff loop has its own
                    // schedule.
                    retry_after: None,
                    reason,
                };
            }
            // Silence is not a refusal: an exchange with no tagged response did not complete, and
            // `Rejected` would claim the server said no. `Disconnected` is transient, so the queue
            // retries rather than losing the mutation; a duplicate copy is visible and reversible.
            if reads_as_incomplete(&reason) {
                return BackendError::Disconnected {
                    context: command.to_owned(),
                    reason,
                };
            }
            BackendError::Rejected {
                command: command.to_owned(),
                reason,
            }
        }
    }
}

/// Matched on wording: IMAP has no throttling response code. Kept narrow, since a permanent refusal
/// must not be retried for ever.
fn reads_as_throttling(reason: &str) -> bool {
    const PHRASES: &[&str] = &[
        "service temporarily unavailable",
        "try again later",
        "too many simultaneous connections",
        "temporarily deferred",
        // The RFC 5530 response code, when a server bothers to send it.
        "[unavailable]",
        "[inuse]",
        "[limit]",
    ];
    let lowered = reason.to_ascii_lowercase();
    PHRASES.iter().any(|phrase| lowered.contains(phrase))
}

/// io-imap's `MissingTagged` means no answer, no refusal and no way to know whether the server
/// acted. Grouping it with refusals made an archive vanish between Postio and iCloud. Matched on
/// wording since every per-command variant renders the same phrase.
fn reads_as_incomplete(reason: &str) -> bool {
    reason
        .to_ascii_lowercase()
        .contains("did not return a tagged response")
}

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

/// `NoWithError` is the ordinary case: both bearer mechanisms report a rejected token with an error
/// challenge then `NO` (RFC 7628 §3.2.3); matching only `No` would leave a token that never
/// refreshes.
fn is_rejection_oauthbearer(error: &ImapAuthOauthbearerError) -> bool {
    matches!(
        error,
        ImapAuthOauthbearerError::No(_)
            | ImapAuthOauthbearerError::NoWithError { .. }
            | ImapAuthOauthbearerError::Bad(_)
            | ImapAuthOauthbearerError::Bye(_)
    )
}

fn is_rejection_xoauth2(error: &ImapAuthXoauth2Error) -> bool {
    matches!(
        error,
        ImapAuthXoauth2Error::No(_)
            | ImapAuthXoauth2Error::NoWithError { .. }
            | ImapAuthXoauth2Error::Bad(_)
            | ImapAuthXoauth2Error::Bye(_)
    )
}

/// Every branch passes the secret through `credential_copy` only, so its zeroization holds for the
/// bearer mechanisms too.
fn sasl_for(settings: &ConnectionSettings, password: &Password) -> Sasl {
    match settings.auth {
        AuthMethod::Password | AuthMethod::AppPassword => Sasl::Plain(SaslPlainCreds {
            authzid: None,
            authcid: settings.username.clone(),
            passwd: credential_copy(password).into(),
        }),
        // RFC 7628: `host` and `port` go verbatim into the GS2 header, so they must be the server
        // actually contacted.
        AuthMethod::OAuth2 => Sasl::Oauthbearer(SaslOauthbearerCreds {
            username: settings.username.clone(),
            host: settings.host.clone(),
            port: settings.port,
            token: credential_copy(password).into(),
        }),
        AuthMethod::XOAuth2 => Sasl::Xoauth2(SaslXoauth2Creds {
            username: settings.username.clone(),
            token: credential_copy(password).into(),
        }),
    }
}

/// The one copy of the secret this module makes, and it must stay exactly one `to_owned`:
/// `SecretString::from(String)` reallocates when capacity exceeds length, freeing the buffer
/// without overwriting it. Spare capacity (`with_capacity`, `push_str`, `format!`) breaks that
/// silently; the test below catches it (#144).
fn credential_copy(password: &Password) -> String {
    password.expose().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Capability;

    #[test]
    fn the_handshake_copy_of_a_password_has_no_spare_capacity() {
        let copy = credential_copy(&Password::new("hunter2"));
        assert_eq!(copy, "hunter2");
        assert_eq!(
            copy.capacity(),
            copy.len(),
            "the copy handed to io-sasl has spare capacity, so building a \
             SecretString from it reallocates and leaves the secret behind in \
             the freed buffer"
        );
    }

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
                io_imap::send::ImapStatusError {
                    kind: io_imap::types::response::StatusKind::No,
                    code: None,
                    text: "[AUTHENTICATIONFAILED] Authentication failed".to_owned(),
                },
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

#[cfg(test)]
mod throttling_is_not_refusal {
    use super::*;

    /// iCloud answers a sustained body backfill with `NO Service temporarily unavailable`; as
    /// `Rejected` it ended the mailbox's sync and mail never arrived (#1438).
    #[test]
    fn a_server_asking_us_to_slow_down_is_transient() {
        for reason in [
            "NO Service temporarily unavailable",
            "NO [UNAVAILABLE] System busy",
            "NO Too many simultaneous connections",
            "NO please try again later",
            "NO [INUSE] Mailbox in use",
        ] {
            assert!(
                reads_as_throttling(reason),
                "{reason:?} should read as throttling"
            );
            let error = BackendError::RateLimited {
                retry_after: None,
                reason: reason.to_owned(),
            };
            assert!(
                error.is_transient(),
                "{reason:?} must be retried, or the sync ends for the session"
            );
        }
    }

    #[test]
    fn a_permanent_refusal_is_not_retried() {
        for reason in [
            "NO Mailbox does not exist",
            "NO Permission denied",
            "NO Message not found",
            "BAD Invalid command",
        ] {
            assert!(
                !reads_as_throttling(reason),
                "{reason:?} is permanent and must not be retried for ever"
            );
        }
    }
}

#[cfg(test)]
mod silence_is_not_refusal {
    use super::*;

    /// A missing tagged response is retried, not given up on: flattened into `Rejected`, the queue
    /// marked the row failed on the first attempt and local and remote diverged for good.
    #[test]
    fn an_exchange_without_a_tagged_response_is_transient() {
        for reason in [
            "IMAP COPY failed: server did not return a tagged response",
            "IMAP STORE failed: server did not return a tagged response",
            "IMAP SEARCH failed: server did not return a tagged response",
        ] {
            assert!(
                reads_as_incomplete(reason),
                "{reason:?} should read as an incomplete exchange"
            );
            let error = BackendError::Disconnected {
                context: "COPY".to_owned(),
                reason: reason.to_owned(),
            };
            assert!(
                error.is_transient(),
                "{reason:?} must be retried, or the mutation is lost silently"
            );
        }
    }

    #[test]
    fn a_real_answer_is_not_mistaken_for_silence() {
        for reason in [
            "IMAP COPY failed: NO Mailbox does not exist",
            "IMAP COPY failed: BAD Invalid command",
            "IMAP FETCH failed: NO Service temporarily unavailable",
        ] {
            assert!(
                !reads_as_incomplete(reason),
                "{reason:?} is an answer, not silence"
            );
        }
    }
}
