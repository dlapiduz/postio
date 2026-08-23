//! Credential storage for Postio accounts.
//!
//! Postio keeps account passwords in the OS keyring (Secret Service, or the
//! Flatpak keyring portal when sandboxed) and nowhere else. `config.toml`
//! records *where* a password lives — never the password. There is
//! deliberately no plaintext option: a config file at mode 0644 holding an
//! app-specific password is exactly the failure this module exists to prevent.
//!
//! Two sources are supported, both behind [`SecretStore`]:
//!
//! * [`KeyringSecretStore`] — the default, backed by `oo7`.
//! * [`CommandSecretStore`] — the escape hatch for people who already keep
//!   their secrets in `pass`, `gopass`, `age`, a hardware token wrapper, and
//!   so on. Read-only by construction.
//!
//! [`MemorySecretStore`] is the in-memory double the test suite runs on so
//! that no test needs a live Secret Service session.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Attribute value identifying items this application owns.
const APPLICATION: &str = "postio";

/// Freedesktop schema attribute, so `seahorse` and friends group our items.
const SCHEMA: &str = "org.postio.Account";

/// What to tell a user whose keyring will not open. Every locked-keyring
/// error ends with this; it is the only path forward, because Postio has no
/// plaintext fallback to offer.
const UNLOCK_HINT: &str = "Unlock it (log in again, or open Passwords and Keys \
                           and unlock the Login keyring) and retry.";

// ---------------------------------------------------------------------------
// Password
// ---------------------------------------------------------------------------

/// A password held in memory.
///
/// Zeroized on drop, redacted in `Debug` and `Display`, and — importantly —
/// neither `Serialize` nor `Deserialize`, so it cannot end up in a config
/// file or a log line by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(Zeroizing<String>);

impl Password {
    /// Wraps a password.
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// Borrows the secret. Every call site should be short-lived and obvious
    /// in review — that is the point of the explicit name.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// True when the stored password is empty, which no backend should ever
    /// return for a configured account.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

impl fmt::Display for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

// ---------------------------------------------------------------------------
// Account key
// ---------------------------------------------------------------------------

/// Identifies the credential belonging to one account.
///
/// Safe to log: it holds the account address, never the secret.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountKey {
    account: String,
}

impl AccountKey {
    /// Keys a credential by the account's address.
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
        }
    }

    /// The account this credential belongs to.
    pub fn account(&self) -> &str {
        &self.account
    }

    /// The label shown by keyring browsers such as `seahorse`.
    pub fn label(&self) -> String {
        format!("Postio ({})", self.account)
    }

    /// Secret Service lookup attributes for this account.
    pub fn attributes(&self) -> [(&'static str, &str); 3] {
        [
            ("application", APPLICATION),
            ("account", self.account.as_str()),
            (oo7::XDG_SCHEMA_ATTRIBUTE, SCHEMA),
        ]
    }
}

impl fmt::Display for AccountKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.account)
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Everything that can go wrong reaching a credential.
///
/// No variant carries a password, so these are safe to log verbatim.
#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    /// The keyring exists but will not open. Recoverable by the user, so the
    /// message says how.
    #[error(
        "the {keyring} keyring is locked, so Postio cannot read the password for {account}. {UNLOCK_HINT}"
    )]
    Locked {
        /// Which keyring refused: usually `login`.
        keyring: String,
        /// The account whose password was wanted.
        account: String,
    },

    /// No credential is stored for this account yet.
    #[error("no password is stored for {account}; add one and Postio will keep it in the keyring")]
    NotFound {
        /// The account with no stored credential.
        account: String,
    },

    /// The `command` source failed, or was asked to do something it cannot.
    #[error("secret command {command} failed: {reason}")]
    Command {
        /// The command line, as configured.
        command: String,
        /// Why it failed — an exit status, its stderr, or a usage error.
        reason: String,
    },

    /// The keyring backend itself errored.
    #[error("keyring error while handling the password for {account}: {reason}")]
    Backend {
        /// The account involved.
        account: String,
        /// The backend's own message.
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// The store trait
// ---------------------------------------------------------------------------

/// Where an account's password lives.
///
/// Abstracted so the whole test suite can run without a Secret Service
/// session; see [`MemorySecretStore`].
#[async_trait]
pub trait SecretStore: Send + Sync + fmt::Debug {
    /// Short name of this backend, for diagnostics and config round-trips.
    fn describe(&self) -> &'static str;

    /// Stores (or replaces) the password for `key`.
    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError>;

    /// Reads the password for `key`.
    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError>;

    /// Removes the password for `key`. Removing an absent password succeeds.
    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError>;
}

// ---------------------------------------------------------------------------
// Config-facing source
// ---------------------------------------------------------------------------

/// The `config.toml` representation of where a password lives.
///
/// There is no `raw` variant, and there never will be. Deserialization goes
/// through [`SecretSourceRepr`] rather than serde's internally-tagged enum
/// derive specifically so that an unknown key is a hard error: serde's
/// `deny_unknown_fields` is silently ineffective on internally-tagged enums
/// (the content is buffered and unit variants ignore whatever is left over),
/// which would let a `raw = "..."` sit in `config.toml` looking honoured
/// while Postio quietly used the keyring — or worse, let a user believe
/// Postio was reading it. A stray plaintext key must stop the load and say
/// so.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SecretSource {
    /// The OS keyring. The default, and the only writable source.
    #[default]
    Keyring,
    /// Run a program and read the password from its standard output.
    Command {
        /// Program and arguments, e.g. `["pass", "show", "mail/icloud"]`.
        argv: Vec<String>,
    },
}

/// Wire shape of [`SecretSource`]. A plain struct, so `deny_unknown_fields`
/// actually bites.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct SecretSourceRepr {
    r#type: SecretSourceKind,
    #[serde(default)]
    argv: Option<Vec<String>>,
}

/// The `type` discriminant. Anything else — `raw`, `plain`, `cleartext` — is
/// an unknown variant and therefore an error.
#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum SecretSourceKind {
    Keyring,
    Command,
}

impl<'de> Deserialize<'de> for SecretSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let repr = SecretSourceRepr::deserialize(deserializer)?;

        match repr.r#type {
            SecretSourceKind::Keyring => match repr.argv {
                None => Ok(Self::Keyring),
                Some(_) => Err(D::Error::custom(
                    "the keyring secret source takes no `argv`",
                )),
            },
            SecretSourceKind::Command => match repr.argv {
                Some(argv) if !argv.is_empty() => Ok(Self::Command { argv }),
                Some(_) => Err(D::Error::custom("`argv` must name a program to run")),
                None => Err(D::Error::missing_field("argv")),
            },
        }
    }
}

impl SecretSource {
    /// Builds the store this source describes.
    pub fn build(&self) -> Arc<dyn SecretStore> {
        match self {
            Self::Keyring => Arc::new(KeyringSecretStore::new()),
            Self::Command { argv } => Arc::new(CommandSecretStore::new(argv.clone())),
        }
    }
}

// ---------------------------------------------------------------------------
// Keyring store
// ---------------------------------------------------------------------------

/// Secret Service / Flatpak portal backed store. The default.
#[derive(Clone, Debug, Default)]
pub struct KeyringSecretStore {
    _private: (),
}

impl KeyringSecretStore {
    /// Builds a store against the session's keyring.
    ///
    /// The connection is opened lazily on first use so that constructing an
    /// account never blocks on D-Bus.
    pub fn new() -> Self {
        Self::default()
    }

    async fn keyring(&self, key: &AccountKey) -> Result<oo7::Keyring, SecretError> {
        let keyring = oo7::Keyring::new()
            .await
            .map_err(|err| map_oo7_error(key, err))?;

        // Best effort: a locked keyring prompts here rather than failing
        // deeper in with a less obvious message.
        if let Err(err) = keyring.unlock().await {
            return Err(map_oo7_error(key, err));
        }

        Ok(keyring)
    }
}

#[async_trait]
impl SecretStore for KeyringSecretStore {
    fn describe(&self) -> &'static str {
        "keyring"
    }

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        let keyring = self.keyring(key).await?;
        keyring
            .create_item(
                &key.label(),
                &key.attributes(),
                oo7::Secret::text(password.expose()),
                true,
            )
            .await
            .map_err(|err| map_oo7_error(key, err))
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        let keyring = self.keyring(key).await?;
        let items = keyring
            .search_items(&key.attributes())
            .await
            .map_err(|err| map_oo7_error(key, err))?;

        let item = items.first().ok_or_else(|| SecretError::NotFound {
            account: key.account().to_owned(),
        })?;

        let secret = item.secret().await.map_err(|err| map_oo7_error(key, err))?;
        let password =
            String::from_utf8(secret.as_bytes().to_vec()).map_err(|_| SecretError::Backend {
                account: key.account().to_owned(),
                reason: "the stored secret is not valid UTF-8".to_owned(),
            })?;

        Ok(Password::new(password))
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        let keyring = self.keyring(key).await?;
        keyring
            .delete(&key.attributes())
            .await
            .map_err(|err| map_oo7_error(key, err))
    }
}

/// Turns an `oo7` failure into something a user can act on.
///
/// The locked cases are the ones that matter: a dismissed prompt and an
/// `IsLocked` service error both mean "the keyring did not open", and both
/// must produce the unlock instructions rather than a D-Bus stack trace.
fn map_oo7_error(key: &AccountKey, err: oo7::Error) -> SecretError {
    let account = key.account().to_owned();

    let locked = |keyring: &str| SecretError::Locked {
        keyring: keyring.to_owned(),
        account: account.clone(),
    };

    match err {
        oo7::Error::File(oo7::file::Error::Locked) => locked("file"),
        oo7::Error::DBus(oo7::dbus::Error::Dismissed) => locked("login"),
        oo7::Error::DBus(oo7::dbus::Error::Service(oo7::dbus::ServiceError::IsLocked(_))) => {
            locked("login")
        }
        other => SecretError::Backend {
            account,
            reason: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// Command store
// ---------------------------------------------------------------------------

/// Runs a program and takes its standard output as the password.
///
/// This is the `pass`/`gopass`/`age` escape hatch. It is read-only: Postio
/// will not try to guess how to write a secret back into someone else's
/// secret manager.
#[derive(Clone, Debug)]
pub struct CommandSecretStore {
    argv: Vec<String>,
}

impl CommandSecretStore {
    /// Builds a store that runs `argv`.
    pub fn new<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
        }
    }

    fn rendered(&self) -> String {
        format!("`{}`", self.argv.join(" "))
    }

    fn failure(&self, reason: impl Into<String>) -> SecretError {
        SecretError::Command {
            command: self.rendered(),
            reason: reason.into(),
        }
    }
}

#[async_trait]
impl SecretStore for CommandSecretStore {
    fn describe(&self) -> &'static str {
        "command"
    }

    async fn store(&self, _key: &AccountKey, _password: &Password) -> Result<(), SecretError> {
        Err(self.failure(
            "the command secret source is read-only; store the password with your own \
             secret manager, or switch this account to the keyring",
        ))
    }

    async fn retrieve(&self, _key: &AccountKey) -> Result<Password, SecretError> {
        let (program, args) = self
            .argv
            .split_first()
            .ok_or_else(|| self.failure("no program was configured"))?;

        let output = tokio::process::Command::new(program)
            .args(args)
            .output()
            .await
            .map_err(|err| self.failure(err.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let status = match output.status.code() {
                Some(code) => format!("exited with status {code}"),
                None => "was killed by a signal".to_owned(),
            };
            return Err(self.failure(if stderr.is_empty() {
                status
            } else {
                format!("{status}: {stderr}")
            }));
        }

        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| self.failure("the command printed something that is not valid UTF-8"))?;

        // `pass show` and friends print a trailing newline that is not part
        // of the password; take the first line and nothing else.
        let password = stdout
            .split('\n')
            .next()
            .unwrap_or_default()
            .trim_end_matches('\r');

        if password.is_empty() {
            return Err(self.failure("the command printed nothing"));
        }

        Ok(Password::new(password))
    }

    async fn delete(&self, _key: &AccountKey) -> Result<(), SecretError> {
        Err(self.failure(
            "the command secret source is read-only; remove the password with your own \
             secret manager",
        ))
    }
}

// ---------------------------------------------------------------------------
// In-memory store (test double)
// ---------------------------------------------------------------------------

/// In-memory [`SecretStore`] for tests.
///
/// Public on purpose: crates above this one need to test their credential
/// paths without a Secret Service session either. [`MemorySecretStore::locked`]
/// simulates a keyring that will not open.
#[derive(Clone, Debug, Default)]
pub struct MemorySecretStore {
    items: Arc<Mutex<HashMap<AccountKey, String>>>,
    locked: bool,
}

impl MemorySecretStore {
    /// An empty, unlocked store.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store that behaves like a keyring nobody has unlocked.
    pub fn locked() -> Self {
        Self {
            items: Arc::new(Mutex::new(HashMap::new())),
            locked: true,
        }
    }

    /// A fresh handle onto the same contents — the equivalent of restarting
    /// Postio and opening the same keyring again.
    pub fn reopen(&self) -> Self {
        self.clone()
    }

    /// How many credentials are held.
    pub fn len(&self) -> usize {
        self.items.lock().expect("secret store mutex").len()
    }

    /// Whether any credential is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn guard(&self, key: &AccountKey) -> Result<(), SecretError> {
        if self.locked {
            return Err(SecretError::Locked {
                keyring: "login".to_owned(),
                account: key.account().to_owned(),
            });
        }
        Ok(())
    }
}

#[async_trait]
impl SecretStore for MemorySecretStore {
    fn describe(&self) -> &'static str {
        "memory"
    }

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        self.guard(key)?;
        self.items
            .lock()
            .expect("secret store mutex")
            .insert(key.clone(), password.expose().to_owned());
        Ok(())
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        self.guard(key)?;
        self.items
            .lock()
            .expect("secret store mutex")
            .get(key)
            .map(Password::new)
            .ok_or_else(|| SecretError::NotFound {
                account: key.account().to_owned(),
            })
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        self.guard(key)?;
        self.items.lock().expect("secret store mutex").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_identify_the_application_and_account() {
        let key = AccountKey::new("ada@example.com");
        let attributes = key.attributes();

        assert!(attributes.contains(&("application", APPLICATION)));
        assert!(attributes.contains(&("account", "ada@example.com")));
        assert!(
            attributes
                .iter()
                .any(|(k, v)| *k == oo7::XDG_SCHEMA_ATTRIBUTE && *v == SCHEMA)
        );
    }

    #[test]
    fn a_source_rejects_a_mismatched_or_empty_argv() {
        assert!(toml::from_str::<SecretSource>("type = \"keyring\"\nargv = [\"pass\"]").is_err());
        assert!(toml::from_str::<SecretSource>("type = \"command\"").is_err());
        assert!(toml::from_str::<SecretSource>("type = \"command\"\nargv = []").is_err());
    }

    #[test]
    fn there_is_no_plaintext_variant_to_select() {
        for kind in ["raw", "plain", "cleartext", "password"] {
            assert!(
                toml::from_str::<SecretSource>(&format!("type = \"{kind}\"")).is_err(),
                "`{kind}` was accepted as a secret source"
            );
        }
    }

    #[test]
    fn the_unlock_hint_never_mentions_a_plaintext_fallback() {
        let rendered = SecretError::Locked {
            keyring: "login".into(),
            account: "ada@example.com".into(),
        }
        .to_string();

        assert!(rendered.contains("Unlock it"));
        assert!(!rendered.contains("config.toml"));
    }
}
