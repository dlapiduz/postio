use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, IdentityId, SignatureId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransportSecurity {
    /// Plaintext. Only ever valid for a local test server.
    None,
    StartTls,
    #[default]
    Tls,
}

impl TransportSecurity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartTls => "starttls",
            Self::Tls => "tls",
        }
    }

    /// `None` for an unknown name: a corrupt row or hand-edited config must not be guessed at.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuthMethod {
    #[default]
    Password,
    AppPassword,
    OAuth2,
    XOAuth2,
}

impl AuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::AppPassword => "app_password",
            Self::OAuth2 => "oauth2",
            Self::XOAuth2 => "xoauth2",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "password" => Some(Self::Password),
            "app_password" => Some(Self::AppPassword),
            "oauth2" => Some(Self::OAuth2),
            "xoauth2" => Some(Self::XOAuth2),
            _ => None,
        }
    }
}

/// Carries no secret: credentials live in the keyring, never in the model, `config.toml` or logs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub security: TransportSecurity,
    pub username: String,
}

/// Which protocol adapter reaches this account's server, chosen at add-account time and persisted
/// (ADR 0018 Q5).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    Imap,
    Jmap {
        /// RFC 8620 session URL, persisted so startup needs no discovery.
        session_url: String,
    },
    Gmail,
    Maildir {
        /// Absolute path of the tree's root.
        root: String,
    },
}

impl Backend {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Imap => "imap",
            Self::Jmap { .. } => "jmap",
            Self::Gmail => "gmail",
            Self::Maildir { .. } => "maildir",
        }
    }
}

/// OAuth data persisted at sign-in so the engine rebuilds its token source offline at every launch
/// (#534).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    /// Resolved at sign-in (preset or RFC 8414) and persisted so startup needs no network.
    pub token_url: String,
    /// Persisted so re-authentication after a revoked grant works offline.
    pub authorize_url: String,
    /// Space-joined, per RFC 6749 §3.3.
    pub scopes: String,
    /// Days the provider says the refresh grant lives, copied from the preset at sign-in so launch
    /// needs no
    /// preset table. `None` means no early deadline (#954).
    #[serde(default)]
    pub refresh_token_lifetime_days: Option<u32>,
}

/// A named signature owned by the account, so a draft can pick any of them; an identity names its
/// default (#12).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Signature {
    #[serde(default)]
    pub id: SignatureId,
    /// Unique within the account.
    #[serde(default)]
    pub name: String,
    pub text: String,
    pub html: Option<String>,
}

impl Signature {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: SignatureId::UNASSIGNED,
            name: name.into(),
            text: text.into(),
            html: None,
        }
    }

    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub id: IdentityId,
    pub account_id: AccountId,
    pub display_name: String,
    pub address: EmailAddress,
    pub reply_to: Option<EmailAddress>,
    pub signature: Option<Signature>,
    pub is_default: bool,
}

impl Identity {
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

    pub fn effective_reply_to(&self) -> &EmailAddress {
        self.reply_to.as_ref().unwrap_or(&self.address)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: AccountId,
    pub display_name: String,
    pub address: EmailAddress,
    pub incoming: ServerConfig,
    pub outgoing: ServerConfig,
    pub auth: AuthMethod,
    /// `None` on password accounts and external-broker accounts. Never a secret: the client secret
    /// and
    /// refresh token live in the keyring.
    pub oauth: Option<OAuthConfig>,
    #[serde(default)]
    pub backend: Backend,
    pub enabled: bool,
    pub identities: Vec<Identity>,
    #[serde(default)]
    pub signatures: Vec<Signature>,
    /// In bytes; `None` means no limit and nothing is checked. Never read from the SMTP `SIZE`
    /// capability:
    /// postio-smtp relies on no capability list.
    pub max_message_size: Option<u64>,
    /// Account-wide override of a new draft's signature; `None` falls through (see
    /// [`crate::signature_default::resolve`]).
    #[serde(default)]
    pub default_signature_id: Option<SignatureId>,
    pub created_at: DateTime<Utc>,
    /// The account new messages come from when the message does not say (#960). It orders nothing
    /// and
    /// reply, reply-all and forward never read it: the replied-to message decides the address
    /// ([`crate::reply`]).
    /// At most one account carries it (`AccountRepository::set_default`); none is the ordinary
    /// state.
    #[serde(default)]
    pub is_default: bool,
    /// Set by Remove in settings (undoable). The cascade delete runs at next startup, before any
    /// engine
    /// exists (`AccountRepository::reap_pending_deletions`) (#464).
    #[serde(default)]
    pub pending_deletion: bool,
}

impl Account {
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
            oauth: None,
            backend: Backend::default(),
            enabled: true,
            identities: Vec::new(),
            signatures: Vec::new(),
            max_message_size: None,
            default_signature_id: None,
            is_default: false,
            created_at: Utc::now(),
            pending_deletion: false,
        }
    }

    /// `None` only when the account has no identities.
    pub fn default_identity(&self) -> Option<&Identity> {
        self.identities
            .iter()
            .find(|identity| identity.is_default)
            .or_else(|| self.identities.first())
    }

    /// The identity a message sent to `recipients` was addressed to: a reply must come from the
    /// address the
    /// mail arrived at. First match wins (`To` before `Cc`); an identity's `Reply-To` counts as its
    /// address.
    /// Falls back to the default identity, since mail can arrive by routes absent from headers.
    pub fn identity_for(&self, recipients: &[EmailAddress]) -> Option<&Identity> {
        recipients
            .iter()
            .find_map(|recipient| {
                self.identities.iter().find(|identity| {
                    identity.address.same_address(recipient)
                        || identity.effective_reply_to().same_address(recipient)
                })
            })
            .or_else(|| self.default_identity())
    }

    pub fn owns_address(&self, address: &EmailAddress) -> bool {
        self.address.same_address(address)
            || self
                .identities
                .iter()
                .any(|identity| identity.address.same_address(address))
    }
}

/// How many accounts a view is about: one, or all. Not an `Option<AccountId>`, whose `None` meant
/// both
/// "none chosen" and "all". `Account` can be a destination; `Unified` is a view and never one.
/// Lives here so list state and `SearchRequest.account` share one type and cannot disagree (ADR
/// 0005 Q10, #186).
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum AccountScope {
    Account(AccountId),
    /// A view, never a destination. The default: unified over zero accounts is empty, as a fresh
    /// install must show.
    #[default]
    Unified,
}

impl AccountScope {
    pub fn account(self) -> Option<AccountId> {
        match self {
            Self::Account(id) => Some(id),
            Self::Unified => None,
        }
    }

    pub fn is_single_account(self) -> bool {
        matches!(self, Self::Account(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scope_names_an_account_only_when_it_is_about_one() {
        let one = AccountScope::Account(AccountId::new(7));
        assert_eq!(one.account(), Some(AccountId::new(7)));
        assert!(one.is_single_account());

        assert_eq!(AccountScope::Unified.account(), None);
        assert!(!AccountScope::Unified.is_single_account());
    }

    #[test]
    fn unified_is_the_default_scope() {
        assert_eq!(AccountScope::default(), AccountScope::Unified);
    }

    #[test]
    fn transport_security_round_trips_through_its_stored_identifier() {
        for security in [
            TransportSecurity::None,
            TransportSecurity::StartTls,
            TransportSecurity::Tls,
        ] {
            assert_eq!(
                TransportSecurity::from_name(security.as_str()),
                Some(security)
            );
        }
        assert_eq!(TransportSecurity::from_name("TLS"), None);
    }

    fn account_with_identities() -> Account {
        let mut account = Account::new(
            "Ada",
            EmailAddress::new(Some("Ada Lovelace"), "ada@example.com"),
        );
        let mut work = Identity::new(
            account.id,
            EmailAddress::new(Some("Ada Lovelace"), "ada@work.example.com"),
        );
        work.id = IdentityId::new(1);
        let mut personal = Identity::new(
            account.id,
            EmailAddress::new(Some("Ada"), "ada@example.com"),
        );
        personal.id = IdentityId::new(2);
        personal.is_default = true;
        let mut list = Identity::new(
            account.id,
            EmailAddress::new(Some("Ada"), "ada+lists@example.com"),
        );
        list.id = IdentityId::new(3);
        list.reply_to = Some(EmailAddress::new(None::<String>, "ada@lists.example.org"));
        account.identities = vec![work, personal, list];
        account
    }

    #[test]
    fn a_reply_comes_from_the_identity_the_mail_was_addressed_to() {
        let account = account_with_identities();
        let to = |address: &str| EmailAddress::new(None::<String>, address);

        assert_eq!(
            account
                .identity_for(&[to("ADA@WORK.EXAMPLE.COM")])
                .map(|identity| identity.id),
            Some(IdentityId::new(1)),
            "matched case-insensitively, as addresses compare"
        );

        assert_eq!(
            account
                .identity_for(&[to("ada@work.example.com"), to("ada@example.com")])
                .map(|identity| identity.id),
            Some(IdentityId::new(1))
        );

        assert_eq!(
            account
                .identity_for(&[to("ada@lists.example.org")])
                .map(|identity| identity.id),
            Some(IdentityId::new(3)),
            "an explicit Reply-To is one of the identity's addresses"
        );
    }

    #[test]
    fn an_unrecognized_recipient_falls_back_to_the_default_identity() {
        let account = account_with_identities();
        let elsewhere = EmailAddress::new(None::<String>, "someone@example.net");
        assert_eq!(
            account.identity_for(&[elsewhere]).map(|i| i.id),
            Some(IdentityId::new(2))
        );
        assert_eq!(
            account.identity_for(&[]).map(|i| i.id),
            Some(IdentityId::new(2)),
            "and so does no recipient at all"
        );

        let empty = Account::new("nobody", EmailAddress::new(None::<String>, "n@example.com"));
        assert_eq!(empty.identity_for(&[]), None);
    }

    #[test]
    fn auth_methods_round_trip_through_their_stored_identifiers() {
        for method in [
            AuthMethod::Password,
            AuthMethod::AppPassword,
            AuthMethod::OAuth2,
            AuthMethod::XOAuth2,
        ] {
            assert_eq!(AuthMethod::from_name(method.as_str()), Some(method));
        }
        assert_eq!(AuthMethod::from_name("app-password"), None);
    }
}
