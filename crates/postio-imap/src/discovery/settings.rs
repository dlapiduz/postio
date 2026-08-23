//! The structured result the onboarding screen renders and the user may
//! override.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Transport security for one server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encryption {
    /// Implicit TLS from the first byte (IMAPS on 993, SMTPS on 465).
    Tls,
    /// Plaintext connection upgraded with STARTTLS.
    StartTls,
    /// No encryption. Never chosen by Postio on its own; only honoured when
    /// a provider's own autoconfig document says so.
    None,
}

impl fmt::Display for Encryption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Tls => "TLS",
            Self::StartTls => "STARTTLS",
            Self::None => "none",
        })
    }
}

/// Where to reach one server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    /// DNS hostname.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Transport security.
    pub encryption: Encryption,
}

impl ServerSettings {
    /// Builds settings for one server.
    pub fn new(host: impl Into<String>, port: u16, encryption: Encryption) -> Self {
        Self {
            host: host.into(),
            port,
            encryption,
        }
    }
}

impl fmt::Display for ServerSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} ({})", self.host, self.port, self.encryption)
    }
}

/// Which probe produced a set of settings. The UI shows this so the user can
/// judge how much to trust it before overriding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsSource {
    /// Postio's own table of known providers (currently iCloud).
    Builtin,
    /// `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml`.
    WellKnown,
    /// `https://autoconfig.<domain>/mail/config-v1.1.xml`.
    Autoconfig,
    /// The Thunderbird ISPDB.
    Ispdb,
    /// RFC 6186 `_imaps._tcp` / `_submission._tcp` SRV records.
    Srv,
    /// A common-name guess. Unverified: only ever used to prefill the manual
    /// form, never presented as a discovered account.
    Guess,
}

impl SettingsSource {
    /// A short human label for the onboarding screen.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Builtin => "known provider",
            Self::WellKnown => "well-known autoconfig",
            Self::Autoconfig => "autoconfig subdomain",
            Self::Ispdb => "Thunderbird ISPDB",
            Self::Srv => "SRV records",
            Self::Guess => "guess",
        }
    }

    /// Whether these settings came from an authoritative source. Guesses are
    /// not.
    pub fn is_authoritative(&self) -> bool {
        !matches!(self, Self::Guess)
    }
}

/// Everything the onboarding screen needs after a probe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSettings {
    /// The address the probe was run for.
    pub email: String,
    /// Incoming server.
    pub imap: ServerSettings,
    /// Outgoing server.
    pub smtp: ServerSettings,
    /// Login to present to both servers. Providers may template this; the
    /// full address is the near-universal answer and the one we fill in.
    pub login: String,
    /// Which probe produced this.
    pub source: SettingsSource,
    /// The provider requires an application-specific password rather than
    /// the account password. True for iCloud, which offers third parties no
    /// OAuth path at all.
    pub requires_app_password: bool,
    /// A sentence for the onboarding screen, when there is something the
    /// user must know before typing a password.
    pub note: Option<String>,
    /// Where to generate an app-specific password, when [`Self::note`] says
    /// one is required. The onboarding screen links to this rather than
    /// making the user go find it.
    pub password_help_url: Option<String>,
    /// The provider's own display name, when it published one.
    pub display_name: Option<String>,
}
