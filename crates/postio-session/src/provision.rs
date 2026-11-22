//! Adding an account without an onboarding screen (#649): the same two writes onboarding makes, in
//! the crate that builds on macOS too.
//! The credential goes first, then the row: a keyring failure after the row left an account that
//! could not sync or be repaired (`postio-67`). A credential with no account is rolled back and
//! harmless.

use postio_account::discovery::{AccountSettings, Encryption, ServerSettings, SettingsSource};
use postio_account::secret::{AccountKey, Password, SecretError, SecretStore};
use postio_model::account::{AuthMethod, TransportSecurity};
use postio_model::ids::AccountId;
use postio_model::{Account, EmailAddress, Identity};
use postio_storage::Store;
use postio_storage::repository::AccountRepository;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provisioned {
    Created(AccountId),
    AlreadyProvisioned(AccountId),
}

#[derive(Debug)]
pub enum ProvisionError {
    Credential(SecretError),
    Store(postio_storage::Error),
    /// Nothing a typed password would put right; see `repair`. The sentence names what the account
    /// signs in with instead.
    Unrepairable(String),
}

impl std::fmt::Display for ProvisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Credential(error) => write!(
                f,
                "the password could not be stored in the keyring: {error}. \
                 Is the keyring unlocked?"
            ),
            Self::Store(error) => write!(f, "the account could not be written: {error}"),
            Self::Unrepairable(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for ProvisionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Credential(error) => Some(error),
            Self::Store(error) => Some(error),
            Self::Unrepairable(_) => None,
        }
    }
}

/// An account row from discovered settings, ready for `provision`. The security mapping must be
/// lossless: flattening to implicit TLS would dial a STARTTLS port expecting TLS, which reads as a
/// wrong password.
///
/// `AccountSettings::login` is the credential's name and differs from the address (every iCloud
/// custom domain). An identity is written here: an account without one can never answer mail.
pub fn account_from(settings: &AccountSettings) -> Account {
    let email = EmailAddress::new(None::<String>, settings.email.clone());
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

fn security(encryption: Encryption) -> TransportSecurity {
    match encryption {
        Encryption::Tls => TransportSecurity::Tls,
        Encryption::StartTls => TransportSecurity::StartTls,
        // Honoured only when a provider's autoconfig says so, and not silently upgraded.
        Encryption::None => TransportSecurity::None,
    }
}

/// Write `account`'s credential and then its row; returns which happened.
///
/// An address already in the store answers `AlreadyProvisioned` and changes nothing: a second row
/// would confuse `first_account`, and re-running from a drifted shell environment must not
/// overwrite a working password. Repairing is onboarding's job.
/// `ProvisionError::Credential` means nothing was written; `ProvisionError::Store` means the stored
/// credential was deleted again.
pub async fn provision(
    database: &Store,
    secrets: &dyn SecretStore,
    mut account: Account,
    password: Password,
) -> Result<Provisioned, ProvisionError> {
    let address = account.address.address.clone();

    {
        let connection = database.connect().await.map_err(ProvisionError::Store)?;
        let existing = AccountRepository::new(&connection)
            .list_enabled()
            .await
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
        let connection = database.connect().await.map_err(ProvisionError::Store)?;
        AccountRepository::new(&connection)
            .create(&mut account)
            .await
    };
    match written {
        Ok(id) => Ok(Provisioned::Created(id)),
        Err(error) => {
            if let Err(cleanup) = secrets.delete(&key).await {
                tracing::warn!(%cleanup, "the rolled-back credential could not be removed");
            }
            Err(ProvisionError::Store(error))
        }
    }
}

/// Replace the password an account already in the store signs in with (#1584).
///
/// Separate from `provision`, which is deliberately inert about an existing credential (a shell
/// re-run must not overwrite a working password). This replaces, and only for an account named by
/// id.
/// It refuses a token account, whose credential lives under a derived key and is minted by a
/// browser round trip: a typed password would be read by nothing and report a repair that changed
/// nothing. Local mail stores sign in to nothing.
/// It turns a disabled account back on, as GTK's repair does (ADR 0019).
pub async fn repair(
    database: &Store,
    secrets: &dyn SecretStore,
    account: AccountId,
    password: Password,
) -> Result<(), ProvisionError> {
    // Read the row and drop the checkout before touching the keyring, which can block on a prompt
    // while a pooled connection is held.
    let (address, enabled) = {
        let connection = database.connect().await.map_err(ProvisionError::Store)?;
        let found = AccountRepository::new(&connection)
            .get(account)
            .await
            .map_err(ProvisionError::Store)?;
        let Some(found) = found else {
            return Err(ProvisionError::Unrepairable(
                "That account is not in the store.".to_owned(),
            ));
        };
        if matches!(
            found.backend,
            postio_model::account::Backend::Maildir { .. }
        ) {
            return Err(ProvisionError::Unrepairable(format!(
                "{} is a mail store on this machine. It signs in to nothing, \
                 so there is no credential to replace.",
                found.address.address
            )));
        }
        if matches!(found.auth, AuthMethod::OAuth2 | AuthMethod::XOAuth2) {
            return Err(ProvisionError::Unrepairable(format!(
                "{} signs in through your browser, so its credential is \
                 granted rather than typed. Sign in again to reconnect it.",
                found.address.address
            )));
        }
        (found.address.address.clone(), found.enabled)
    };

    secrets
        .store(&AccountKey::new(address), &password)
        .await
        .map_err(ProvisionError::Credential)?;

    if !enabled {
        let connection = database.connect().await.map_err(ProvisionError::Store)?;
        AccountRepository::new(&connection)
            .set_enabled(account, true)
            .await
            .map_err(ProvisionError::Store)?;
    }
    Ok(())
}

/// Settings for `address`: the provider preset table, with the environment overriding it field by
/// field (#69). The real autoconfig probe stays onboarding's; a guess here would be worse than a
/// refusal.
///
/// `Err` is a sentence naming the variables to set, for an address whose domain the table does not
/// know.
pub fn settings_for(address: &str) -> Result<AccountSettings, String> {
    settings_from(address, |key| {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    })
}

fn settings_from<F>(address: &str, env: F) -> Result<AccountSettings, String>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(domain) = address.rsplit('@').next().map(str::to_ascii_lowercase) else {
        return Err(format!("postio: {address} does not look like an address"));
    };
    let mut settings = match postio_account::discovery::preset_for_domain(&domain) {
        Some(preset) => preset.settings_for(address),
        // Hosts left empty: deriving `imap.<domain>` is the guess that dials somebody else's
        // server.
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

/// Add an account that is a directory on this machine (#1278). One write, not two: a maildir
/// account has no credential, and an empty keyring entry would be a secret-shaped nothing. The root
/// is stored as an absolute path.
pub async fn provision_local(
    database: &Store,
    address: &str,
    root: &str,
) -> Result<Provisioned, ProvisionError> {
    let email = EmailAddress::new(None::<String>, address.to_owned());
    let connection = database.connect().await.map_err(ProvisionError::Store)?;
    let existing = AccountRepository::new(&connection)
        .list_enabled()
        .await
        .map_err(ProvisionError::Store)?
        .into_iter()
        .find(|found| found.address.address.eq_ignore_ascii_case(address));
    if let Some(found) = existing {
        return Ok(Provisioned::AlreadyProvisioned(found.id));
    }

    let mut account = Account::new(address.to_owned(), email.clone());
    account.backend = postio_model::account::Backend::Maildir {
        root: absolute(root).to_string_lossy().into_owned(),
    };
    // The auth column is not nullable; `Password` with nothing in the keyring already means
    // "nothing stored".
    account.auth = AuthMethod::Password;
    let mut identity = Identity::new(AccountId::UNASSIGNED, email);
    identity.is_default = true;
    account.identities = vec![identity];

    AccountRepository::new(&connection)
        .create(&mut account)
        .await
        .map(Provisioned::Created)
        .map_err(ProvisionError::Store)
}

/// `~/mail` as the directory it names. Only a leading `~` as the whole first component; `~ada` is
/// not guessed at.
pub fn absolute(path: &str) -> std::path::PathBuf {
    let trimmed = path.trim();
    let Some(rest) = trimmed.strip_prefix('~') else {
        return std::path::PathBuf::from(trimmed);
    };
    let Some(rest) = rest
        .strip_prefix('/')
        .or(if rest.is_empty() { Some("") } else { None })
    else {
        return std::path::PathBuf::from(trimmed);
    };
    match std::env::var_os("HOME") {
        Some(home) if !home.is_empty() => std::path::PathBuf::from(home).join(rest),
        _ => std::path::PathBuf::from(trimmed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let preset = postio_account::discovery::presets()
            .first()
            .expect("the preset table is not empty");
        let address = format!("ada@{}", preset.domains()[0]);

        let settings = settings_from(&address, env(&[("POSTIO_IMAP_PORT", "not-a-port")]))
            .expect("the table answers");

        assert_eq!(settings.imap.port, preset.imap_port());
    }
}
