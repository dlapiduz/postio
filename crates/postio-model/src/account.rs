//! Accounts, their servers, and the identities that send from them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, IdentityId};

/// How a connection is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransportSecurity {
    /// Plaintext. Only ever valid for a local test server.
    None,
    /// Upgrade an initially plaintext connection with `STARTTLS`.
    StartTls,
    /// TLS from the first byte (implicit TLS).
    #[default]
    Tls,
}

/// How Postio authenticates to a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuthMethod {
    /// An ordinary account password.
    #[default]
    Password,
    /// A provider-issued app-specific password. This is the v1 iCloud path.
    AppPassword,
    /// OAuth 2 with a refresh token.
    OAuth2,
    /// `XOAUTH2` SASL, as used by Gmail and Outlook.
    XOAuth2,
}

/// Where and how to reach one of an account's servers.
///
/// Carries no secret. Credentials live in the Secret Service keyring and are
/// never part of the domain model, never written to `config.toml` and never
/// logged.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Hostname.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Connection security.
    pub security: TransportSecurity,
    /// Login username, usually the full address.
    pub username: String,
}

/// A signature appended to outgoing mail.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Signature {
    /// Plain-text form.
    pub text: String,
    /// Rich form, when the identity has one.
    pub html: Option<String>,
}

/// An address the user can send from, within an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Local id.
    pub id: IdentityId,
    /// Owning account.
    pub account_id: AccountId,
    /// Name shown in the identity picker.
    pub display_name: String,
    /// The `From` address this identity sends as.
    pub address: EmailAddress,
    /// An explicit `Reply-To`, when it differs from `address`.
    pub reply_to: Option<EmailAddress>,
    /// Signature for this identity.
    pub signature: Option<Signature>,
    /// Whether this is the account's default identity.
    pub is_default: bool,
}

impl Identity {
    /// Builds an unpersisted identity for `account_id`.
    pub fn new(account_id: AccountId, address: EmailAddress) -> Self {
        Self {
            id: IdentityId::UNASSIGNED,
            account_id,
            display_name: address.display().to_owned(),
            address,
            reply_to: None,
            signature: None,
            is_default: false,
        }
    }

    /// The address replies to mail from this identity should go to.
    pub fn effective_reply_to(&self) -> &EmailAddress {
        self.reply_to.as_ref().unwrap_or(&self.address)
    }
}

/// A configured mail account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// Local id.
    pub id: AccountId,
    /// Name shown in the sidebar.
    pub display_name: String,
    /// The account's primary address.
    pub address: EmailAddress,
    /// IMAP server.
    pub incoming: ServerConfig,
    /// SMTP server.
    pub outgoing: ServerConfig,
    /// How to authenticate.
    pub auth: AuthMethod,
    /// Whether the account participates in sync.
    pub enabled: bool,
    /// Addresses this account can send from.
    pub identities: Vec<Identity>,
    /// When the account was added.
    pub created_at: DateTime<Utc>,
}

impl Account {
    /// Builds an unpersisted account with conventional IMAPS/submission ports.
    pub fn new(display_name: impl Into<String>, address: EmailAddress) -> Self {
        let username = address.address.clone();
        Self {
            id: AccountId::UNASSIGNED,
            display_name: display_name.into(),
            address,
            incoming: ServerConfig {
                host: String::new(),
                port: 993,
                security: TransportSecurity::Tls,
                username: username.clone(),
            },
            outgoing: ServerConfig {
                host: String::new(),
                port: 587,
                security: TransportSecurity::StartTls,
                username,
            },
            auth: AuthMethod::Password,
            enabled: true,
            identities: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// The identity marked default, or the first one when none is marked.
    ///
    /// Returns `None` only when the account has no identities at all.
    pub fn default_identity(&self) -> Option<&Identity> {
        self.identities
            .iter()
            .find(|identity| identity.is_default)
            .or_else(|| self.identities.first())
    }

    /// Whether `address` is one of this account's own addresses.
    ///
    /// Used to pick the "to me" affordances and to drop the user from
    /// reply-all recipients.
    pub fn owns_address(&self, address: &EmailAddress) -> bool {
        self.address.same_address(address)
            || self
                .identities
                .iter()
                .any(|identity| identity.address.same_address(address))
    }
}
