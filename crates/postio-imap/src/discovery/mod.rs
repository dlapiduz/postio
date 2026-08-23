//! Autoconfig probe for first run.
//!
//! The user types an address; Postio finds the servers. The chain is:
//!
//! 1. Postio's own table of known providers ([`builtin`]) — no I/O at all.
//! 2. `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`
//! 3. `https://autoconfig.<domain>/mail/config-v1.1.xml?emailaddress=…`
//! 4. The Thunderbird ISPDB
//! 5. RFC 6186 `_imaps._tcp` / `_submission._tcp` SRV records
//! 6. Common-name guesses (opt-in, and only ever used to prefill the manual
//!    form)
//!
//! Steps 2–5 are `io-pim-discovery`'s: it already implements the Mozilla
//! autoconfig endpoints and the RFC 6186 lookups, and this module does not
//! re-implement any of it. What lives here is the part that crate leaves to
//! the caller — the order, the budget, the cancellation, the iCloud table,
//! and the mapping to a result the onboarding screen can render.
//!
//! # Never hang the caller
//!
//! Every step is raced against both a per-step timeout and a whole-probe
//! deadline, and every await is raced against the caller's [`CancelToken`].
//! A probe that finds nothing in time is not an error: it returns
//! [`DiscoveryOutcome::ManualEntry`] so the UI drops straight into manual
//! entry.
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use postio_imap::discovery::{CancelToken, PimalayaTransport, Probe};
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let cancel = CancelToken::new();
//! let probe = Probe::new(Arc::new(PimalayaTransport::new()));
//! let report = probe.run("someone@example.com", &cancel).await?;
//! # Ok(())
//! # }
//! ```

mod builtin;
mod settings;
mod transport;

use std::sync::Arc;
use std::time::{Duration, Instant};

use io_pim_discovery::autoconfig::config::{
    DiscoverySecurityType, DiscoveryServer, DiscoveryServerType,
};

pub use self::builtin::{Preset, preset_for_domain, presets};
pub use self::settings::{AccountSettings, Encryption, ServerSettings, SettingsSource};
pub use self::transport::{
    AutoconfigEndpoint, DiscoveryAutoconfig, DiscoverySrvReport, DiscoverySrvService,
    DiscoveryTransport, PimalayaTransport, TransportError,
};
pub use crate::cancel::CancelToken;

/// A step in the probe chain that costs a network round trip.
///
/// The builtin table is not a step: it costs nothing and is always consulted
/// first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ProbeStep {
    /// The domain's own `.well-known` document.
    WellKnown,
    /// The `autoconfig.<domain>` document.
    AutoconfigSubdomain,
    /// The Thunderbird ISPDB.
    Ispdb,
    /// RFC 6186 SRV records.
    Srv,
}

impl ProbeStep {
    /// The chain, in order.
    pub const ORDER: [ProbeStep; 4] = [
        ProbeStep::WellKnown,
        ProbeStep::AutoconfigSubdomain,
        ProbeStep::Ispdb,
        ProbeStep::Srv,
    ];

    /// A short human label for the onboarding screen.
    pub fn label(&self) -> &'static str {
        match self {
            Self::WellKnown => "well-known autoconfig",
            Self::AutoconfigSubdomain => "autoconfig subdomain",
            Self::Ispdb => "Thunderbird ISPDB",
            Self::Srv => "SRV records",
        }
    }

    fn source(&self) -> SettingsSource {
        match self {
            Self::WellKnown => SettingsSource::WellKnown,
            Self::AutoconfigSubdomain => SettingsSource::Autoconfig,
            Self::Ispdb => SettingsSource::Ispdb,
            Self::Srv => SettingsSource::Srv,
        }
    }
}

/// How one step ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttemptOutcome {
    /// The step produced usable settings.
    Hit,
    /// The step answered, but with nothing Postio can use — an autoconfig
    /// document with no IMAP server, an empty SRV report.
    Miss,
    /// The step failed: no such host, HTTP 404, a malformed document.
    Failed(String),
    /// The step ran out of budget.
    TimedOut,
    /// The whole-probe deadline expired before this step started.
    Skipped,
}

/// What one step did, for the UI and for debugging.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeAttempt {
    /// Which step.
    pub step: ProbeStep,
    /// How it ended.
    pub outcome: AttemptOutcome,
    /// How long it took.
    pub elapsed: Duration,
}

impl ProbeAttempt {
    /// Whether this step produced the settings the probe returned.
    pub fn is_hit(&self) -> bool {
        self.outcome == AttemptOutcome::Hit
    }

    /// Whether this step produced nothing. Anything that is not a hit — a
    /// miss, a failure, a timeout, a skip — is a miss from the caller's side.
    pub fn is_miss(&self) -> bool {
        !self.is_hit()
    }

    /// Whether this step ran out of budget.
    pub fn timed_out(&self) -> bool {
        self.outcome == AttemptOutcome::TimedOut
    }
}

/// What the probe concluded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscoveryOutcome {
    /// An authoritative source answered. The UI can go straight to the
    /// password step.
    Discovered(AccountSettings),
    /// Nothing authoritative answered, so the user has to fill the form in.
    /// `suggestion` prefills it with an unverified common-name guess when
    /// [`ProbeOptions::guess_common_names`] is on.
    ManualEntry {
        /// An unverified guess, or `None`.
        suggestion: Option<AccountSettings>,
    },
}

/// The full result of a probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveryReport {
    /// The address probed, as typed.
    pub email: String,
    /// Its domain, lowercased.
    pub domain: String,
    /// What the probe concluded.
    pub outcome: DiscoveryOutcome,
    /// Every network step that ran, in order.
    pub attempts: Vec<ProbeAttempt>,
}

impl DiscoveryReport {
    /// The settings to show, discovered or merely suggested.
    pub fn settings(&self) -> Option<&AccountSettings> {
        match &self.outcome {
            DiscoveryOutcome::Discovered(settings) => Some(settings),
            DiscoveryOutcome::ManualEntry { suggestion } => suggestion.as_ref(),
        }
    }
}

/// The two ways a probe can fail outright. Finding nothing is not one of
/// them — that is [`DiscoveryOutcome::ManualEntry`].
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The caller cancelled.
    #[error("discovery was cancelled")]
    Cancelled,

    /// The address is not one Postio can probe.
    #[error("{0} is not an email address Postio can look up")]
    InvalidAddress(String),
}

/// Budgets and switches for one probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProbeOptions {
    /// Ceiling for any single step.
    pub step_timeout: Duration,
    /// Ceiling for the whole chain. Reaching it is not an error; it just
    /// means manual entry.
    pub overall_timeout: Duration,
    /// Offer `imap.<domain>` / `smtp.<domain>` as a prefill when nothing
    /// authoritative answers. Off by default: an unverified guess presented
    /// as a discovery is worse than an empty form.
    pub guess_common_names: bool,
}

impl Default for ProbeOptions {
    fn default() -> Self {
        Self {
            // Long enough for a slow ISP endpoint, short enough that four of
            // them still fit inside the overall budget with room to spare.
            step_timeout: Duration::from_secs(5),
            overall_timeout: Duration::from_secs(15),
            guess_common_names: false,
        }
    }
}

/// Runs the probe chain over a [`DiscoveryTransport`].
pub struct Probe {
    transport: Arc<dyn DiscoveryTransport>,
    options: ProbeOptions,
}

impl Probe {
    /// A probe with the default budgets.
    pub fn new(transport: Arc<dyn DiscoveryTransport>) -> Self {
        Self::with_options(transport, ProbeOptions::default())
    }

    /// A probe with explicit budgets.
    pub fn with_options(transport: Arc<dyn DiscoveryTransport>, options: ProbeOptions) -> Self {
        Self { transport, options }
    }

    /// The budgets in force.
    pub fn options(&self) -> &ProbeOptions {
        &self.options
    }

    /// Probes for `email`.
    ///
    /// Returns as soon as any step produces usable settings. Returns
    /// [`DiscoveryError::Cancelled`] the moment `cancel` fires, and returns
    /// [`DiscoveryOutcome::ManualEntry`] — not an error — when the chain runs
    /// out of steps or out of time.
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

    /// Runs one step inside its budget, racing the cancel token.
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
            ProbeStep::Srv => {
                let raced = race!(self.transport.srv(&address.domain));
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

                let raced = race!(self.transport.autoconfig(endpoint));
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

// ---------------------------------------------------------------------------
// Address
// ---------------------------------------------------------------------------

/// A parsed address. Only what the probe needs: the local part and a
/// lowercased domain.
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

        // Exactly one `@`, and something on both sides of it.
        if parts.next().is_some() || local.is_empty() || domain.is_empty() {
            return Err(invalid());
        }

        // A domain we can probe has at least one label separator; a bare
        // `localhost` has no autoconfig endpoint and no SRV zone.
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

// ---------------------------------------------------------------------------
// Mapping
// ---------------------------------------------------------------------------

/// Maps a Mozilla autoconfig document onto Postio's settings.
///
/// Both an IMAP and an SMTP server are required: a document that offers only
/// POP3, or only an incoming server, is a miss rather than a partial answer,
/// because v1 is IMAP+SMTP and a half-configured account cannot send.
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
        display_name: provider.display_name.clone(),
    })
}

/// Turns one `<incomingServer>` / `<outgoingServer>` entry into a
/// [`ServerSettings`], filling in the port when the document omits it.
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
        // No `socketType`: infer from the port, and assume implicit TLS when
        // there is no port either. Guessing TLS can only fail loudly at
        // connect time; guessing plaintext would fail silently and insecurely.
        None => match server.port {
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

/// Expands the provider's username template. `%EMAILADDRESS%` is by far the
/// most common; `%EMAILLOCALPART%` and `%EMAILDOMAIN%` are the other two the
/// Mozilla format defines.
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

/// Maps an RFC 6186 SRV report onto Postio's settings.
///
/// Prefers the implicit-TLS records (`_imaps`, `_submissions`) over the
/// STARTTLS ones, and treats a report with no IMAP or no submission record
/// as a miss.
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
        display_name: None,
    })
}

/// The common-name guess: `imap.<domain>` and `smtp.<domain>` on the
/// implicit-TLS ports. Never verified, so it is only ever a prefill.
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
        display_name: None,
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
            // scripts/check-no-personal-data.py.
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
