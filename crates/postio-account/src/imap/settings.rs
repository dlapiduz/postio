//! Carries no secret: the password comes from `SecretStore` when a connection opens, never stored
//! beside the host, in `config.toml` or in logs.

use std::fmt;
use std::time::Duration;

use postio_model::{AuthMethod, ServerConfig, TransportSecurity};

use crate::backend::{BackendError, BackendResult};
use crate::discovery::{Encryption, preset_for_domain};

pub const IMAPS_PORT: u16 = 993;

pub const IMAP_PORT: u16 = 143;

pub use postio_transport::DEFAULT_CONNECT_TIMEOUT;

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub host: String,
    pub port: u16,
    pub security: TransportSecurity,
    pub username: String,
    /// Data, not a branch: the credential is the same `Password` either way; this says how to
    /// present it. Defaults to `AuthMethod::Password` (#193).
    pub auth: AuthMethod,
    pub connect_timeout: Duration,
}

impl ConnectionSettings {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        security: TransportSecurity,
        username: impl Into<String>,
    ) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            username: username.into(),
            auth: AuthMethod::default(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Reads the same table the first-run probe consults. `None` for a domain the table does not
    /// cover.
    pub fn preset_for(email: &str) -> Option<Self> {
        let (_, domain) = email.rsplit_once('@')?;
        let preset = preset_for_domain(domain)?;
        let settings = preset.settings_for(email);

        Some(Self::new(
            settings.imap.host,
            settings.imap.port,
            match settings.imap.encryption {
                Encryption::Tls => TransportSecurity::Tls,
                Encryption::StartTls => TransportSecurity::StartTls,
                Encryption::None => TransportSecurity::None,
            },
            settings.login,
        ))
    }

    pub fn from_server_config(config: &ServerConfig) -> Self {
        Self::new(
            config.host.clone(),
            config.port,
            config.security,
            config.username.clone(),
        )
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    pub fn with_auth(mut self, auth: AuthMethod) -> Self {
        self.auth = auth;
        self
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// `TransportSecurity::None` is allowed only against loopback; elsewhere it is refused before a
    /// socket opens, so a mistyped port cannot hand a password to whoever listens.
    pub fn validate(&self) -> BackendResult<()> {
        if self.host.trim().is_empty() {
            return Err(BackendError::Protocol {
                reason: "no IMAP host is configured for this account".to_owned(),
            });
        }
        if self.security == TransportSecurity::None && !postio_model::net::is_loopback(&self.host) {
            return Err(BackendError::Tls {
                host: self.host.clone(),
                reason: "refusing to send credentials over an unencrypted connection; \
                         use implicit TLS on 993 or STARTTLS on 143"
                    .to_owned(),
            });
        }
        Ok(())
    }
}

impl fmt::Debug for ConnectionSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionSettings")
            .field("endpoint", &self.endpoint())
            .field("security", &self.security)
            .field("username", &self.username)
            .field("connect_timeout", &self.connect_timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shipped_provider_resolves_to_encrypted_settings_that_validate() {
        for preset in crate::discovery::presets() {
            let email = format!("a@{}", preset.domains()[0]);
            let settings = ConnectionSettings::preset_for(&email).expect("a table row");

            assert_eq!(settings.host, preset.imap_host());
            assert_eq!(settings.username, email);
            assert_ne!(
                settings.security,
                TransportSecurity::None,
                "{} would send credentials in the clear",
                preset.display_name()
            );
            settings.validate().unwrap();
        }
    }

    #[test]
    fn a_domain_the_table_does_not_cover_has_no_preset() {
        assert!(ConnectionSettings::preset_for("someone@example.org").is_none());
        assert!(ConnectionSettings::preset_for("not-an-address").is_none());
    }

    #[test]
    fn cleartext_to_a_remote_host_is_refused_before_a_socket_opens() {
        let settings = ConnectionSettings::new(
            "imap.example.com",
            143,
            TransportSecurity::None,
            "someone@example.com",
        );

        let error = settings.validate().unwrap_err();

        assert!(matches!(error, BackendError::Tls { .. }));
        assert!(error.to_string().contains("refusing"));
    }

    #[test]
    fn cleartext_to_loopback_is_allowed_for_a_local_test_server() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            let settings =
                ConnectionSettings::new(host, 143, TransportSecurity::None, "someone@localhost");
            settings.validate().expect(host);
        }
    }

    #[test]
    fn an_empty_host_is_a_configuration_error() {
        let settings = ConnectionSettings::new("  ", 993, TransportSecurity::Tls, "someone");

        assert!(settings.validate().is_err());
    }

    #[test]
    fn debug_output_carries_no_secret_because_settings_hold_none() {
        let settings = ConnectionSettings::new(
            "imap.example.com",
            IMAPS_PORT,
            TransportSecurity::Tls,
            "someone@example.com",
        );

        let rendered = format!("{settings:?}");

        assert!(rendered.contains("imap.example.com:993"));
        assert!(!rendered.to_lowercase().contains("password"));
    }
}
