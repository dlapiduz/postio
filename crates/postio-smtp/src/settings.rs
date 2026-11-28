//! Where and how to reach a submission server. Carries no secret: the password comes from the
//! keyring when a connection opens and is never stored beside the host.

use std::fmt;
use std::time::Duration;

use postio_model::{AuthMethod, ServerConfig, TransportSecurity};

use crate::error::{SmtpError, SmtpResult};

pub const SMTPS_PORT: u16 = 465;

pub const SUBMISSION_PORT: u16 = 587;

pub use postio_transport::DEFAULT_CONNECT_TIMEOUT;

#[derive(Clone, PartialEq, Eq)]
pub struct ConnectionSettings {
    pub host: String,
    pub port: u16,
    pub security: TransportSecurity,
    pub username: String,
    /// Defaults to [`AuthMethod::Password`], so an account that never mentions auth authenticates
    /// as it always has (#193).
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

    pub fn from_server_config(config: &ServerConfig) -> Self {
        Self::new(
            config.host.clone(),
            config.port,
            config.security,
            config.username.clone(),
        )
    }

    pub fn with_auth(mut self, auth: AuthMethod) -> Self {
        self.auth = auth;
        self
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// `TransportSecurity::None` is allowed only against loopback; anywhere else it is refused
    /// before a socket is opened, so a mistyped port cannot hand a password to whoever is
    /// listening.
    pub fn validate(&self) -> SmtpResult<()> {
        if self.host.trim().is_empty() {
            return Err(SmtpError::Configuration {
                reason: "no submission host is configured for this account".to_owned(),
            });
        }
        if self.security == TransportSecurity::None && !postio_model::net::is_loopback(&self.host) {
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
