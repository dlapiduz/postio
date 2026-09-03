//! The resolver/HTTP layer the probe runs on.
//!
//! The probe never opens a socket itself. It asks a [`DiscoveryTransport`]
//! for an autoconfig document or an SRV report and does the rest — ordering,
//! timeouts, cancellation, mapping — in pure code. That is what lets the
//! whole default test suite run with a mocked transport and no network.
//!
//! [`PimalayaTransport`] is the real one. It delegates to `io-pim-discovery`,
//! which already implements Mozilla autoconfig (all three endpoints) and
//! RFC 6186 SRV lookups; the only thing it adds is moving those blocking
//! clients onto the blocking pool so an async caller is never stalled.
//!
//! # Cancelling a blocking call you cannot abort
//!
//! Moving a blocking call onto [`tokio::task::spawn_blocking`] keeps the
//! *caller's* runtime responsive; it does not make the call itself
//! cancellable. [`blocking`] awaits the spawned task's `JoinHandle`, and
//! [`Probe::attempt`](super::Probe::attempt) races that await against a
//! timeout and the caller's [`CancelToken`] — so
//! when either wins, this function's own future is dropped, but the
//! `JoinHandle` being dropped only *detaches* the spawned task. Nothing can
//! abort it; tokio has no way to interrupt a thread mid-`read`.
//!
//! So the socket is cancelled instead of the task. `io-pim-discovery` owns
//! the protocol but not the stream: both of its std clients take a
//! [`with_factory`](io_pim_discovery::autoconfig::client::DiscoveryAutoconfigClientStd::with_factory)
//! hook, and `DiscoveryStream` is nothing more than `Read + Write`. This
//! module therefore hands them a [`Cancellable`] stream of its own, which
//! checks the probe's token before every read and every write and fails the
//! exchange the moment it is set. The detached task then unwinds through
//! `io-pim-discovery`'s own error path and drops its socket, rather than
//! running to whatever that client decides is done.
//!
//! Two bounds rather than one, because a check between reads does nothing
//! while a read is parked:
//!
//! * **Cancellation** ends the request at its next I/O boundary — for an
//!   autoconfig fetch, essentially immediately.
//! * **[`DISCOVERY_IO_TIMEOUT`]** ends it anyway if the peer accepted the
//!   connection and then went silent. Before this, that case had no bound at
//!   all: `pimalaya-stream`'s default `Retry` budget is a minute *per
//!   read*, and the DNS path armed no socket deadline whatsoever.
//!
//! What is still not bounded is `connect` on the HTTPS path.
//! `Stream::connect_tcp`/`connect_tls` take no connect deadline, so a peer
//! that accepts nothing leaves the OS default (a couple of minutes) as the
//! ceiling. The DNS path does not have that hole — it resolves and calls
//! [`TcpStream::connect_timeout`] itself. See #57.
//!
//! `postio-iigq` is the audit that found the original defect.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use async_trait::async_trait;

use crate::cancel::CancelToken;

pub use io_pim_discovery::autoconfig::config::DiscoveryAutoconfig;
pub use io_pim_discovery::rfc6186::service::{DiscoverySrvReport, DiscoverySrvService};

use crate::discovery::ProbeStep;

/// Whatever went wrong reaching a discovery endpoint.
///
/// Intentionally opaque: the probe treats every transport failure the same
/// way (move on to the next step) and only keeps the message for the report.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct TransportError {
    message: String,
}

impl TransportError {
    /// Builds a transport error from a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The underlying message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Which autoconfig URL to fetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoconfigEndpoint<'a> {
    /// `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`, served
    /// by the domain itself.
    WellKnown {
        /// The domain part of the address.
        domain: &'a str,
    },
    /// `https://autoconfig.<domain>/mail/config-v1.1.xml?emailaddress=...`,
    /// the ISP-hosted variant. Needs the local part, which some providers use
    /// to return per-user settings.
    Subdomain {
        /// The local part of the address.
        local_part: &'a str,
        /// The domain part of the address.
        domain: &'a str,
    },
    /// `https://autoconfig.thunderbird.net/v1.1/<domain>`, the community
    /// database.
    Ispdb {
        /// The domain part of the address.
        domain: &'a str,
    },
}

impl AutoconfigEndpoint<'_> {
    /// The probe step this endpoint belongs to.
    pub fn step(&self) -> ProbeStep {
        match self {
            Self::WellKnown { .. } => ProbeStep::WellKnown,
            Self::Subdomain { .. } => ProbeStep::AutoconfigSubdomain,
            Self::Ispdb { .. } => ProbeStep::Ispdb,
        }
    }

    /// The domain being probed.
    pub fn domain(&self) -> &str {
        match self {
            Self::WellKnown { domain }
            | Self::Subdomain { domain, .. }
            | Self::Ispdb { domain } => domain,
        }
    }

    fn to_owned_endpoint(self) -> OwnedEndpoint {
        match self {
            Self::WellKnown { domain } => OwnedEndpoint::WellKnown(domain.to_owned()),
            Self::Subdomain { local_part, domain } => {
                OwnedEndpoint::Subdomain(local_part.to_owned(), domain.to_owned())
            }
            Self::Ispdb { domain } => OwnedEndpoint::Ispdb(domain.to_owned()),
        }
    }
}

/// Owned mirror of [`AutoconfigEndpoint`], so the borrowed form can cross
/// into a blocking task.
enum OwnedEndpoint {
    WellKnown(String),
    Subdomain(String, String),
    Ispdb(String),
}

/// The network operations discovery needs.
///
/// Every method takes the caller's [`CancelToken`]. It is not decoration: a
/// transport that cannot see the token cannot stop the request it started,
/// and the probe abandoning its *await* leaves the socket open. #57.
#[async_trait]
pub trait DiscoveryTransport: Send + Sync {
    /// Fetches and parses one Mozilla autoconfig document.
    async fn autoconfig(
        &self,
        endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError>;

    /// Runs the RFC 6186 SRV lookups for `domain`.
    async fn srv(
        &self,
        domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError>;
}

// ---------------------------------------------------------------------------
// A stream that stops when the probe does
// ---------------------------------------------------------------------------

/// How long a discovery socket may make no progress before it is given up on.
///
/// This is the ceiling on an *abandoned* request's life, so it wants to be
/// close to the probe's own per-step budget
/// ([`ProbeOptions::step_timeout`](super::ProbeOptions::step_timeout), five
/// seconds by default) rather than generous: past that point the probe has
/// already stopped caring about the answer, and every further second is a
/// socket held open for nobody. A healthy exchange never comes near it —
/// each read and each write gets the budget afresh, so it bounds silence,
/// not slowness.
pub const DISCOVERY_IO_TIMEOUT: Duration = Duration::from_secs(5);

/// What a cancelled read or write reports.
const CANCELLED: &str = "the discovery probe was cancelled";

/// Wraps a blocking stream so the probe's cancellation reaches the socket.
///
/// `io-pim-discovery`'s `DiscoveryStream` is `Read + Write` and nothing more,
/// which is what makes this possible: the protocol stays theirs, the stream
/// becomes ours.
///
/// The error kind is deliberately **not** [`io::ErrorKind::Interrupted`].
/// `Interrupted` means "retry me" throughout `std` — `read_to_end` and
/// friends loop on it — so a cancelled stream reporting it would spin
/// forever instead of stopping, which is the exact opposite of the point.
/// [`io::ErrorKind::ConnectionAborted`] says what happened and no layer
/// retries it.
pub(crate) struct Cancellable<S> {
    inner: S,
    cancel: CancelToken,
}

impl<S> Cancellable<S> {
    pub(crate) fn new(inner: S, cancel: CancelToken) -> Self {
        Self { inner, cancel }
    }

    fn check(&self) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, CANCELLED));
        }
        Ok(())
    }
}

impl<S: Read> Read for Cancellable<S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.read(buf)
    }
}

impl<S: Write> Write for Cancellable<S> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.check()?;
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.check()?;
        self.inner.flush()
    }
}

// ---------------------------------------------------------------------------
// The real transport
// ---------------------------------------------------------------------------

/// [`DiscoveryTransport`] backed by `io-pim-discovery`.
#[derive(Clone)]
pub struct PimalayaTransport {
    resolver: url::Url,
    /// Where every connection attempt is reported (#151), or nowhere.
    /// Discovery probes servers for an account that does not exist yet, so
    /// its events carry no account id; the wiring's sink records them as
    /// pre-account traffic.
    egress: Option<std::sync::Arc<dyn postio_model::egress::EgressSink>>,
}

/// Fallback resolver when the host has none we can read (a Flatpak sandbox
/// without `/etc/resolv.conf`, say).
const FALLBACK_RESOLVER: &str = "tcp://1.1.1.1:53";

impl PimalayaTransport {
    /// Builds a transport using the host's own resolver where one can be
    /// found, so split-horizon and corporate DNS resolve the way every other
    /// program on the machine resolves.
    pub fn new() -> Self {
        let resolver = io_pim_discovery::shared::dns::system_resolver().unwrap_or_else(|| {
            FALLBACK_RESOLVER
                .parse()
                .expect("the fallback resolver URL is valid")
        });

        Self::with_resolver(resolver)
    }

    /// Builds a transport against a specific DNS-over-TCP resolver.
    pub fn with_resolver(resolver: url::Url) -> Self {
        Self {
            resolver,
            egress: None,
        }
    }

    /// Report every connection attempt to `sink` — the egress log's seam
    /// (#151), the same contract as the IMAP and SMTP connectors.
    pub fn with_egress(
        mut self,
        sink: std::sync::Arc<dyn postio_model::egress::EgressSink>,
    ) -> Self {
        self.egress = Some(sink);
        self
    }

    /// The autoconfig endpoints speak plain HTTP/1.1 over TLS.
    pub(crate) fn tls() -> pimalaya_stream::tls::Tls {
        pimalaya_stream::tls::Tls {
            rustls: pimalaya_stream::tls::Rustls {
                alpn: vec!["http/1.1".into()],
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// The connect options the HTTP factories use.
///
/// `Retry::Until` is what arms the socket's own read deadline as well as
/// bounding the retry loop above it — connecting with the default would
/// leave a minute per read, which for an abandoned request is a minute of
/// socket held for nobody.
pub(crate) fn connect_options() -> pimalaya_stream::stream::TcpConnectOptions {
    pimalaya_stream::stream::TcpConnectOptions {
        retry: pimalaya_stream::retry::Retry::Until(DISCOVERY_IO_TIMEOUT),
        ..Default::default()
    }
}

/// Opens a plain TCP stream to `url`, bounded and cancellable.
///
/// This is the DNS path. Written out rather than delegating to
/// `pimalaya-stream` so it keeps `io-pim-discovery`'s own semantics for the
/// `tcp` scheme exactly — a direct connection, no proxy — while gaining a
/// connect deadline and read/write deadlines it never had.
pub(crate) fn connect_tcp(
    url: &url::Url,
    cancel: &CancelToken,
) -> anyhow::Result<Cancellable<TcpStream>> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("TCP URL `{url}` has no host"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("TCP URL `{url}` has no port"))?;

    // `TcpStream::connect` tries every resolved address with no deadline, so
    // a host that accepts nothing parks the blocking thread on the OS
    // default. Resolving first is what makes a deadline expressible.
    let mut last: Option<io::Error> = None;
    for address in (host, port).to_socket_addrs()? {
        if cancel.is_cancelled() {
            anyhow::bail!("{CANCELLED}");
        }
        match TcpStream::connect_timeout(&address, DISCOVERY_IO_TIMEOUT) {
            Ok(stream) => {
                stream.set_read_timeout(Some(DISCOVERY_IO_TIMEOUT))?;
                stream.set_write_timeout(Some(DISCOVERY_IO_TIMEOUT))?;
                return Ok(Cancellable::new(stream, cancel.clone()));
            }
            Err(err) => last = Some(err),
        }
    }

    Err(match last {
        Some(err) => err.into(),
        None => anyhow::anyhow!("`{url}` resolved to no address"),
    })
}

impl std::fmt::Debug for PimalayaTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PimalayaTransport")
            .field("resolver", &self.resolver)
            .finish_non_exhaustive()
    }
}

/// Report one discovery connection attempt, if a sink is listening.
fn report_egress(
    egress: &Option<std::sync::Arc<dyn postio_model::egress::EgressSink>>,
    host: &str,
    port: u16,
    connected: bool,
) {
    if let Some(sink) = egress {
        sink.record(postio_model::egress::EgressEvent {
            at: chrono::Utc::now(),
            subsystem: postio_model::egress::EgressSubsystem::Discovery,
            account: None,
            host: host.to_owned(),
            port,
            outcome: if connected {
                postio_model::egress::EgressOutcome::Connected
            } else {
                postio_model::egress::EgressOutcome::Failed
            },
        });
    }
}

impl Default for PimalayaTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DiscoveryTransport for PimalayaTransport {
    async fn autoconfig(
        &self,
        endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        use io_pim_discovery::autoconfig::client::DiscoveryAutoconfigClientStd;
        use pimalaya_stream::stream::{Stream, TlsConnectOptions};

        let resolver = self.resolver.clone();
        let endpoint = endpoint.to_owned_endpoint();
        let cancel = cancel.clone();
        let egress = self.egress.clone();

        // `io-pim-discovery`'s std clients block. Parking them on the
        // blocking pool is what keeps the caller's runtime — and therefore
        // the UI — responsive while a probe is in flight. The probe races
        // this future against its own timeout and the cancel token, and a
        // straggling blocking task is abandoned rather than awaited — which
        // is why every stream below carries the token: abandoning the await
        // has to end the request, not merely stop listening for its answer.
        blocking(move || {
            let http = {
                let cancel = cancel.clone();
                let egress = egress.clone();
                move |url: &url::Url| -> anyhow::Result<Cancellable<Stream>> {
                    let host = url
                        .host_str()
                        .ok_or_else(|| anyhow::anyhow!("HTTP URL `{url}` has no host"))?;
                    let port = url.port_or_known_default().unwrap_or(80);
                    let stream = Stream::connect_tcp(host, port, connect_options());
                    report_egress(&egress, host, port, stream.is_ok());
                    Ok(Cancellable::new(stream?, cancel.clone()))
                }
            };
            let https = {
                let cancel = cancel.clone();
                let egress = egress.clone();
                move |url: &url::Url| -> anyhow::Result<Cancellable<Stream>> {
                    let host = url
                        .host_str()
                        .ok_or_else(|| anyhow::anyhow!("HTTPS URL `{url}` has no host"))?;
                    let port = url.port_or_known_default().unwrap_or(443);
                    let options = TlsConnectOptions {
                        tls: PimalayaTransport::tls(),
                        retry: pimalaya_stream::retry::Retry::Until(DISCOVERY_IO_TIMEOUT),
                        ..Default::default()
                    };
                    let stream = Stream::connect_tls(host, port, options);
                    report_egress(&egress, host, port, stream.is_ok());
                    Ok(Cancellable::new(stream?, cancel.clone()))
                }
            };
            let tcp = {
                let cancel = cancel.clone();
                let egress = egress.clone();
                move |url: &url::Url| {
                    let stream = connect_tcp(url, &cancel);
                    if let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default())
                    {
                        report_egress(&egress, host, port, stream.is_ok());
                    }
                    stream
                }
            };

            // Registered after `new`, which installs the plain defaults, so
            // these replace them. The autoconfig flow can reach DNS too (the
            // `mailconf` TXT redirect), hence the `tcp` factory here as well.
            let mut client = DiscoveryAutoconfigClientStd::new(resolver)
                .with_factory("tcp", tcp)
                .with_factory("http", http)
                .with_factory("https", https);

            let result = match &endpoint {
                OwnedEndpoint::WellKnown(domain) => client.isp_fallback(domain, true),
                OwnedEndpoint::Subdomain(local_part, domain) => {
                    client.isp(local_part, domain, true)
                }
                OwnedEndpoint::Ispdb(domain) => client.ispdb(domain, true),
            };

            result.map_err(|err| transport_error(&cancel, err))
        })
        .await
    }

    async fn srv(
        &self,
        domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError> {
        use io_pim_discovery::rfc6186::client::DiscoverySrvClientStd;

        let resolver = self.resolver.clone();
        let domain = domain.to_owned();
        let cancel = cancel.clone();
        let egress = self.egress.clone();

        blocking(move || {
            let tcp = {
                let cancel = cancel.clone();
                move |url: &url::Url| {
                    let stream = connect_tcp(url, &cancel);
                    if let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default())
                    {
                        report_egress(&egress, host, port, stream.is_ok());
                    }
                    stream
                }
            };
            let mut client = DiscoverySrvClientStd::new(resolver).with_factory("tcp", tcp);
            client
                .discover(&domain)
                .map_err(|err| transport_error(&cancel, err))
        })
        .await
    }
}

/// Turns a client failure into a [`TransportError`], saying so when the
/// reason was this probe stopping rather than the network.
///
/// The distinction never reaches the user — [`crate::discovery::Probe::run`] has already
/// returned `Cancelled` by the time this lands — but it is the difference
/// between a log that reads "the domain has no autoconfig" and one that reads
/// "we stopped asking", and only one of those is true.
fn transport_error(cancel: &CancelToken, err: impl std::fmt::Display) -> TransportError {
    if cancel.is_cancelled() {
        return TransportError::new(CANCELLED);
    }
    TransportError::new(err.to_string())
}

/// Runs a blocking discovery call on the blocking pool.
async fn blocking<T, F>(op: F) -> Result<T, TransportError>
where
    F: FnOnce() -> Result<T, TransportError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(op).await {
        Ok(result) => result,
        Err(err) => Err(TransportError::new(format!(
            "the discovery task did not finish: {err}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_knows_its_step_and_domain() {
        assert_eq!(
            AutoconfigEndpoint::WellKnown {
                domain: "example.org"
            }
            .step(),
            ProbeStep::WellKnown
        );
        assert_eq!(
            AutoconfigEndpoint::Subdomain {
                local_part: "a",
                domain: "example.org",
            }
            .step(),
            ProbeStep::AutoconfigSubdomain
        );
        assert_eq!(
            AutoconfigEndpoint::Ispdb {
                domain: "example.org"
            }
            .domain(),
            "example.org"
        );
    }

    #[test]
    fn the_fallback_resolver_parses() {
        assert!(FALLBACK_RESOLVER.parse::<url::Url>().is_ok());
    }

    // -- Cancellation reaching the socket (#57) ---------------------------

    /// A stream that would happily serve bytes forever, so the only thing
    /// that can stop a read is the token.
    struct Endless;

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(b'x');
            Ok(buf.len())
        }
    }

    impl Write for Endless {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_uncancelled_stream_is_the_stream_underneath() {
        let cancel = CancelToken::new();
        let mut stream = Cancellable::new(Endless, cancel);
        let mut buf = [0u8; 4];

        assert_eq!(stream.read(&mut buf).unwrap(), 4);
        assert_eq!(&buf, b"xxxx");
        assert_eq!(stream.write(b"hello").unwrap(), 5);
        assert!(stream.flush().is_ok());
    }

    #[test]
    fn a_cancelled_stream_stops_reading_and_writing() {
        let cancel = CancelToken::new();
        let mut stream = Cancellable::new(Endless, cancel.clone());
        let mut buf = [0u8; 4];

        assert!(stream.read(&mut buf).is_ok(), "not cancelled yet");
        cancel.cancel();

        for outcome in [
            stream.read(&mut buf).map(|_| ()),
            stream.write(b"hello").map(|_| ()),
            stream.flush(),
        ] {
            let err = outcome.expect_err("a cancelled stream refuses");
            assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted);
            assert!(err.to_string().contains("cancelled"), "{err}");
        }
    }

    #[test]
    fn a_cancelled_stream_never_reports_interrupted() {
        // `Interrupted` means "retry me" throughout std -- `read_to_end` and
        // friends loop on it -- so reporting it would spin a cancelled
        // stream forever instead of stopping it. This is the one wrong
        // answer that looks right.
        let cancel = CancelToken::new();
        cancel.cancel();
        let mut stream = Cancellable::new(Endless, cancel);

        let err = stream.read(&mut [0u8; 4]).expect_err("refused");
        assert_ne!(err.kind(), io::ErrorKind::Interrupted);
    }

    #[test]
    fn a_cancelled_probe_is_not_reported_as_a_network_failure() {
        let cancel = CancelToken::new();
        assert_eq!(
            transport_error(&cancel, "connection refused").message(),
            "connection refused"
        );

        cancel.cancel();
        assert_eq!(
            transport_error(&cancel, "connection refused").message(),
            CANCELLED,
            "a request we stopped must not be logged as a domain that failed"
        );
    }

    #[test]
    fn the_io_timeout_covers_a_default_probe_step() {
        // The bound only means anything if it outlasts the step budget the
        // probe itself uses -- otherwise the socket deadline would cut
        // healthy requests short instead of bounding abandoned ones.
        assert!(DISCOVERY_IO_TIMEOUT >= super::super::ProbeOptions::default().step_timeout);
    }
}
