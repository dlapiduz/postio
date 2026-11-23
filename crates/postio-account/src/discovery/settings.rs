use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encryption {
    Tls,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSettings {
    pub host: String,
    pub port: u16,
    pub encryption: Encryption,
}

impl ServerSettings {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SettingsSource {
    Builtin,
    WellKnown,
    Autoconfig,
    Ispdb,
    Srv,
    /// Better evidence than a guess, but not what the domain published about mail configuration: it
    /// prefills the manual form (#94).
    Mx,
    /// A common-name guess. Unverified: only ever used to prefill the manual
    /// form, never presented as a discovered account.
    Guess,
}

impl SettingsSource {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Builtin => "known provider",
            Self::WellKnown => "well-known autoconfig",
            Self::Autoconfig => "autoconfig subdomain",
            Self::Ispdb => "Thunderbird ISPDB",
            Self::Srv => "SRV records",
            Self::Mx => "MX records",
            Self::Guess => "guess",
        }
    }

    pub fn is_authoritative(&self) -> bool {
        // `Mx` and `Guess` are inference, not the domain answering how to reach a mailbox, so they
        // prefill the manual form (#94).
        !matches!(self, Self::Guess | Self::Mx)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSettings {
    pub email: String,
    pub imap: ServerSettings,
    pub smtp: ServerSettings,
    pub login: String,
    pub source: SettingsSource,
    pub requires_app_password: bool,
    pub note: Option<String>,
    pub password_help_url: Option<String>,
    pub display_name: Option<String>,
    /// Only preset-sourced discoveries carry one (#534).
    pub oauth: Option<OAuthOffer>,
    /// Preset-only, like `oauth`: autoconfig and SRV describe IMAP servers (ADR 0018 Q5).
    pub jmap: Option<JmapOffer>,
    /// `["imap"]` unless a preset row says otherwise.
    pub backends: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JmapOffer {
    pub session_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthOffer {
    pub issuer: Option<String>,
    pub authorize: Option<String>,
    pub token: Option<String>,
    pub scopes: Vec<String>,
    /// Carried to the account row at sign-in so the engine can re-record the deadline on every
    /// refresh without the preset table (#954).
    #[serde(default)]
    pub refresh_token_lifetime_days: Option<u32>,
}
