//! Passwords live in the OS keyring (Secret Service, or the Flatpak portal) and nowhere else;
//! `config.toml` records where, never the password, and there is no plaintext option.
//! `MemorySecretStore` is the test double.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const APPLICATION: &str = "postio";

const SCHEMA: &str = "org.postio.Account";

/// Names the application they would actually look for, which differs per platform.
#[cfg(not(target_os = "macos"))]
const UNLOCK_HINT: &str = "Unlock it (log in again, or open Passwords and Keys \
                           and unlock the Login keyring) and retry.";
#[cfg(target_os = "macos")]
const UNLOCK_HINT: &str = "Unlock it (log in again, or open Keychain Access \
                           and unlock the login keychain) and retry.";

/// Zeroized on drop, redacted in `Debug` and `Display`, and neither `Serialize` nor `Deserialize`,
/// so it cannot reach a config file or log by accident. `Clone` is safe: each clone zeroizes
/// independently.
/// `PartialEq` is not constant time; never use it to verify a credential.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(Zeroizing<String>);

impl Password {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// Every call site should be short-lived and obvious in review.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Takes `Zeroizing` and borrows: `String::from_utf8` would hand back an un-zeroized allocation,
/// and on failure drop the bytes without overwriting them, the path the secret used to escape on
/// (#144).
fn secret_text(bytes: &Zeroizing<Vec<u8>>) -> Option<&str> {
    std::str::from_utf8(bytes).ok()
}

/// `pass show` prints the password on line one and metadata after it. A password may contain
/// spaces, so only the line ending is trimmed.
fn first_line(text: &str) -> &str {
    text.split('\n')
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
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

/// Safe to log: holds the address, never the secret.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AccountKey {
    account: String,
}

impl AccountKey {
    pub fn new(account: impl Into<String>) -> Self {
        Self {
            account: account.into(),
        }
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub fn label(&self) -> String {
        format!("Postio ({})", self.account)
    }

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

/// No variant carries a password, so these are safe to log verbatim. `Clone` because one refresh
/// attempt answers every waiting caller (ADR 0006 Q5, #194).
#[derive(Clone, Debug, thiserror::Error)]
pub enum SecretError {
    #[error(
        "the {keyring} keyring is locked, so Postio cannot read the password for {account}. {UNLOCK_HINT}"
    )]
    Locked { keyring: String, account: String },

    /// Recoverable only by signing in again, so it counts as an authentication failure and routes
    /// the account to "sign in again" (#954).
    #[error("the sign-in for {account} has expired; sign in again to keep this account syncing")]
    GrantExpired { account: String },

    #[error("no password is stored for {account}; add one and Postio will keep it in the keyring")]
    NotFound { account: String },

    #[error("secret command {command} failed: {reason}")]
    Command { command: String, reason: String },

    #[error("keyring error while handling the password for {account}: {reason}")]
    Backend { account: String, reason: String },

    /// Under Flatpak `RetrieveSecret` can never get a `Response` signal; without this bound a
    /// credential read hangs forever.
    #[error(
        "the keyring did not respond about {account} within {seconds}s; it may be unavailable or unresponsive"
    )]
    Timeout { account: String, seconds: u64 },
}

#[async_trait]
pub trait SecretStore: Send + Sync + fmt::Debug {
    fn describe(&self) -> &'static str;

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError>;

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError>;

    /// Removes the password for `key`. Removing an absent password succeeds.
    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError>;
}

/// No `raw` variant, ever. Deserialization goes through `SecretSourceRepr` because
/// `deny_unknown_fields` is silently ineffective on internally-tagged enums, which would let a `raw
/// = "..."` sit in `config.toml` looking honoured. A stray plaintext key must stop the load.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SecretSource {
    #[default]
    Keyring,
    Command {
        /// Program and arguments, e.g. `["pass", "show", "mail/icloud"]`.
        argv: Vec<String>,
    },
}

/// A plain struct, so `deny_unknown_fields` bites.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct SecretSourceRepr {
    r#type: SecretSourceKind,
    #[serde(default)]
    argv: Option<Vec<String>>,
}

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
    pub fn build(&self) -> Arc<dyn SecretStore> {
        match self {
            Self::Keyring => platform_keyring(),
            Self::Command { argv } => Arc::new(CommandSecretStore::new(argv.clone())),
        }
    }
}

/// `SecretSource::Keyring` means "wherever this system keeps secrets", so the choice is made here,
/// by `cfg` since the type differs per platform.
pub fn platform_keyring() -> Arc<dyn SecretStore> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(KeychainSecretStore::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(KeyringSecretStore::new())
    }
}

/// Tolerates a cold D-Bus activation but makes a hung Secret portal a visible error in seconds.
const KEYRING_TIMEOUT: Duration = Duration::from_secs(10);

async fn with_timeout<T>(
    key: &AccountKey,
    fut: impl Future<Output = Result<T, SecretError>>,
) -> Result<T, SecretError> {
    tokio::time::timeout(KEYRING_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| {
            Err(SecretError::Timeout {
                account: key.account().to_owned(),
                seconds: KEYRING_TIMEOUT.as_secs(),
            })
        })
}

/// A separate implementation of the trait, not a branch: which keyring is used is a choice about
/// the installation (ADR 0019).
#[cfg(target_os = "macos")]
pub mod keychain;
#[cfg(target_os = "macos")]
pub use keychain::KeychainSecretStore;

#[derive(Clone, Debug, Default)]
pub struct KeyringSecretStore {
    _private: (),
}

impl KeyringSecretStore {
    /// Opened lazily so constructing an account never blocks on D-Bus.
    pub fn new() -> Self {
        Self::default()
    }

    /// What `oo7::Keyring::new()` does for an unsandboxed process, lifted out so a sandboxed one
    /// can ask too. Boxed because `oo7::dbus::Error` is large.
    async fn secret_service() -> Result<oo7::Keyring, Box<oo7::dbus::Error>> {
        let service = oo7::dbus::Service::new().await.map_err(Box::new)?;
        let collection = service.default_collection().await.map_err(Box::new)?;
        Ok(oo7::Keyring::DBus(collection))
    }

    /// `oo7::Keyring::new()` takes the file backend when sandboxed, and under Flatpak the portal's
    /// `RetrieveSecret` never gets a `Response`, so every read times out.
    /// Asks the Secret Service directly (the manifest already allows `org.freedesktop.secrets`);
    /// the portal path is the fallback.
    async fn keyring(&self, key: &AccountKey) -> Result<oo7::Keyring, SecretError> {
        let keyring = match Self::secret_service().await {
            Ok(keyring) => keyring,
            Err(absent) => {
                tracing::debug!(
                    %absent,
                    "no Secret Service on the bus; falling back to oo7's own choice of backend"
                );
                oo7::Keyring::new()
                    .await
                    .map_err(|err| map_oo7_error(key, err))?
            }
        };

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
        with_timeout(key, async {
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
        })
        .await
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        with_timeout(key, async {
            let keyring = self.keyring(key).await?;
            let items = keyring
                .search_items(&key.attributes())
                .await
                .map_err(|err| map_oo7_error(key, err))?;

            let item = items.first().ok_or_else(|| SecretError::NotFound {
                account: key.account().to_owned(),
            })?;

            let secret = item.secret().await.map_err(|err| map_oo7_error(key, err))?;
            // Wrapped before anything else happens to it: `String::from_utf8` dropped an
            // un-zeroized copy on failure.
            let bytes = Zeroizing::new(secret.as_bytes().to_vec());
            let text = secret_text(&bytes).ok_or_else(|| SecretError::Backend {
                account: key.account().to_owned(),
                reason: "the stored secret is not valid UTF-8".to_owned(),
            })?;

            // Not split at a newline, unlike the command store: whatever the
            // keyring holds is the password, including any byte in it.
            Ok(Password::new(text))
        })
        .await
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        with_timeout(key, async {
            let keyring = self.keyring(key).await?;
            keyring
                .delete(&key.attributes())
                .await
                .map_err(|err| map_oo7_error(key, err))
        })
        .await
    }
}

/// A dismissed prompt and an `IsLocked` error both mean the keyring did not open, and must produce
/// unlock instructions rather than a D-Bus trace.
fn map_oo7_error(key: &AccountKey, err: oo7::Error) -> SecretError {
    let account = key.account().to_owned();

    let locked = |keyring: &str| SecretError::Locked {
        keyring: keyring.to_owned(),
        account: account.clone(),
    };

    match err {
        oo7::Error::File(error) if matches!(*error, oo7::file::Error::Locked) => locked("file"),
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

/// Read-only: Postio will not guess how to write into someone else's secret manager.
#[derive(Clone, Debug)]
pub struct CommandSecretStore {
    argv: Vec<String>,
}

impl CommandSecretStore {
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

        // Wrapped the moment it is ours, before the UTF-8 check and the split, which used to leak
        // un-zeroized copies.
        let stdout = Zeroizing::new(output.stdout);
        let text = secret_text(&stdout)
            .ok_or_else(|| self.failure("the command printed something that is not valid UTF-8"))?;

        let password = first_line(text);
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

/// Public so crates above can test credential paths without a Secret Service. It holds `Password`,
/// not `String`, so replaced or dropped entries are zeroized (#144).
#[derive(Clone, Debug, Default)]
pub struct MemorySecretStore {
    items: Arc<Mutex<HashMap<AccountKey, Password>>>,
    locked: bool,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn locked() -> Self {
        Self {
            items: Arc::new(Mutex::new(HashMap::new())),
            locked: true,
        }
    }

    /// Equivalent of restarting Postio and opening the same keyring again.
    pub fn reopen(&self) -> Self {
        self.clone()
    }

    pub fn len(&self) -> usize {
        self.items.lock().expect("secret store mutex").len()
    }

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
            .insert(key.clone(), password.clone());
        Ok(())
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        self.guard(key)?;
        self.items
            .lock()
            .expect("secret store mutex")
            .get(key)
            .cloned()
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

    // `secret_text` accepts only `&Zeroizing<Vec<u8>>` and borrows, so a bare buffer must be
    // wrapped and no second allocation exists to leak (#144).

    #[test]
    fn a_secret_buffer_reads_back_as_text_without_a_second_allocation() {
        let bytes = Zeroizing::new(b"hunter2".to_vec());
        let text = secret_text(&bytes).expect("valid UTF-8");
        assert_eq!(text, "hunter2");
        // Borrowed from the zeroized buffer itself, not copied out of it.
        assert_eq!(text.as_ptr(), bytes.as_ptr());
    }

    #[test]
    fn a_secret_buffer_that_is_not_utf8_yields_nothing_to_copy() {
        let bytes = Zeroizing::new(vec![0xff, 0xfe, 0xfd]);
        assert!(secret_text(&bytes).is_none());
    }

    #[test]
    fn only_the_first_line_of_a_command_is_the_password() {
        assert_eq!(first_line("hunter2\n"), "hunter2");
        assert_eq!(first_line("hunter2\r\n"), "hunter2");
        assert_eq!(first_line("hunter2\nlogin: ada@example.com\n"), "hunter2");
        assert_eq!(first_line("hunter2"), "hunter2");
        assert_eq!(first_line("\n"), "");
        assert_eq!(first_line(""), "");
        assert_eq!(first_line(" hunter 2 \n"), " hunter 2 ");
    }

    #[tokio::test]
    async fn a_command_that_prints_a_password_yields_it() {
        let store = CommandSecretStore::new(["/bin/sh", "-c", "printf 'hunter2\\n'"]);
        let password = store
            .retrieve(&AccountKey::new("ada@example.com"))
            .await
            .expect("the command printed a password");
        assert_eq!(password.expose(), "hunter2");
    }

    #[tokio::test]
    async fn a_command_that_prints_something_that_is_not_utf8_is_an_error() {
        // The invalid-UTF-8 path: before #144 the buffer was dropped here unzeroized.
        let store = CommandSecretStore::new(["/bin/sh", "-c", "printf '\\377\\376'"]);
        let error = store
            .retrieve(&AccountKey::new("ada@example.com"))
            .await
            .expect_err("not valid UTF-8");
        assert!(error.to_string().contains("UTF-8"), "{error}");
    }

    #[tokio::test]
    async fn a_command_that_prints_nothing_is_an_error() {
        let store = CommandSecretStore::new(["/bin/sh", "-c", "printf ''"]);
        let error = store
            .retrieve(&AccountKey::new("ada@example.com"))
            .await
            .expect_err("no password");
        assert!(error.to_string().contains("nothing"), "{error}");
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

    /// Needs a Secret Service session, so `#[ignore]`d. A test binary is never sandboxed, so it
    /// does not prove the Flatpak choice; it guards the fallback staying usable.
    #[tokio::test]
    #[ignore = "needs a Secret Service session on the bus"]
    async fn a_password_survives_a_round_trip_through_the_secret_service() {
        let key = AccountKey::new("ada@example.com");
        let store = KeyringSecretStore::new();
        let password = Password::new("hunter2");

        store.store(&key, &password).await.expect("a store");
        let read = store.retrieve(&key).await.expect("a retrieve");
        assert_eq!(read.expose(), password.expose());

        store.delete(&key).await.expect("a delete");
        assert!(
            matches!(
                store.retrieve(&key).await,
                Err(SecretError::NotFound { .. })
            ),
            "the deleted password is still readable"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_keyring_call_that_never_resolves_times_out() {
        let key = AccountKey::new("ada@example.com");
        let never = std::future::pending::<Result<(), SecretError>>();

        let result = with_timeout(&key, never).await;

        assert!(matches!(
            result,
            Err(SecretError::Timeout { account, seconds })
                if account == "ada@example.com" && seconds == KEYRING_TIMEOUT.as_secs()
        ));
    }

    #[tokio::test]
    async fn with_timeout_passes_through_a_fast_result() {
        let key = AccountKey::new("ada@example.com");

        let result = with_timeout(&key, async { Ok::<_, SecretError>(42) }).await;

        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn with_timeout_passes_through_a_fast_error() {
        let key = AccountKey::new("ada@example.com");

        let result = with_timeout(&key, async {
            Err::<(), _>(SecretError::NotFound {
                account: "ada@example.com".to_owned(),
            })
        })
        .await;

        assert!(matches!(result, Err(SecretError::NotFound { .. })));
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
