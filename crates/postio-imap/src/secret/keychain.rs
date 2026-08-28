//! A [`SecretStore`](super::SecretStore) over the macOS Keychain.
//!
//! The macOS answer to what `KeyringSecretStore` does on freedesktop. Not an
//! optional nicety: ADR 0014 keeps the local store's own encryption key in the
//! OS keyring with no plaintext fallback, so on a machine with no Secret
//! Service this is what makes the mail open at all.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use security_framework::os::macos::passwords::find_generic_password;

use super::{AccountKey, Password, SecretError, SecretStore};

/// `errSecItemNotFound` — nothing is stored under that name.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// `errSecInteractionNotAllowed` — the keychain is locked and cannot ask.
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
/// `errSecAuthFailed` — the user declined, or the keychain refused to unlock.
const ERR_SEC_AUTH_FAILED: i32 = -25293;

/// The service name every Postio item is filed under.
///
/// The Keychain's own grouping, and the analogue of the Secret Service schema
/// attribute: it is what makes "Postio's passwords" a set a person can find in
/// Keychain Access, and what keeps two applications that both store a password
/// for `ada@example.com` out of each other's items.
const SERVICE: &str = "dev.postio.Postio";

/// Keychain-backed store. The default on macOS.
///
/// Holds a *path* rather than an open `SecKeychain`, so the store stays
/// `Send + Sync` without a lock: the handle is a Core Foundation object, and
/// opening one per call costs a file open against work that already blocks on
/// the Keychain itself.
#[derive(Clone, Debug, Default)]
pub struct KeychainSecretStore {
    /// The keychain to use, or the user's default when `None`.
    ///
    /// Only tests set this. A test against the login keychain would prompt on
    /// a developer's machine and hang every headless run.
    keychain: Option<Scratch>,
}

/// A keychain file the caller owns, and the password that unlocks it.
#[derive(Clone, Debug)]
struct Scratch {
    path: PathBuf,
    password: String,
}

impl KeychainSecretStore {
    /// A store against the user's default keychain.
    pub fn new() -> Self {
        Self::default()
    }

    /// A store against a keychain at `path`, creating it if it is not there.
    ///
    /// For tests. The password is kept so each call can unlock the keychain
    /// again: a keychain is locked when it is reopened, and a test that
    /// prompted would never finish.
    pub fn at(path: &Path, password: &str) -> Result<Self, SecretError> {
        if !path.exists() {
            CreateOptions::new()
                .password(password)
                .create(path)
                .map_err(|error| SecretError::Backend {
                    account: SERVICE.to_owned(),
                    reason: format!("cannot create a keychain at {}: {error}", path.display()),
                })?;
        }
        Ok(Self {
            keychain: Some(Scratch {
                path: path.to_path_buf(),
                password: password.to_owned(),
            }),
        })
    }

    /// The keychains to search, or `None` for the user's default.
    fn keychains(&self) -> Result<Option<Vec<SecKeychain>>, SecretError> {
        let Some(scratch) = &self.keychain else {
            return Ok(None);
        };
        let mut keychain =
            SecKeychain::open(&scratch.path).map_err(|error| SecretError::Backend {
                account: SERVICE.to_owned(),
                reason: format!("cannot open {}: {error}", scratch.path.display()),
            })?;
        keychain
            .unlock(Some(&scratch.password))
            .map_err(|error| SecretError::Backend {
                account: SERVICE.to_owned(),
                reason: format!("cannot unlock {}: {error}", scratch.path.display()),
            })?;
        Ok(Some(vec![keychain]))
    }
}

/// What a Keychain status code means to the caller.
///
/// The two that are routed on rather than merely reported:
///
/// * **not found** is a *first run*, not a failure. `store_key` mints a key
///   for it and for nothing else — minting on a transient failure would
///   encrypt the next write under a key the existing store knows nothing
///   about, and the mailbox would be gone rather than unavailable.
/// * **locked** is recoverable by the user, and ADR 0014 requires it to reach
///   the surface that asks them to unlock rather than onboarding, which would
///   ask them to set up an account they already have.
fn map_error(key: &AccountKey, error: &security_framework::base::Error) -> SecretError {
    match error.code() {
        ERR_SEC_ITEM_NOT_FOUND => SecretError::NotFound {
            account: key.account().to_owned(),
        },
        ERR_SEC_INTERACTION_NOT_ALLOWED | ERR_SEC_AUTH_FAILED => SecretError::Locked {
            keyring: "login".to_owned(),
            account: key.account().to_owned(),
        },
        _ => SecretError::Backend {
            account: key.account().to_owned(),
            reason: error.to_string(),
        },
    }
}

#[async_trait]
impl SecretStore for KeychainSecretStore {
    fn describe(&self) -> &'static str {
        "keychain"
    }

    async fn store(&self, key: &AccountKey, password: &Password) -> Result<(), SecretError> {
        let this = self.clone();
        let key = key.clone();
        let secret = password.expose().to_owned();
        blocking(move || {
            let keychains = this.keychains()?;
            let account = key.account().to_owned();
            // Replace rather than add: the trait says "stores (or replaces)",
            // and a keychain holding two items for one account answers with
            // whichever it finds first — a password that silently stops
            // changing when the user updates it.
            if let Ok((_, mut item)) =
                find_generic_password(keychains.as_deref(), SERVICE, &account)
            {
                return item
                    .set_password(secret.as_bytes())
                    .map_err(|error| map_error(&key, &error));
            }
            let keychain = match keychains {
                Some(mut found) => found.remove(0),
                None => SecKeychain::default().map_err(|error| map_error(&key, &error))?,
            };
            keychain
                .set_generic_password(SERVICE, &account, secret.as_bytes())
                .map_err(|error| map_error(&key, &error))
        })
        .await
    }

    async fn retrieve(&self, key: &AccountKey) -> Result<Password, SecretError> {
        let this = self.clone();
        let key = key.clone();
        blocking(move || {
            let keychains = this.keychains()?;
            let (found, _item) =
                find_generic_password(keychains.as_deref(), SERVICE, key.account())
                    .map_err(|error| map_error(&key, &error))?;
            let text = std::str::from_utf8(&found).map_err(|error| SecretError::Backend {
                account: key.account().to_owned(),
                reason: format!("the stored secret is not text: {error}"),
            })?;
            Ok(Password::new(text))
        })
        .await
    }

    async fn delete(&self, key: &AccountKey) -> Result<(), SecretError> {
        let this = self.clone();
        let key = key.clone();
        blocking(move || {
            let keychains = this.keychains()?;
            match find_generic_password(keychains.as_deref(), SERVICE, key.account()) {
                Ok((_, item)) => {
                    item.delete();
                    Ok(())
                }
                // Removing an absent password succeeds, as the trait requires.
                Err(error) if error.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                Err(error) => Err(map_error(&key, &error)),
            }
        })
        .await
    }
}

/// Run a blocking Keychain call off the caller's thread.
///
/// `SecItem` is synchronous and can wait on a user prompt — the same reason
/// `KeyringSecretStore` bounds its own round trips rather than letting them
/// run wherever the caller happened to be.
async fn blocking<T, F>(work: F) -> Result<T, SecretError>
where
    F: FnOnce() -> Result<T, SecretError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(error) => Err(SecretError::Backend {
            account: SERVICE.to_owned(),
            reason: format!("the keychain task did not finish: {error}"),
        }),
    }
}
