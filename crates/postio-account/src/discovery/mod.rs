//! First-run probe: builtin table, then autoconfig, ISPDB, MX and SRV, under a per-step timeout and
//! a whole-probe deadline, all raced against the caller's `CancelToken`. Finding nothing is
//! `ManualEntry`, not an error.
//! Privacy: only the typed domain is ever probed, a guess is never a request, and cancellation
//! reaches the socket through the transport (#57). Step 4 discloses the domain to the Thunderbird
//! ISPDB; `Probe::new` has one production call site.

mod builtin;
mod providers_toml;
mod settings;
// `pub(crate)` because `crate::oauth`'s token exchange reuses this transport's TCP/TLS connect
// helpers (ADR 0006 Q2).
pub(crate) mod transport;

use std::sync::Arc;
use std::time::{Duration, Instant};

use io_pim_discovery::autoconfig::config::{
    DiscoverySecurityType, DiscoveryServer, DiscoveryServerType,
};

pub use self::builtin::{
    Preset, authserv_ids_for_imap_host, preset_for_domain, preset_for_imap_host, presets,
};
pub use self::settings::{
    AccountSettings, Encryption, JmapOffer, OAuthOffer, ServerSettings, SettingsSource,
};
pub use self::transport::{
    AutoconfigEndpoint, DISCOVERY_IO_TIMEOUT, DiscoveryAutoconfig, DiscoverySrvReport,
    DiscoverySrvService, DiscoveryTransport, PimalayaTransport, TransportError,
};
pub use crate::cancel::CancelToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProbeStep {
    WellKnown,
    AutoconfigSubdomain,
    Ispdb,
    Mx,
    Srv,
}

impl ProbeStep {
    pub const ORDER: [ProbeStep; 5] = [
        ProbeStep::WellKnown,
        ProbeStep::AutoconfigSubdomain,
        ProbeStep::Ispdb,
        // MX is an inference, so it goes after the steps that read published mail configuration and
        // before SRV (#94).
        ProbeStep::Mx,
        ProbeStep::Srv,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::WellKnown => "well-known autoconfig",
            Self::AutoconfigSubdomain => "autoconfig subdomain",
            Self::Ispdb => "Thunderbird ISPDB",
            Self::Mx => "MX records",
            Self::Srv => "SRV records",
        }
    }

    fn source(&self) -> SettingsSource {
        match self {
            Self::WellKnown => SettingsSource::WellKnown,
            Self::AutoconfigSubdomain => SettingsSource::Autoconfig,
            Self::Ispdb => SettingsSource::Ispdb,
            Self::Mx => SettingsSource::Mx,
            Self::Srv => SettingsSource::Srv,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptOutcome {
    Hit,
    Miss,
    Failed(String),
    TimedOut,
    Skipped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeAttempt {
    pub step: ProbeStep,
    pub outcome: AttemptOutcome,
    pub elapsed: Duration,
}

impl ProbeAttempt {
    pub fn is_hit(&self) -> bool {
        self.outcome == AttemptOutcome::Hit
    }

    pub fn is_miss(&self) -> bool {
        !self.is_hit()
    }

    pub fn timed_out(&self) -> bool {
        self.outcome == AttemptOutcome::TimedOut
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryOutcome {
    Discovered(AccountSettings),
    ManualEntry { suggestion: Option<AccountSettings> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryReport {
    pub email: String,
    pub domain: String,
    pub outcome: DiscoveryOutcome,
    pub attempts: Vec<ProbeAttempt>,
}

impl DiscoveryReport {
    pub fn settings(&self) -> Option<&AccountSettings> {
        match &self.outcome {
            DiscoveryOutcome::Discovered(settings) => Some(settings),
            DiscoveryOutcome::ManualEntry { suggestion } => suggestion.as_ref(),
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    #[error("discovery was cancelled")]
    Cancelled,

    #[error("{0} is not an email address Postio can look up")]
    InvalidAddress(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeOptions {
    pub step_timeout: Duration,
    /// Reaching it is not an error; it means manual entry.
    pub overall_timeout: Duration,
    /// Off by default: an unverified guess presented as a discovery is worse than an empty form.
    pub guess_common_names: bool,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            step_timeout: Duration::from_secs(5),
            overall_timeout: Duration::from_secs(15),
            guess_common_names: false,
        }
    }
}

pub struct Probe {
    transport: Arc<dyn DiscoveryTransport>,
    options: ProbeOptions,
}

impl Probe {
    pub fn new(transport: Arc<dyn DiscoveryTransport>) -> Self {
        Self::with_options(transport, ProbeOptions::default())
    }

    pub fn with_options(transport: Arc<dyn DiscoveryTransport>, options: ProbeOptions) -> Self {
        Self { transport, options }
    }

    pub fn options(&self) -> &ProbeOptions {
        &self.options
    }

    pub async fn run(
        &self,
        email: &str,
        cancel: &CancelToken,
    ) -> Result<DiscoveryReport, DiscoveryError> {
        let address = Address::parse(email)?;

        if cancel.is_cancelled() {
            return Err(DiscoveryError::Cancelled);
        }

        // Known providers first: instant, offline, and the only way iCloud
        // resolves at all since Apple publishes neither autoconfig nor SRV.
        if let Some(settings) = builtin::lookup(&address.email, &address.local, &address.domain) {
            return Ok(DiscoveryReport {
                email: address.email,
                domain: address.domain,
                outcome: DiscoveryOutcome::Discovered(settings),
                attempts: Vec::new(),
            });
        }

        let deadline = Instant::now() + self.options.overall_timeout;
        let mut attempts = Vec::with_capacity(ProbeStep::ORDER.len());

        for step in ProbeStep::ORDER {
            let started = Instant::now();
            let (outcome, settings) = self.attempt(step, &address, cancel, deadline).await?;

            let hit = settings.is_some();
            attempts.push(ProbeAttempt {
                step,
                outcome,
                elapsed: started.elapsed(),
            });

            if let Some(settings) = settings {
                debug_assert!(hit);
                return Ok(DiscoveryReport {
                    email: address.email,
                    domain: address.domain,
                    outcome: DiscoveryOutcome::Discovered(settings),
                    attempts,
                });
            }
        }

        let suggestion = self
            .options
            .guess_common_names
            .then(|| guess(&address))
            .flatten();

        Ok(DiscoveryReport {
            email: address.email,
            domain: address.domain,
            outcome: DiscoveryOutcome::ManualEntry { suggestion },
            attempts,
        })
    }

    async fn attempt(
        &self,
        step: ProbeStep,
        address: &Address,
        cancel: &CancelToken,
        deadline: Instant,
    ) -> Result<(AttemptOutcome, Option<AccountSettings>), DiscoveryError> {
        let now = Instant::now();
        if now >= deadline {
            return Ok((AttemptOutcome::Skipped, None));
        }

        let budget = self.options.step_timeout.min(deadline - now);

        macro_rules! race {
            ($call:expr) => {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => return Err(DiscoveryError::Cancelled),
                    result = tokio::time::timeout(budget, $call) => result,
                }
            };
        }

        match step {
            ProbeStep::Mx => {
                let raced = race!(self.transport.mx(&address.domain, cancel));
                Ok(match raced {
                    Err(_elapsed) => (AttemptOutcome::TimedOut, None),
                    Ok(Err(err)) => (AttemptOutcome::Failed(err.message().to_owned()), None),
                    Ok(Ok(hosts)) => match settings_from_mx(address, &hosts) {
                        Some(settings) => (AttemptOutcome::Hit, Some(settings)),
                        // Records answered but named nobody known: a miss, not a failure.
                        None => (AttemptOutcome::Miss, None),
                    },
                })
            }
            ProbeStep::Srv => {
                let raced = race!(self.transport.srv(&address.domain, cancel));
                Ok(match raced {
                    Err(_elapsed) => (AttemptOutcome::TimedOut, None),
                    Ok(Err(err)) => (AttemptOutcome::Failed(err.message().to_owned()), None),
                    Ok(Ok(report)) => match settings_from_srv(address, &report) {
                        Some(settings) => (AttemptOutcome::Hit, Some(settings)),
                        None => (AttemptOutcome::Miss, None),
                    },
                })
            }
            step => {
                let endpoint = match step {
                    ProbeStep::WellKnown => AutoconfigEndpoint::WellKnown {
                        domain: &address.domain,
                    },
                    ProbeStep::AutoconfigSubdomain => AutoconfigEndpoint::Subdomain {
                        local_part: &address.local,
                        domain: &address.domain,
                    },
                    _ => AutoconfigEndpoint::Ispdb {
                        domain: &address.domain,
                    },
                };

                let raced = race!(self.transport.autoconfig(endpoint, cancel));
                Ok(match raced {
                    Err(_elapsed) => (AttemptOutcome::TimedOut, None),
                    Ok(Err(err)) => (AttemptOutcome::Failed(err.message().to_owned()), None),
                    Ok(Ok(document)) => {
                        match settings_from_autoconfig(address, &document, step.source()) {
                            Some(settings) => (AttemptOutcome::Hit, Some(settings)),
                            None => (AttemptOutcome::Miss, None),
                        }
                    }
                })
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Address {
    email: String,
    local: String,
    domain: String,
}

impl Address {
    fn parse(input: &str) -> Result<Self, DiscoveryError> {
        let email = input.trim();
        let invalid = || DiscoveryError::InvalidAddress(email.to_owned());

        let mut parts = email.split('@');
        let local = parts.next().ok_or_else(invalid)?;
        let domain = parts.next().ok_or_else(invalid)?;

        if parts.next().is_some() || local.is_empty() || domain.is_empty() {
            return Err(invalid());
        }

        if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
            return Err(invalid());
        }

        if email.chars().any(char::is_whitespace) {
            return Err(invalid());
        }

        Ok(Self {
            email: email.to_owned(),
            local: local.to_owned(),
            domain: domain.to_ascii_lowercase(),
        })
    }
}

/// Both IMAP and SMTP are required: a POP3-only or incoming-only document is a miss, since a
/// half-configured account cannot send.
fn settings_from_autoconfig(
    address: &Address,
    document: &DiscoveryAutoconfig,
    source: SettingsSource,
) -> Option<AccountSettings> {
    let provider = &document.email_provider;

    let incoming = provider
        .incoming_server
        .iter()
        .find(|server| matches!(server.r#type, DiscoveryServerType::Imap))?;
    let outgoing = provider
        .outgoing_server
        .iter()
        .find(|server| matches!(server.r#type, DiscoveryServerType::Smtp))?;

    let imap = server_settings(incoming, 993, 143)?;
    let smtp = server_settings(outgoing, 465, 587)?;

    Some(AccountSettings {
        email: address.email.clone(),
        imap,
        smtp,
        login: login_for(incoming, address),
        source,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: provider.display_name.clone(),
        oauth: None,
        jmap: None,
        backends: vec!["imap".to_owned()],
    })
}

fn server_settings(
    server: &DiscoveryServer,
    tls_port: u16,
    plain_port: u16,
) -> Option<ServerSettings> {
    let host = server.hostname.clone()?;
    if host.trim().is_empty() {
        return None;
    }

    let encryption = match server.socket_type {
        Some(DiscoverySecurityType::Tls) => Encryption::Tls,
        Some(DiscoverySecurityType::Starttls) => Encryption::StartTls,
        Some(DiscoverySecurityType::Plain) => Encryption::None,
        // Guessing TLS fails loudly at connect time; guessing plaintext would fail silently and
        // insecurely, so no inference lands on it.
        Some(DiscoverySecurityType::Unknown) | None => match server.port {
            Some(port) if port == tls_port => Encryption::Tls,
            Some(_) => Encryption::StartTls,
            None => Encryption::Tls,
        },
    };

    let port = server.port.unwrap_or(match encryption {
        Encryption::Tls => tls_port,
        Encryption::StartTls | Encryption::None => plain_port,
    });

    Some(ServerSettings::new(host, port, encryption))
}

fn login_for(server: &DiscoveryServer, address: &Address) -> String {
    let Some(template) = server.username.as_deref() else {
        return address.email.clone();
    };

    let expanded = template
        .replace("%EMAILADDRESS%", &address.email)
        .replace("%EMAILLOCALPART%", &address.local)
        .replace("%EMAILDOMAIN%", &address.domain);

    if expanded.trim().is_empty() {
        address.email.clone()
    } else {
        expanded
    }
}

/// Prefers implicit-TLS SRV records over STARTTLS; no IMAP or no submission record is a miss.
/// MX settings: `hosts` is in preference order and the first matching row wins; a backup MX at a
/// third party is no evidence.
/// The login is deliberately not derived: an iCloud custom domain authenticates as the Apple ID,
/// which no MX lookup can guess (#69).
fn settings_from_mx(address: &Address, hosts: &[String]) -> Option<AccountSettings> {
    let preset = hosts
        .iter()
        .find_map(|host| builtin::preset_for_mx_host(host))?;
    let mut settings = preset.settings_for(&address.email);
    // The row is real; what is inferred is that this domain belongs to it, so the source says MX,
    // not `Builtin`.
    settings.source = SettingsSource::Mx;
    Some(settings)
}

fn settings_from_srv(address: &Address, report: &DiscoverySrvReport) -> Option<AccountSettings> {
    let imap = match (&report.imaps, &report.imap) {
        (Some(service), _) => ServerSettings::new(&service.host, service.port, Encryption::Tls),
        (None, Some(service)) => {
            ServerSettings::new(&service.host, service.port, Encryption::StartTls)
        }
        (None, None) => return None,
    };

    let smtp = match (&report.submissions, &report.submission) {
        (Some(service), _) => ServerSettings::new(&service.host, service.port, Encryption::Tls),
        (None, Some(service)) => {
            // RFC 8314: 465 is implicit TLS, 587 is submission with STARTTLS.
            let encryption = if service.port == 465 {
                Encryption::Tls
            } else {
                Encryption::StartTls
            };
            ServerSettings::new(&service.host, service.port, encryption)
        }
        (None, None) => return None,
    };

    Some(AccountSettings {
        email: address.email.clone(),
        imap,
        smtp,
        login: address.email.clone(),
        source: SettingsSource::Srv,
        requires_app_password: false,
        note: None,
        password_help_url: None,
        display_name: None,
        oauth: None,
        jmap: None,
        backends: vec!["imap".to_owned()],
    })
}

fn guess(address: &Address) -> Option<AccountSettings> {
    Some(AccountSettings {
        email: address.email.clone(),
        imap: ServerSettings::new(format!("imap.{}", address.domain), 993, Encryption::Tls),
        smtp: ServerSettings::new(format!("smtp.{}", address.domain), 465, Encryption::Tls),
        login: address.email.clone(),
        source: SettingsSource::Guess,
        requires_app_password: false,
        note: Some(
            "Postio could not find published settings for this domain. These are guesses \
             — check them against your provider's documentation."
                .to_owned(),
        ),
        password_help_url: None,
        display_name: None,
        oauth: None,
        jmap: None,
        backends: vec!["imap".to_owned()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(email: &str) -> Address {
        Address::parse(email).expect("valid address")
    }

    #[test]
    fn an_address_is_split_and_its_domain_lowercased() {
        let parsed = address("  A.User+tag@Example.ORG ");
        assert_eq!(parsed.email, "A.User+tag@Example.ORG");
        assert_eq!(parsed.local, "A.User+tag");
        assert_eq!(parsed.domain, "example.org");
    }

    #[test]
    fn unprobeable_addresses_are_rejected() {
        for input in [
            "",
            "  ",
            "not-an-address",
            "@example.org",
            "user@",
            "user@@example.org",
            "user@localhost",
            // Built, not written out: a literal here reads as an address to
            // scripts/checks/check-no-personal-data.py.
            &["user@", ".example.org"].concat(),
            "user@example.org.",
            "user name@example.org",
        ] {
            assert!(
                Address::parse(input).is_err(),
                "{input:?} should not be probeable"
            );
        }
    }

    #[test]
    fn the_step_order_matches_the_documented_chain() {
        assert_eq!(
            ProbeStep::ORDER,
            [
                ProbeStep::WellKnown,
                ProbeStep::AutoconfigSubdomain,
                ProbeStep::Ispdb,
                ProbeStep::Mx,
                ProbeStep::Srv,
            ]
        );
    }

    #[test]
    fn a_srv_report_prefers_the_implicit_tls_records() {
        let address = address("user@example.org");
        let service = |host: &str, port| DiscoverySrvService {
            host: host.to_owned(),
            port,
            priority: 0,
            weight: 1,
        };

        let report = DiscoverySrvReport {
            imap: Some(service("imap.example.org", 143)),
            imaps: Some(service("imaps.example.org", 993)),
            submission: Some(service("submission.example.org", 587)),
            submissions: Some(service("submissions.example.org", 465)),
        };

        let settings = settings_from_srv(&address, &report).expect("usable report");
        assert_eq!(settings.imap.host, "imaps.example.org");
        assert_eq!(settings.imap.encryption, Encryption::Tls);
        assert_eq!(settings.smtp.host, "submissions.example.org");
        assert_eq!(settings.smtp.encryption, Encryption::Tls);
    }

    #[test]
    fn a_srv_report_missing_either_half_is_a_miss() {
        let address = address("user@example.org");
        let service = DiscoverySrvService {
            host: "imaps.example.org".to_owned(),
            port: 993,
            priority: 0,
            weight: 1,
        };

        assert!(settings_from_srv(&address, &DiscoverySrvReport::default()).is_none());
        assert!(
            settings_from_srv(
                &address,
                &DiscoverySrvReport {
                    imaps: Some(service),
                    ..DiscoverySrvReport::default()
                }
            )
            .is_none()
        );
    }

    #[test]
    fn an_attempt_that_is_not_a_hit_is_a_miss() {
        let attempt = |outcome| ProbeAttempt {
            step: ProbeStep::WellKnown,
            outcome,
            elapsed: Duration::ZERO,
        };

        assert!(attempt(AttemptOutcome::Hit).is_hit());
        assert!(!attempt(AttemptOutcome::Hit).is_miss());
        assert!(attempt(AttemptOutcome::Miss).is_miss());
        assert!(attempt(AttemptOutcome::Failed("404".into())).is_miss());
        assert!(attempt(AttemptOutcome::TimedOut).timed_out());
        assert!(attempt(AttemptOutcome::Skipped).is_miss());
    }

    #[test]
    fn a_guess_is_never_authoritative() {
        let settings = guess(&address("user@example.org")).unwrap();
        assert!(!settings.source.is_authoritative());
        assert_eq!(settings.imap.host, "imap.example.org");
        assert_eq!(settings.smtp.host, "smtp.example.org");
    }
}
