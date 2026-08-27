//! Accounts, their servers, and the identities that send from them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::address::EmailAddress;
use crate::ids::{AccountId, IdentityId, SignatureId};

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

impl TransportSecurity {
    /// A stable lowercase identifier, for storage and config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::StartTls => "starttls",
            Self::Tls => "tls",
        }
    }

    /// The inverse of [`TransportSecurity::as_str`].
    ///
    /// `None` for anything else: a value that is not one of these came from a
    /// corrupt row or a hand-edited config, and guessing at it would be worse
    /// than saying so.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "none" => Some(Self::None),
            "starttls" => Some(Self::StartTls),
            "tls" => Some(Self::Tls),
            _ => None,
        }
    }
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

impl AuthMethod {
    /// A stable lowercase identifier, for storage and config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::AppPassword => "app_password",
            Self::OAuth2 => "oauth2",
            Self::XOAuth2 => "xoauth2",
        }
    }

    /// The inverse of [`AuthMethod::as_str`].
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
///
/// Owned by the account and named, rather than being a property of one
/// identity (#12): a person has a long form and a short one, or one with a
/// disclaimer for mail leaving the company, and which to use is a decision
/// per message rather than per address. An identity names the one it signs
/// with by default; the composer can point a draft at any of the others.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Signature {
    /// Local id. Unassigned until it has been stored.
    #[serde(default)]
    pub id: SignatureId,
    /// What the composer's picker shows. Unique within the account.
    #[serde(default)]
    pub name: String,
    /// Plain-text form.
    pub text: String,
    /// Rich form, when there is one.
    pub html: Option<String>,
}

impl Signature {
    /// A signature called `name`, with `text` as its only form.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: SignatureId::UNASSIGNED,
            name: name.into(),
            text: text.into(),
            html: None,
        }
    }

    /// The same signature with a rich form as well.
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.html = Some(html.into());
        self
    }
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
    /// Every signature this account can sign with, in picker order (#12).
    ///
    /// An identity's own signature is one of these; the composer offers the
    /// rest so a draft can sign differently without changing who it is from.
    #[serde(default)]
    pub signatures: Vec<Signature>,
    /// An account-wide override of what a new draft starts signed with
    /// (#12's last item, #394) — see [`signature_default::resolve`] for the
    /// full precedence this participates in, alongside a mailbox's own
    /// override and an identity's own signature.
    ///
    /// `None` means this account has no opinion, and resolution falls
    /// through to whatever would have applied without it.
    #[serde(default)]
    pub default_signature_id: Option<SignatureId>,
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
            signatures: Vec::new(),
            default_signature_id: None,
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

    /// The identity a message sent to `recipients` was addressed to.
    ///
    /// A reply has to come from the address the mail arrived at — answering
    /// from the wrong one is how a thread ends up split across two mailboxes
    /// at the other end, and how a personal address leaks into a work thread.
    ///
    /// `recipients` is walked in order and the first match wins, so a caller
    /// passing `To` before `Cc` gets the identity the message was actually
    /// *sent* to rather than one that was merely copied. An identity's
    /// explicit `Reply-To` counts as one of its addresses, because mail
    /// answering that identity is addressed to it however it is spelled.
    ///
    /// Falls back to [`Account::default_identity`] when nothing matches: mail
    /// reaches an account by routes that are not in any header, and a reply
    /// with no `From` at all is worse than one from the usual address.
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

/// How many accounts a view is about: one, or all of them.
///
/// Deliberately not an `Option<AccountId>`. There is always a scope, and the
/// old `None` had to mean both "no account is chosen" and "every account at
/// once" — see [`AccountScope::Unified`].
///
/// The asymmetry is the point. [`AccountScope::Account`] names real mailboxes
/// on a real server, so it can be a *destination*; [`AccountScope::Unified`]
/// is a view assembled across every enabled account and can never be one.
///
/// # Why this lives in `postio-model` (#186)
///
/// It began in `postio_core::state` (#182), which is the natural home for a
/// thing the UI's state machine holds — and then search needed exactly the
/// same value. `postio-index` cannot depend on `postio-core` and should not,
/// so the choice was between defining a second, parallel enum of the same two
/// variants and moving this one down to the crate both already share.
///
/// A duplicate would be small and would look harmless. It would also be two
/// things that must agree about what "unified" means, in a codebase where
/// `AppState.scope` and `SearchRequest.account` are supposed to be the *same*
/// answer to the same question — so the list and the search bar could
/// disagree about which accounts they are showing, which is precisely the
/// class of bug ADR 0005 Q10 is about. One type cannot drift from itself.
/// `postio_core::state::Scope` re-exports this, so nothing #182 wrote had to
/// change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AccountScope {
    /// One account's own mailboxes.
    Account(AccountId),
    /// Every enabled account at once. A view, never a destination.
    ///
    /// The default, and it is not a placeholder: unified over zero accounts
    /// is empty, which is exactly what a fresh install has to show.
    #[default]
    Unified,
}

impl AccountScope {
    /// The account this scope names, or `None` for [`AccountScope::Unified`].
    ///
    /// The one place the old `Option<AccountId>` survives, and it now means
    /// exactly one thing: "this view is not about a single account."
    pub fn account(self) -> Option<AccountId> {
        match self {
            Self::Account(id) => Some(id),
            Self::Unified => None,
        }
    }

    /// Whether this scope can be the destination of a move.
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
        // Not a placeholder: unified over zero accounts is empty, which is
        // what a fresh install has to show.
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

        // To first, then Cc: the identity it was sent to beats one copied.
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
