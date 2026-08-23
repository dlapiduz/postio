//! Where and how to reach an IMAP server.
//!
//! Carries no secret. The password comes from [`SecretStore`](crate::secret)
//! at the moment a connection is opened and is never stored beside the host
//! name, never written to `config.toml`, and never logged.

use std::fmt;
use std::time::Duration;

use postio_model::{ServerConfig, TransportSecurity};

use crate::backend::{BackendError, BackendResult};
use crate::discovery::{Encryption, preset_for_domain};

/// The implicit-TLS IMAP port (RFC 8314).
pub const IMAPS_PORT: u16 = 993;

/// The cleartext IMAP port, reachable only through `STARTTLS`.
pub const IMAP_PORT: u16 = 143;

/// How long a connection attempt may take before it is abandoned.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything needed to open a session, minus the password.
#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    /// The server host name. Also the name the certificate must match.
    pub host: String,
    /// The TCP port.
    pub port: u16,
    /// How the connection is protected.
    pub security: TransportSecurity,
    /// The login name. Usually the full address; some providers template it.
    pub username: String,
    /// How long a connect or handshake may take.
    pub connect_timeout: Duration,
}

impl ConnectionSettings {
    /// Settings for an arbitrary server.
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
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Settings from Postio's provider preset table, when it ships a row for
    /// the address's domain.
    ///
    /// Providers are data: this reads the same table the first-run probe
    /// consults, so adding a provider never means adding a branch here.
    /// Returns `None` for a domain the table does not cover — the probe or
    /// the manual form answers those.
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

    /// Settings from an account's stored incoming-server configuration.
    pub fn from_server_config(config: &ServerConfig) -> Self {
        Self::new(
            config.host.clone(),
            config.port,
            config.security,
            config.username.clone(),
        )
    }

    /// Sets how long a connect or handshake may take.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// `host:port`, for logs and error messages.
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Whether the connection is encrypted from the first byte.
    pub fn is_implicit_tls(&self) -> bool {
        self.security == TransportSecurity::Tls
    }

    /// Rejects settings that would put a password on the wire in the clear.
    ///
    /// `TransportSecurity::None` is allowed only against the loopback
    /// interface, where the "network" is a test server in the same machine.
    /// Anywhere else it is refused *before* a socket is opened, because a
    /// mistyped port is not a reason to hand an app-specific password to
    /// whoever is listening.
    pub fn validate(&self) -> BackendResult<()> {
        if self.host.trim().is_empty() {
            return Err(BackendError::Protocol {
                reason: "no IMAP host is configured for this account".to_owned(),
            });
        }
        if self.security == TransportSecurity::None && !is_loopback(&self.host) {
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

/// Whether `host` names this machine.
fn is_loopback(host: &str) -> bool {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("127.")
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
