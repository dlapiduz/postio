//! Adding an account without an onboarding screen (#649).
//!
//! The two writes onboarding makes — an account row in the encrypted store,
//! and a credential in the OS keyring — with nothing else attached to them.
//!
//! # Why this is here rather than in `postio-app`
//!
//! `postio-app/examples/provision.rs` did this first, and did it well enough
//! that this is largely a port of it. But `postio-app` links GTK, and GTK is
//! precisely what does not build on macOS: ADR 0019 measured it, and the one
//! boundary in the whole workspace falls on `postio-gtk` and `postio-app`. So
//! the only platform with no onboarding screen was also the only platform
//! that could not run the helper that stands in for one.
//!
//! `postio-session` builds and tests on `aarch64-apple-darwin` today, which
//! is what makes this reachable from a Mac. It also makes the helper the same
//! code on both platforms, rather than a second copy that drifts — the risk
//! ADR 0019 keeps naming, and the one that already cost this table its own
//! issue (#69, two provider lists disagreeing about one domain).
//!
//! # What stands in for `config.toml`
//!
//! Nothing. `[accounts.<id>]` was retired by #470 because nothing read it:
//! host, port, security and display name come from the store, written once by
//! onboarding, and editing that section saved and changed nothing. #649's
//! option (a) was written as "hand-write eight lines of TOML", which has not
//! been a path since. What is left is to make the same two writes onboarding
//! makes, which is this.
//!
//! # The order of the two writes
//!
//! **The credential first, then the row**, and this is not a preference.
//! 0.1.0 did it the other way and `postio-67` is what that cost: a keyring
//! write that failed after the row was committed left an account with no
//! reachable password. It could not sync, could not authenticate, and could
//! not be repaired from inside the application — `first_account().is_some()`
//! meant onboarding never ran again, so the one screen that writes
//! credentials was unreachable for exactly the account that needed it.
//!
//! The failure the safe order leaves behind is a credential no account row
//! names. Nothing reads one, it is rolled back here, and it would be harmless
//! even if the rollback failed. `postio_app::onboarding::persist` records the
//! same reasoning; this is the same rule in the crate that can be reached
//! without a toolkit.

use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{AccountKey, Password, SecretError, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity};
use postio_storage::Database;
use postio_storage::repository::AccountRepository;

/// What one provisioning run did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provisioned {
    /// The account was written, and its password stored.
    Created(AccountId),
    /// An account for that address was already here, and nothing was
    /// changed — see [`provision`] for why a re-run is deliberately inert.
    AlreadyProvisioned(AccountId),
}

/// Why an account could not be added.
///
/// Two cases rather than a string, because they call for different answers:
/// a keyring that will not open is something the user can fix and retry,
/// and a store that will not take a row is not.
#[derive(Debug)]
pub enum ProvisionError {
    /// The keyring would not take the password.
    Credential(SecretError),
    /// The store would not take the row.
    Store(postio_storage::Error),
}

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Named as a question the user can act on. A locked keyring is by
            // far the likeliest cause and it reads as a Postio bug otherwise.
            Self::Credential(error) => write!(
                f,
                "the password could not be stored in the keyring: {error}. \
                 Is the keyring unlocked?"
            ),
            Self::Store(error) => write!(f, "the account could not be written: {error}"),
        }
    }
}

impl std::error::Error for ProvisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Credential(error) => Some(error),
            Self::Store(error) => Some(error),
        }
    }
}

/// An account row from discovered settings, ready for [`provision`].
///
/// The one place discovery's vocabulary meets the store's. They describe the
/// same two servers twice — [`Encryption`] and [`TransportSecurity`] are the
/// same idea — and the join has to be lossless in both directions, because a
/// mapping that flattened everything to implicit TLS would dial the STARTTLS
/// port expecting TLS from the first byte. That fails as a connection error,
/// which a user reads as a wrong password.
///
/// The address is the identity; [`AccountSettings::login`] is the
/// credential's name, and the two differ more often than they look like they
/// should — every iCloud custom domain, for one. Both servers are told the
/// login and the identity keeps the address.
///
/// An identity is written here rather than left to the caller because an
/// account without one can receive mail and never answer it: the composer
/// picks a from-address off this list, and an empty list is not a state any
/// surface downstream handles.
pub fn account_from(settings: &AccountSettings) -> Account {
    let email = EmailAddress::new(None::<String>, settings.email.clone());
    // The provider's own name when it published one — "Example Mail" reads
    // better in the sidebar than the address does, and it is what the
    // onboarding screen shows too. The address is the honest fallback.
    let display_name = settings
        .display_name
        .clone()
        .unwrap_or_else(|| settings.email.clone());
    let mut account = Account::new(display_name, email.clone());
    account.incoming.host = settings.imap.host.clone();
    account.incoming.port = settings.imap.port;
    account.incoming.security = security(settings.imap.encryption);
    account.incoming.username = settings.login.clone();
    account.outgoing.host = settings.smtp.host.clone();
    account.outgoing.port = settings.smtp.port;
    account.outgoing.security = security(settings.smtp.encryption);
    account.outgoing.username = settings.login.clone();
    account.auth = AuthMethod::Password;

    let mut identity = Identity::new(AccountId::UNASSIGNED, email);
    identity.is_default = true;
    account.identities = vec![identity];
    account
}

/// Discovery's spelling of connection security, in the store's.
fn security(encryption: Encryption) -> TransportSecurity {
    match encryption {
        Encryption::Tls => TransportSecurity::Tls,
        Encryption::StartTls => TransportSecurity::StartTls,
        // Never chosen by Postio on its own; honoured only when a provider's
        // own autoconfig document says so, and carried rather than silently
        // upgraded so the settings a user is shown are the settings in use.
        Encryption::None => TransportSecurity::None,
    }
}

/// Write `account`'s credential and then its row. Answers which happened.
///
/// # Why a re-run changes nothing
///
/// An address already in the store answers [`Provisioned::AlreadyProvisioned`]
/// and stops. Two reasons, and the second is the one that bites:
///
/// - A second row for one address would leave `first_account` choosing
///   between them, and every draft that names an identity pointing at one of
///   the two arbitrarily.
/// - The credential already there is *working*. Re-running this from a script
///   or a shell whose environment has drifted would otherwise overwrite a
///   password that authenticates with one that does not, and turn a healthy
///   account into one that stopped syncing for no visible reason.
///
/// Repairing an account is onboarding's job, where there is a person to
/// confirm it. This helper only ever adds.
///
/// # Errors
///
/// [`ProvisionError::Credential`] if the keyring will not take the password —
/// nothing has been written, and a retry after unlocking it is safe.
/// [`ProvisionError::Store`] if the row cannot be written, in which case the
/// credential just stored is deleted again; see the [module docs](self) for
/// why that leftover would be harmless even if the delete failed.
pub async fn provision(
    database: &Database,
    secrets: &dyn SecretStore,
    mut account: Account,
    password: Password,
) -> Result<Provisioned, ProvisionError> {
    let address = account.address.address.clone();

    // Read before writing anything: an address already here is not an error
    // and must not cost a keyring round trip, let alone a write.
    {
        let connection = database.connection().map_err(ProvisionError::Store)?;
        let existing = AccountRepository::new(&connection)
            .list_enabled()
            .map_err(ProvisionError::Store)?
            .into_iter()
            .find(|found| found.address.address.eq_ignore_ascii_case(&address));
        if let Some(found) = existing {
            return Ok(Provisioned::AlreadyProvisioned(found.id));
        }
    }

    let key = AccountKey::new(address);
    secrets
        .store(&key, &password)
        .await
        .map_err(ProvisionError::Credential)?;

    let written = {
        let connection = database.connection().map_err(ProvisionError::Store)?;
        AccountRepository::new(&connection).create(&mut account)
    };
    match written {
        Ok(id) => Ok(Provisioned::Created(id)),
        Err(error) => {
            if let Err(cleanup) = secrets.delete(&key).await {
                // Safe to log: no `SecretError` carries a password.
                tracing::warn!(%cleanup, "the rolled-back credential could not be removed");
            }
            Err(ProvisionError::Store(error))
        }
    }
}

/// Settings for `address`: the provider preset table, with the environment
/// overriding it field by field.
///
/// The same table the onboarding screen reads, rather than a second copy of
/// it. This helper's ancestor carried its own three-provider version, so a
/// provider added for the screen was invisible here and the two could
/// disagree about one domain — #69 is what that cost, and "providers are data,
/// not code" is the rule it produced.
///
/// The real autoconfig probe stays onboarding's. Reimplementing it here would
/// be a second discovery path with nobody to confirm what it found, and a
/// guess about somebody's mail server is worse than a refusal: explicit hosts
/// are the escape hatch, and they are what a custom domain needs anyway.
///
/// # Errors
///
/// A sentence for the person running this, naming the variables to set — an
/// address whose domain the table does not know, and no hosts given, is the
/// ordinary case rather than a fault.
pub fn settings_for(address: &str) -> Result<AccountSettings, String> {
    settings_from(address, |key| {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    })
}

/// [`settings_for`], over an arbitrary environment lookup.
///
/// Split for the reason [`crate::paths`] splits `store_path_from`: the
/// override rules are worth asserting, and a function that reads the real
/// environment cannot be asserted about without setting variables in a
/// process shared by every other test in the binary.
fn settings_from<F>(address: &str, env: F) -> Result<AccountSettings, String>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(domain) = address.rsplit('@').next().map(str::to_ascii_lowercase) else {
        return Err(format!("postio: {address} does not look like an address"));
    };
    let mut settings = match postio_account::discovery::preset_for_domain(&domain) {
        Some(preset) => preset.settings_for(address),
        // Hosts left empty on purpose, and refused below if the environment
        // does not fill them in. Deriving `imap.<domain>` from the address is
        // the guess that dials somebody else's server.
        None => AccountSettings {
            email: address.to_owned(),
            imap: ServerSettings::new(String::new(), 993, Encryption::Tls),
            smtp: ServerSettings::new(String::new(), 465, Encryption::Tls),
            login: address.to_owned(),
            source: SettingsSource::Guess,
            requires_app_password: false,
            note: None,
            password_help_url: None,
            display_name: None,
            oauth: None,
            jmap: None,
            backends: Vec::new(),
        },
    };

    if let Some(host) = env("POSTIO_IMAP_HOST") {
        settings.imap.host = host;
    }
    if let Some(host) = env("POSTIO_SMTP_HOST") {
        settings.smtp.host = host;
    }
    if let Some(port) = env("POSTIO_IMAP_PORT").and_then(|value| value.parse().ok()) {
        settings.imap.port = port;
    }
    if let Some(port) = env("POSTIO_SMTP_PORT").and_then(|value| value.parse().ok()) {
        settings.smtp.port = port;
    }
    if let Some(login) = env("POSTIO_USERNAME") {
        settings.login = login;
    }

    if settings.imap.host.is_empty() || settings.smtp.host.is_empty() {
        return Err(format!(
            "postio: no built-in settings for {domain}, so the servers must be given:\n\
             \x20   export POSTIO_IMAP_HOST='imap.example.com'\n\
             \x20   export POSTIO_SMTP_HOST='smtp.example.com'\n\
             \x20   export POSTIO_USERNAME='...'   # if the login differs from the address\n\
             \n\
             For an iCloud custom domain those are imap.mail.me.com and \
             smtp.mail.me.com, with POSTIO_USERNAME set to the Apple ID \
             address rather than the custom one."
        ));
    }

    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An environment made of pairs, so a test says what is set and nothing
    /// else is.
    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + use<'a> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn a_domain_the_preset_table_knows_needs_no_environment_at_all() {
        // The whole point of the table: the ordinary provider is one address
        // and nothing else. Taken from the table rather than written out, so
        // this cannot go stale against it -- and so no real provider's
        // address appears in this repository (check-no-personal-data.py).
        let preset = postio_account::discovery::presets()
            .first()
            .expect("the preset table is not empty");
        let address = format!("ada@{}", preset.domains()[0]);

        let settings = settings_from(&address, env(&[])).expect("the table answers");

        assert_eq!(settings.imap.host, preset.imap_host());
        assert_eq!(settings.imap.port, preset.imap_port());
        assert_eq!(settings.smtp.host, preset.smtp_host());
        assert_eq!(settings.smtp.port, preset.smtp_port());
        assert_eq!(settings.login, address, "the login defaults to the address");
    }

    #[test]
    fn a_domain_nobody_publishes_settings_for_is_refused_rather_than_guessed() {
        // Refusing is the safe answer. `imap.<domain>` resolves for a great
        // many domains that are not mail servers, and an account pointed at
        // one of those is a password typed into somebody else's host.
        let error = settings_from("ada@nowhere.example", env(&[]))
            .expect_err("nothing publishes settings for that");

        assert!(error.contains("POSTIO_IMAP_HOST"), "got: {error}");
        assert!(error.contains("POSTIO_SMTP_HOST"), "got: {error}");
        assert!(
            error.contains("nowhere.example"),
            "the message has to name the domain it could not place, got: {error}"
        );
    }

    #[test]
    fn explicit_hosts_are_enough_for_a_domain_the_table_does_not_know() {
        // A self-hosted server, and every custom domain. This is the escape
        // hatch that keeps the refusal above from being a dead end.
        let settings = settings_from(
            "ada@nowhere.example",
            env(&[
                ("POSTIO_IMAP_HOST", "imap.example.com"),
                ("POSTIO_SMTP_HOST", "smtp.example.com"),
            ]),
        )
        .expect("given the servers, there is nothing left to look up");

        assert_eq!(settings.imap.host, "imap.example.com");
        assert_eq!(settings.smtp.host, "smtp.example.com");
        assert_eq!(settings.imap.port, 993, "the implicit-TLS defaults stand");
        assert_eq!(settings.smtp.port, 465);
    }

    #[test]
    fn the_environment_overrides_a_preset_field_by_field() {
        // An account on a provider the table knows but reached through a
        // different host -- a corporate gateway, a proxy, a migration in
        // progress. Overriding one field must not discard the rest.
        let preset = postio_account::discovery::presets()
            .first()
            .expect("the preset table is not empty");
        let address = format!("ada@{}", preset.domains()[0]);

        let settings = settings_from(
            &address,
            env(&[
                ("POSTIO_IMAP_HOST", "gateway.example.com"),
                ("POSTIO_IMAP_PORT", "1993"),
                ("POSTIO_USERNAME", "ada"),
            ]),
        )
        .expect("the table answers, the environment amends");

        assert_eq!(settings.imap.host, "gateway.example.com");
        assert_eq!(settings.imap.port, 1993);
        assert_eq!(
            settings.smtp.host,
            preset.smtp_host(),
            "overriding the incoming server must not blank the outgoing one"
        );
        assert_eq!(settings.login, "ada");
    }

    #[test]
    fn a_port_that_is_not_a_number_leaves_the_published_one_alone() {
        // `POSTIO_IMAP_PORT=993 ` with a stray character, or a shell that
        // exported the wrong thing. Falling back to the published port is
        // right; falling back to 0 would fail to connect with no clue why.
        let preset = postio_account::discovery::presets()
            .first()
            .expect("the preset table is not empty");
        let address = format!("ada@{}", preset.domains()[0]);

        let settings = settings_from(&address, env(&[("POSTIO_IMAP_PORT", "not-a-port")]))
            .expect("the table answers");

        assert_eq!(settings.imap.port, preset.imap_port());
    }
}
