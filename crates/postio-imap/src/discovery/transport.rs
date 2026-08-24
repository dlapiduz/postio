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
//! # What "never stalled" does not cover
//!
//! Moving a blocking call onto [`tokio::task::spawn_blocking`] keeps the
//! *caller's* runtime responsive; it does not make the call itself
//! cancellable. [`blocking`] awaits the spawned task's `JoinHandle`, and
//! [`Probe::attempt`](super::Probe::attempt) races that await against a
//! timeout and the caller's [`CancelToken`](crate::cancel::CancelToken) — so
//! when either wins, this function's own future is dropped, but the
//! `JoinHandle` being dropped only *detaches* the spawned task. Nothing
//! aborts it: `io-pim-discovery`'s std client keeps running on its blocking
//! thread, with whatever socket it opened still open, for as long as that
//! client itself takes to finish or fail. `postio-iigq` is the audit that
//! found this; `postio-brp.2` is the follow-up, since a real fix needs a
//! transport this crate can actually cancel rather than `io-pim-discovery`'s
//! blocking one.

use async_trait::async_trait;

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
#[async_trait]
pub trait DiscoveryTransport: Send + Sync {
    /// Fetches and parses one Mozilla autoconfig document.
    async fn autoconfig(
        &self,
        endpoint: AutoconfigEndpoint<'_>,
    ) -> Result<DiscoveryAutoconfig, TransportError>;

    /// Runs the RFC 6186 SRV lookups for `domain`.
    async fn srv(&self, domain: &str) -> Result<DiscoverySrvReport, TransportError>;
}

// ---------------------------------------------------------------------------
// The real transport
// ---------------------------------------------------------------------------

/// [`DiscoveryTransport`] backed by `io-pim-discovery`.
#[derive(Clone, Debug)]
pub struct PimalayaTransport {
    resolver: url::Url,
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
        Self { resolver }
    }

    /// The autoconfig endpoints speak plain HTTP/1.1 over TLS.
    fn tls() -> pimalaya_stream::tls::Tls {
        pimalaya_stream::tls::Tls {
            rustls: pimalaya_stream::tls::Rustls {
                alpn: vec!["http/1.1".into()],
                ..Default::default()
            },
            ..Default::default()
        }
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
    ) -> Result<DiscoveryAutoconfig, TransportError> {
        use io_pim_discovery::autoconfig::client::DiscoveryAutoconfigClientStd;

        let resolver = self.resolver.clone();
        let endpoint = endpoint.to_owned_endpoint();

        // `io-pim-discovery`'s std clients block. Parking them on the
        // blocking pool is what keeps the caller's runtime — and therefore
        // the UI — responsive while a probe is in flight. The probe races
        // this future against its own timeout and the cancel token, so a
        // straggling blocking task is abandoned rather than awaited.
        blocking(move || {
            let mut client =
                DiscoveryAutoconfigClientStd::new(resolver).with_tls(PimalayaTransport::tls());

            let result = match &endpoint {
                OwnedEndpoint::WellKnown(domain) => client.isp_fallback(domain, true),
                OwnedEndpoint::Subdomain(local_part, domain) => {
                    client.isp(local_part, domain, true)
                }
                OwnedEndpoint::Ispdb(domain) => client.ispdb(domain, true),
            };

            result.map_err(|err| TransportError::new(err.to_string()))
        })
        .await
    }

    async fn srv(&self, domain: &str) -> Result<DiscoverySrvReport, TransportError> {
        use io_pim_discovery::rfc6186::client::DiscoverySrvClientStd;

        let resolver = self.resolver.clone();
        let domain = domain.to_owned();

        blocking(move || {
            let mut client = DiscoverySrvClientStd::new(resolver);
            client
                .discover(&domain)
                .map_err(|err| TransportError::new(err.to_string()))
        })
        .await
    }
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
}
