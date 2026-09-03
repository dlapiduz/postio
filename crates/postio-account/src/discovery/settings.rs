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
    /// The provider's OAuth offer, when its preset row prefers `oauth2`
    /// (#534): what the sign-in flow needs to run. Only preset-sourced
    /// discoveries carry one — autoconfig and SRV say nothing about OAuth.
    pub oauth: Option<OAuthOffer>,
    /// The provider's JMAP offer, when its preset row advertises the
    /// backend (ADR 0018 Q5). Like [`oauth`](Self::oauth), preset-only:
    /// autoconfig and SRV describe IMAP servers.
    pub jmap: Option<JmapOffer>,
    /// Preference order among the backends the provider is reached over.
    /// `["imap"]` for everything but a preset row that says otherwise.
    pub backends: Vec<String>,
}

/// What a preset row offers a JMAP add (ADR 0018 Q5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapOffer {
    /// The RFC 8620 session resource URL.
    pub session_url: String,
}

/// What a preset row's `[provider.<id>.oauth]` table offers the sign-in
/// flow (#534, ADR 0006 Q4 as amended by #152): endpoints directly, or an
/// issuer to resolve them from, plus the scopes to request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthOffer {
    /// The RFC 8414 issuer to resolve endpoints from, when named.
    pub issuer: Option<String>,
    /// The authorization endpoint, when the row carries it directly.
    pub authorize: Option<String>,
    /// The token endpoint, when the row carries it directly.
    pub token: Option<String>,
    /// The scopes the sign-in requests.
    pub scopes: Vec<String>,
}
