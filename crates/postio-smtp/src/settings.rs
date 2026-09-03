//! Where and how to reach a submission server.
//!
//! Carries no secret. The password comes from wherever the caller keeps it
//! (the Secret Service keyring, via `postio-account::secret` — the same
//! account password serves both protocols) at the moment a connection is
//! opened, and is never stored beside the host name.

use std::fmt;
use std::time::Duration;

use postio_model::{AuthMethod, ServerConfig, TransportSecurity};

use crate::error::{SmtpError, SmtpResult};

/// The implicit-TLS submission port (RFC 8314).
pub const SMTPS_PORT: u16 = 465;

/// The `STARTTLS` submission port (RFC 6409).
pub const SUBMISSION_PORT: u16 = 587;

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
    /// Which SASL mechanism the credential is presented with.
    ///
    /// The IMAP side of the same account carries the identical field, and for
    /// the same reason: the credential is one `SecretString` either way — a
    /// stored app password or whatever a `TokenSource` returned — and this
    /// says how to present it. Defaults to [`AuthMethod::Password`], so an
    /// account that never mentions auth authenticates as it always has (#193).
    pub auth: AuthMethod,
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
            auth: AuthMethod::default(),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        }
    }

    /// Settings from an account's stored outgoing-server configuration.
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

    /// Sets which SASL mechanism the credential is presented with.
    pub fn with_auth(mut self, auth: AuthMethod) -> Self {
        self.auth = auth;
        self
    }

    /// `host:port`, for logs and error messages.
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Rejects settings that would put a password on the wire in the clear.
    ///
    /// `TransportSecurity::None` is allowed only against the loopback
    /// interface, where the "network" is a test server on the same machine.
    /// Anywhere else it is refused *before* a socket is opened, because a
    /// mistyped port is not a reason to hand an app-specific password to
    /// whoever is listening.
    pub fn validate(&self) -> SmtpResult<()> {
        if self.host.trim().is_empty() {
            return Err(SmtpError::Configuration {
                reason: "no submission host is configured for this account".to_owned(),
            });
        }
        if self.security == TransportSecurity::None && !is_loopback(&self.host) {
            return Err(SmtpError::Tls {
                host: self.host.clone(),
                reason: "refusing to send credentials over an unencrypted connection; \
                         use implicit TLS on 465 or STARTTLS on 587"
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
    fn cleartext_to_a_remote_host_is_refused_before_a_socket_opens() {
        let settings = ConnectionSettings::new(
            "smtp.example.com",
            SUBMISSION_PORT,
            TransportSecurity::None,
            "someone@example.com",
        );

        let error = settings.validate().unwrap_err();

        assert!(matches!(error, SmtpError::Tls { .. }));
        assert!(error.to_string().contains("refusing"));
    }

    #[test]
    fn cleartext_to_loopback_is_allowed_for_a_local_test_server() {
        for host in ["localhost", "127.0.0.1", "::1"] {
            let settings = ConnectionSettings::new(
                host,
                SUBMISSION_PORT,
                TransportSecurity::None,
                "someone@localhost",
            );
            settings.validate().expect(host);
        }
    }

    #[test]
    fn an_empty_host_is_a_configuration_error() {
        let settings = ConnectionSettings::new("  ", SMTPS_PORT, TransportSecurity::Tls, "someone");

        assert!(settings.validate().is_err());
    }

    #[test]
    fn debug_output_carries_no_secret_because_settings_hold_none() {
        let settings = ConnectionSettings::new(
            "smtp.example.com",
            SMTPS_PORT,
            TransportSecurity::Tls,
            "someone@example.com",
        );

        let rendered = format!("{settings:?}");

        assert!(rendered.contains("smtp.example.com:465"));
        assert!(!rendered.to_lowercase().contains("password"));
    }
}
