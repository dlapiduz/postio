//! `[accounts]` — IMAP and SMTP settings.
//!
//! **No credential ever appears here.** An account references a keyring entry;
//! the password itself lives in the Secret Service keyring. See
//! [`crate::secrets`] for how that is enforced.
//!
//! ```toml
//! [accounts.icloud]
//! email = "ada@example.com"
//! display_name = "Person"
//! default = true
//!
//! [accounts.icloud.imap]
//! host = "imap.mail.me.com"
//! port = 993
//! security = "implicit-tls"
//! # keyring_entry = "postio:icloud:imap"   # defaults to this
//!
//! [accounts.icloud.smtp]
//! host = "smtp.mail.me.com"
//! port = 465
//! security = "implicit-tls"
//! ```

use serde::{Deserialize, Serialize};

use crate::Extras;

/// How the connection is encrypted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MailSecurity {
    /// TLS from the first byte — 993 for IMAP, 465 for SMTP. What iCloud uses.
    #[default]
    #[serde(
        alias = "implicit_tls",
        alias = "implicittls",
        alias = "tls",
        alias = "ssl"
    )]
    ImplicitTls,
    /// Plain connection upgraded with `STARTTLS` — the 587 fallback.
    #[serde(alias = "start_tls", alias = "starttls")]
    StartTls,
    /// No encryption. Only ever sensible against localhost.
    #[serde(alias = "plaintext", alias = "insecure")]
    None,
}

/// SASL mechanism used to authenticate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    /// `AUTHENTICATE PLAIN` with an app-specific password. The v1 path.
    #[default]
    Plain,
    /// The older `LOGIN` command, for servers that lack `PLAIN`.
    Login,
}

fn imap_port() -> u16 {
    993
}

fn smtp_port() -> u16 {
    465
}

/// `[accounts.<id>.imap]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImapConfig {
    /// Server host name.
    #[serde(default)]
    pub host: String,
    /// Port. Defaults to 993, the implicit-TLS port.
    #[serde(default = "imap_port")]
    pub port: u16,
    /// Transport encryption.
    #[serde(default)]
    pub security: MailSecurity,
    /// Login name, when it differs from the account's email address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Authentication mechanism.
    #[serde(default)]
    pub auth: AuthMethod,
    /// Name of the Secret Service entry holding the password.
    ///
    /// This is a *reference*, never the secret. Defaults to
    /// `postio:<account id>:imap`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyring_entry: Option<String>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for ImapConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: imap_port(),
            security: MailSecurity::default(),
            username: None,
            auth: AuthMethod::default(),
            keyring_entry: None,
            extra: Extras::new(),
        }
    }
}

/// `[accounts.<id>.smtp]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmtpConfig {
    /// Server host name.
    #[serde(default)]
    pub host: String,
    /// Port. Defaults to 465, the implicit-TLS submission port.
    #[serde(default = "smtp_port")]
    pub port: u16,
    /// Transport encryption.
    #[serde(default)]
    pub security: MailSecurity,
    /// Login name, when it differs from the account's email address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Authentication mechanism.
    #[serde(default)]
    pub auth: AuthMethod,
    /// Name of the Secret Service entry holding the password. A reference,
    /// never the secret. Defaults to `postio:<account id>:smtp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyring_entry: Option<String>,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: smtp_port(),
            security: MailSecurity::default(),
            username: None,
            auth: AuthMethod::default(),
            keyring_entry: None,
            extra: Extras::new(),
        }
    }
}

/// One entry of `[accounts]`.
///
/// Every field has a default so that a half-written account still parses;
/// telling the user what is missing is the validation pass's job.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AccountConfig {
    /// The `[accounts.<id>]` table key. Filled in after parsing, never written.
    #[serde(skip)]
    pub id: String,
    /// The account's email address.
    #[serde(default)]
    pub email: String,
    /// Human name used in the `From:` header and the sidebar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Select this account at startup.
    #[serde(default, rename = "default")]
    pub is_default: bool,
    /// Incoming mail.
    #[serde(default)]
    pub imap: ImapConfig,
    /// Outgoing mail.
    #[serde(default)]
    pub smtp: SmtpConfig,
    /// Keys this version of Postio does not know, preserved verbatim.
    #[serde(flatten)]
    pub extra: Extras,
}

impl AccountConfig {
    /// Keyring entry holding the IMAP password.
    pub fn imap_keyring_entry(&self) -> String {
        self.imap
            .keyring_entry
            .clone()
            .unwrap_or_else(|| format!("postio:{}:imap", self.id))
    }

    /// Keyring entry holding the SMTP password.
    pub fn smtp_keyring_entry(&self) -> String {
        self.smtp
            .keyring_entry
            .clone()
            .unwrap_or_else(|| format!("postio:{}:smtp", self.id))
    }

    /// IMAP login name: the explicit `username`, else the email address.
    pub fn imap_username(&self) -> &str {
        self.imap.username.as_deref().unwrap_or(&self.email)
    }

    /// SMTP login name: the explicit `username`, else the email address.
    pub fn smtp_username(&self) -> &str {
        self.smtp.username.as_deref().unwrap_or(&self.email)
    }
}
