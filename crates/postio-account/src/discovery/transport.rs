//! The probe never opens a socket itself; a mocked `DiscoveryTransport` runs the default suite with
//! no network.
//! Cancellation reaches the socket through `Cancellable`, since a dropped `spawn_blocking` handle
//! only detaches the task; `DISCOVERY_IO_TIMEOUT` bounds a peer that accepts and goes silent (#57,
//! #281).

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use async_trait::async_trait;

use crate::cancel::CancelToken;

pub use io_pim_discovery::autoconfig::config::DiscoveryAutoconfig;
pub use io_pim_discovery::rfc6186::service::{DiscoverySrvReport, DiscoverySrvService};

use crate::discovery::ProbeStep;

#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct TransportError {
    message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoconfigEndpoint<'a> {
    WellKnown {
        domain: &'a str,
    },
    Subdomain {
        local_part: &'a str,
        domain: &'a str,
    },
    Ispdb {
        domain: &'a str,
    },
}

impl AutoconfigEndpoint<'_> {
    pub fn step(&self) -> ProbeStep {
        match self {
            Self::WellKnown { .. } => ProbeStep::WellKnown,
            Self::Subdomain { .. } => ProbeStep::AutoconfigSubdomain,
            Self::Ispdb { .. } => ProbeStep::Ispdb,
        }
    }

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

enum OwnedEndpoint {
    WellKnown(String),
    Subdomain(String, String),
    Ispdb(String),
}

/// Every method takes the caller's `CancelToken`: a transport that cannot see it cannot stop the
/// request it started (#57).
#[async_trait]
pub trait DiscoveryTransport: Send + Sync {
    async fn autoconfig(
        &self,
        endpoint: AutoconfigEndpoint<'_>,
        cancel: &CancelToken,
    ) -> Result<DiscoveryAutoconfig, TransportError>;

    async fn srv(
        &self,
        domain: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoverySrvReport, TransportError>;

    /// Hostnames only. An empty list is a domain with no MX: a miss, not a failure.
    async fn mx(&self, domain: &str, cancel: &CancelToken) -> Result<Vec<String>, TransportError>;
}

/// The ceiling on an abandoned request's life, close to the probe's per-step budget: it bounds
/// silence, not slowness, since each read and write gets it afresh.
pub const DISCOVERY_IO_TIMEOUT: Duration = Duration::from_secs(5);

const CANCELLED: &str = "the discovery probe was cancelled";

/// The error kind is not `Interrupted`: `std` loops on it ("retry me"), so a cancelled stream would
/// spin forever. `ConnectionAborted` is not retried by any layer.
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

#[derive(Clone)]
pub struct PimalayaTransport {
    resolver: url::Url,
    /// Discovery events carry no account id; the sink records them as pre-account traffic (#151).
    egress: Option<std::sync::Arc<dyn postio_model::egress::EgressSink>>,
}

const FALLBACK_RESOLVER: &str = "tcp://1.1.1.1:53";

impl PimalayaTransport {
    pub fn new() -> Self {
        let resolver = io_pim_discovery::shared::dns::system_resolver().unwrap_or_else(|| {
            FALLBACK_RESOLVER
                .parse()
                .expect("the fallback resolver URL is valid")
        });

        Self::with_resolver(resolver)
    }

    pub fn with_resolver(resolver: url::Url) -> Self {
        Self {
            resolver,
            egress: None,
        }
    }

    pub fn with_egress(
        mut self,
        sink: std::sync::Arc<dyn postio_model::egress::EgressSink>,
    ) -> Self {
        self.egress = Some(sink);
        self
    }

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

/// `Retry::Until` arms the socket's read deadline too; the default leaves a minute per read for an
/// abandoned request.
pub(crate) fn connect_options() -> pimalaya_stream::stream::TcpConnectOptions {
    pimalaya_stream::stream::TcpConnectOptions {
        retry: pimalaya_stream::retry::Retry::Until(DISCOVERY_IO_TIMEOUT),
        ..Default::default()
    }
}

/// `pimalaya-stream` connects with no deadline and cannot be given a socket, so the deadline is
/// spent here first.
pub(crate) fn reach(
    host: &str,
    port: u16,
    cancel: &CancelToken,
    timeout: Duration,
) -> anyhow::Result<()> {
    let mut last: Option<io::Error> = None;
    for address in (host, port).to_socket_addrs()? {
        if cancel.is_cancelled() {
            anyhow::bail!("{CANCELLED}");
        }
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(_) => return Ok(()),
            Err(err) => last = Some(err),
        }
    }
    Err(match last {
        Some(err) => err.into(),
        None => anyhow::anyhow!("`{host}` resolved to no address"),
    })
}

pub(crate) fn connect_https(
    host: &str,
    port: u16,
    cancel: &CancelToken,
    timeout: Duration,
) -> anyhow::Result<Cancellable<pimalaya_stream::stream::Stream>> {
    reach(host, port, cancel, timeout)?;
    let options = pimalaya_stream::stream::TlsConnectOptions {
        tls: PimalayaTransport::tls(),
        retry: pimalaya_stream::retry::Retry::Until(timeout),
        ..Default::default()
    };
    let stream = pimalaya_stream::stream::Stream::connect_tls(host, port, options)?;
    Ok(Cancellable::new(stream, cancel.clone()))
}

/// The DNS path, written out to keep `io-pim-discovery`'s `tcp` semantics (direct, no proxy) while
/// adding connect, read and write deadlines.
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

    // `TcpStream::connect` has no deadline; resolving first makes one expressible.
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
        use pimalaya_stream::stream::Stream;

        let resolver = self.resolver.clone();
        let endpoint = endpoint.to_owned_endpoint();
        let cancel = cancel.clone();
        let egress = self.egress.clone();

        // The blocking pool keeps the UI responsive; abandoned tasks are not awaited, so every
        // stream carries the token to end the request.
        blocking(move || {
            let http = {
                let cancel = cancel.clone();
                let egress = egress.clone();
                move |url: &url::Url| -> anyhow::Result<Cancellable<Stream>> {
                    let host = url
                        .host_str()
                        .ok_or_else(|| anyhow::anyhow!("HTTP URL `{url}` has no host"))?;
                    let port = url.port_or_known_default().unwrap_or(80);
                    let stream = reach(host, port, &cancel, DISCOVERY_IO_TIMEOUT)
                        .and_then(|()| Stream::connect_tcp(host, port, connect_options()));
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
                    let stream = connect_https(host, port, &cancel, DISCOVERY_IO_TIMEOUT);
                    report_egress(&egress, host, port, stream.is_ok());
                    stream
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

            // Registered after `new`, which installs the plain defaults; the `mailconf` TXT
            // redirect can reach DNS too.
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

    async fn mx(&self, domain: &str, cancel: &CancelToken) -> Result<Vec<String>, TransportError> {
        let resolver = self.resolver.clone();
        let query = domain.to_owned();
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
            mx_lookup(&query, resolver, tcp).map_err(|err| transport_error(&cancel, err))
        })
        .await
    }
}

/// `io-pim-discovery` has no MX lookup, so the query goes through its `DiscoveryDnsExchange`
/// extension point: one resolver, one set of sockets the egress log sees (#94).
fn mx_lookup<F, S>(domain_name: &str, resolver: url::Url, factory: F) -> Result<Vec<String>, String>
where
    F: FnMut(&url::Url) -> anyhow::Result<S> + 'static,
    S: io_pim_discovery::shared::pool::DiscoveryStream + 'static,
{
    use domain::new::base::name::{NameCompressor, RevNameBuf};
    use domain::new::base::parse::MessageParser;
    use domain::new::base::wire::{AsBytes, U16};
    use domain::new::base::{
        HeaderFlags, MessageItem, QClass, QType, Question, build::MessageBuilder,
    };
    use domain::new::rdata::RecordData;
    use io_pim_discovery::coroutine::{
        DiscoveryCoroutine, DiscoveryCoroutineState, DiscoveryYield,
    };
    use io_pim_discovery::shared::dns::DiscoveryDnsExchange;
    use io_pim_discovery::shared::pool::DiscoveryStreamPool;

    // `RevNameBuf` rejects a relative name, and the caller's domain is always
    // relative — the same `absolute_name` step the crate's own builders take.
    let absolute = if domain_name.ends_with('.') {
        domain_name.to_owned()
    } else {
        format!("{domain_name}.")
    };
    let qname = absolute
        .parse::<RevNameBuf>()
        .map_err(|err| format!("`{domain_name}` is not a domain name: {err}"))?;

    let mut buf = vec![0u8; QUERY_BUF_SIZE];
    let mut compressor = NameCompressor::default();
    let mut builder = MessageBuilder::new(
        &mut buf,
        &mut compressor,
        U16::new(1),
        *HeaderFlags::default().set_rd(true),
    );
    builder
        .push_question(&Question {
            qname,
            qtype: QType::MX,
            qclass: QClass::IN,
        })
        .map_err(|err| format!("the MX query did not fit: {err}"))?;
    let message = builder.finish().as_bytes().to_vec();

    let mut pool = DiscoveryStreamPool::new().with_factory("tcp", factory);
    let mut exchange = DiscoveryDnsExchange::new(message, resolver);
    let mut read = [0u8; MX_READ_BUFFER_SIZE];
    let mut arg: Option<&[u8]> = None;
    let response = loop {
        match exchange.resume(arg.take()) {
            DiscoveryCoroutineState::Complete(Ok(response)) => break response,
            DiscoveryCoroutineState::Complete(Err(err)) => {
                return Err(format!("the MX lookup failed: {err}"));
            }
            DiscoveryCoroutineState::Yielded(DiscoveryYield::WantsRead { url }) => {
                let stream = pool.get(&url).map_err(|err| err.to_string())?;
                let n = stream.read(&mut read).map_err(|err| err.to_string())?;
                arg = Some(&read[..n]);
            }
            DiscoveryCoroutineState::Yielded(DiscoveryYield::WantsWrite { url, bytes }) => {
                let stream = pool.get(&url).map_err(|err| err.to_string())?;
                stream.write_all(&bytes).map_err(|err| err.to_string())?;
            }
        }
    };

    // Preference ascending: RFC 5321 §5.1 says the lowest number is tried
    // first, and "where this domain's mail actually goes" is the primary.
    let mut found: Vec<(u16, String)> = Vec::new();
    let parser = MessageParser::new(&response)
        .map_err(|err| format!("the MX answer did not parse: {err}"))?;
    for item in parser {
        let Ok(MessageItem::Answer(record)) = item else {
            continue;
        };
        let RecordData::Mx(mx) = record.rdata else {
            continue;
        };
        found.push((mx.preference.get(), mx.exchange.to_string()));
    }
    found.sort_by_key(|(preference, _)| *preference);
    Ok(found
        .into_iter()
        .map(|(_, host)| host.trim_end_matches('.').to_owned())
        .collect())
}

const QUERY_BUF_SIZE: usize = 4 * 1024;

const MX_READ_BUFFER_SIZE: usize = 8 * 1024;

/// A cancellation is reported as such so the log does not say "the domain has no autoconfig".
fn transport_error(cancel: &CancelToken, err: impl std::fmt::Display) -> TransportError {
    if cancel.is_cancelled() {
        return TransportError::new(CANCELLED);
    }
    TransportError::new(err.to_string())
}

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
        // `Interrupted` means "retry me" throughout std, so reporting it would spin a cancelled
        // stream forever.
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
        // Must outlast the step budget, or the socket deadline would cut healthy requests short.
        assert!(DISCOVERY_IO_TIMEOUT >= super::super::ProbeOptions::default().step_timeout);
    }

    use std::net::TcpListener;
    use std::time::Instant;

    const SHORT: Duration = Duration::from_millis(400);

    #[test]
    fn a_listening_host_is_reachable() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        reach("127.0.0.1", port, &CancelToken::new(), SHORT).expect("accepts");
    }

    #[test]
    fn a_host_that_refuses_is_not_reachable() {
        let port = TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(reach("127.0.0.1", port, &CancelToken::new(), SHORT).is_err());
    }

    #[test]
    fn a_cancelled_probe_does_not_connect() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();

        let err = reach("127.0.0.1", port, &cancel, SHORT).expect_err("cancelled");
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(listener.accept().is_err(), "no connection was made");
    }

    #[test]
    fn https_to_a_host_that_never_handshakes_gives_up_in_time() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let held = std::thread::spawn(move || {
            let (socket, _) = listener.accept().unwrap();
            // POSTIO-NOT-A-WAIT: the held socket is the subject; the client gives up in under 1.2 s
            std::thread::sleep(Duration::from_millis(1500));
            drop(socket);
        });

        let started = Instant::now();
        // The handshake is lazy: it runs on the first read or write.
        let mut stream = connect_https("127.0.0.1", port, &CancelToken::new(), SHORT)
            .expect("the connect itself succeeds");
        assert!(stream.write_all(b"GET / HTTP/1.1\r\n\r\n").is_err() || stream.flush().is_err());
        assert!(
            started.elapsed() < Duration::from_millis(1200),
            "took {:?}",
            started.elapsed()
        );
        held.join().unwrap();
    }
}
